use eframe::egui::{self, Color32, RichText};

use crate::app::{App, RED};
use crate::i18n::tr;
use crate::relay::Relay;
use crate::views::kit;

/// An agent's icon: one image for both themes, one per theme, or a lettered badge.
enum Icon {
    Image(&'static [u8]),
    Themed {
        dark: &'static [u8],
        light: &'static [u8],
    },
    Letter(Color32),
}

macro_rules! png {
    ($name:literal) => {
        include_bytes!(concat!("../../assets/launcher/", $name, ".png"))
    };
}

const LAUNCHERS: &[(&str, &str, &str, Icon)] = &[
    (
        "claude",
        "Claude Code",
        tr_key!("Anthropic's coding agent"),
        Icon::Image(png!("claude")),
    ),
    (
        "codex",
        "Codex",
        tr_key!("OpenAI's coding agent"),
        Icon::Image(png!("codex")),
    ),
    (
        "opencode",
        "OpenCode",
        tr_key!("Open source coding agent"),
        Icon::Themed {
            dark: png!("opencode-dark"),
            light: png!("opencode-light"),
        },
    ),
    (
        "pi",
        "Pi",
        tr_key!("Minimal coding agent toolkit"),
        Icon::Image(png!("pi")),
    ),
    (
        "omp",
        "OMP",
        tr_key!("AI coding agent with IDE integration"),
        Icon::Image(png!("omp")),
    ),
    (
        "hermes",
        "Hermes",
        tr_key!("Personal AI agent"),
        Icon::Themed {
            dark: png!("hermes-dark"),
            light: png!("hermes-light"),
        },
    ),
    (
        "openclaw",
        "OpenClaw",
        tr_key!("Personal AI assistant"),
        Icon::Image(png!("openclaw")),
    ),
    (
        "copilot",
        "Copilot CLI",
        tr_key!("GitHub's coding agent"),
        Icon::Themed {
            dark: png!("copilot-dark"),
            light: png!("copilot-light"),
        },
    ),
    (
        "cline",
        "Cline",
        tr_key!("Autonomous coding agent"),
        Icon::Themed {
            dark: png!("cline-dark"),
            light: png!("cline-light"),
        },
    ),
    (
        "droid",
        "Droid",
        tr_key!("Factory's coding agent"),
        Icon::Letter(Color32::from_rgb(230, 130, 50)),
    ),
];

const TILE: egui::Vec2 = egui::vec2(132.0, 128.0);
const TILE_ICON: f32 = 64.0;

impl App {
    /// Paints an agent's icon (or its lettered badge) into `rect`.
    fn agent_icon(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        key: &'static str,
        name: &str,
        icon: &Icon,
    ) {
        let dark = ui.visuals().dark_mode;
        let bytes = match icon {
            Icon::Image(bytes) => bytes,
            Icon::Themed { dark: d, light: l } => {
                if dark {
                    d
                } else {
                    l
                }
            }
            Icon::Letter(color) => {
                let painter = ui.painter();
                painter.rect_filled(rect, rect.width() * 0.25, *color);
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    name.chars().next().unwrap_or('?'),
                    egui::FontId::proportional(rect.width() * 0.55),
                    Color32::WHITE,
                );
                return;
            }
        };
        let ctx = ui.ctx().clone();
        let texture = self
            .launcher_icons
            .entry((key, dark))
            .or_insert_with(|| crate::app::load_logo(&ctx, bytes, key));
        egui::Image::new(&*texture).paint_at(ui, rect);
    }

