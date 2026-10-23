use eframe::egui::{self, Color32, RichText};

use crate::app::{AMBER, App, RED};
use crate::config::LogSource;
use crate::logs::{Level, LogLine, Origin};
use crate::views::kit;

fn level_color(level: Level, ui: &egui::Ui) -> Color32 {
    match level {
        Level::Error => RED,
        Level::Warn => AMBER,
        Level::Info => ui.visuals().text_color(),
        Level::Debug => ui.visuals().weak_text_color(),
    }
}

impl App {
    fn refresh_log_cache(&mut self) {
        let v = &self.log_view;
        let key = (
            self.logs.generation(),
            v.filter.clone(),
            v.min_level,
            [v.show_ollama, v.show_caddy, v.show_app],
        );
        if v.paused || v.cache_key.as_ref() == Some(&key) {
            return;
        }
        let needle = v.filter.to_lowercase();
        let (min, origins) = (v.min_level, key.3);
        self.log_view.cache = self.logs.snapshot_filtered(|l: &LogLine| {
            let origin_ok = match l.origin {
                Origin::Ollama => origins[0],
                Origin::Caddy => origins[1],
                Origin::App => origins[2],
            };
            origin_ok
                && l.level >= min
                && (needle.is_empty() || l.text.to_lowercase().contains(&needle))
        });
        self.log_view.cache_key = Some(key);
    }

    pub fn logs_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Logs");
        ui.add_space(8.0);
        kit::panel(ui, ic!(SEARCH, "Source & filters"), None, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Source");
                let before = self.draft.logs.source;
                egui::ComboBox::from_id_salt("log_source")
                    .selected_text(match self.draft.logs.source {
                        LogSource::Managed => "Managed server only",
                        LogSource::File => "Log file",
                        LogSource::Journal => "systemd journal",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.draft.logs.source,
                            LogSource::Managed,
                            "Managed server only",
                        );
                        ui.selectable_value(
                            &mut self.draft.logs.source,
                            LogSource::File,
                            "Log file",
                        );
                        ui.selectable_value(
                            &mut self.draft.logs.source,
                            LogSource::Journal,
                            "systemd journal",
                        );
                    });
                if self.draft.logs.source != before {
                    // Switching sources is harmless; apply just that field immediately.
                    self.settings.logs.source = self.draft.logs.source;
                    let _ = self.settings.save();
                    self.sync_log_follower();
                }
                ui.separator();
                ui.label("Minimum level");
                egui::ComboBox::from_id_salt("log_level")
                    .selected_text(self.log_view.min_level.label())
                    .show_ui(ui, |ui| {
                        for l in [Level::Debug, Level::Info, Level::Warn, Level::Error] {
                            ui.selectable_value(&mut self.log_view.min_level, l, l.label());
                        }
                    });
                ui.toggle_value(&mut self.log_view.show_ollama, "ollama");
                ui.toggle_value(&mut self.log_view.show_caddy, "caddy");
                ui.toggle_value(&mut self.log_view.show_app, "app");
                ui.separator();
                ui.add(
                    egui::TextEdit::singleline(&mut self.log_view.filter)
                        .hint_text(ic!(SEARCH, "search"))
                        .desired_width(240.0),
                );
            });
            ui.horizontal(|ui| {
                kit::switch(ui, &mut self.log_view.autoscroll, "Auto-scroll");
                kit::switch(ui, &mut self.log_view.wrap, "Wrap");
                let pause_label = if self.log_view.paused {
                    ic!(PLAY, "Resume")
                } else {
                    ic!(PAUSE, "Pause")
                };
                if ui.button(pause_label).clicked() {
                    self.log_view.paused = !self.log_view.paused;
                }
                if ui.button(ic!(DELETE, "Clear")).clicked() {
                    self.logs.clear();
                }
                if ui.button(ic!(COPY, "Copy")).clicked() {
                    ui.ctx().copy_text(self.visible_log_text());
                }
                if ui.button(ic!(SAVE, "Save")).clicked() {
                    let path = crate::config::config_dir().join(format!(
                        "ollama-{}.log",
                        chrono::Local::now().format("%Y%m%d-%H%M%S")
                    ));
                    let res = std::fs::create_dir_all(crate::config::config_dir())
                        .and_then(|_| std::fs::write(&path, self.visible_log_text()));
                    match res {
                        Ok(()) => self.logs.app(format!("Saved logs to {}", path.display())),
                        Err(e) => self.logs.app_error(format!("Failed to save logs: {e}")),
                    }
                }
                ui.label(RichText::new(format!("{} lines", self.log_view.cache.len())).weak());
                if self.log_view.paused {
                    ui.label(RichText::new("paused").color(AMBER));
                }
            });
            match self.settings.logs.source {
                LogSource::File => {
                    ui.label(
                        RichText::new(format!("Following {}", self.settings.logs.file_path)).weak(),
                    );
                }
                LogSource::Journal => {
                    ui.label(
                        RichText::new(format!(
                            "Following journalctl -u {}",
                            self.settings.logs.journal_unit
                        ))
                        .weak(),
                    );
                }
                LogSource::Managed if !self.settings.server.manage_server => {
                    ui.label(
                    RichText::new("The server is not managed by this app; pick a log file or the journal to see its output.")
                        .color(AMBER),
                );
                }
                LogSource::Managed => {}
            }
        });

        self.refresh_log_cache();
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
        let wrap = self.log_view.wrap;
        let lines = &self.log_view.cache;
        let mut area = egui::ScrollArea::both().auto_shrink(false);
        if self.log_view.autoscroll && !self.log_view.paused {
            area = area.stick_to_bottom(true);
        }
        let render = |ui: &mut egui::Ui, l: &LogLine| {
            let color = level_color(l.level, ui);
            let text = format!("{} {:6} {}", l.time, l.origin.label(), l.text);
            let label = egui::Label::new(RichText::new(text).monospace().color(color));
            ui.add(if wrap { label.wrap() } else { label.extend() });
        };
        egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(10.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                if wrap {
                    // Variable row heights: render everything (buffer is bounded).
                    area.show(ui, |ui| {
                        for l in lines {
                            render(ui, l);
                        }
                    });
                } else {
                    area.show_rows(ui, row_height, lines.len(), |ui, range| {
                        for l in &lines[range] {
                            render(ui, l);
                        }
                    });
                }
            });
    }

    fn visible_log_text(&self) -> String {
        self.log_view
            .cache
            .iter()
            .map(|l| format!("{} [{}] {}", l.time, l.origin.label(), l.text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
