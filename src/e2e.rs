//! End-to-end check of the managed server + HTTPS proxy, without the UI.
//!
//! Needs `ferron` in PATH (or `FERRON=/path/to/ferron`): `cargo test e2e -- --ignored`.
//! The Ollama stand-in is `examples/mock_ollama.rs`, which `cargo test` builds.

use std::time::{Duration, Instant};

use crate::config::{ApiToken, Connection, GpuMode, Settings, TlsMode};
use crate::ferron;
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
#[ignore = "needs ferron"]
fn managed_server_behind_token_proxy() {
    // target/<profile>/deps/<test binary> -> target/<profile>/examples/mock_ollama
    let mock = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(|deps| deps.parent())
        .unwrap()
        .join("examples")
        .join("mock_ollama");
    assert!(
        mock.exists(),
        "{} is missing: run `cargo build --example mock_ollama`",
        mock.display()
    );
    let mock = mock.display();
    let mut s = Settings::default();
    s.server.host = "0.0.0.0".into(); // must be overridden to loopback by the proxy
    s.server.port = 11691;
    s.server.context_length = 16384;
    s.gpu.mode = GpuMode::CpuOnly;
    s.proxy.enabled = true;
    s.proxy.tls = TlsMode::Custom;
    s.proxy.port = 18491;
    s.proxy.tokens = vec![ApiToken {
        label: "e2e".into(),
        token: "olk_e2e".into(),
    }];

    let logs = LogBuffer::new(1000);
    let mut ollama = Managed::new("ollama", Origin::Ollama);
    let script =
        format!("env | grep -E '^(OLLAMA|CUDA)_' | sort; exec '{mock}' \"${{OLLAMA_HOST##*:}}\"");
    ollama
        .start("sh", &["-c", &script], &s.ollama_env(&[]), &logs)
        .unwrap();

    let dir = std::env::temp_dir().join(format!("llegion-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Same generator as the self-signed mode, but kept out of the real config dir.
    s.proxy.cert_path = dir.join("e2e.crt").display().to_string();
    s.proxy.key_path = dir.join("e2e.key").display().to_string();
    ferron::write_self_signed(
        vec!["localhost".into()],
        dir.join("e2e.crt").as_path(),
        dir.join("e2e.key").as_path(),
    )
    .unwrap();
    let config = dir.join("ferron.kdl");
    std::fs::write(&config, ferron::generate(&s)).unwrap();
    let ferron_bin = std::env::var("FERRON").unwrap_or_else(|_| "ferron".into());
    let mut proxy = Managed::new("ferron", Origin::Ferron);
    proxy
        .start(
            &ferron_bin,
            &["--config", &config.display().to_string()],
            &[],
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
        base_url: ferron::public_url(&s),
        api_token: token.into(),
        accept_invalid_certs: true,
    };
    let good = Client::new(&conn("olk_e2e"));
    wait_until("proxy up", || good.version().is_ok());
    assert!(
        has(Origin::Ferron, "\"GET /api/version"),
        "ferron logs are captured"
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
