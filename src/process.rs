//! Lifecycle of child processes owned by the app (`ollama serve`, `caddy run`).

use std::process::{Child, Stdio};
use std::time::Instant;

use anyhow::{Context, Result};

use crate::logs::{LogBuffer, Origin, pump};
use crate::util::command;

pub struct Managed {
    name: &'static str,
    origin: Origin,
    child: Option<Child>,
    pub started: Option<Instant>,
}

impl Managed {
    pub fn new(name: &'static str, origin: Origin) -> Self {
        Self {
            name,
            origin,
            child: None,
            started: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    pub fn start(
        &mut self,
        program: &str,
        args: &[&str],
        env: &[(String, String)],
        logs: &LogBuffer,
    ) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }
        let mut cmd = command(program);
        cmd.args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().with_context(|| {
            format!("failed to start `{program}` — is it installed and in PATH?")
        })?;
        if let Some(out) = child.stdout.take() {
            pump(out, self.origin, logs.clone());
        }
        if let Some(err) = child.stderr.take() {
            pump(err, self.origin, logs.clone());
        }
        let env_desc: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        logs.app(format!(
            "Started {} (pid {}): {program} {} [{}]",
            self.name,
            child.id(),
            args.join(" "),
            env_desc.join(" ")
        ));
        self.child = Some(child);
        self.started = Some(Instant::now());
        Ok(())
    }

    pub fn stop(&mut self, logs: &LogBuffer) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            logs.app(format!("Stopped {}", self.name));
        }
        self.started = None;
    }

    /// Detects an unexpected exit; call regularly. Returns true when the process just died.
    pub fn poll(&mut self, logs: &LogBuffer) -> bool {
        if let Some(child) = &mut self.child
            && let Ok(Some(status)) = child.try_wait()
        {
            logs.app_error(format!("{} exited unexpectedly: {status}", self.name));
            self.child = None;
            self.started = None;
            return true;
        }
        false
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::logs::Level;
    use std::time::Duration;

    fn wait_for(logs: &LogBuffer, origin: Origin, needle: &str) -> Vec<crate::logs::LogLine> {
        for _ in 0..50 {
            let lines = logs.snapshot_filtered(|l| l.origin == origin && l.text.contains(needle));
            if !lines.is_empty() {
                return lines;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("log line containing {needle:?} never arrived");
    }

    #[test]
    fn captures_output_env_and_exit() {
        let logs = LogBuffer::new(1000);
        let mut p = Managed::new("test", Origin::Ollama);
        let env = [("OLLAMA_HOST".to_string(), "127.0.0.1:1234".to_string())];
        p.start(
            "sh",
            &[
                "-c",
                "echo host=$OLLAMA_HOST; echo 'level=ERROR msg=boom' >&2; exit 3",
            ],
            &env,
            &logs,
        )
        .unwrap();
        assert!(p.is_running());
        wait_for(&logs, Origin::Ollama, "host=127.0.0.1:1234");
        assert_eq!(
            wait_for(&logs, Origin::Ollama, "msg=boom")[0].level,
            Level::Error
        );

        std::thread::sleep(Duration::from_millis(200));
        p.poll(&logs);
        assert!(!p.is_running(), "exit is detected");
        assert_eq!(
            wait_for(&logs, Origin::App, "exited unexpectedly")[0].level,
            Level::Error
        );
    }

    #[test]
    fn stop_kills_the_child() {
        let logs = LogBuffer::new(1000);
        let mut p = Managed::new("sleeper", Origin::Caddy);
        p.start("sleep", &["30"], &[], &logs).unwrap();
        p.stop(&logs);
        assert!(!p.is_running());
        wait_for(&logs, Origin::App, "Stopped sleeper");
    }

    #[test]
    fn missing_binary_is_an_error() {
        let logs = LogBuffer::new(10);
        let mut p = Managed::new("nope", Origin::Ollama);
        let err = p
            .start("definitely-not-a-binary-xyz", &[], &[], &logs)
            .unwrap_err();
        assert!(format!("{err:#}").contains("is it installed"));
    }
}
