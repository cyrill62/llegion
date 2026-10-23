//! Minimal blocking client for the Ollama REST API.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Connection;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ModelDetails {
    pub format: String,
    pub family: String,
    pub parameter_size: String,
    pub quantization_level: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalModel {
    pub name: String,
    #[serde(default)]
    pub modified_at: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub details: ModelDetails,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunningModel {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub size_vram: u64,
    #[serde(default)]
    pub expires_at: String,
    #[serde(default)]
    pub context_length: Option<u64>,
}

impl RunningModel {
    /// Same wording as `ollama ps`: "100% GPU", "48%/52% CPU/GPU".
    pub fn processor(&self) -> String {
        if self.size == 0 {
            return "-".into();
        }
        let gpu = (self.size_vram as f64 / self.size as f64 * 100.0).round() as u32;
        match gpu {
            0 => "100% CPU".into(),
            100.. => "100% GPU".into(),
            g => format!("{}%/{}% CPU/GPU", 100 - g, g),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ShowResponse {
    pub license: String,
    pub modelfile: String,
    pub parameters: String,
    pub template: String,
    pub details: ModelDetails,
    pub model_info: serde_json::Map<String, Value>,
    pub capabilities: Vec<String>,
}

impl ShowResponse {
    pub fn native_context_length(&self) -> Option<u64> {
        self.model_info
            .iter()
            .find(|(k, _)| k.ends_with(".context_length"))
            .and_then(|(_, v)| v.as_u64())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct GenerateStats {
    pub total_duration: u64,
    pub load_duration: u64,
    pub prompt_eval_count: u64,
    pub prompt_eval_duration: u64,
    pub eval_count: u64,
    pub eval_duration: u64,
}

impl GenerateStats {
    pub fn prompt_tps(&self) -> f64 {
        rate(self.prompt_eval_count, self.prompt_eval_duration)
    }
    pub fn gen_tps(&self) -> f64 {
        rate(self.eval_count, self.eval_duration)
    }
}

fn rate(count: u64, nanos: u64) -> f64 {
    if nanos == 0 {
        0.0
    } else {
        count as f64 / (nanos as f64 / 1e9)
    }
}

/// One message of a `/api/chat` conversation.
#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

/// One line of a streamed `/api/chat` response.
#[derive(Debug, Default, PartialEq)]
pub struct ChatChunk {
    pub content: String,
    pub thinking: String,
    /// Timing counters, sent with the last line only.
    pub done: Option<GenerateStats>,
}

pub fn parse_chat_line(line: &str) -> Result<ChatChunk> {
    let v: Value = serde_json::from_str(line)?;
    if let Some(err) = v["error"].as_str() {
        bail!("{err}");
    }
    let text = |key: &str| v["message"][key].as_str().unwrap_or_default().to_string();
    Ok(ChatChunk {
        content: text("content"),
        thinking: text("thinking"),
        done: if v["done"].as_bool() == Some(true) {
            Some(serde_json::from_value(v)?)
        } else {
            None
        },
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PullEvent {
    pub status: String,
    pub digest: String,
    pub total: u64,
    pub completed: u64,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct Client {
    base: String,
    token: String,
    agent: ureq::Agent,
    long_agent: ureq::Agent,
}

impl Client {
    pub fn new(conn: &Connection) -> Self {
        let build = |timeout: Option<Duration>| -> ureq::Agent {
            #[allow(unused_mut)]
            let mut cfg = ureq::Agent::config_builder()
                .timeout_connect(Some(Duration::from_secs(3)))
                .timeout_global(timeout)
                .http_status_as_error(false);
            #[cfg(feature = "rustls")]
            if conn.accept_invalid_certs {
                cfg = cfg.tls_config(
                    ureq::tls::TlsConfig::builder()
                        .disable_verification(true)
                        .build(),
                );
            }
            cfg.build().into()
        };
        Self {
            base: conn.base_url.trim_end_matches('/').to_string(),
            token: conn.api_token.trim().to_string(),
            agent: build(Some(Duration::from_secs(5))),
            long_agent: build(None),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn check(resp: &mut ureq::http::Response<ureq::Body>) -> Result<()> {
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        let msg = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or(body);
        match status.as_u16() {
            401 => bail!("401 Unauthorized — check the API token"),
            403 => bail!("403 Forbidden — {msg}"),
            code => bail!("HTTP {code}: {msg}"),
        }
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let mut req = self.agent.get(self.url(path));
        if !self.token.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.token));
        }
        let mut resp = req.call().with_context(|| format!("GET {path}"))?;
        Self::check(&mut resp)?;
        Ok(resp.body_mut().read_json()?)
    }

    fn post(
        &self,
        agent: &ureq::Agent,
        path: &str,
        body: &Value,
    ) -> Result<ureq::http::Response<ureq::Body>> {
        let mut req = agent.post(self.url(path));
        if !self.token.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.token));
        }
        let mut resp = req
            .send_json(body)
            .with_context(|| format!("POST {path}"))?;
        Self::check(&mut resp)?;
        Ok(resp)
    }

    pub fn version(&self) -> Result<String> {
        let v: Value = self.get("/api/version")?;
        Ok(v["version"].as_str().unwrap_or("?").to_string())
    }

    pub fn list_models(&self) -> Result<Vec<LocalModel>> {
        #[derive(Deserialize)]
        struct Tags {
            models: Vec<LocalModel>,
        }
        Ok(self.get::<Tags>("/api/tags")?.models)
    }

    pub fn running_models(&self) -> Result<Vec<RunningModel>> {
        #[derive(Deserialize)]
        struct Ps {
            models: Vec<RunningModel>,
        }
        Ok(self.get::<Ps>("/api/ps")?.models)
    }

    pub fn show(&self, model: &str) -> Result<ShowResponse> {
        let mut resp = self.post(&self.long_agent, "/api/show", &json!({ "model": model }))?;
        Ok(resp
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_json()?)
    }

    pub fn delete(&self, model: &str) -> Result<()> {
        let mut req = self.agent.delete(self.url("/api/delete"));
        if !self.token.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.token));
        }
        let mut resp = req
            .force_send_body()
            .send_json(json!({ "model": model }))
            .context("DELETE /api/delete")?;
        Self::check(&mut resp)
    }

    /// Loads a model into memory (empty prompt) with an optional context size.
    pub fn load(&self, model: &str, num_ctx: Option<u32>) -> Result<()> {
        let mut body = json!({ "model": model, "prompt": "", "stream": false });
        if let Some(ctx) = num_ctx {
            body["options"] = json!({ "num_ctx": ctx });
        }
        self.post(&self.long_agent, "/api/generate", &body)?;
        Ok(())
    }

    pub fn unload(&self, model: &str) -> Result<()> {
        self.post(
            &self.long_agent,
            "/api/generate",
            &json!({ "model": model, "prompt": "", "keep_alive": 0, "stream": false }),
        )?;
        Ok(())
    }

    pub fn benchmark(
        &self,
        model: &str,
        prompt: &str,
        num_predict: u32,
        num_ctx: Option<u32>,
    ) -> Result<GenerateStats> {
        let mut options = json!({ "num_predict": num_predict, "temperature": 0 });
        if let Some(ctx) = num_ctx {
            options["num_ctx"] = json!(ctx);
        }
        let mut resp = self.post(
            &self.long_agent,
            "/api/generate",
            &json!({ "model": model, "prompt": prompt, "stream": false, "options": options }),
        )?;
        Ok(resp.body_mut().read_json()?)
    }

    /// Streams `/api/chat`, invoking `on_chunk` for every line until it returns false.
    /// Returns the final timing counters, or `None` when cancelled.
    pub fn chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        options: Value,
        mut on_chunk: impl FnMut(&ChatChunk) -> bool,
    ) -> Result<Option<GenerateStats>> {
        let resp = self.post(
            &self.long_agent,
            "/api/chat",
            &json!({ "model": model, "messages": messages, "stream": true, "options": options }),
        )?;
        let reader = BufReader::new(resp.into_body().into_reader());
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let chunk = parse_chat_line(&line)?;
            if !on_chunk(&chunk) {
                // Dropping the connection makes Ollama stop generating.
                return Ok(None);
            }
            if chunk.done.is_some() {
                return Ok(chunk.done);
            }
        }
        bail!("stream ended before the reply was complete")
    }

    /// Streams `/api/pull`, invoking `on_event` for every progress line.
    pub fn pull(&self, model: &str, mut on_event: impl FnMut(&PullEvent) -> bool) -> Result<()> {
        let resp = self.post(
            &self.long_agent,
            "/api/pull",
            &json!({ "model": model, "stream": true }),
        )?;
        let reader = BufReader::new(resp.into_body().into_reader());
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let ev: PullEvent = serde_json::from_str(&line)?;
            if let Some(err) = &ev.error {
                bail!("{err}");
            }
            if !on_event(&ev) {
                bail!("cancelled");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_stream_lines() {
        let c = parse_chat_line(r#"{"message":{"role":"assistant","content":"Hel"},"done":false}"#)
            .unwrap();
        assert_eq!(c.content, "Hel");
        assert!(c.done.is_none());

        let c = parse_chat_line(
            r#"{"message":{"role":"assistant","content":"","thinking":"hmm"},"done":false}"#,
        )
        .unwrap();
        assert_eq!(c.thinking, "hmm");

        let c = parse_chat_line(
            r#"{"message":{"role":"assistant","content":""},"done":true,"eval_count":10,"eval_duration":500000000}"#,
        )
        .unwrap();
        assert_eq!(c.done.unwrap().gen_tps(), 20.0);

        let err = parse_chat_line(r#"{"error":"model not found"}"#).unwrap_err();
        assert_eq!(err.to_string(), "model not found");
    }
}
