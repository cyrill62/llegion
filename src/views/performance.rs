use eframe::egui::{self, RichText};
use egui_plot::{Legend, Line, Plot, PlotPoints};

use crate::app::App;
use crate::backend::{BenchRecord, Sample};
use crate::i18n::tr;
use crate::ollama::{LocalModel, RunningModel};
use crate::util::{ctx_short, format_bytes};
use crate::views::kit;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

fn series(history: &[Sample], now: f64, f: impl Fn(&Sample) -> Option<f64>) -> PlotPoints<'static> {
    history
        .iter()
        .filter_map(|s| f(s).map(|y| [s.t - now, y]))
        .collect::<Vec<[f64; 2]>>()
        .into()
}

fn chart(
    ui: &mut egui::Ui,
    id: &str,
    y_max: Option<f64>,
    unit: &'static str,
    lines: Vec<Line<'static>>,
) {
    let mut plot = Plot::new(id.to_string())
        .height(190.0)
        .legend(Legend::default())
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .include_y(0.0)
        .x_axis_label(tr("seconds"))
        .y_axis_formatter(move |mark, _| format!("{:.0}{unit}", mark.value));
    if let Some(max) = y_max {
        plot = plot.include_y(max);
    }
    plot.show(ui, |p| {
        for line in lines {
            p.line(line);
        }
    });
}

