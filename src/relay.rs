//! Loopback relay for the agents started from the Launcher. Neither `ollama launch` nor the
//! agents can send an API token (they send `Bearer ollama`) or accept a self-signed
//! certificate, so they talk to this relay, which forwards to the configured server with the
//! real token. It listens on 127.0.0.1 only and lives as long as the app.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;

use crate::config::Connection;

pub struct Relay {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    /// The connection it relays to, to restart it when the settings change.
    pub conn: Connection,
}

impl Relay {
    /// Whether launched agents need the relay to reach `conn`.
    pub fn needed(conn: &Connection) -> bool {
        !conn.api_token.trim().is_empty() || conn.accept_invalid_certs
    }

    pub fn start(conn: Connection) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let agent = crate::ollama::agent(&conn, None);
        let base = conn.base_url.trim().trim_end_matches('/').to_string();
        let token = conn.api_token.trim().to_string();
        let stopped = stop.clone();
        std::thread::Builder::new()
            .name("relay".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let (agent, base, token) = (agent.clone(), base.clone(), token.clone());
                    std::thread::spawn(move || {
                        if let Err(e) = forward(stream.try_clone(), &agent, &base, &token) {
                            let _ = respond_error(stream, &format!("{e:#}"));
                        }
                    });
                }
            })?;
        Ok(Self { addr, stop, conn })
    }

    /// Base URL to give the agents as `OLLAMA_HOST`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wakes the accept loop up so it sees the flag.
        let _ = TcpStream::connect(self.addr);
    }
}

/// Request headers not passed on: the relay sets its own token, length and connection
/// handling, and ureq decompresses the response itself.
const DROPPED: &[&str] = &[
    "host",
    "authorization",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "accept-encoding",
];

/// Relays one request (one per connection: the reply closes it).
fn forward(
    stream: std::io::Result<TcpStream>,
    agent: &ureq::Agent,
    base: &str,
    token: &str,
) -> anyhow::Result<()> {
    let mut client = stream?;
    let mut reader = BufReader::new(client.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Ok(());
    };
    let url = format!("{base}{path}");
    let mut request = match method {
        "GET" => agent.get(&url).force_send_body(),
        "HEAD" => agent.head(&url).force_send_body(),
        "DELETE" => agent.delete(&url).force_send_body(),
        "OPTIONS" => agent.options(&url).force_send_body(),
        "POST" => agent.post(&url),
        "PUT" => agent.put(&url),
        "PATCH" => agent.patch(&url),
        _ => anyhow::bail!("unsupported method {method}"),
    };
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
        match name.as_str() {
            "content-length" => length = value.parse().context("bad Content-Length")?,
            // shortcut: chunked request bodies are refused; Ollama clients send a length, decode chunks if one doesn't.
            "transfer-encoding" => anyhow::bail!("chunked request bodies are not supported"),
            n if DROPPED.contains(&n) => {}
            _ => request = request.header(name, value),
        }
    }
    if !token.is_empty() {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let response = request.send(&body[..])?;

    let status = response.status();
    write!(
        client,
        "HTTP/1.1 {} {}\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    )?;
    for (name, value) in response.headers() {
        if !matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "connection" | "content-encoding"
        ) {
            client.write_all(name.as_str().as_bytes())?;
            client.write_all(b": ")?;
            client.write_all(value.as_bytes())?;
            client.write_all(b"\r\n")?;
        }
    }
    // The body ends when the connection closes, so streamed replies pass through as they come.
    client.write_all(b"Connection: close\r\n\r\n")?;
    std::io::copy(&mut response.into_body().into_reader(), &mut client)?;
    Ok(())
}

fn respond_error(mut client: TcpStream, message: &str) -> std::io::Result<()> {
    let body = serde_json::json!({ "error": format!("llegion relay: {message}") }).to_string();
    write!(
        client,
        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One-shot upstream that records the request it got and answers `reply`.
    fn upstream(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            request.push_str(&String::from_utf8(body).unwrap());
            stream.write_all(reply.as_bytes()).unwrap();
            request
        });
        (url, handle)
    }

    #[test]
    fn injects_the_token_and_relays_the_reply() {
        let (url, upstream) = upstream(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\n\r\n{\"ok\":true}",
        );
        let relay = Relay::start(Connection {
            base_url: url,
            api_token: "secret".into(),
            accept_invalid_certs: false,
        })
        .unwrap();
        let mut client = TcpStream::connect(relay.addr).unwrap();
        client
            .write_all(b"DELETE /api/delete HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer ollama\r\nContent-Length: 15\r\n\r\n{\"model\":\"m\"}\r\n")
            .unwrap();
        let mut reply = String::new();
        client.read_to_string(&mut reply).unwrap();
        let request = upstream.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("delete /api/delete http/1.1\r\n"));
        assert!(request.contains("authorization: bearer secret\r\n"));
        assert!(!request.contains("bearer ollama"));
        assert!(request.ends_with("{\"model\":\"m\"}\r\n"));
        assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(reply.ends_with("\r\n\r\n{\"ok\":true}"));
        assert!(Relay::needed(&relay.conn));
        assert!(!Relay::needed(&Connection::default()));
    }
}
