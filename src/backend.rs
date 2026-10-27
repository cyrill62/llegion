//! Background polling of the Ollama API / system metrics, plus one-shot tasks.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::chat_store::{ChatConfig, ChatStore, ChatSummary, SavedChat, SavedEntry, SavedReply};
use crate::config;
use crate::gpu::{self, GpuInfo};
use crate::i18n::tr;
use crate::logs::LogBuffer;
use crate::ollama::{ChatMessage, Client, GenerateStats, LocalModel, RunningModel, ShowResponse};
use crate::util::Repaint;

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
    /// Context size requested for the run (`None` = server default).
    pub num_ctx: Option<u32>,
    /// Context size the model was actually loaded with, from `/api/ps` after the run.
    pub loaded_ctx: Option<u64>,
    /// CPU/GPU split after the run, as `ollama ps` shows it.
    pub processor: Option<String>,
    pub server_label: Option<String>,
    /// `ollama serve` environment when the app manages the server; `None` for an external one.
    pub server_env: Option<Vec<(String, String)>>,
    pub stats: GenerateStats,
}

impl BenchRecord {
    /// Compact summary of the server config the run used, e.g. "ctx 32K · FA · kv q8_0".
    pub fn server_summary(&self) -> String {
        if let Some(label) = &self.server_label {
            return label.clone();
        }
        let Some(env) = &self.server_env else {
            return "external".into();
        };
        let get = |k: &str| {
            env.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
        };
        let mut parts = Vec::new();
        if let Some(c) = get("OLLAMA_CONTEXT_LENGTH").and_then(|c| c.parse().ok()) {
            parts.push(format!("ctx {}", crate::util::ctx_short(c)));
        }
        if get("OLLAMA_FLASH_ATTENTION") == Some("1") {
            parts.push("FA".into());
        }
        if let Some(kv) = get("OLLAMA_KV_CACHE_TYPE") {
            parts.push(format!("kv {kv}"));
        }
        if let Some(n) = get("OLLAMA_NUM_PARALLEL") {
            parts.push(format!("par {n}"));
        }
        if get("OLLAMA_VULKAN") == Some("1") {
            parts.push("Vulkan".into());
        }
        if parts.is_empty() {
            "defaults".into()
        } else {
            parts.join(" · ")
        }
    }

    /// Full server environment, one `KEY=VALUE` per line.
    pub fn server_details(&self) -> String {
        match &self.server_env {
            None => "Server not managed by this app: its configuration is unknown.".into(),
            Some(env) if env.is_empty() => "No overrides: server defaults.".into(),
            Some(env) => env
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

fn csv_field(value: impl std::fmt::Display) -> String {
    let value = value.to_string();
    format!("\"{}\"", value.replace('"', "\"\""))
}

struct BenchLock(PathBuf);

impl Drop for BenchLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn benchmark_lock(path: &Path) -> anyhow::Result<BenchLock> {
    let lock_path = path.with_extension("csv.lock");
    // shortcut: a crashed process can leave this lock behind; manual removal or an OS lock is needed then.
    let lock_file = (0..500)
        .find_map(|_| {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(file) => Some(Ok(file)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                }
                Err(e) => Some(Err(e)),
            }
        })
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("timed out waiting for benchmark CSV lock"))?;
    drop(lock_file);
    Ok(BenchLock(lock_path))
}

fn benchmark_path() -> PathBuf {
    crate::config::data_dir().join("benchmarks.csv")
}

fn load_benchmarks() -> anyhow::Result<Vec<BenchRecord>> {
    let path = benchmark_path();
    fs::create_dir_all(path.parent().unwrap())?;
    let _lock = benchmark_lock(&path)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut reader = csv::Reader::from_path(path)?;
    let mut benchmarks = Vec::new();
    for row in reader.records() {
        let row = row?;
        let text = |i: usize| row.get(i).unwrap_or_default();
        let number = |i: usize| -> anyhow::Result<u64> {
            Ok(text(i)
                .parse()
                .with_context(|| format!("invalid CSV field {}", i + 1))?)
        };
        let optional = |i: usize| -> anyhow::Result<Option<u64>> {
            if text(i).is_empty() {
                Ok(None)
            } else {
                Ok(Some(number(i)?))
            }
        };
        let timestamp = text(0);
        let when = chrono::DateTime::parse_from_rfc3339(timestamp)
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|_| timestamp.to_owned());
        benchmarks.push(BenchRecord {
            model: text(1).to_owned(),
            when,
            num_ctx: optional(2)?.map(|value| value as u32),
            loaded_ctx: optional(3)?,
            processor: (!text(4).is_empty()).then(|| text(4).to_owned()),
            server_label: Some(text(5).to_owned()),
            server_env: None,
            stats: GenerateStats {
                total_duration: number(6)?,
                load_duration: number(7)?,
                prompt_eval_count: number(8)?,
                prompt_eval_duration: number(9)?,
                eval_count: number(10)?,
                eval_duration: number(11)?,
            },
        });
    }
    Ok(benchmarks)
}

