use eframe::egui::{self, RichText};

use crate::app::{AMBER, App, GREEN, RED};
use crate::i18n::tr;
use crate::ollama::RunningModel;
use crate::util::{CONTEXT_PRESETS, ctx_label, format_bytes, format_duration_secs};
use crate::views::kit;

/// Context size picker: presets plus free numeric input.
pub fn context_picker(ui: &mut egui::Ui, id: &str, value: &mut u32) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(ctx_label(*value))
        .show_ui(ui, |ui| {
            for preset in CONTEXT_PRESETS {
                ui.selectable_value(value, preset, ctx_label(preset));
            }
        });
    ui.add(context_drag(value));
}

/// Context size slider: auto, then powers of two from 2K to 1M, plus free numeric input.
pub fn context_slider(ui: &mut egui::Ui, value: &mut u32) {
    // Step 0 = auto, step n = 2^(10 + n) tokens. Values in between snap down.
    let mut step = match *value {
        0 => 0,
        v => (v.max(2048).ilog2() - 10).min(10),
    };
    ui.spacing_mut().slider_width = 338.0;
    if ui
        .add(egui::Slider::new(&mut step, 0..=10).show_value(false))
        .changed()
    {
        *value = if step == 0 { 0 } else { 1 << (10 + step) };
    }
    ui.add(context_drag(value));
}

fn context_drag(value: &mut u32) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .range(0..=1_048_576)
        .speed(256)
        .custom_formatter(|v, _| {
            if v == 0.0 {
                "auto".into()
            } else {
                format!("{v} tok")
            }
        })
        .custom_parser(|s| s.trim_end_matches("tok").trim().parse().ok())
}

