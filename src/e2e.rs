//! End-to-end check of the managed server + HTTPS proxy, without the UI.
//!
//! Needs `python3` and `caddy` in PATH (or `CADDY=/path/to/caddy`):
//! `cargo test e2e -- --ignored`

use std::time::{Duration, Instant};

use crate::caddy;
use crate::config::{ApiToken, Connection, GpuMode, Settings, TlsMode};
use crate::logs::{LogBuffer, Origin};
use crate::ollama::Client;
use crate::process::Managed;

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "needs python3 and caddy"]
fn managed_server_behind_token_proxy() {
    let mock = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mock_ollama.py");
    let mut s = Settings::default();
    s.server.host = "0.0.0.0".into(); // must be overridden to loopback by the proxy
    s.server.port = 11691;
    s.server.context_length = 16384;
    s.gpu.mode = GpuMode::CpuOnly;
    s.proxy.enabled = true;
    s.proxy.tls = TlsMode::Internal;
    s.proxy.port = 18491;
    s.proxy.tokens = vec![ApiToken {
        label: "e2e".into(),
        token: "olk_e2e".into(),
    }];

    let logs = LogBuffer::new(1000);
    let mut ollama = Managed::new("ollama", Origin::Ollama);
    let script = format!(
        "env | grep -E '^(OLLAMA|CUDA)_' | sort; exec python3 -I {mock} \"${{OLLAMA_HOST##*:}}\""
    );
    ollama
        .start("sh", &["-c", &script], &s.ollama_env(&[]), &logs)
        .unwrap();

    let dir = std::env::temp_dir().join(format!("llegion-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let caddyfile = dir.join("Caddyfile");
    std::fs::write(&caddyfile, caddy::generate(&s)).unwrap();
    let caddy_bin = std::env::var("CADDY").unwrap_or_else(|_| "caddy".into());
    let mut proxy = Managed::new("caddy", Origin::Caddy);
    let data_dir = dir.display().to_string();
    proxy
        .start(
            &caddy_bin,
            &[
                "run",
                "--config",
                &caddyfile.display().to_string(),
                "--adapter",
                "caddyfile",
            ],
            &[("XDG_DATA_HOME".into(), data_dir)],
            &logs,
        )
        .unwrap();

    let has = |origin: Origin, needle: &str| {
        !logs
            .snapshot_filtered(|l| l.origin == origin && l.text.contains(needle))
            .is_empty()
    };
    wait_until("server env in logs", || {
        has(Origin::Ollama, "OLLAMA_HOST=127.0.0.1:11691")
    });
    assert!(has(Origin::Ollama, "OLLAMA_CONTEXT_LENGTH=16384"));
    assert!(has(Origin::Ollama, "CUDA_VISIBLE_DEVICES=-1"));

    let conn = |token: &str| Connection {
        base_url: caddy::public_url(&s),
        api_token: token.into(),
        accept_invalid_certs: true,
    };
    let good = Client::new(&conn("olk_e2e"));
    wait_until("proxy up", || good.version().is_ok());
    assert!(
        has(Origin::Caddy, "\"level\":\"info\""),
        "caddy logs are captured"
    );

    let err = Client::new(&conn("wrong")).version().unwrap_err();
    assert!(format!("{err:#}").contains("401"));

    let mut events = 0;
    good.pull("e2e:1b", |_| {
        events += 1;
        true
    })
    .unwrap();
    assert!(events > 5, "pull progress streams through the proxy");
    assert!(
        good.list_models()
            .unwrap()
            .iter()
            .any(|m| m.name == "e2e:1b")
    );
    good.load("e2e:1b", Some(4096)).unwrap();
    assert!(
        good.running_models()
            .unwrap()
            .iter()
            .any(|m| m.name == "e2e:1b")
    );
    let mut text = String::new();
    let msgs = [crate::ollama::ChatMessage {
        role: "user",
        content: "hello there".into(),
    }];
    let stats = good
        .chat("e2e:1b", &msgs, serde_json::json!({}), |c| {
            text.push_str(&c.content);
            true
        })
        .unwrap()
        .expect("chat finished");
    assert_eq!(text.trim(), "You said: hello there");
    assert!(stats.gen_tps() > 0.0, "chat streams through the proxy");
    good.unload("e2e:1b").unwrap();
    good.delete("e2e:1b").unwrap();
    assert!(
        !good
            .list_models()
            .unwrap()
            .iter()
            .any(|m| m.name == "e2e:1b")
    );
    assert!(
        has(Origin::Ollama, "[GIN] POST /api/pull"),
        "server request logs are captured"
    );

    proxy.stop(&logs);
    ollama.stop(&logs);
    assert!(
        Client::new(&Connection {
            base_url: "http://127.0.0.1:11691".into(),
            ..Connection::default()
        })
        .version()
        .is_err()
    );
    let _ = std::fs::remove_dir_all(dir);
}