impl App {
    pub fn performance_tab(&mut self, ui: &mut egui::Ui) {
        let (history, gpus, models, running, benchmarks, bench_running) = {
            let s = self.ctl.backend.shared.lock().unwrap();
            (
                s.history.iter().cloned().collect::<Vec<_>>(),
                s.gpus.clone(),
                s.models.clone(),
                s.running.clone(),
                s.benchmarks.clone(),
                s.bench_running,
            )
        };
        let now = history.last().map(|s| s.t).unwrap_or(0.0);

        ui.heading(tr("Performance"));
        ui.add_space(8.0);
        if let Some(last) = history.last() {
            ui.columns(4, |cols| {
                kit::tile(
                    &mut cols[0],
                    tr("System CPU"),
                    RichText::new(format!("{:.0}%", last.cpu)),
                    None,
                );
                kit::tile(
                    &mut cols[1],
                    tr("System RAM"),
                    RichText::new(format_bytes(last.ram_used)),
                    Some(format!("of {}", format_bytes(last.ram_total))),
                );
                kit::tile(
                    &mut cols[2],
                    "Ollama",
                    RichText::new(format!("{:.0}%", last.ollama_cpu)),
                    Some(format!("{} RAM", format_bytes(last.ollama_mem))),
                );
                kit::tile(
                    &mut cols[3],
                    tr("Models in VRAM"),
                    RichText::new(format_bytes(last.models_vram)),
                    None,
                );
            });
            ui.add_space(14.0);
        }

        let has_util = history
            .iter()
            .any(|s| s.gpus.iter().any(|g| g.util.is_some()));
        let has_vram = history
            .iter()
            .any(|s| s.gpus.iter().any(|g| g.vram_used.is_some()));

        ui.columns(2, |cols| {
            kit::panel(&mut cols[0], tr("CPU usage (%)"), None, |ui| {
                chart(
                    ui,
                    "cpu",
                    Some(100.0),
                    "%",
                    vec![
                        Line::new(tr("System"), series(&history, now, |s| Some(s.cpu as f64))),
                        Line::new(
                            "Ollama",
                            series(&history, now, |s| Some(s.ollama_cpu as f64)),
                        ),
                    ],
                );
            });
            kit::panel(&mut cols[1], tr("Memory (GiB)"), None, |ui| {
                let ram_total = history.last().map(|s| s.ram_total as f64 / GIB);
                chart(
                    ui,
                    "ram",
                    ram_total,
                    "",
                    vec![
                        Line::new(
                            tr("System RAM"),
                            series(&history, now, |s| Some(s.ram_used as f64 / GIB)),
                        ),
                        Line::new(
                            tr("Ollama RAM"),
                            series(&history, now, |s| Some(s.ollama_mem as f64 / GIB)),
                        ),
                    ],
                );
            });
        });

        if gpus.is_empty() {
            kit::panel(ui, "GPU", None, |ui| {
                ui.label(RichText::new(tr("No GPU detected.")).weak());
            });
        } else if !has_util && !has_vram {
            kit::panel(ui, "GPU", None, |ui| {
                ui.label(RichText::new(tr("Live GPU metrics are not available for this GPU/OS (supported: NVIDIA via nvidia-smi, AMD on Linux, and any GPU on Windows).")).weak());
            });
        } else {
            ui.columns(2, |cols| {
                kit::panel(&mut cols[0], tr("GPU utilisation (%)"), None, |ui| {
                    let lines = gpus
                        .iter()
                        .enumerate()
                        .map(|(i, g)| {
                            Line::new(
                                format!("{i}: {}", g.name),
                                series(&history, now, move |s| {
                                    s.gpus.get(i).and_then(|g| g.util).map(f64::from)
                                }),
                            )
                        })
                        .collect();
                    chart(ui, "gpu_util", Some(100.0), "%", lines);
                });
                kit::panel(&mut cols[1], tr("VRAM used (GiB)"), None, |ui| {
                    let max = gpus.iter().map(|g| g.vram_total).max().unwrap_or(0) as f64 / GIB;
                    let lines = gpus
                        .iter()
                        .enumerate()
                        .map(|(i, g)| {
                            Line::new(
                                format!("{i}: {}", g.name),
                                series(&history, now, move |s| {
                                    s.gpus
                                        .get(i)
                                        .and_then(|g| g.vram_used)
                                        .map(|v| v as f64 / GIB)
                                }),
                            )
                        })
                        .collect();
                    chart(ui, "gpu_vram", Some(max), "", lines);
                });
            });
        }

        // Side by side when there is room, the results getting the larger share.
        if benchmarks.is_empty() {
            self.bench_form(ui, &models, &running, bench_running);
        } else if ui.available_width() >= 1400.0 {
            ui.horizontal_top(|ui| {
                let left = (ui.available_width() - ui.spacing().item_spacing.x) * 0.38;
                ui.vertical(|ui| {
                    ui.set_width(left);
                    self.bench_form(ui, &models, &running, bench_running);
                });
                ui.vertical(|ui| self.bench_results(ui, &benchmarks));
            });
        } else {
            self.bench_form(ui, &models, &running, bench_running);
            self.bench_results(ui, &benchmarks);
        }
    }