impl App {
    pub fn running_table(&mut self, ui: &mut egui::Ui, running: &[RunningModel]) {
        let busy = self.ctl.backend.shared.lock().unwrap().busy.clone();
        let now = chrono::Utc::now();
        egui::Grid::new("running")
            .striped(true)
            .num_columns(7)
            .spacing([28.0, 12.0])
            .show(ui, |ui| {
                for h in [tr("Model"), tr("Size"), "VRAM", tr("Processor"), tr("Context"), tr("Unloads in"), ""] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                for m in running {
                    ui.label(RichText::new(&m.name).monospace().strong());
                    ui.label(format_bytes(m.size));
                    ui.label(format_bytes(m.size_vram));
                    let proc = m.processor();
                    let color = match proc.as_str() {
                        "100% GPU" => GREEN,
                        "100% CPU" => RED,
                        _ => AMBER,
                    };
                    kit::badge(ui, proc, color).on_hover_text(
                        tr("Share of the model held in GPU memory. Anything on CPU is much slower — reduce the context size or use a smaller quantization."),
                    );
                    ui.label(
                        m.context_length
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "-".into()),
                    );
                    let expires = chrono::DateTime::parse_from_rfc3339(&m.expires_at)
                        .map(|t| {
                            format_duration_secs((t.with_timezone(&chrono::Utc) - now).num_seconds())
                        })
                        .unwrap_or_else(|_| "-".into());
                    ui.label(expires);
                    if busy.contains(&m.name) {
                        ui.spinner();
                    } else if ui.button(ic!(EJECT, "Unload")).clicked() {
                        self.ctl.backend.unload(&m.name);
                    }
                    ui.end_row();
                }
            });
    }

    pub fn models_tab(&mut self, ui: &mut egui::Ui) {
        let (models, running, pulls, busy, connected) = {
            let s = self.ctl.backend.shared.lock().unwrap();
            (
                s.models.clone(),
                s.running.clone(),
                s.pulls.clone(),
                s.busy.clone(),
                s.connected,
            )
        };
        ui.heading(tr("Models"));
        ui.add_space(8.0);
        if !connected {
            kit::panel(ui, ic!(WARNING, "Not connected"), None, |ui| {
                ui.label(
                    RichText::new(tr(
                        "The Ollama API is unreachable. Check Settings → Connection.",
                    ))
                    .color(RED),
                );
            });
        }

        let disk: u64 = models.iter().map(|m| m.size).sum();
        let vram: u64 = running.iter().map(|m| m.size_vram).sum();
        ui.columns(3, |cols| {
            kit::tile(
                &mut cols[0],
                tr("Running"),
                RichText::new(running.len().to_string()),
                Some(format!("{} in VRAM", format_bytes(vram))),
            );
            kit::tile(
                &mut cols[1],
                tr("Downloaded"),
                RichText::new(models.len().to_string()),
                Some("models".into()),
            );
            kit::tile(
                &mut cols[2],
                tr("Disk usage"),
                RichText::new(format_bytes(disk)),
                None,
            );
        });
        ui.add_space(14.0);

        kit::panel(ui, ic!(GPU, "Running models"), None, |ui| {
            if running.is_empty() {
                ui.label(RichText::new(tr("No model is currently loaded in memory.")).weak());
            } else {
                self.running_table(ui, &running);
            }
        });

        kit::panel(
            ui,
            ic!(DOWNLOAD, "Pull a model"),
            Some(tr(
                "Download a model from the Ollama library by name and tag.",
            )),
            |ui| {
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.pull_name)
                            .hint_text(tr("e.g. llama3.2:3b, qwen3:8b, gemma3:12b-it-qat"))
                            .desired_width(420.0),
                    );
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button(ic!(DOWNLOAD, "Pull")).clicked() || enter)
                        && !self.pull_name.trim().is_empty()
                    {
                        self.ctl.backend.pull(&self.pull_name);
                        self.pull_name.clear();
                    }
                    ui.hyperlink_to("Browse library", "https://ollama.com/library");
                });
                let mut clear = Vec::new();
                for (name, p) in &pulls {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).monospace().strong());
                        if let Some(err) = &p.error {
                            kit::badge(ui, err.clone(), RED);
                        } else if p.done {
                            kit::badge(ui, ic!(CHECK, "Done"), GREEN);
                        } else {
                            let frac = if p.total > 0 {
                                p.completed as f32 / p.total as f32
                            } else {
                                0.0
                            };
                            let text = if p.total > 0 {
                                format!(
                                    "{} — {} / {}",
                                    p.status,
                                    format_bytes(p.completed),
                                    format_bytes(p.total)
                                )
                            } else {
                                p.status.clone()
                            };
                            ui.add(
                                egui::ProgressBar::new(frac)
                                    .desired_width(420.0)
                                    .desired_height(24.0)
                                    .text(text)
                                    .animate(true),
                            );
                            if ui.button(tr("Cancel")).clicked() {
                                p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        if p.done
                            && ui
                                .button(icon!(CLOSE))
                                .on_hover_text(tr("Dismiss"))
                                .clicked()
                        {
                            clear.push(name.clone());
                        }
                    });
                }
                if !clear.is_empty() {
                    let mut s = self.ctl.backend.shared.lock().unwrap();
                    for n in clear {
                        s.pulls.remove(&n);
                    }
                }
            },
        );

        kit::panel(ui, ic!(MODELS, "Downloaded models"), None, |ui| {
            ui.horizontal(|ui| {
                ui.label(icon!(SEARCH));
                ui.add(
                    egui::TextEdit::singleline(&mut self.model_filter)
                        .hint_text(tr("Filter by name"))
                        .desired_width(260.0),
                );
                ui.add_space(16.0);
                ui.label(tr("Load with context"));
                context_picker(ui, "load_ctx", &mut self.load_ctx);
            });
            ui.add_space(10.0);

            let filter = self.model_filter.to_lowercase();
            egui::Grid::new("models")
                .striped(true)
                .num_columns(7)
                .spacing([28.0, 12.0])
                .show(ui, |ui| {
                    for h in [
                        tr("Name"),
                        tr("Params"),
                        tr("Quant"),
                        tr("Family"),
                        tr("Size"),
                        tr("Modified"),
                        "",
                    ] {
                        ui.label(RichText::new(h).weak());
                    }
                    ui.end_row();
                    for m in models
                        .iter()
                        .filter(|m| filter.is_empty() || m.name.to_lowercase().contains(&filter))
                    {
                        let is_running = running.iter().any(|r| r.name == m.name);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&m.name).monospace().strong())
                                .on_hover_text(&m.digest);
                            if is_running {
                                kit::badge(ui, tr("loaded"), GREEN);
                            }
                        });
                        ui.label(&m.details.parameter_size);
                        ui.label(&m.details.quantization_level);
                        ui.label(&m.details.family);
                        ui.label(format_bytes(m.size));
                        ui.label(m.modified_at.get(..10).unwrap_or(&m.modified_at));
                        ui.horizontal(|ui| {
                            if busy.contains(&m.name) {
                                ui.spinner();
                                return;
                            }
                            if is_running {
                                if ui.button(ic!(EJECT, "Unload")).clicked() {
                                    self.ctl.backend.unload(&m.name);
                                }
                            } else if ui.button(ic!(PLAY, "Load")).clicked() {
                                let ctx = (self.load_ctx > 0).then_some(self.load_ctx);
                                self.ctl.backend.load(&m.name, ctx);
                            }
                            if ui
                                .button(icon!(INFO))
                                .on_hover_text(tr("Details"))
                                .clicked()
                            {
                                self.selected_model = Some(m.name.clone());
                                self.ctl.backend.show(&m.name);
                            }
                            if ui
                                .button(icon!(TIMER))
                                .on_hover_text(tr("Benchmark"))
                                .clicked()
                            {
                                self.bench_model = m.name.clone();
                                self.tab = crate::app::Tab::Performance;
                            }
                            if ui
                                .button(RichText::new(icon!(DELETE)).color(RED))
                                .on_hover_text(tr("Delete"))
                                .clicked()
                            {
                                self.confirm_delete = Some(m.name.clone());
                            }
                        });
                        ui.end_row();
                    }
                });
        });

        self.delete_dialog(ui.ctx());
        self.info_window(ui.ctx());
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(name) = self.confirm_delete.clone() else {
            return;
        };
        egui::Window::new(tr("Delete model?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(trf!("Permanently delete {name} from disk?", name = name));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new(tr("Delete")).color(RED)).clicked() {
                        self.ctl.backend.delete(&name);
                        self.confirm_delete = None;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        self.confirm_delete = None;
                    }
                });
            });
    }

    fn info_window(&mut self, ctx: &egui::Context) {
        let Some(name) = self.selected_model.clone() else {
            return;
        };
        let detail = self
            .ctl
            .backend
            .shared
            .lock()
            .unwrap()
            .details
            .get(&name)
            .cloned();
        let mut open = true;
        egui::Window::new(format!("ℹ {name}"))
            .open(&mut open)
            .default_size([620.0, 480.0])
            .show(ctx, |ui| match detail {
                None => {
                    ui.spinner();
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(RED));
                }
                Some(Ok(d)) => {
                    egui::Grid::new("info")
                        .num_columns(2)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            let mut row = |k: &str, v: String| {
                                ui.label(RichText::new(k).strong());
                                ui.label(v);
                                ui.end_row();
                            };
                            row(tr("Family"), d.details.family.clone());
                            row(tr("Parameters"), d.details.parameter_size.clone());
                            row(tr("Quantization"), d.details.quantization_level.clone());
                            row(tr("Format"), d.details.format.clone());
                            row(
                                tr("Native context"),
                                d.native_context_length()
                                    .map(|c| c.to_string())
                                    .unwrap_or_else(|| "-".into()),
                            );
                            row(tr("Capabilities"), d.capabilities.join(", "));
                        });
                    ui.add_space(10.0);
                    let tab_id = egui::Id::unique("model_info_tab");
                    let mut tab: usize = ui.data(|d| d.get_temp(tab_id)).unwrap_or(0);
                    let model_info = d
                        .model_info
                        .iter()
                        .map(|(k, v)| (k, v.to_string()))
                        .filter(|(_, v)| v.len() < 200)
                        .map(|(k, v)| format!("{k}: {v}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let sections = [
                        (tr("Parameters"), d.parameters.as_str()),
                        (tr("Template"), d.template.as_str()),
                        (tr("Modelfile"), d.modelfile.as_str()),
                        (tr("Model info"), model_info.as_str()),
                        (tr("License"), d.license.as_str()),
                    ];
                    let mut options = Vec::new();
                    for (i, (label, content)) in sections.iter().enumerate() {
                        if !content.is_empty() {
                            options.push((i, *label));
                        }
                    }
                    if !options.iter().any(|(i, _)| *i == tab) {
                        tab = options.first().map(|(i, _)| *i).unwrap_or(0);
                    }
                    kit::choice(ui, &mut tab, &options);
                    ui.data_mut(|d| d.insert_temp(tab_id, tab));
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .corner_radius(8.0)
                        .inner_margin(12.0)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(320.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    let content = sections[tab].1;
                                    if tab == 4 {
                                        ui.label(content);
                                    } else {
                                        ui.label(RichText::new(content).monospace());
                                    }
                                });
                        });
                }
            });
        if !open {
            self.selected_model = None;
        }
    }
}
