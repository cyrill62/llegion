use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};

use crate::config::Settings;
use crate::control::Control;
use crate::ferron;
use crate::i18n::tr;
use crate::logs::{Level, LogLine};
use crate::util::Repaint;
use crate::views::kit;
use crate::views::settings::SettingsPane;

pub const GREEN: Color32 = Color32::from_rgb(80, 200, 120);
pub const RED: Color32 = Color32::from_rgb(230, 90, 80);
pub const AMBER: Color32 = Color32::from_rgb(235, 175, 60);

/// Decodes an embedded 8-bit RGBA PNG into a texture.
pub(crate) fn load_logo(ctx: &egui::Context, bytes: &[u8], name: &str) -> egui::TextureHandle {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("embedded logo PNG");
    let mut pixels = vec![0; reader.output_buffer_size().expect("logo dimensions")];
    let info = reader
        .next_frame(&mut pixels)
        .expect("embedded logo PNG frame");
    pixels.truncate(info.buffer_size());
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(
            [info.width as usize, info.height as usize],
            &pixels,
        ),
        egui::TextureOptions::LINEAR,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Models,
    Chat,
    Launcher,
    Performance,
    Gpu,
    Logs,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 8] = [
        Tab::Dashboard,
        Tab::Models,
        Tab::Chat,
        Tab::Launcher,
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
            Tab::Launcher => ic!(PLAY, "Launcher"),
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
    pub show_ferron: bool,
    pub show_app: bool,
    pub autoscroll: bool,
    pub paused: bool,
    pub wrap: bool,
    pub cache: Vec<LogLine>,
    pub cache_key: Option<(u64, String, Level, [bool; 3])>,
}

pub struct App {
    /// Settings in effect, backend, logs and child processes.
    pub ctl: Control,
    /// Settings being edited in the Settings / GPU tabs.
    pub draft: Settings,
    logo_light: egui::TextureHandle,
    logo_dark: egui::TextureHandle,
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
    pub chat_active_id: String,
    pub chat_rename_id: Option<String>,
    pub chat_rename_text: String,
    pub chat_subject_name: String,
    pub collapsed_chat_subjects: HashSet<String>,
    pub chat_subjects_open: bool,
    /// Main navigation shows labels; when off it shrinks to icons.
    pub nav_open: bool,
    pub chat_model: String,
    pub chat_input: String,
    pub chat_system: String,
    pub chat_ctx: u32,
    /// `None` keeps the model's own default.
    pub chat_temperature: Option<f32>,
    pub chat_markdown: bool,
    pub launcher_selected: String,
    /// Agent icons by (key, dark theme), decoded on first use.
    pub launcher_icons: HashMap<(&'static str, bool), egui::TextureHandle>,
    /// Install methods of each agent missing from this machine; `None` until checked.
    pub launcher_missing: Arc<Mutex<Option<crate::install::Missing>>>,
    /// Window focus last frame: agents are checked again when it comes back (after an install).
    pub launcher_focused: bool,
    /// Folder the launched agent works in.
    pub launcher_dir: String,
    /// Token-adding relay the launched agents talk to, started on first need.
    pub launch_relay: Option<crate::relay::Relay>,
    pub markdown_cache: egui_commonmark::CommonMarkCache,

    pub log_view: LogView,

    // Settings tab
    pub reveal_tokens: bool,
    pub new_token_label: String,
    pub new_backend_url: String,
    pub backend_tests: crate::views::settings::BackendTests,
    pub proxy_test: Arc<Mutex<Option<(bool, String)>>>,
    pub confirm_reset: bool,
    pub settings_pane: SettingsPane,
    /// Zoom factor currently applied to the context (follows the draft for live preview).
    applied_scale: f32,
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
        let ctx = cc.egui_ctx.clone();
        let logo_light = load_logo(
            &ctx,
            include_bytes!("../assets/logo-light@2x.png"),
            "logo-light",
        );
        let logo_dark = load_logo(
            &ctx,
            include_bytes!("../assets/logo-dark@2x.png"),
            "logo-dark",
        );
        let repaint: Repaint = Arc::new(move |after| ctx.request_repaint_after(after));
        egui_material_icons::initialize(&cc.egui_ctx);
        apply_style(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx, settings.ui.dark_mode);

        let mut app = Self {
            draft: settings.clone(),
            logo_light,
            logo_dark,
            ctl: Control::new(settings, repaint),
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
            chat_active_id: String::new(),
            chat_rename_id: None,
            chat_rename_text: String::new(),
            chat_subject_name: String::new(),
            collapsed_chat_subjects: HashSet::new(),
            chat_subjects_open: true,
            nav_open: true,
            chat_model: String::new(),
            chat_input: String::new(),
            chat_system: String::new(),
            chat_ctx: 0,
            chat_temperature: None,
            chat_markdown: true,
            launcher_selected: "claude".into(),
            launcher_icons: HashMap::new(),
            launcher_missing: Arc::new(Mutex::new(None)),
            launcher_focused: false,
            launch_relay: None,
            launcher_dir: directories::BaseDirs::new()
                .map(|d| d.home_dir().display().to_string())
                .unwrap_or_default(),
            markdown_cache: Default::default(),
            log_view: LogView {
                filter: String::new(),
                min_level: Level::Debug,
                show_ollama: true,
                show_ferron: true,
                show_app: true,
                autoscroll: true,
                paused: false,
                wrap: false,
                cache: Vec::new(),
                cache_key: None,
            },
            reveal_tokens: false,
            new_token_label: String::new(),
            new_backend_url: String::new(),
            backend_tests: Default::default(),
            proxy_test: Arc::new(Mutex::new(None)),
            confirm_reset: false,
            settings_pane: SettingsPane::Connection,
            applied_scale: 0.0,
            #[cfg(debug_assertions)]
            capture,
        };
        #[cfg(debug_assertions)]
        if app.capture.is_some() {
            // Drops the "Settings: <config dir>" line, which shows the user's home path.
            app.ctl.logs.clear();
        }
        app.ctl.start();
        app
    }