fn save_benchmark(record: &BenchRecord) -> anyhow::Result<()> {
    let path = benchmark_path();
    fs::create_dir_all(path.parent().unwrap())?;
    let _lock = benchmark_lock(&path)?;

    let new_file = !path.exists();
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if new_file {
        writeln!(
            file,
            "timestamp,model,requested_context,loaded_context,processor,server_config,total_duration_ns,load_duration_ns,prompt_tokens,prompt_duration_ns,generation_tokens,generation_duration_ns,prompt_tokens_per_second,generation_tokens_per_second"
        )?;
    }
    writeln!(
        file,
        "{},{},{},{},{},{},{},{},{},{},{},{},{:.3},{:.3}",
        csv_field(chrono::Local::now().to_rfc3339()),
        csv_field(&record.model),
        record.num_ctx.map(|v| v.to_string()).unwrap_or_default(),
        record.loaded_ctx.map(|v| v.to_string()).unwrap_or_default(),
        csv_field(record.processor.as_deref().unwrap_or("")),
        csv_field(record.server_summary()),
        record.stats.total_duration,
        record.stats.load_duration,
        record.stats.prompt_eval_count,
        record.stats.prompt_eval_duration,
        record.stats.eval_count,
        record.stats.eval_duration,
        record.stats.prompt_tps(),
        record.stats.gen_tps()
    )?;
    Ok(())
}

