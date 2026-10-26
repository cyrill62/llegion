//! Minimal stand-in for the Ollama REST API, used by the end-to-end test and the
//! screenshot scripts.
//!
//! `cargo run --example mock_ollama -- <port>`

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};

/// Canned answers for the prompt used by the screenshot scripts; anything else is echoed.
const ANSWERS: &[(&str, &str)] = &[
    (
        "llama3.2:3b",
        "It limits how many **tokens** the model can attend to at once:\n\n- your prompt and the earlier conversation\n- the reply it is still writing\n\nAnything older is dropped, so long chats need a larger window, which costs more memory. Set the default with `OLLAMA_CONTEXT_LENGTH`:\n\n```sh\nOLLAMA_CONTEXT_LENGTH=8192 ollama serve\n```",
    ),
    (
        "qwen3:8b",
        "The context window caps the total number of tokens, input plus output, that the model can attend to in a single request. Once a conversation outgrows it, the oldest messages are truncated and the model no longer remembers them.",
    ),
    (
        "gemma3:12b",
        "It bounds the amount of text, measured in tokens, that fits in the model's working memory for one request. A bigger window lets it follow longer conversations, at the cost of more VRAM and slower prompt processing.",
    ),
];

struct State {
    /// Downloaded models in listing order, with their size.
    models: Vec<(String, u64)>,
    running: Vec<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    models: Vec::new(),
    running: Vec::new(),
});

fn details() -> Value {
    json!({"format": "gguf", "family": "llama", "parameter_size": "3.2B", "quantization_level": "Q4_K_M"})
}

fn send(out: &mut TcpStream, code: u16, body: &Value) -> std::io::Result<()> {
    let reason = match code {
        200 => "OK",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let body = body.to_string();
    write!(
        out,
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Starts an NDJSON stream; the body ends when the connection closes.
fn start_stream(out: &mut TcpStream) -> std::io::Result<()> {
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n"
    )
}

fn line(out: &mut TcpStream, v: &Value) -> std::io::Result<()> {
    writeln!(out, "{v}")?;
    out.flush()
}

fn handle(stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let (mut length, mut auth) = (0, false);
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "authorization" => auth = true,
                _ => {}
            }
        }
    }
    let mut raw = vec![0; length];
    reader.read_exact(&mut raw)?;
    let body: Value = serde_json::from_slice(&raw).unwrap_or_else(|_| json!({}));
    let model = body["model"].as_str().unwrap_or_default().to_string();
    eprintln!("[GIN] {method} {path}");
    let mut out = stream;

    match (method.as_str(), path.as_str()) {
        // The proxy must strip the token before forwarding.
        ("GET", _) if auth => send(
            &mut out,
            500,
            &json!({"error": "auth header leaked to upstream"}),
        ),
        ("GET", "/api/version") => send(&mut out, 200, &json!({"version": "0.12.6-mock"})),
        ("GET", "/api/tags") => {
            let state = STATE.lock().unwrap();
            let models: Vec<Value> = state
                .models
                .iter()
                .map(|(name, size)| json!({"name": name, "model": name, "modified_at": "2026-10-01T10:00:00Z", "size": size, "digest": "abc", "details": details()}))
                .collect();
            send(&mut out, 200, &json!({"models": models}))
        }
        ("GET", "/api/ps") => {
            let expires = (chrono::Utc::now() + chrono::Duration::minutes(4)).to_rfc3339();
            let state = STATE.lock().unwrap();
            let models: Vec<Value> = state
                .running
                .iter()
                .filter_map(|name| {
                    let size = state.models.iter().find(|(m, _)| m == name)?.1 + 500_000_000;
                    Some(json!({"name": name, "model": name, "size": size, "size_vram": (size as f64 * 0.7) as u64, "expires_at": expires, "context_length": 8192, "details": details()}))
                })
                .collect();
            send(&mut out, 200, &json!({"models": models}))
        }
        ("DELETE", _) => {
            let mut state = STATE.lock().unwrap();
            state.models.retain(|(m, _)| *m != model);
            state.running.retain(|m| *m != model);
            send(&mut out, 200, &json!({}))
        }
        ("POST", "/api/show") => send(
            &mut out,
            200,
            &json!({"modelfile": "FROM x", "parameters": "temperature 0.7", "template": "{{ .Prompt }}", "details": details(), "model_info": {"llama.context_length": 131072}, "capabilities": ["completion"]}),
        ),
        ("POST", "/api/generate") => {
            let mut state = STATE.lock().unwrap();
            state.running.retain(|m| *m != model);
            if body["keep_alive"] != json!(0) {
                state.running.push(model);
            }
            send(
                &mut out,
                200,
                &json!({"done": true, "total_duration": 3_100_000_000u64, "load_duration": 900_000_000, "prompt_eval_count": 24, "prompt_eval_duration": 120_000_000, "eval_count": 256, "eval_duration": 2_000_000_000}),
            )
        }
        ("POST", "/api/chat") => {
            {
                let mut state = STATE.lock().unwrap();
                if !state.models.iter().any(|(m, _)| *m == model) {
                    drop(state);
                    return send(
                        &mut out,
                        404,
                        &json!({"error": format!("model '{model}' not found")}),
                    );
                }
                if !state.running.contains(&model) {
                    state.running.push(model.clone());
                }
            }
            let question = body["messages"]
                .as_array()
                .and_then(|m| m.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default();
            let answer = if question.contains("context window") {
                ANSWERS
                    .iter()
                    .find(|(m, _)| *m == model)
                    .unwrap_or(&ANSWERS[0])
                    .1
                    .to_string()
            } else {
                format!("You said: {question}")
            };
            let words: Vec<&str> = answer.split(' ').collect();
            start_stream(&mut out)?;
            std::thread::sleep(Duration::from_millis(400));
            for word in &words {
                line(
                    &mut out,
                    &json!({"model": model, "message": {"role": "assistant", "content": format!("{word} ")}, "done": false}),
                )?;
                std::thread::sleep(Duration::from_millis(50));
            }
            line(
                &mut out,
                &json!({"model": model, "message": {"role": "assistant", "content": ""}, "done": true, "total_duration": 1_500_000_000, "load_duration": 0, "prompt_eval_count": 12, "prompt_eval_duration": 60_000_000, "eval_count": words.len(), "eval_duration": words.len() as u64 * 50_000_000}),
            )
        }
        ("POST", "/api/pull") => {
            start_stream(&mut out)?;
            for i in (0..=100).step_by(10) {
                line(
                    &mut out,
                    &json!({"status": "pulling sha256:ff", "digest": "ff", "total": 1000, "completed": i * 10}),
                )?;
                std::thread::sleep(Duration::from_millis(200));
            }
            line(&mut out, &json!({"status": "success"}))?;
            let mut state = STATE.lock().unwrap();
            if !state.models.iter().any(|(m, _)| *m == model) {
                state.models.push((model, 1000));
            }
            Ok(())
        }
        _ => send(&mut out, 404, &json!({"error": "not found"})),
    }
}

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .expect("usage: mock_ollama <port>");
    {
        let mut state = STATE.lock().unwrap();
        state.models = vec![
            ("llama3.2:3b".into(), 2_019_393_189),
            ("qwen3:8b".into(), 5_225_387_923),
            ("gemma3:12b".into(), 8_149_190_253),
        ];
        state.running = vec!["llama3.2:3b".into()];
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind the mock port");
    // The screenshot scripts wait for this line.
    println!("mock Ollama listening on 127.0.0.1:{port}");
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || {
            let _ = handle(stream);
        });
    }
}
