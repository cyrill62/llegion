//! Background polling of the Ollama API / system metrics, plus one-shot tasks.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::gpu::{self, GpuInfo};
use crate::logs::LogBuffer;
use crate::ollama::{ChatMessage, Client, GenerateStats, LocalModel, RunningModel, ShowResponse};

#[derive(Debug, Clone)]
pub struct Sample {
    /// Seconds since app start.
    pub t: f64,
    pub cpu: f32,
    pub ram_used: u64,
    pub ram_total: u64,
    pub ollama_cpu: f32,
    pub ollama_mem: u64,
    pub models_vram: u64,
    pub gpus: Vec<GpuSample>,
}

#[derive(Debug, Clone)]
pub struct GpuSample {
    pub util: Option<f32>,
    pub vram_used: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PullState {
    pub status: String,
    pub completed: u64,
    pub total: u64,
    pub done: bool,
    pub error: Option<String>,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct BenchRecord {
    pub model: String,
    pub when: String,
    pub num_ctx: Option<u32>,
    pub stats: GenerateStats,
}

/// One answer from a model. An assistant turn keeps every regenerated variant.
#[derive(Debug, Clone)]
pub struct Reply {
    pub model: String,
    pub content: String,
    pub thinking: String,
    pub started: Instant,
    /// Time to the first streamed token, including model load and prompt processing.
    pub first_token: Option<Duration>,
    /// Streamed chunks so far, about one per token.
    pub chunks: u32,
    pub stats: Option<GenerateStats>,
    pub error: Option<String>,
    pub done: bool,
    pub stopped: bool,
}

impl Reply {
    fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            content: String::new(),
            thinking: String::new(),
            started: Instant::now(),
            first_token: None,
            chunks: 0,
            stats: None,
            error: None,
            done: false,
            stopped: false,
        }
    }

    /// Generation speed while streaming, estimated from the chunk count.
    pub fn live_tps(&self) -> Option<f64> {
        let first = self.started + self.first_token?;
        let secs = first.elapsed().as_secs_f64();
        (secs > 0.2 && self.chunks > 1).then(|| (self.chunks - 1) as f64 / secs)
    }
}

