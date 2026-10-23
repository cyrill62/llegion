use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::backend::Backend;
use crate::caddy;
use crate::config::{LogSource, Settings};
use crate::logs::{Follower, Level, LogBuffer, LogLine, Origin, follow_file, follow_journal};
use crate::ollama::Client;
use crate::process::Managed;
use crate::views::kit;
use crate::views::settings::SettingsPane;

pub const GREEN: Color32 = Color32::from_rgb(80, 200, 120);
pub const RED: Color32 = Color32::from_rgb(230, 90, 80);
pub const AMBER: Color32 = Color32::from_rgb(235, 175, 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Models,
    Chat,
    Performance,
    Gpu,
    Logs,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Dashboard,
        Tab::Models,
        Tab::Chat,
        Tab::Performance,
        Tab::Gpu,
        Tab::Logs,
        Tab::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Dashboard => ic!(HOME, "Dashboard"),
            Tab::Models => ic!(MODELS, "Models"),
            Tab::Chat => ic!(CHAT, "Chat"),
            Tab::Performance => ic!(CHART, "Performance"),
            Tab::Gpu => ic!(GPU, "GPU"),
            Tab::Logs => ic!(LOGS, "Logs"),
            Tab::Settings => ic!(SETTINGS, "Settings"),
        }
    }
}

pub struct LogView {
    pub filter: String,
    pub min_level: Level,
    pub show_ollama: bool,
    pub show_caddy: bool,
    pub show_app: bool,
    pub autoscroll: bool,
    pub paused: bool,
    pub wrap: bool,
    pub cache: Vec<LogLine>,
    pub cache_key: Option<(u64, String, Level, [bool; 3])>,
}

pub struct App {
    /// Settings currently in effect.
    pub settings: Settings,
    /// Settings being edited in the Settings / GPU tabs.
    pub draft: Settings,
    pub backend: Backend,
    pub logs: LogBuffer,
    pub ollama_proc: Managed,
    pub caddy_proc: Managed,
    follower: Option<Follower>,
    follower_key: Option<(LogSource, String, String)>,
    pub tab: Tab,

    // Models tab
    pub model_filter: String,
    pub pull_name: String,
    pub confirm_delete: Option<String>,
    pub selected_model: Option<String>,
    pub load_ctx: u32,

    // Performance tab
    pub bench_model: String,
    pub bench_prompt: String,
    pub bench_tokens: u32,
    pub bench_ctx: u32,

    // Chat tab
    pub chat_model: String,
    pub chat_input: String,
    pub chat_system: String,
    pub chat_ctx: u32,
    /// `None` keeps the model's own default.
    pub chat_temperature: Option<f32>,

    pub log_view: LogView,