fn clear_benchmark_csv() -> anyhow::Result<()> {
    let path = benchmark_path();
    fs::create_dir_all(path.parent().unwrap())?;
    let _lock = benchmark_lock(&path)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
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

    /// Speed and token counts: final timings when done, an estimate while streaming.
    pub fn stats_line(&self) -> Option<String> {
        let ttft = self
            .first_token
            .map(|d| format!("first token {:.2}s", d.as_secs_f64()));
        if let Some(st) = &self.stats {
            let mut parts = vec![
                format!("{:.1} tok/s", st.gen_tps()),
                format!("{} tokens", st.eval_count),
            ];
            parts.extend(ttft);
            parts.push(format!(
                "prompt {} tokens at {:.0} tok/s",
                st.prompt_eval_count,
                st.prompt_tps()
            ));
            if st.load_duration > 50_000_000 {
                parts.push(format!("load {:.2}s", st.load_duration as f64 / 1e9));
            }
            parts.push(format!("total {:.2}s", st.total_duration as f64 / 1e9));
            return Some(parts.join(" · "));
        }
        let mut parts = Vec::new();
        if let Some(tps) = self.live_tps() {
            parts.push(format!("~{tps:.1} tok/s"));
        }
        if self.chunks > 0 {
            parts.push(format!("{} tokens", self.chunks));
        }
        parts.extend(ttft);
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// `current` if it is still downloaded, else a model already in memory (so the first
/// answer comes quickly), else the first downloaded one.
pub fn preferred_model(current: &str, models: &[LocalModel], running: &[RunningModel]) -> String {
    if models.iter().any(|m| m.name == current) {
        return current.to_string();
    }
    running
        .iter()
        .find(|r| models.iter().any(|m| m.name == r.name))
        .map(|r| r.name.clone())
        .or_else(|| models.first().map(|m| m.name.clone()))
        .unwrap_or_default()
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
    pub chat_id: String,
    pub chat_title: Option<String>,
    pub chat_group: String,
    pub chat_config: ChatConfig,
    pub saved_chats: Vec<ChatSummary>,
    pub chat_storage_error: Option<String>,
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

fn new_chat_id() -> String {
    let mut bytes = [0; 16];
    if getrandom::fill(&mut bytes).is_err() {
        bytes[..8].copy_from_slice(
            &chrono::Utc::now()
                .timestamp_nanos_opt()
                .unwrap_or_default()
                .to_le_bytes(),
        );
        bytes[8..].copy_from_slice(&(std::process::id() as u64).to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn save_entry(entry: &ChatEntry) -> SavedEntry {
    match entry {
        ChatEntry::User(text) => SavedEntry::User(text.clone()),
        ChatEntry::Assistant { variants, selected } => SavedEntry::Assistant {
            variants: variants
                .iter()
                .map(|reply| SavedReply {
                    model: reply.model.clone(),
                    content: reply.content.clone(),
                    thinking: reply.thinking.clone(),
                    stats: reply.stats.clone(),
                    error: reply.error.clone(),
                    stopped: reply.stopped,
                })
                .collect(),
            selected: *selected,
        },
    }
}

fn load_entry(entry: SavedEntry) -> ChatEntry {
    match entry {
        SavedEntry::User(text) => ChatEntry::User(text),
        SavedEntry::Assistant { variants, selected } => ChatEntry::Assistant {
            selected: selected.min(variants.len().saturating_sub(1)),
            variants: variants
                .into_iter()
                .map(|saved| {
                    let mut reply = Reply::new(&saved.model);
                    reply.content = saved.content;
                    reply.thinking = saved.thinking;
                    reply.stats = saved.stats;
                    reply.error = saved.error;
                    reply.stopped = saved.stopped;
                    reply.done = true;
                    reply
                })
                .collect(),
        },
    }
}

#[derive(Clone)]
pub struct Backend {
    pub shared: Arc<Mutex<Shared>>,
    client: Arc<Mutex<Client>>,
    repaint: Repaint,
    logs: LogBuffer,
    stop: Arc<AtomicBool>,
    refresh: Arc<AtomicBool>,
    poll_ms: Arc<AtomicU64>,
    history_secs: Arc<AtomicU64>,
    start: Instant,
    chat_store: Arc<Mutex<Option<ChatStore>>>,
}

impl Backend {
    pub fn new(
        repaint: Repaint,
        client: Client,
        logs: LogBuffer,
        poll_ms: u64,
        history_secs: u64,
    ) -> Self {
        let mut shared = Shared::default();
        shared.chat_id = new_chat_id();
        shared.benchmarks = match load_benchmarks() {
            Ok(records) => records,
            Err(e) => {
                logs.app_error(format!("Failed to load benchmark CSV: {e:#}"));
                Vec::new()
            }
        };
        let chat_store = match ChatStore::open(config::chat_store_path()) {
            Ok(store) => {
                match store.list() {
                    Ok(chats) => shared.saved_chats = chats,
                    Err(e) => shared.chat_storage_error = Some(format!("{e:#}")),
                }
                Some(store)
            }
            Err(e) => {
                shared.chat_storage_error = Some(format!("{e:#}"));
                None
            }
        };
        let backend = Self {
            shared: Arc::new(Mutex::new(shared)),
            client: Arc::new(Mutex::new(client)),
            repaint,
            logs,
            stop: Arc::new(AtomicBool::new(false)),
            refresh: Arc::new(AtomicBool::new(true)),
            poll_ms: Arc::new(AtomicU64::new(poll_ms)),
            history_secs: Arc::new(AtomicU64::new(history_secs)),
            start: Instant::now(),
            chat_store: Arc::new(Mutex::new(chat_store)),
        };
        // Reopen the latest chat rather than starting (and saving) an empty one.
        let latest = backend
            .shared
            .lock()
            .unwrap()
            .saved_chats
            .first()
            .map(|c| c.id.clone());
        if let Some(id) = latest
            && let Err(e) = backend.load_saved_chat(&id)
        {
            backend
                .logs
                .app_error(format!("Failed to open the latest chat: {e:#}"));
        }
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
        match load_benchmarks() {
            Ok(records) => self.shared.lock().unwrap().benchmarks = records,
            Err(e) => self
                .logs
                .app_error(format!("Failed to reload benchmark CSV: {e:#}")),
        }
        self.refresh.store(true, Ordering::Relaxed);
    }

    pub fn clear_benchmarks(&self) {
        match clear_benchmark_csv() {
            Ok(()) => self.shared.lock().unwrap().benchmarks.clear(),
            Err(e) => self
                .logs
                .app_error(format!("Failed to clear benchmark CSV: {e:#}")),
        }
        (self.repaint)(Duration::ZERO);
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

    pub fn save_current_chat(&self) {
        let chat = {
            let s = self.shared.lock().unwrap();
            let title = s.chat_title.clone().unwrap_or_else(|| {
                s.chat
                    .entries
                    .iter()
                    .find_map(|entry| match entry {
                        ChatEntry::User(text) => {
                            Some(text.trim().chars().take(64).collect::<String>())
                        }
                        _ => None,
                    })
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| "New chat".into())
            });
            SavedChat {
                id: s.chat_id.clone(),
                title,
                updated: chrono::Local::now().to_rfc3339(),
                entries: s.chat.entries.iter().map(save_entry).collect(),
                group: s.chat_group.clone(),
                config: s.chat_config.clone(),
            }
        };
        let (store, shared, repaint) = (
            self.chat_store.clone(),
            self.shared.clone(),
            self.repaint.clone(),
        );
        std::thread::spawn(move || {
            let result = {
                let store = store.lock().unwrap();
                store
                    .as_ref()
                    .context("chat database is unavailable")
                    .and_then(|store| {
                        store.save(&chat)?;
                        store.list()
                    })
            };
            let mut s = shared.lock().unwrap();
            match result {
                Ok(chats) => {
                    s.saved_chats = chats;
                    s.chat_storage_error = None;
                }
                Err(e) => s.chat_storage_error = Some(format!("{e:#}")),
            }
            drop(s);
            repaint(Duration::ZERO);
        });
    }

    pub fn set_chat_config(&self, config: ChatConfig) {
        let changed = {
            let mut s = self.shared.lock().unwrap();
            if s.chat_config == config {
                false
            } else {
                s.chat_config = config;
                true
            }
        };
        if changed {
            self.save_current_chat();
        }
    }

    pub fn set_current_chat_group(&self, group: String) {
        let changed = {
            let mut s = self.shared.lock().unwrap();
            if s.chat_group == group {
                false
            } else {
                s.chat_group = group;
                true
            }
        };
        if changed {
            self.save_current_chat();
        }
    }

    pub fn new_chat(&self) {
        self.save_current_chat();
        let mut s = self.shared.lock().unwrap();
        s.chat.clear();
        s.chat_id = new_chat_id();
        s.chat_title = None;
        s.chat_group.clear();
        s.chat_config = ChatConfig::default();
        drop(s);
        self.save_current_chat();
        (self.repaint)(Duration::ZERO);
    }

    pub fn load_saved_chat(&self, id: &str) -> anyhow::Result<()> {
        let chat = self
            .chat_store
            .lock()
            .unwrap()
            .as_ref()
            .context("chat database is unavailable")?
            .load(id)?;
        let entries = chat.entries.into_iter().map(load_entry).collect();
        let mut s = self.shared.lock().unwrap();
        s.chat.clear();
        s.chat.entries = entries;
        s.chat_id = chat.id;
        s.chat_title = Some(chat.title);
        s.chat_group = chat.group;
        s.chat_config = chat.config;
        drop(s);
        (self.repaint)(Duration::ZERO);
        Ok(())
    }

    pub fn delete_saved_chat(&self, id: &str) -> anyhow::Result<()> {
        let chats = {
            let store = self.chat_store.lock().unwrap();
            let store = store.as_ref().context("chat database is unavailable")?;
            store.delete(id)?;
            store.list()?
        };
        let mut s = self.shared.lock().unwrap();
        s.saved_chats = chats;
        if s.chat_id == id {
            s.chat.clear();
            s.chat_id = new_chat_id();
            s.chat_title = None;
            s.chat_group.clear();
            s.chat_config = ChatConfig::default();
            drop(s);
            self.save_current_chat();
            (self.repaint)(Duration::ZERO);
            return Ok(());
        }
        drop(s);
        (self.repaint)(Duration::ZERO);
        Ok(())
    }

    pub fn rename_saved_chat(&self, id: &str, title: String) -> anyhow::Result<()> {
        let title = title.trim().to_owned();
        anyhow::ensure!(!title.is_empty(), "chat name cannot be empty");
        let chats = {
            let store = self.chat_store.lock().unwrap();
            let store = store.as_ref().context("chat database is unavailable")?;
            let mut chat = store.load(id)?;
            chat.title.clone_from(&title);
            chat.updated = chrono::Local::now().to_rfc3339();
            store.save(&chat)?;
            store.list()?
        };
        let mut s = self.shared.lock().unwrap();
        s.saved_chats = chats;
        if s.chat_id == id {
            s.chat_title = Some(title);
        }
        drop(s);
        (self.repaint)(Duration::ZERO);
        Ok(())
    }

    pub fn set_saved_chat_group(&self, id: &str, group: String) -> anyhow::Result<()> {
        let group = group.trim().to_owned();
        let chats = {
            let store = self.chat_store.lock().unwrap();
            let store = store.as_ref().context("chat database is unavailable")?;
            let mut chat = store.load(id)?;
            chat.group.clone_from(&group);
            chat.updated = chrono::Local::now().to_rfc3339();
            store.save(&chat)?;
            store.list()?
        };
        let mut s = self.shared.lock().unwrap();
        s.saved_chats = chats;
        if s.chat_id == id {
            s.chat_group = group;
        }
        drop(s);
        (self.repaint)(Duration::ZERO);
        Ok(())
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
            (self.repaint)(Duration::ZERO);

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
        // Failure message with {model} and {error}, marked with `tr_key!`.
        failed: &'static str,
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
                    let msg = tr(failed)
                        .replace("{model}", &model)
                        .replace("{error}", &format!("{e:#}"));
                    me.logs.app_error(&msg);
                    s.notify(msg, true);
                }
            }
            drop(s);
            me.refresh_now();
            (me.repaint)(Duration::ZERO);
        });
    }

    pub fn load(&self, model: &str, num_ctx: Option<u32>) {
        let m = model.to_string();
        self.task(
            model,
            tr_key!("Loading {model} failed: {error}"),
            move |c| {
                c.load(&m, num_ctx)?;
                Ok(trf!("Loaded {model}", model = m))
            },
        );
    }

    pub fn unload(&self, model: &str) {
        let m = model.to_string();
        self.task(
            model,
            tr_key!("Unloading {model} failed: {error}"),
            move |c| {
                c.unload(&m)?;
                Ok(trf!("Unloaded {model}", model = m))
            },
        );
    }

    pub fn delete(&self, model: &str) {
        let m = model.to_string();
        self.task(
            model,
            tr_key!("Deleting {model} failed: {error}"),
            move |c| {
                c.delete(&m)?;
                Ok(trf!("Deleted {model}", model = m))
            },
        );
    }

    pub fn show(&self, model: &str) {
        let me = self.clone();
        let m = model.to_string();
        std::thread::spawn(move || {
            let res = me.client().show(&m).map_err(|e| format!("{e:#}"));
            me.shared.lock().unwrap().details.insert(m, res);
            (me.repaint)(Duration::ZERO);
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
                (me.repaint)(Duration::ZERO);
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
                Ok(()) => s.notify(trf!("Pulled {model}", model = model), false),
                Err(e) => {
                    me.logs.app_error(format!("Pull {model} failed: {e:#}"));
                    s.notify(
                        trf!(
                            "Pulling {model} failed: {error}",
                            model = model,
                            error = format!("{e:#}")
                        ),
                        true,
                    );
                }
            }
            drop(s);
            me.refresh_now();
        });
    }

    pub fn benchmark(
        &self,
        model: &str,
        prompt: &str,
        num_predict: u32,
        num_ctx: Option<u32>,
        server_env: Option<Vec<(String, String)>>,
    ) {
        let me = self.clone();
        let (model, prompt) = (model.to_string(), prompt.to_string());
        self.shared.lock().unwrap().bench_running = true;
        std::thread::spawn(move || {
            me.logs
                .app(format!("Benchmarking {model} ({num_predict} tokens)"));
            let client = me.client();
            let res = client.benchmark(&model, &prompt, num_predict, num_ctx);
            // The model stays loaded after the run, so `/api/ps` tells what it really got.
            let loaded = res
                .is_ok()
                .then(|| client.running_models().ok())
                .flatten()
                .and_then(|r| r.into_iter().find(|r| r.name == model));
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
                    let record = BenchRecord {
                        model,
                        when: chrono::Local::now().format("%H:%M:%S").to_string(),
                        num_ctx,
                        loaded_ctx: loaded.as_ref().and_then(|r| r.context_length),
                        processor: loaded.as_ref().map(RunningModel::processor),
                        server_env,
                        server_label: None,
                        stats,
                    };
                    if let Err(e) = save_benchmark(&record) {
                        me.logs
                            .app_error(format!("Failed to save benchmark CSV: {e:#}"));
                    }
                    s.benchmarks.push(record);
                }
                Err(e) => s.notify(
                    trf!("Benchmark failed: {error}", error = format!("{e:#}")),
                    true,
                ),
            }
            drop(s);
            me.refresh_now();
            (me.repaint)(Duration::ZERO);
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
        self.save_current_chat();
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
        self.save_current_chat();

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
                (me.repaint)(Duration::ZERO);
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
            me.save_current_chat();
            me.refresh_now();
            (me.repaint)(Duration::ZERO);
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
            (me.repaint)(Duration::ZERO);
        });
    }
}
