//! Debug-only capture mode for the README screenshots.
//!
//! `LLEGION_SCREENSHOTS=<dir>` starts the app with default settings (never the
//! user's file) against `LLEGION_URL` (default: the mock server on port 11500),
//! fills the Chat and Performance tabs, saves one PNG per tab into `<dir>` and quits.
//! `scripts/screenshots.sh` / `screenshots.ps1` run it against `examples/mock_ollama.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::app::{App, Tab};
use crate::backend::ChatOptions;
use crate::config::Settings;

const PROMPT: &str = "In two sentences, what does the context window of a language model limit?";
/// Long enough for the Performance charts to show a few seconds of history.
const MIN_WARMUP: Duration = Duration::from_secs(8);
const TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Waiting for the server and its model list.
    Connect,
    /// First answer and the benchmark are running.
    FirstReply,
    /// Same prompt answered by a second model, to show the ‹ 1/2 › switcher.
    SecondReply,
    /// Switched to `Tab::ALL[step]`, letting the layout settle.
    Settle(usize),
    /// Screenshot requested, waiting for the PNG to be written.
    Saving(usize),
}

pub struct Capture {
    dir: PathBuf,
    phase: Phase,
    since: Instant,
    started: Instant,
    saved: Arc<AtomicBool>,
}

impl Capture {
    pub fn from_env() -> Option<Self> {
        let dir = std::env::var_os("LLEGION_SCREENSHOTS")?;
        let now = Instant::now();
        Some(Self {
            dir: dir.into(),
            phase: Phase::Connect,
            since: now,
            started: now,
            saved: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Defaults only, so no address, token or path of the user ends up in a picture.
    pub fn settings() -> Settings {
        let mut s = Settings::default();
        s.connection.base_url =
            std::env::var("LLEGION_URL").unwrap_or_else(|_| "http://127.0.0.1:11500".into());
        s.ui.poll_interval_ms = 500;
        // Managed (but not started) so the GPU tab and the server options are enabled.
        s.server.manage_server = true;
        s
    }

    /// Window size for the pictures: tall enough for the Chat tab to show the whole exchange.
    /// Set at creation, as some compositors (WSLg) scale later resize requests.
    pub fn window_size() -> Option<[f32; 2]> {
        std::env::var_os("LLEGION_SCREENSHOTS").map(|_| [1440.0, 1080.0])
    }

    fn goto(&mut self, phase: Phase) {
        self.phase = phase;
        self.since = Instant::now();
    }
}

fn save_png(path: &Path, image: &egui::ColorImage) -> anyhow::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let [w, h] = image.size;
    let mut enc = png::Encoder::new(file, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    enc.write_header()?.write_image_data(image.as_raw())?;
    Ok(())
}

fn file_name(tab: Tab) -> String {
    format!("{tab:?}.png").to_lowercase()
}

impl App {
    /// Advances the capture script by one frame.
    pub fn capture_frame(&mut self, ctx: &egui::Context) {
        let Some(cap) = self.capture.as_mut() else {
            return;
        };
        ctx.request_repaint_after(Duration::from_millis(100));
        if cap.started.elapsed() > TIMEOUT {
            eprintln!("screenshots: timed out in {:?}", cap.phase);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.capture = None;
            return;
        }
        let (connected, models, streaming, bench_running) = {
            let s = self.ctl.backend.shared.lock().unwrap();
            let models: Vec<String> = s.models.iter().map(|m| m.name.clone()).collect();
            (s.connected, models, s.chat.streaming, s.bench_running)
        };
        let opts = ChatOptions::default();
        match cap.phase {
            Phase::Connect => {
                if connected && !models.is_empty() {
                    self.chat_model = models[0].clone();
                    self.bench_model = models[0].clone();
                    self.ctl.backend.chat_send(PROMPT.into(), &models[0], &opts);
                    // As if the app managed the server, so the Server column has content.
                    let env = [
                        ("OLLAMA_FLASH_ATTENTION", "1"),
                        ("OLLAMA_KV_CACHE_TYPE", "q8_0"),
                    ]
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .to_vec();
                    self.ctl.backend.benchmark(
                        &models[0],
                        &self.bench_prompt,
                        self.bench_tokens,
                        None,
                        Some(env),
                    );
                    cap.goto(Phase::FirstReply);
                }
            }
            Phase::FirstReply => {
                if !streaming && !bench_running && cap.since.elapsed() > Duration::from_secs(1) {
                    if let Some(second) = models.get(1) {
                        self.ctl.backend.chat_reply(1, second, &opts);
                    }
                    cap.goto(Phase::SecondReply);
                }
            }
            Phase::SecondReply => {
                if !streaming
                    && cap.since.elapsed() > Duration::from_secs(1)
                    && cap.started.elapsed() > MIN_WARMUP
                {
                    std::fs::create_dir_all(&cap.dir).ok();
                    self.tab = Tab::ALL[0];
                    cap.goto(Phase::Settle(0));
                }
            }
            Phase::Settle(step) => {
                if cap.since.elapsed() > Duration::from_millis(800) {
                    let path = cap.dir.join(file_name(Tab::ALL[step]));
                    let saved = cap.saved.clone();
                    saved.store(false, Ordering::Relaxed);
                    let repaint = ctx.clone();
                    ctx.request_screenshot(move |image| {
                        match save_png(&path, &image) {
                            Ok(()) => eprintln!("screenshots: wrote {}", path.display()),
                            Err(e) => eprintln!("screenshots: {}: {e:#}", path.display()),
                        }
                        saved.store(true, Ordering::Relaxed);
                        repaint.request_repaint();
                    });
                    cap.goto(Phase::Saving(step));
                }
            }
            Phase::Saving(step) => {
                if cap.saved.load(Ordering::Relaxed) {
                    if let Some(&tab) = Tab::ALL.get(step + 1) {
                        self.tab = tab;
                        cap.goto(Phase::Settle(step + 1));
                    } else {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        self.capture = None;
                    }
                }
            }
        }
    }
}