    // Settings tab
    pub reveal_tokens: bool,
    pub new_token_label: String,
    pub proxy_test: Arc<Mutex<Option<(bool, String)>>>,
    pub confirm_reset: bool,
    pub settings_pane: SettingsPane,
    /// Zoom factor currently applied to the context (follows the draft for live preview).
    applied_scale: f32,
    /// Start the server once GPU detection finished (device filters depend on it).
    pending_autostart: bool,
    /// Recent automatic restarts, to give up on a crash loop.
    crash_restarts: Vec<Instant>,
    #[cfg(debug_assertions)]
    pub capture: Option<crate::screenshots::Capture>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(debug_assertions)]
        let capture = crate::screenshots::Capture::from_env();
        #[cfg(debug_assertions)]
        let settings = if capture.is_some() {
            crate::screenshots::Capture::settings()
        } else {
            Settings::load()
        };
        #[cfg(not(debug_assertions))]
        let settings = Settings::load();
        let logs = LogBuffer::new(settings.logs.max_lines);
        logs.set_repaint(cc.egui_ctx.clone());
        logs.app(format!(
            "Settings: {}",
            crate::config::config_dir().display()
        ));
        let backend = Backend::new(
            cc.egui_ctx.clone(),
            Client::new(&settings.connection),
            logs.clone(),
            settings.ui.poll_interval_ms,
            settings.ui.history_seconds,
        );
        egui_material_icons::initialize(&cc.egui_ctx);
        apply_style(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx, settings.ui.dark_mode);

        let mut app = Self {
            draft: settings.clone(),
            settings,
            backend,
            logs,
            ollama_proc: Managed::new("ollama serve", Origin::Ollama),
            caddy_proc: Managed::new("caddy", Origin::Caddy),
            follower: None,
            follower_key: None,
            tab: Tab::Dashboard,
            model_filter: String::new(),
            pull_name: String::new(),
            confirm_delete: None,
            selected_model: None,
            load_ctx: 0,
            bench_model: String::new(),
            bench_prompt: "Write a 300-word story about a lighthouse keeper who discovers a message in a bottle.".into(),
            bench_tokens: 256,
            bench_ctx: 0,
            chat_model: String::new(),
            chat_input: String::new(),
            chat_system: String::new(),
            chat_ctx: 0,
            chat_temperature: None,
            log_view: LogView {
                filter: String::new(),
                min_level: Level::Debug,
                show_ollama: true,
                show_caddy: true,
                show_app: true,
                autoscroll: true,
                paused: false,
                wrap: false,
                cache: Vec::new(),
                cache_key: None,
            },
            reveal_tokens: false,
            new_token_label: String::new(),
            proxy_test: Arc::new(Mutex::new(None)),
            confirm_reset: false,
            settings_pane: SettingsPane::Connection,
            applied_scale: 0.0,
            pending_autostart: false,
            crash_restarts: Vec::new(),
            #[cfg(debug_assertions)]
            capture,
        };
        #[cfg(debug_assertions)]
        if app.capture.is_some() {
            // Drops the "Settings: <config dir>" line, which shows the user's home path.
            app.logs.clear();
        }
        app.sync_log_follower();
        app.pending_autostart = app.settings.server.manage_server && app.settings.server.auto_start;
        if app.settings.proxy.enabled && app.settings.proxy.auto_start {
            app.start_caddy();
        }
        app
    }

    // ------------------------------------------------------------------
    // Settings application

    /// Commits the draft: persists it and reconfigures running components.
    pub fn apply_draft(&mut self, restart_services: bool) {
        let old = std::mem::replace(&mut self.settings, self.draft.clone());
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
            && let Err(e) = caddy::write(&self.settings)
        {
            self.logs
                .app_error(format!("Failed to write Caddyfile: {e:#}"));
        }
        self.sync_log_follower();
        if restart_services {
            if self.ollama_proc.is_running() || self.settings.server.manage_server {
                self.stop_ollama();
                if self.settings.server.manage_server {
                    self.start_ollama();
                }
            }
            if self.caddy_proc.is_running() {
                self.stop_caddy();
                if self.settings.proxy.enabled {
                    self.start_caddy();
                }
            }
        }
    }

    pub fn draft_dirty(&self) -> bool {
        self.draft != self.settings
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

    pub fn start_caddy(&mut self) {
        match caddy::write(&self.settings) {
            Ok(path) => {
                let path = path.display().to_string();
                let program = self.settings.proxy.caddy_path.clone();
                if let Err(e) = self.caddy_proc.start(
                    &program,
                    &["run", "--config", &path, "--adapter", "caddyfile"],
                    &[],
                    &self.logs,
                ) {
                    self.logs.app_error(format!("{e:#}"));
                }
            }
            Err(e) => self
                .logs
                .app_error(format!("Failed to write Caddyfile: {e:#}")),
        }
    }

    pub fn stop_caddy(&mut self) {
        self.caddy_proc.stop(&self.logs);
    }

    // ------------------------------------------------------------------
    // Layout

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let (connected, version, error) = {
            let s = self.backend.shared.lock().unwrap();
            (s.connected, s.version.clone(), s.error.clone())
        };
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(ic!(LLAMA, "Ollama Control"))
                    .size(22.0)
                    .strong(),
            );
            ui.add_space(12.0);
            let r = if connected {
                kit::badge(
                    ui,
                    format!("Connected · v{}", version.unwrap_or_default()),
                    GREEN,
                )
            } else {
                kit::badge(ui, "Disconnected", RED)
            };
            let tip = match error {
                Some(err) => format!("{}\n{err}", self.settings.connection.base_url),
                None => self.settings.connection.base_url.clone(),
            };
            r.on_hover_text(tip);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let theme_icon = if self.settings.ui.dark_mode {
                    icon!(LIGHT)
                } else {
                    icon!(DARK)
                };
                if ui
                    .button(theme_icon)
                    .on_hover_text("Toggle theme")
                    .clicked()
                {
                    self.settings.ui.dark_mode = !self.settings.ui.dark_mode;
                    self.draft.ui.dark_mode = self.settings.ui.dark_mode;
                    apply_theme(ui.ctx(), self.settings.ui.dark_mode);
                    let _ = self.settings.save();
                }
                if ui
                    .button(icon!(REFRESH))
                    .on_hover_text("Refresh now")
                    .clicked()
                {
                    self.backend.refresh_now();
                }
                ui.separator();
                self.proxy_controls(ui);
                ui.separator();
                self.server_controls(ui);
            });
        });
    }

    pub fn server_controls(&mut self, ui: &mut egui::Ui) {
        if !self.settings.server.manage_server {
            kit::badge(ui, "External server", Color32::GRAY).on_hover_text(
                "Enable \"Manage server\" in Settings to start/stop Ollama from here.",
            );
            return;
        }
        if self.ollama_proc.is_running() {
            if ui.button(ic!(STOP, "Stop")).clicked() {
                self.stop_ollama();
            }
            if ui.button(ic!(REGENERATE, "Restart")).clicked() {
                self.stop_ollama();
                self.start_ollama();
            }
            kit::badge(
                ui,
                format!(
                    "Server running · pid {}",
                    self.ollama_proc.pid().unwrap_or(0)
                ),
                GREEN,
            );
        } else {
            if ui.button(ic!(PLAY, "Start server")).clicked() {
                self.start_ollama();
            }
            kit::badge(ui, "Server stopped", AMBER);
        }
    }

    pub fn proxy_controls(&mut self, ui: &mut egui::Ui) {
        if !self.settings.proxy.enabled {
            return;
        }
        if self.caddy_proc.is_running() {
            if ui
                .button(icon!(STOP))
                .on_hover_text("Stop HTTPS proxy")
                .clicked()
            {
                self.stop_caddy();
            }
            kit::badge(ui, ic!(LOCK, "Proxy on"), GREEN)
                .on_hover_text(caddy::public_url(&self.settings));
        } else {
            if ui
                .button(icon!(PLAY))
                .on_hover_text("Start HTTPS proxy")
                .clicked()
            {
                self.start_caddy();
            }
            kit::badge(ui, ic!(LOCK_OPEN, "Proxy off"), AMBER);
        }
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        for tab in Tab::ALL {
            if kit::nav_item(ui, self.tab == tab, tab.label()) {
                self.tab = tab;
            }
            ui.add_space(2.0);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(6.0);
            if self.draft_dirty() {
                ui.label(RichText::new("● Unsaved settings").color(AMBER));
            }
            ui.label(RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).weak());
        });
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        let notices = {
            let mut s = self.backend.shared.lock().unwrap();
            s.notices
                .retain(|n| n.at.elapsed() < Duration::from_secs(6));
            s.notices.clone()
        };
        if notices.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
            .show(ctx, |ui| {
                for n in notices.iter().rev().take(5) {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(380.0);
                        ui.label(RichText::new(&n.text).color(if n.error { RED } else { GREEN }));
                    });
                    ui.add_space(4.0);
                }
            });
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.ollama_proc.poll(&self.logs) && self.settings.server.restart_on_crash {
            self.restart_after_crash();
        }
        self.caddy_proc.poll(&self.logs);
        if self.pending_autostart && self.backend.shared.lock().unwrap().gpus_detected {
            self.pending_autostart = false;
            self.start_ollama();
        }

        // Live preview of the scale slider; Ctrl +/- zoom still works on top of it.
        let scale = self.draft.ui.scale.clamp(0.75, 2.5);
        if scale != self.applied_scale {
            ui.ctx().set_zoom_factor(scale);
            self.applied_scale = scale;
        }

        egui::Panel::top("top")
            .frame(
                egui::Frame::side_top_panel(ui.style())
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::left("nav")
            .resizable(false)
            .exact_size(220.0)
            .frame(
                egui::Frame::side_top_panel(ui.style())
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| self.nav(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(20.0))
            .show(ui, |ui| match self.tab {
                Tab::Logs => self.logs_tab(ui),
                Tab::Chat => self.chat_tab(ui),
                Tab::Settings => self.settings_tab(ui),
                tab => {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| match tab {
                            Tab::Dashboard => self.dashboard(ui),
                            Tab::Models => self.models_tab(ui),
                            Tab::Performance => self.performance_tab(ui),
                            Tab::Gpu => self.gpu_tab(ui),
                            Tab::Logs | Tab::Chat | Tab::Settings => unreachable!(),
                        });
                }
            });
        let ctx = ui.ctx().clone();
        self.toasts(&ctx);
        #[cfg(debug_assertions)]
        self.capture_frame(&ctx);
    }
}