#[derive(Debug, Clone)]
pub enum ChatEntry {
    User(String),
    Assistant {
        variants: Vec<Reply>,
        selected: usize,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ChatOptions {
    pub system: String,
    pub temperature: Option<f32>,
    pub num_ctx: Option<u32>,
}

#[derive(Default)]
pub struct Chat {
    pub entries: Vec<ChatEntry>,
    pub streaming: bool,
    cancel: Arc<AtomicBool>,
    /// Bumped on every new reply or clear, so a stale worker stops writing.
    run: u64,
}

impl Chat {
    fn reply_mut(&mut self, at: usize, variant: usize) -> Option<&mut Reply> {
        match self.entries.get_mut(at)? {
            ChatEntry::Assistant { variants, .. } => variants.get_mut(variant),
            ChatEntry::User(_) => None,
        }
    }

    /// Messages before `at`, using the selected variant of each answer.
    fn history(&self, at: usize, system: &str) -> Vec<ChatMessage> {
        let mut msgs = Vec::new();
        if !system.trim().is_empty() {
            msgs.push(ChatMessage {
                role: "system",
                content: system.to_string(),
            });
        }
        for e in &self.entries[..at] {
            match e {
                ChatEntry::User(text) => msgs.push(ChatMessage {
                    role: "user",
                    content: text.clone(),
                }),
                ChatEntry::Assistant { variants, selected } => {
                    if let Some(r) = variants.get(*selected)
                        && r.error.is_none()
                        && !r.content.is_empty()
                    {
                        msgs.push(ChatMessage {
                            role: "assistant",
                            content: r.content.clone(),
                        });
                    }
                }
            }
        }
        msgs
    }

    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn clear(&mut self) {
        self.stop();
        self.run += 1;
        self.streaming = false;
        self.entries.clear();
    }
}

#[derive(Debug, Clone)]
pub struct Notice {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

#[derive(Default)]
pub struct Shared {
    pub connected: bool,
    pub version: Option<String>,
    pub error: Option<String>,
    pub models: Vec<LocalModel>,
    pub running: Vec<RunningModel>,
    pub gpus: Vec<GpuInfo>,
    pub gpus_detected: bool,
    pub history: VecDeque<Sample>,
    pub pulls: BTreeMap<String, PullState>,
    pub busy: HashSet<String>,
    pub details: HashMap<String, Result<ShowResponse, String>>,
    pub benchmarks: Vec<BenchRecord>,
    pub bench_running: bool,
    pub chat: Chat,
    pub notices: Vec<Notice>,
    pub last_poll: Option<Instant>,
}

impl Shared {
    fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.notices.push(Notice {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }
}

#[derive(Clone)]
pub struct Backend {
    pub shared: Arc<Mutex<Shared>>,
    client: Arc<Mutex<Client>>,
    ctx: egui::Context,
    logs: LogBuffer,
    stop: Arc<AtomicBool>,
    refresh: Arc<AtomicBool>,
    poll_ms: Arc<AtomicU64>,
    history_secs: Arc<AtomicU64>,
    start: Instant,
}

impl Backend {
    pub fn new(
        ctx: egui::Context,
        client: Client,
        logs: LogBuffer,
        poll_ms: u64,
        history_secs: u64,
    ) -> Self {
        let backend = Self {
            shared: Arc::new(Mutex::new(Shared::default())),
            client: Arc::new(Mutex::new(client)),
            ctx,
            logs,
            stop: Arc::new(AtomicBool::new(false)),
            refresh: Arc::new(AtomicBool::new(true)),
            poll_ms: Arc::new(AtomicU64::new(poll_ms)),
            history_secs: Arc::new(AtomicU64::new(history_secs)),
            start: Instant::now(),
        };
        backend.redetect_gpus();
        let b = backend.clone();
        std::thread::Builder::new()
            .name("poller".into())
            .spawn(move || b.poll_loop())
            .expect("spawn poller");
        backend
    }

    pub fn set_client(&self, client: Client) {
        *self.client.lock().unwrap() = client;
        self.refresh_now();
    }

    pub fn set_timing(&self, poll_ms: u64, history_secs: u64) {
        self.poll_ms.store(poll_ms.max(250), Ordering::Relaxed);
        self.history_secs
            .store(history_secs.max(30), Ordering::Relaxed);
    }

    pub fn refresh_now(&self) {
        self.refresh.store(true, Ordering::Relaxed);
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn elapsed(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn client(&self) -> Client {
        self.client.lock().unwrap().clone()
    }

    fn poll_loop(&self) {
        let mut sys = System::new();
        let mut tick: u64 = 0;
        while !self.stop.load(Ordering::Relaxed) {
            let client = self.client();
            let full = self.refresh.swap(false, Ordering::Relaxed) || tick.is_multiple_of(10);

            let version = client.version();
            let running = version.as_ref().ok().map(|_| client.running_models());
            let models = if full && version.is_ok() {
                Some(client.list_models())
            } else {
                None
            };

            let mut gpus = self.shared.lock().unwrap().gpus.clone();
            gpu::sample(&mut gpus);

            sys.refresh_cpu_usage();
            sys.refresh_memory();
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_cpu().with_memory(),
            );
            let ncpu = sys.cpus().len().max(1) as f32;
            let (mut ollama_cpu, mut ollama_mem) = (0.0f32, 0u64);
            for p in sys.processes().values() {
                if p.name()
                    .to_string_lossy()
                    .to_lowercase()
                    .starts_with("ollama")
                {
                    ollama_cpu += p.cpu_usage() / ncpu;
                    ollama_mem += p.memory();
                }
            }

            {
                let mut s = self.shared.lock().unwrap();
                match version {
                    Ok(v) => {
                        if !s.connected {
                            self.logs.app(format!("Connected to Ollama {v}"));
                        }
                        s.connected = true;
                        s.version = Some(v);
                        s.error = None;
                    }
                    Err(e) => {
                        if s.connected || s.error.is_none() {
                            self.logs.app(format!("Ollama API unreachable: {e:#}"));
                        }
                        s.connected = false;
                        s.error = Some(format!("{e:#}"));
                        s.running.clear();
                    }
                }
                match running {
                    Some(Ok(r)) => s.running = r,
                    Some(Err(e)) => s.error = Some(format!("{e:#}")),
                    None => {}
                }
                match models {
                    Some(Ok(mut m)) => {
                        m.sort_by(|a, b| a.name.cmp(&b.name));
                        s.models = m;
                    }
                    Some(Err(e)) => s.error = Some(format!("{e:#}")),
                    None => {}
                }
                let models_vram = s.running.iter().map(|m| m.size_vram).sum();
                let sample = Sample {
                    t: self.elapsed(),
                    cpu: sys.global_cpu_usage(),
                    ram_used: sys.used_memory(),
                    ram_total: sys.total_memory(),
                    ollama_cpu,
                    ollama_mem,
                    models_vram,
                    gpus: gpus
                        .iter()
                        .map(|g| GpuSample {
                            util: g.utilization,
                            vram_used: g.vram_used,
                        })
                        .collect(),
                };
                // Keep the detected list if a redetect happened meanwhile.
                if s.gpus.len() == gpus.len() {
                    s.gpus = gpus;
                }
                s.history.push_back(sample);
                let keep = self.history_secs.load(Ordering::Relaxed) as f64;
                let now = self.elapsed();
                while s.history.front().is_some_and(|f| now - f.t > keep) {
                    s.history.pop_front();
                }
                s.last_poll = Some(Instant::now());
            }
            self.ctx.request_repaint();

            tick += 1;
            let deadline =
                Instant::now() + Duration::from_millis(self.poll_ms.load(Ordering::Relaxed));
            while Instant::now() < deadline
                && !self.stop.load(Ordering::Relaxed)
                && !self.refresh.load(Ordering::Relaxed)
            {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    /// Runs `f` on a worker thread with the model marked busy.
    fn task(
        &self,
        model: &str,
        label: &'static str,
        f: impl FnOnce(&Client) -> anyhow::Result<String> + Send + 'static,
    ) {
        let me = self.clone();
        let model = model.to_string();
        self.shared.lock().unwrap().busy.insert(model.clone());
        std::thread::spawn(move || {
            let res = f(&me.client());
            let mut s = me.shared.lock().unwrap();
            s.busy.remove(&model);
            match res {
                Ok(msg) => {
                    me.logs.app(&msg);
                    s.notify(msg, false);
                }
                Err(e) => {
                    let msg = format!("{label} {model} failed: {e:#}");
                    me.logs.app_error(&msg);
                    s.notify(msg, true);
                }
            }
            drop(s);
            me.refresh_now();
            me.ctx.request_repaint();
        });
    }

    pub fn load(&self, model: &str, num_ctx: Option<u32>) {
        let m = model.to_string();
        self.task(model, "Load", move |c| {
            c.load(&m, num_ctx)?;
            Ok(format!("Loaded {m}"))
        });
    }

    pub fn unload(&self, model: &str) {
        let m = model.to_string();
        self.task(model, "Unload", move |c| {
            c.unload(&m)?;
            Ok(format!("Unloaded {m}"))
        });
    }

    pub fn delete(&self, model: &str) {
        let m = model.to_string();
        self.task(model, "Delete", move |c| {
            c.delete(&m)?;
            Ok(format!("Deleted {m}"))
        });
    }

    pub fn show(&self, model: &str) {
        let me = self.clone();
        let m = model.to_string();
        std::thread::spawn(move || {
            let res = me.client().show(&m).map_err(|e| format!("{e:#}"));
            me.shared.lock().unwrap().details.insert(m, res);
            me.ctx.request_repaint();
        });
    }

    pub fn pull(&self, model: &str) {
        let model = model.trim().to_string();
        if model.is_empty() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.shared.lock().unwrap().pulls.insert(
            model.clone(),
            PullState {
                status: "starting".into(),
                completed: 0,
                total: 0,
                done: false,
                error: None,
                cancel: cancel.clone(),
            },
        );
        let me = self.clone();
        std::thread::spawn(move || {
            me.logs.app(format!("Pulling {model}"));
            let res = me.client().pull(&model, |ev| {
                if let Some(p) = me.shared.lock().unwrap().pulls.get_mut(&model) {
                    p.status = ev.status.clone();
                    if ev.total > 0 {
                        p.total = ev.total;
                        p.completed = ev.completed;
                    }
                }
                me.ctx.request_repaint();
                !cancel.load(Ordering::Relaxed)
            });
            let mut s = me.shared.lock().unwrap();
            if let Some(p) = s.pulls.get_mut(&model) {
                p.done = true;
                match &res {
                    Ok(()) => p.status = "success".into(),
                    Err(e) => p.error = Some(format!("{e:#}")),
                }
            }
            match res {
                Ok(()) => s.notify(format!("Pulled {model}"), false),
                Err(e) => {
                    me.logs.app_error(format!("Pull {model} failed: {e:#}"));
                    s.notify(format!("Pull {model} failed: {e:#}"), true);
                }
            }
            drop(s);
            me.refresh_now();
        });
    }

    pub fn benchmark(&self, model: &str, prompt: &str, num_predict: u32, num_ctx: Option<u32>) {
        let me = self.clone();
        let (model, prompt) = (model.to_string(), prompt.to_string());
        self.shared.lock().unwrap().bench_running = true;
        std::thread::spawn(move || {
            me.logs
                .app(format!("Benchmarking {model} ({num_predict} tokens)"));
            let res = me.client().benchmark(&model, &prompt, num_predict, num_ctx);
            let mut s = me.shared.lock().unwrap();
            s.bench_running = false;
            match res {
                Ok(stats) => {
                    let msg = format!(
                        "{model}: {:.1} tok/s generation, {:.1} tok/s prompt",
                        stats.gen_tps(),
                        stats.prompt_tps()
                    );
                    me.logs.app(&msg);
                    s.notify(msg, false);
                    s.benchmarks.push(BenchRecord {
                        model,
                        when: chrono::Local::now().format("%H:%M:%S").to_string(),
                        num_ctx,
                        stats,
                    });
                }
                Err(e) => s.notify(format!("Benchmark failed: {e:#}"), true),
            }
            drop(s);
            me.refresh_now();
            me.ctx.request_repaint();
        });
    }

    /// Adds a user message and streams the answer.
    pub fn chat_send(&self, text: String, model: &str, opts: &ChatOptions) {
        let at = {
            let mut s = self.shared.lock().unwrap();
            if s.chat.streaming {
                return;
            }
            s.chat.entries.push(ChatEntry::User(text));
            s.chat.entries.len()
        };
        self.chat_reply(at, model, opts);
    }

    /// Streams an answer at `at`: a new turn at the end, or a new variant of an
    /// existing answer (later messages are dropped, as they followed the old one).
    pub fn chat_reply(&self, at: usize, model: &str, opts: &ChatOptions) {
        let mut s = self.shared.lock().unwrap();
        let chat = &mut s.chat;
        if chat.streaming || at > chat.entries.len() {
            return;
        }
        let variant = if at == chat.entries.len() {
            chat.entries.push(ChatEntry::Assistant {
                variants: vec![Reply::new(model)],
                selected: 0,
            });
            0
        } else if let ChatEntry::Assistant { variants, selected } = &mut chat.entries[at] {
            variants.push(Reply::new(model));
            *selected = variants.len() - 1;
            let variant = *selected;
            chat.entries.truncate(at + 1);
            variant
        } else {
            return;
        };
        let messages = chat.history(at, &opts.system);
        chat.run += 1;
        chat.cancel = Arc::new(AtomicBool::new(false));
        chat.streaming = true;
        let (run, cancel) = (chat.run, chat.cancel.clone());
        drop(s);

        let mut options = serde_json::json!({});
        if let Some(t) = opts.temperature {
            options["temperature"] = t.into();
        }
        if let Some(ctx) = opts.num_ctx {
            options["num_ctx"] = ctx.into();
        }
        let me = self.clone();
        let model = model.to_string();
        std::thread::spawn(move || {
            let res = me.client().chat(&model, &messages, options, |chunk| {
                let mut s = me.shared.lock().unwrap();
                if s.chat.run != run {
                    return false;
                }
                if let Some(r) = s.chat.reply_mut(at, variant) {
                    if r.first_token.is_none()
                        && !(chunk.content.is_empty() && chunk.thinking.is_empty())
                    {
                        r.first_token = Some(r.started.elapsed());
                    }
                    if !(chunk.content.is_empty() && chunk.thinking.is_empty()) {
                        r.chunks += 1;
                    }
                    r.content.push_str(&chunk.content);
                    r.thinking.push_str(&chunk.thinking);
                }
                drop(s);
                me.ctx.request_repaint();
                !cancel.load(Ordering::Relaxed)
            });
            let mut s = me.shared.lock().unwrap();
            if s.chat.run != run {
                return;
            }
            s.chat.streaming = false;
            if let Some(r) = s.chat.reply_mut(at, variant) {
                r.done = true;
                match res {
                    Ok(Some(stats)) => r.stats = Some(stats),
                    Ok(None) => r.stopped = true,
                    Err(e) => {
                        let msg = format!("Chat with {model} failed: {e:#}");
                        me.logs.app_error(&msg);
                        r.error = Some(format!("{e:#}"));
                    }
                }
            }
            drop(s);
            me.refresh_now();
            me.ctx.request_repaint();
        });
    }

    pub fn redetect_gpus(&self) {
        let me = self.clone();
        std::thread::spawn(move || {
            let gpus = gpu::detect();
            me.logs.app(format!("Detected {} GPU(s)", gpus.len()));
            for g in &gpus {
                me.logs.app(format!(
                    "  {} {} — {} — {}",
                    g.vendor.label(),
                    g.name,
                    g.kind.label(),
                    crate::util::format_bytes(g.vram_total)
                ));
            }
            let mut s = me.shared.lock().unwrap();
            s.gpus = gpus;
            s.gpus_detected = true;
            s.history.clear();
            drop(s);
            me.ctx.request_repaint();
        });
    }
}
