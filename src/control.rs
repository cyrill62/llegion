//! UI-independent core shared by both front-ends: settings in effect, the backend
//! poller, the log buffer and the supervised `ollama serve` / `ferron` processes.

use std::time::{Duration, Instant};

use crate::backend::Backend;
use crate::config::{LogSource, Settings};
use crate::ferron;
use crate::logs::{Follower, LogBuffer, Origin, follow_file, follow_journal};
use crate::ollama::Client;
use crate::process::Managed;
use crate::util::Repaint;

pub struct Control {
    /// Settings currently in effect.
    pub settings: Settings,
    pub backend: Backend,
    pub logs: LogBuffer,
    pub ollama_proc: Managed,
    pub ferron_proc: Managed,
    follower: Option<Follower>,
    follower_key: Option<(LogSource, String, String)>,
    /// Start the server once GPU detection finished (device filters depend on it).
    pending_autostart: bool,
    /// Recent automatic restarts, to give up on a crash loop.
    crash_restarts: Vec<Instant>,
}

impl Control {
    /// Creates the core without starting anything; call [`Control::start`] next.
    pub fn new(settings: Settings, repaint: Repaint) -> Self {
        crate::i18n::set_language(&settings.ui.language);
        let logs = LogBuffer::new(settings.logs.max_lines);
        logs.set_repaint(repaint.clone());
        logs.app(format!(
            "Settings: {}",
            crate::config::config_dir().display()
        ));
        let backend = Backend::new(
            repaint,
            Client::new(&settings.connection),
            logs.clone(),
            settings.ui.poll_interval_ms,
            settings.ui.history_seconds,
        );
        Self {
            settings,
            backend,
            logs,
            ollama_proc: Managed::new("ollama serve", Origin::Ollama),
            ferron_proc: Managed::new("ferron", Origin::Ferron),
            follower: None,
            follower_key: None,
            pending_autostart: false,
            crash_restarts: Vec::new(),
        }
    }

    /// Starts following logs and the services configured to start automatically.
    pub fn start(&mut self) {
        self.sync_log_follower();
        self.pending_autostart =
            self.settings.server.manage_server && self.settings.server.auto_start;
        if self.settings.proxy.enabled && self.settings.proxy.auto_start {
            self.start_ferron();
        }
    }

    /// Housekeeping to run on every UI frame: crash detection and deferred autostart.
    pub fn tick(&mut self) {
        if self.ollama_proc.poll(&self.logs) && self.settings.server.restart_on_crash {
            self.restart_after_crash();
        }
        self.ferron_proc.poll(&self.logs);
        if self.pending_autostart && self.backend.shared.lock().unwrap().gpus_detected {
            self.pending_autostart = false;
            self.start_ollama();
        }
    }

    /// Puts `settings` in effect: persists them and reconfigures running components.
    pub fn apply(&mut self, settings: Settings, restart_services: bool) {
        let old = std::mem::replace(&mut self.settings, settings);
        crate::i18n::set_language(&self.settings.ui.language);
        match self.settings.save() {
            Ok(path) => self
                .logs
                .app(format!("Saved settings to {}", path.display())),
            Err(e) => self
                .logs
                .app_error(format!("Failed to save settings: {e:#}")),
        }
        if old.connection != self.settings.connection {
            self.backend
                .set_client(Client::new(&self.settings.connection));
        }
        self.backend.set_timing(
            self.settings.ui.poll_interval_ms,
            self.settings.ui.history_seconds,
        );
        self.logs.set_max(self.settings.logs.max_lines);
        if self.settings.proxy.enabled
            && let Err(e) = ferron::write(&self.settings)
        {
            self.logs
                .app_error(format!("Failed to write Ferron configuration: {e:#}"));
        }
        self.sync_log_follower();
        if restart_services {
            if self.ollama_proc.is_running() || self.settings.server.manage_server {
                self.stop_ollama();
                if self.settings.server.manage_server {
                    self.start_ollama();
                }
            }
            if self.ferron_proc.is_running() {
                self.stop_ferron();
                if self.settings.proxy.enabled {
                    self.start_ferron();
                }
            }
        }
    }

    pub fn sync_log_follower(&mut self) {
        let l = &self.settings.logs;
        let key = (l.source, l.file_path.clone(), l.journal_unit.clone());
        if self.follower_key.as_ref() == Some(&key) {
            return;
        }
        self.follower = match l.source {
            LogSource::Managed => None,
            LogSource::File => Some(follow_file(l.file_path.clone(), self.logs.clone())),
            LogSource::Journal => Some(follow_journal(l.journal_unit.clone(), self.logs.clone())),
        };
        self.follower_key = Some(key);
    }

    // ------------------------------------------------------------------
    // Process control

    pub fn start_ollama(&mut self) {
        let (connected, gpus) = {
            let s = self.backend.shared.lock().unwrap();
            (s.connected, s.gpus.clone())
        };
        if connected && !self.ollama_proc.is_running() {
            self.logs.app_warn(
                "Another Ollama server already answers on the configured address. Quit the Ollama tray app / stop the system service first, or the new server will fail to bind.",
            );
        }
        let env = self.settings.ollama_env(&gpus);
        let path = self.settings.server.ollama_path.clone();
        if let Err(e) = self.ollama_proc.start(&path, &["serve"], &env, &self.logs) {
            self.logs.app_error(format!("{e:#}"));
        }
        self.backend.refresh_now();
    }

    fn restart_after_crash(&mut self) {
        self.crash_restarts
            .retain(|t| t.elapsed() < Duration::from_secs(60));
        if self.crash_restarts.len() >= 3 {
            self.logs.app_error(
                "Ollama crashed 3 times within a minute; not restarting again. Check the logs, then start it manually.",
            );
            return;
        }
        self.crash_restarts.push(Instant::now());
        self.logs.app_warn("Restarting Ollama after a crash");
        self.start_ollama();
    }

    pub fn stop_ollama(&mut self) {
        self.ollama_proc.stop(&self.logs);
        self.backend.refresh_now();
    }

    pub fn restart_ollama(&mut self) {
        self.stop_ollama();
        self.start_ollama();
    }

    pub fn start_ferron(&mut self) {
        match ferron::write(&self.settings) {
            Ok(path) => {
                let path = path.display().to_string();
                let program = self.settings.proxy.ferron_path.clone();
                if let Err(e) =
                    self.ferron_proc
                        .start(&program, &["--config", &path], &[], &self.logs)
                {
                    self.logs.app_error(format!("{e:#}"));
                }
            }
            Err(e) => self
                .logs
                .app_error(format!("Failed to write Ferron configuration: {e:#}")),
        }
    }

    pub fn stop_ferron(&mut self) {
        self.ferron_proc.stop(&self.logs);
    }

    /// Environment of the managed server for benchmark records, `None` for an external one.
    pub fn server_env(&self) -> Option<Vec<(String, String)>> {
        self.ollama_proc.is_running().then(|| {
            self.ollama_proc
                .env
                .iter()
                .filter(|(k, _)| k != "OLLAMA_HOST")
                .cloned()
                .collect()
        })
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        self.backend.shutdown();
        if self.settings.server.stop_on_exit {
            self.stop_ollama();
        }
        self.stop_ferron();
    }
}