/// Cleanup lives in `Drop` rather than `App::on_exit`, whose signature depends on the renderer feature.
impl Drop for App {
    fn drop(&mut self) {
        self.backend.shutdown();
        if self.settings.server.stop_on_exit {
            self.stop_ollama();
        }
        self.stop_caddy();
    }
}

/// Text sizes (points, before the interface scale) and roomier spacing than egui's defaults.
fn apply_style(ctx: &egui::Context) {
    use egui::FontFamily::{Monospace, Proportional};
    use egui::{FontId, TextStyle};
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(14.0, Proportional)),
            (TextStyle::Body, FontId::new(17.0, Proportional)),
            (TextStyle::Button, FontId::new(17.0, Proportional)),
            (TextStyle::Heading, FontId::new(26.0, Proportional)),
            (TextStyle::Monospace, FontId::new(16.0, Monospace)),
        ]
        .into();
        let sp = &mut style.spacing;
        sp.item_spacing = egui::vec2(10.0, 10.0);
        sp.button_padding = egui::vec2(14.0, 8.0);
        sp.interact_size = egui::vec2(52.0, 34.0);
        sp.icon_width = 22.0;
        sp.icon_width_inner = 14.0;
        sp.combo_height = 320.0;
        sp.text_edit_width = 340.0;
        sp.slider_width = 300.0;
    });
}

pub fn apply_theme(ctx: &egui::Context, dark: bool) {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    // Slightly darker/lighter background so the content boxes stand out.
    visuals.panel_fill = if dark {
        Color32::from_rgb(24, 25, 28)
    } else {
        Color32::from_rgb(241, 243, 246)
    };
    visuals.window_corner_radius = 10.into();
    for w in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = 6.into();
    }
    ctx.set_visuals(visuals);
}
