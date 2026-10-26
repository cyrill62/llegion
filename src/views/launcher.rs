use eframe::egui::{self, Color32, RichText};

use crate::app::App;
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
        "Anthropic's coding agent",
        Icon::Image(png!("claude")),
    ),
    (
        "codex",
        "Codex",
        "OpenAI's coding agent",
        Icon::Image(png!("codex")),
    ),
    (
        "opencode",
        "OpenCode",
        "Open source coding agent",
        Icon::Themed {
            dark: png!("opencode-dark"),
            light: png!("opencode-light"),
        },
    ),
    (
        "pi",
        "Pi",
        "Minimal coding agent toolkit",
        Icon::Image(png!("pi")),
    ),
    (
        "omp",
        "OMP",
        "AI coding agent with IDE integration",
        Icon::Themed {
            dark: png!("omp-dark"),
            light: png!("omp-light"),
        },
    ),
    (
        "hermes",
        "Hermes",
        "Personal AI agent",
        Icon::Themed {
            dark: png!("hermes-dark"),
            light: png!("hermes-light"),
        },
    ),
    (
        "openclaw",
        "OpenClaw",
        "Personal AI assistant",
        Icon::Image(png!("openclaw")),
    ),
    (
        "copilot",
        "Copilot CLI",
        "GitHub's coding agent",
        Icon::Themed {
            dark: png!("copilot-dark"),
            light: png!("copilot-light"),
        },
    ),
    (
        "cline",
        "Cline",
        "Autonomous coding agent",
        Icon::Themed {
            dark: png!("cline-dark"),
            light: png!("cline-light"),
        },
    ),
    (
        "droid",
        "Droid",
        "Factory's coding agent",
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
        let response = response.on_hover_text(description);
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

    pub fn launcher_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Launch with Ollama");
        ui.label(RichText::new("Choose an agent to see its launch command.").weak());
        ui.add_space(12.0);
        kit::panel(ui, "Agents", None, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                for (key, name, description, icon) in LAUNCHERS {
                    self.agent_tile(ui, key, name, description, icon);
                }
            });
        });
        let command = format!("ollama launch {}", self.launcher_selected);
        kit::panel(ui, "Command", None, |ui| {
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
                        ui.label(RichText::new(*description).weak());
                    });
                });
                ui.add_space(12.0);
            }
            ui.label("Run this from the project directory:");
            ui.add_space(8.0);
            ui.code(&command);
            if ui.button("Copy command").clicked() {
                ui.ctx().copy_text(command.clone());
            }
        });
    }
}