    /// One agent in the grid: logo on top, name centred under it.
    fn agent_tile(
        &mut self,
        ui: &mut egui::Ui,
        key: &'static str,
        name: &str,
        description: &str,
        icon: &Icon,
    ) {
        let selected = self.launcher_selected == key;
        let (rect, response) = ui.allocate_exact_size(TILE, egui::Sense::click());
        let response = response.on_hover_text(tr(description));
        if response.clicked() {
            self.launcher_selected = key.into();
        }
        let visuals = ui.visuals();
        let (fill, stroke) = if selected {
            (visuals.selection.bg_fill, visuals.selection.stroke)
        } else if response.hovered() {
            (
                visuals.widgets.hovered.weak_bg_fill,
                visuals.widgets.hovered.bg_stroke,
            )
        } else {
            (Color32::TRANSPARENT, egui::Stroke::NONE)
        };
        ui.painter()
            .rect(rect, 10.0, fill, stroke, egui::StrokeKind::Inside);
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, rect.top() + 14.0 + TILE_ICON / 2.0),
            egui::vec2(TILE_ICON, TILE_ICON),
        );
        self.agent_icon(ui, icon_rect, key, name, icon);
        let methods = self
            .launcher_missing
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|m| m.get(key).cloned());
        if let Some(methods) = methods {
            let button = egui::Rect::from_min_size(
                rect.right_top() + egui::vec2(-30.0, 4.0),
                egui::vec2(26.0, 26.0),
            );
            let mut chosen = None;
            ui.put(button, |ui: &mut egui::Ui| {
                ui.menu_button(icon!(DOWNLOAD), |ui| {
                    ui.label(RichText::new(trf!("Install {name} with", name = name)).weak());
                    for m in &methods {
                        if ui.button(m.via).on_hover_text(&m.command).clicked() {
                            chosen = Some(m.command.clone());
                            ui.close();
                        }
                    }
                })
                .response
            })
            .on_hover_text(trf!("{name} is not installed", name = name));
            if let Some(cmd) = chosen
                && let Err(e) = crate::install::open_terminal(&cmd)
            {
                self.ctl
                    .logs
                    .app_error(format!("Cannot open a terminal to install {name}: {e}"));
            }
        }
        let text_color = if selected {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            egui::pos2(rect.center().x, icon_rect.bottom() + 10.0),
            egui::Align2::CENTER_TOP,
            name,
            egui::TextStyle::Body.resolve(ui.style()),
            text_color,
        );
    }

    /// What Launch runs and "Copy command" copies: `ollama launch` in the project folder, with
    /// OLLAMA_HOST at the API URL, or at the relay that adds its token or accepts its
    /// self-signed certificate.
    fn launch_command(&mut self) -> anyhow::Result<String> {
        let conn = &self.ctl.settings.connection;
        let host = if Relay::needed(conn) {
            if self.launch_relay.as_ref().is_none_or(|r| r.conn != *conn) {
                self.launch_relay = Some(Relay::start(conn.clone())?);
            }
            self.launch_relay
                .as_ref()
                .map(Relay::url)
                .unwrap_or_default()
        } else {
            conn.base_url.clone()
        };
        Ok(crate::install::launch_command(
            &self.launcher_selected,
            self.launcher_dir.trim(),
            &host,
        ))
    }

    /// Looks for every agent's binary in the background.
    fn check_agents(&self, ctx: &egui::Context) {
        let slot = self.launcher_missing.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let missing = LAUNCHERS
                .iter()
                .filter(|(key, ..)| !crate::install::is_installed(key))
                .map(|(key, ..)| (*key, crate::install::methods(key)))
                .collect();
            *slot.lock().unwrap() = Some(missing);
            ctx.request_repaint();
        });
    }

    pub fn launcher_tab(&mut self, ui: &mut egui::Ui) {
        let focused = ui.ctx().input(|i| i.focused);
        if focused && !self.launcher_focused {
            self.check_agents(ui.ctx());
        }
        self.launcher_focused = focused;
        ui.heading(tr("Launch with Ollama"));
        ui.label(RichText::new(tr("Choose an agent to see its launch command.")).weak());
        ui.add_space(12.0);
        kit::panel(ui, tr("Agents"), None, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                for (key, name, description, icon) in LAUNCHERS {
                    self.agent_tile(ui, key, name, description, icon);
                }
            });
        });
        kit::panel(ui, tr("Command"), None, |ui| {
            if let Some((key, name, description, icon)) = LAUNCHERS
                .iter()
                .find(|(key, ..)| *key == self.launcher_selected)
            {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                    self.agent_icon(ui, rect, key, name, icon);
                    ui.vertical(|ui| {
                        ui.strong(*name);
                        ui.label(RichText::new(tr(description)).weak());
                    });
                });
                ui.add_space(12.0);
            }
            ui.horizontal(|ui| {
                ui.label(ic!(FOLDER, "Project folder"));
                ui.add(egui::TextEdit::singleline(&mut self.launcher_dir).desired_width(360.0));
            });
            ui.add_space(8.0);
            let command = self.launch_command();
            match &command {
                Ok(command) => {
                    ui.code(command);
                }
                Err(e) => {
                    kit::badge(
                        ui,
                        trf!(
                            "Cannot start the token relay: {error}",
                            error = format!("{e:#}")
                        ),
                        RED,
                    );
                }
            }
            let conn = &self.ctl.settings.connection;
            ui.label(
                RichText::new(if Relay::needed(conn) {
                    trf!(
                        "OLLAMA_HOST is a local relay that adds the token for {url}; it runs while llegion is open.",
                        url = conn.base_url
                    )
                } else {
                    trf!(
                        "OLLAMA_HOST is {url}, the API URL in Settings → Connection.",
                        url = conn.base_url
                    )
                })
                .weak(),
            );
            ui.add_space(8.0);
            let key = self.launcher_selected.clone();
            let missing = self
                .launcher_missing
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|m| m.contains_key(key.as_str()));
            let Ok(command) = command else { return };
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!missing, egui::Button::new(ic!(PLAY, "Launch")))
                    .on_hover_text(tr("Opens a terminal running this command"))
                    .on_disabled_hover_text(tr("Not installed: use the install button on its tile"))
                    .clicked()
                {
                    let dir = self.launcher_dir.trim();
                    let result = if std::path::Path::new(dir).is_dir() {
                        crate::install::open_terminal(&command)
                    } else {
                        Err(std::io::Error::other(format!("{dir}: no such folder")))
                    };
                    if let Err(e) = result {
                        self.ctl.logs.app_error(format!("Cannot launch {key}: {e}"));
                    }
                }
                if ui.button(tr("Copy command")).clicked() {
                    ui.ctx().copy_text(command.clone());
                }
            });
        });
    }
}