    // ------------------------------------------------------------------
    // Settings application

    /// Commits the draft: persists it and reconfigures running components.
    pub fn apply_draft(&mut self, restart_services: bool) {
        self.ctl.apply(self.draft.clone(), restart_services);
    }

    /// GPU selection and server options only reach a server this app starts.
    pub fn gpu_tab_enabled(&self) -> bool {
        self.draft.server.manage_server
    }

    pub fn draft_dirty(&self) -> bool {
        self.draft != self.ctl.settings
    }

    // ------------------------------------------------------------------
    // Layout

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let (connected, version, error) = {
            let s = self.ctl.backend.shared.lock().unwrap();
            (s.connected, s.version.clone(), s.error.clone())
        };
        ui.horizontal(|ui| {
            let logo = if self.ctl.settings.ui.dark_mode {
                &self.logo_dark
            } else {
                &self.logo_light
            };
            let logo_size = logo.size_vec2() * (30.0 / logo.size_vec2().y);
            ui.add(egui::Image::new(logo).fit_to_exact_size(logo_size));
            ui.add_space(12.0);
            let r = if connected {
                kit::badge(
                    ui,
                    trf!(
                        "Connected · v{version}",
                        version = version.unwrap_or_default()
                    ),
                    GREEN,
                )
            } else {
                kit::badge(ui, tr("Disconnected"), RED)
            };
            let tip = match error {
                Some(err) => format!("{}\n{err}", self.ctl.settings.connection.base_url),
                None => self.ctl.settings.connection.base_url.clone(),
            };
            r.on_hover_text(tip);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let theme_icon = if self.ctl.settings.ui.dark_mode {
                    icon!(LIGHT)
                } else {
                    icon!(DARK)
                };
                if ui
                    .button(theme_icon)
                    .on_hover_text(tr("Toggle theme"))
                    .clicked()
                {
                    self.ctl.settings.ui.dark_mode = !self.ctl.settings.ui.dark_mode;
                    self.draft.ui.dark_mode = self.ctl.settings.ui.dark_mode;
                    apply_theme(ui.ctx(), self.ctl.settings.ui.dark_mode);
                    let _ = self.ctl.settings.save();
                }
                if ui
                    .button(icon!(REFRESH))
                    .on_hover_text(tr("Refresh now"))
                    .clicked()
                {
                    self.ctl.backend.refresh_now();
                }
                ui.separator();
                self.proxy_controls(ui);
                ui.separator();
                self.server_controls(ui);
            });
        });
    }

    pub fn server_controls(&mut self, ui: &mut egui::Ui) {
        if !self.ctl.settings.server.manage_server {
            kit::badge(ui, tr("External server"), Color32::GRAY).on_hover_text(tr(
                "Enable \"Manage server\" in Settings to start/stop Ollama from here.",
            ));
            return;
        }
        if self.ctl.ollama_proc.is_running() {
            if ui.button(ic!(STOP, "Stop")).clicked() {
                self.ctl.stop_ollama();
            }
            if ui.button(ic!(REGENERATE, "Restart")).clicked() {
                self.ctl.restart_ollama();
            }
            kit::badge(
                ui,
                trf!(
                    "Server running · pid {pid}",
                    pid = self.ctl.ollama_proc.pid().unwrap_or(0)
                ),
                GREEN,
            );
        } else {
            if ui.button(ic!(PLAY, "Start server")).clicked() {
                self.ctl.start_ollama();
            }
            kit::badge(ui, tr("Server stopped"), AMBER);
        }
    }

    pub fn proxy_controls(&mut self, ui: &mut egui::Ui) {
        if !self.ctl.settings.proxy.enabled {
            return;
        }
        if self.ctl.ferron_proc.is_running() {
            if ui
                .button(icon!(STOP))
                .on_hover_text(tr("Stop HTTPS proxy"))
                .clicked()
            {
                self.ctl.stop_ferron();
            }
            kit::badge(ui, ic!(LOCK, "Proxy on"), GREEN)
                .on_hover_text(ferron::public_url(&self.ctl.settings));
        } else {
            if ui
                .button(icon!(PLAY))
                .on_hover_text(tr("Start HTTPS proxy"))
                .clicked()
            {
                self.ctl.start_ferron();
            }
            kit::badge(ui, ic!(LOCK_OPEN, "Proxy off"), AMBER);
        }
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
        let (toggle, tip) = if self.nav_open {
            (icon!(PANEL_CLOSE), tr("Shrink the menu"))
        } else {
            (icon!(PANEL_OPEN), tr("Expand the menu"))
        };
        if ui.button(toggle).on_hover_text(tip).clicked() {
            self.nav_open = !self.nav_open;
        }
        ui.add_space(6.0);
        for tab in Tab::ALL {
            let enabled = tab != Tab::Gpu || self.gpu_tab_enabled();
            let label = tab.label();
            // Labels start with their icon; a shrunk menu keeps only that.
            let shown = if self.nav_open {
                label
            } else {
                label.split("  ").next().unwrap_or(label)
            };
            let mut r = ui
                .add_enabled_ui(enabled, |ui| kit::nav_item(ui, self.tab == tab, shown))
                .inner;
            if !self.nav_open {
                r = r.on_hover_text(label);
            }
            let r = r.on_disabled_hover_text(
                    tr("GPU selection applies to a server started by this app: enable \"Manage the server\" in Settings → Server."),
                );
            if r.clicked() {
                self.tab = tab;
            }
            ui.add_space(2.0);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(6.0);
            if self.draft_dirty() {
                ui.label(
                    RichText::new(if self.nav_open {
                        tr("● Unsaved settings")
                    } else {
                        "●"
                    })
                    .color(AMBER),
                )
                .on_hover_text(tr("Unsaved settings"));
            }
            if self.nav_open {
                ui.label(RichText::new(format!("v{}", include_str!("../VERSION").trim())).weak());
            }
        });
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        let notices = {
            let mut s = self.ctl.backend.shared.lock().unwrap();
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
        self.ctl.tick();
        if self.tab == Tab::Gpu && !self.gpu_tab_enabled() {
            self.tab = Tab::Dashboard;
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
            .exact_size(if self.nav_open { 220.0 } else { 64.0 })
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
                Tab::Launcher => self.launcher_tab(ui),
                Tab::Settings => self.settings_tab(ui),
                tab => {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| match tab {
                            Tab::Dashboard => self.dashboard(ui),
                            Tab::Models => self.models_tab(ui),
                            Tab::Performance => self.performance_tab(ui),
                            Tab::Gpu => self.gpu_tab(ui),
                            Tab::Logs | Tab::Chat | Tab::Launcher | Tab::Settings => unreachable!(),
                        });
                }
            });
        let ctx = ui.ctx().clone();
        self.toasts(&ctx);
        #[cfg(debug_assertions)]
        self.capture_frame(&ctx);
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