    fn bench_form(
        &mut self,
        ui: &mut egui::Ui,
        models: &[LocalModel],
        running: &[RunningModel],
        bench_running: bool,
    ) {
        kit::panel(
            ui,
            ic!(TIMER, "Benchmark"),
            Some(tr(
                "Runs one non-streaming generation and reports Ollama's own timing counters.",
            )),
            |ui| {
                kit::form(ui, "bench_form", |ui| {
                    kit::field(ui, tr("Model"), None);
                    egui::ComboBox::from_id_salt("bench_model")
                        .selected_text(if self.bench_model.is_empty() {
                            tr("Select…")
                        } else {
                            &self.bench_model
                        })
                        .width(360.0)
                        .show_ui(ui, |ui| {
                            for m in models {
                                ui.selectable_value(&mut self.bench_model, m.name.clone(), &m.name);
                            }
                        });
                    ui.end_row();
                    kit::field(ui, tr("Prompt"), None);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.bench_prompt)
                            .desired_rows(3)
                            .desired_width(ui.available_width().min(560.0)),
                    );
                    ui.end_row();
                    kit::field(ui, tr("Tokens to generate"), None);
                    ui.add(egui::DragValue::new(&mut self.bench_tokens).range(16..=4096));
                    ui.end_row();
                    kit::field(ui, tr("Context"), None);
                    ui.horizontal(|ui| {
                        crate::views::models::context_picker(ui, "bench_ctx", &mut self.bench_ctx)
                    });
                    ui.end_row();
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let can_run = !self.bench_model.is_empty() && !bench_running;
                    if ui
                        .add_enabled(can_run, egui::Button::new(ic!(TIMER, "Run benchmark")))
                        .clicked()
                    {
                        let ctx = (self.bench_ctx > 0).then_some(self.bench_ctx);
                        self.ctl.backend.benchmark(
                            &self.bench_model,
                            &self.bench_prompt,
                            self.bench_tokens,
                            ctx,
                            self.ctl.server_env(),
                        );
                    }
                    if bench_running {
                        ui.spinner();
                        ui.label(tr("Running…"));
                    }
                    if let Some(r) = running.iter().find(|r| r.name == self.bench_model) {
                        let ctx = r
                            .context_length
                            .map(|c| format!(" · {} ctx", ctx_short(c)))
                            .unwrap_or_default();
                        kit::badge(
                            ui,
                            format!(
                                "{}{ctx}",
                                trf!("loaded · {processor}", processor = r.processor())
                            ),
                            crate::app::GREEN,
                        );
                    }
                });
            },
        );
    }

    fn bench_results(&mut self, ui: &mut egui::Ui, benchmarks: &[BenchRecord]) {
        kit::panel_with_actions(
            ui,
            tr("Results"),
            |ui| {
                if ui.button(tr("Clear results")).clicked() {
                    self.ctl.backend.clear_benchmarks();
                }
            },
            |ui| {
                egui::Grid::new("bench_results")
                    .striped(true)
                    .num_columns(10)
                    .spacing([28.0, 12.0])
                    .show(ui, |ui| {
                        for h in [
                            tr("Time"),
                            tr("Model"),
                            tr("Context"),
                            tr("Processor"),
                            tr("Server"),
                            tr("Prompt tok/s"),
                            tr("Gen tok/s"),
                            tr("Tokens"),
                            tr("Load"),
                            tr("Total"),
                        ] {
                            ui.label(RichText::new(h).weak());
                        }
                        ui.end_row();
                        for b in benchmarks.iter().rev() {
                            ui.label(&b.when);
                            ui.label(RichText::new(&b.model).monospace().strong());
                            let requested = b
                                .num_ctx
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "auto".into());
                            ui.label(
                                b.loaded_ctx
                                    .map(|c| c.to_string())
                                    .unwrap_or_else(|| requested.clone()),
                            )
                            .on_hover_text(trf!(
                                "Requested: {requested}\nLoaded: {loaded}",
                                requested = requested,
                                loaded = b
                                    .loaded_ctx
                                    .map(|c| c.to_string())
                                    .unwrap_or_else(|| tr("unknown").into())
                            ));
                            ui.label(b.processor.as_deref().unwrap_or("-"));
                            server_cell(ui, b);
                            ui.label(format!("{:.1}", b.stats.prompt_tps()));
                            ui.label(
                                RichText::new(format!("{:.1}", b.stats.gen_tps()))
                                    .size(18.0)
                                    .strong(),
                            );
                            ui.label(format!(
                                "{} + {}",
                                b.stats.prompt_eval_count, b.stats.eval_count
                            ));
                            ui.label(format!("{:.2}s", b.stats.load_duration as f64 / 1e9));
                            ui.label(format!("{:.2}s", b.stats.total_duration as f64 / 1e9));
                            ui.end_row();
                        }
                    });
            },
        );
    }
}

/// Compact summary of the server config a benchmark ran under; the full env on hover.
fn server_cell(ui: &mut egui::Ui, b: &BenchRecord) {
    let text = RichText::new(b.server_summary());
    let text = if b.server_env.is_none() {
        text.weak()
    } else {
        text
    };
    ui.label(text).on_hover_text(b.server_details());
}
