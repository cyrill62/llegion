//! Log collection: managed process pipes, file tailing and journalctl.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::util::{Repaint, command};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }

    fn detect(line: &str) -> Self {
        let l = line.to_ascii_lowercase();
        if l.contains("level=error")
            || l.contains("\"level\":\"error\"")
            || l.contains("panic")
            || l.contains("error:")
        {
            Level::Error
        } else if l.contains("level=warn")
            || l.contains("\"level\":\"warn\"")
            || l.contains("warning")
        {
            Level::Warn
        } else if l.contains("level=debug")
            || l.contains("level=trace")
            || l.contains("\"level\":\"debug\"")
        {
            Level::Debug
        } else {
            Level::Info
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Ollama,
    Caddy,
    App,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Ollama => "ollama",
            Origin::Caddy => "caddy",
            Origin::App => "app",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub time: String,
    pub origin: Origin,
    pub level: Level,
    pub text: String,
}

#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<LogInner>>,
}

struct LogInner {
    lines: VecDeque<LogLine>,
    max: usize,
    /// Monotonic counter so the UI can tell when new lines arrived.
    generation: u64,
    repaint: Option<Repaint>,
}

impl LogBuffer {
    pub fn new(max: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LogInner {
                lines: VecDeque::new(),
                max,
                generation: 0,
                repaint: None,
            })),
        }
    }

    pub fn set_repaint(&self, repaint: Repaint) {
        self.inner.lock().unwrap().repaint = Some(repaint);
    }

    pub fn set_max(&self, max: usize) {
        self.inner.lock().unwrap().max = max.max(100);
    }

    pub fn push(&self, origin: Origin, text: impl Into<String>) {
        self.push_with(origin, None, text);
    }

    fn push_with(&self, origin: Origin, level: Option<Level>, text: impl Into<String>) {
        let text: String = text.into();
        let text = text.trim_end().to_string();
        if text.is_empty() {
            return;
        }
        let line = LogLine {
            time: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
            origin,
            level: level.unwrap_or_else(|| Level::detect(&text)),
            text,
        };
        let mut inner = self.inner.lock().unwrap();
        inner.lines.push_back(line);
        while inner.lines.len() > inner.max {
            inner.lines.pop_front();
        }
        inner.generation += 1;
        if let Some(repaint) = &inner.repaint {
            repaint(Duration::from_millis(100));
        }
    }

    pub fn app(&self, text: impl Into<String>) {
        self.push_with(Origin::App, Some(Level::Info), text);
    }

    pub fn app_warn(&self, text: impl Into<String>) {
        self.push_with(Origin::App, Some(Level::Warn), text);
    }

    pub fn app_error(&self, text: impl Into<String>) {
        self.push_with(Origin::App, Some(Level::Error), text);
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.lines.clear();
        inner.generation += 1;
    }

    pub fn generation(&self) -> u64 {
        self.inner.lock().unwrap().generation
    }

    pub fn snapshot_filtered(&self, filter: impl Fn(&LogLine) -> bool) -> Vec<LogLine> {
        self.inner
            .lock()
            .unwrap()
            .lines
            .iter()
            .filter(|l| filter(l))
            .cloned()
            .collect()
    }
}

/// Line filter shared by the log views: enabled origins (ollama, caddy, app),
/// minimum level and a case-insensitive substring.
pub fn line_filter(origins: [bool; 3], min: Level, needle: &str) -> impl Fn(&LogLine) -> bool {
    let needle = needle.to_lowercase();
    move |l| {
        let origin_ok = match l.origin {
            Origin::Ollama => origins[0],
            Origin::Caddy => origins[1],
            Origin::App => origins[2],
        };
        origin_ok
            && l.level >= min
            && (needle.is_empty() || l.text.to_lowercase().contains(&needle))
    }
}

/// Pipes a reader into the log buffer on a background thread.
pub fn pump(reader: impl Read + Send + 'static, origin: Origin, logs: LogBuffer) {
    std::thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.split(b'\n') {
            match line {
                Ok(bytes) => logs.push(origin, String::from_utf8_lossy(&bytes)),
                Err(_) => break,
            }
        }
    });
}

/// A background follower for an external log source; stops when dropped.
pub struct Follower {
    stop: Arc<AtomicBool>,
    child: Option<Child>,
}

impl Drop for Follower {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Follows a file like `tail -F`, starting with roughly the last 64 KB.
pub fn follow_file(path: String, logs: LogBuffer) -> Follower {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = stop.clone();
    std::thread::spawn(move || {
        let mut pos: Option<u64> = None;
        let mut carry = String::new();
        let mut warned = false;
        while !stop_t.load(Ordering::Relaxed) {
            match std::fs::File::open(&path) {
                Ok(mut file) => {
                    warned = false;
                    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
                    let start = match pos {
                        None => len.saturating_sub(64 * 1024),
                        Some(p) if p > len => 0, // truncated / rotated
                        Some(p) => p,
                    };
                    if start < len && file.seek(SeekFrom::Start(start)).is_ok() {
                        let mut buf = Vec::new();
                        if file.read_to_end(&mut buf).is_ok() {
                            carry.push_str(&String::from_utf8_lossy(&buf));
                            let skip_first = pos.is_none() && start > 0;
                            let mut parts: Vec<&str> = carry.split('\n').collect();
                            let rest = parts.pop().unwrap_or_default().to_string();
                            for (i, line) in parts.into_iter().enumerate() {
                                if !(skip_first && i == 0) {
                                    logs.push(Origin::Ollama, line);
                                }
                            }
                            carry = rest;
                        }
                    }
                    pos = Some(len);
                }
                Err(e) => {
                    if !warned {
                        logs.app_error(format!("Cannot open log file {path}: {e}"));
                        warned = true;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    });
    Follower { stop, child: None }
}

/// Follows `journalctl -u <unit> -f`.
pub fn follow_journal(unit: String, logs: LogBuffer) -> Follower {
    let stop = Arc::new(AtomicBool::new(false));
    let spawned = command("journalctl")
        .args(["-u", &unit, "-f", "-n", "500", "-o", "cat", "--no-pager"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    match spawned {
        Ok(mut child) => {
            if let Some(out) = child.stdout.take() {
                pump(out, Origin::Ollama, logs.clone());
            }
            if let Some(err) = child.stderr.take() {
                pump(err, Origin::App, logs.clone());
            }
            Follower {
                stop,
                child: Some(child),
            }
        }
        Err(e) => {
            logs.app_error(format!("Cannot run journalctl: {e}"));
            Follower { stop, child: None }
        }
    }
}
