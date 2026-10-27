use eframe::egui::{self, Color32, RichText};

use crate::app::{AMBER, App, GREEN, RED};
use crate::ferron;
use crate::gpu::GpuInfo;
use crate::i18n::tr;
use crate::util::{format_bytes, format_duration_secs};
use crate::views::kit;

impl App {
    pub fn dashboard(&mut self, ui: &mut egui::Ui) {
        let s = self.ctl.backend.shared.lock().unwrap();
        let last = s.history.back().cloned();
        let running = s.running.clone();
        let n_models = s.models.len();
        let disk: u64 = s.models.iter().map(|m| m.size).sum();
        let gpus = s.gpus.clone();
        let connected = s.connected;
        let version = s.version.clone();
        drop(s);

        ui.heading(tr("Overview"));
        ui.add_space(8.0);
        ui.columns(4, |cols| {
            let (value, sub) = if connected {
                (
                    RichText::new(tr("Online")).color(GREEN),
                    format!("v{}", version.unwrap_or_default()),
                )
            } else {
                (
                    RichText::new(tr("Offline")).color(RED),
                    tr("API unreachable").into(),
                )
            };
            kit::tile(&mut cols[0], tr("Server"), value, Some(sub));

            let vram: u64 = running.iter().map(|m| m.size_vram).sum();
            kit::tile(
                &mut cols[1],
                tr("Running models"),
                RichText::new(running.len().to_string()),
                Some(format!("{} in VRAM", format_bytes(vram))),
            );
            kit::tile(
                &mut cols[2],
                tr("Downloaded models"),
                RichText::new(n_models.to_string()),
                Some(format!("{} on disk", format_bytes(disk))),
            );
            match &last {
                Some(sample) => kit::tile(
                    &mut cols[3],
                    "CPU",
                    RichText::new(format!("{:.0}%", sample.cpu)),
                    Some(format!(
                        "RAM {} / {}",
                        format_bytes(sample.ram_used),
                        format_bytes(sample.ram_total)
                    )),
                ),
                None => kit::tile(&mut cols[3], "CPU", RichText::new("…"), None),
            }
        });
        ui.add_space(14.0);

        ui.columns(2, |cols| {
            kit::panel(&mut cols[0], ic!(SERVER, "Server"), None, |ui| {
                kit::kv_grid(ui, "dash_server", |ui| {
                    ui.label(tr("Mode"));
                    ui.label(if self.ctl.settings.server.manage_server {
                        tr("Managed by this app")
                    } else {
                        tr("External (service / tray app)")
                    });
                    ui.end_row();
                    ui.label(tr("Listening on"));
                    ui.label(format!(
                        "{}:{}",
                        self.ctl.settings.effective_bind_host(),
                        self.ctl.settings.server.port
                    ));
                    ui.end_row();
                    if let Some(started) = self.ctl.ollama_proc.started {
                        ui.label(tr("Uptime"));
                        ui.label(format_duration_secs(started.elapsed().as_secs() as i64));
                        ui.end_row();
                    }
                    if let Some(sample) = &last {
                        ui.label(tr("Ollama processes"));
                        ui.label(format!(
                            "{:.0}% CPU · {}",
                            sample.ollama_cpu,
                            format_bytes(sample.ollama_mem)
                        ));
                        ui.end_row();
                    }
                });
            });
            kit::panel(&mut cols[1], ic!(LOCK, "Network & security"), None, |ui| {
                kit::kv_grid(ui, "dash_net", |ui| {
                    ui.label(tr("HTTPS proxy"));
                    if self.ctl.settings.proxy.enabled {
                        let (text, color) = if self.ctl.ferron_proc.is_running() {
                            (tr("Running"), GREEN)
                        } else {
                            (tr("Stopped"), AMBER)
                        };
                        kit::badge(ui, text, color);
                    } else {
                        kit::badge(ui, tr("Disabled"), Color32::GRAY);
                    }
                    ui.end_row();
                    if self.ctl.settings.proxy.enabled {
                        ui.label(tr("Public URL"));
                        ui.add(
                            egui::Label::new(
                                RichText::new(ferron::public_url(&self.ctl.settings)).monospace(),
                            )
                            .wrap(),
                        );
                        ui.end_row();
                        ui.label(tr("API token"));
                        if self.ctl.settings.proxy.require_token {
                            kit::badge(
                                ui,
                                trf!(
                                    "{count} active",
                                    count = self.ctl.settings.proxy.tokens.len()
                                ),
                                GREEN,
                            );
                        } else {
                            kit::badge(ui, tr("Not required"), RED);
                        }
                        ui.end_row();
                    }
                });
            });
        });

        ui.heading(tr("GPUs"));
        ui.add_space(8.0);
        if gpus.is_empty() {
            kit::panel(ui, tr("No GPU detected"), None, |ui| {
                ui.label(RichText::new(tr("Inference will run on the CPU.")).weak());
            });
        } else {
            let per_row = 2;
            for chunk in gpus.chunks(per_row) {
                ui.columns(per_row, |cols| {
                    for (col, g) in cols.iter_mut().zip(chunk) {
                        self.gpu_card(col, g);
                    }
                });
            }
        }

        ui.heading(tr("Running models"));
        ui.add_space(8.0);
        kit::panel(ui, ic!(MODELS, "Loaded in memory"), None, |ui| {
            if running.is_empty() {
                ui.label(RichText::new(tr("No model loaded.")).weak());
            } else {
                self.running_table(ui, &running);
            }
        });
    }

    /// Compact GPU summary box (used on the dashboard and the GPU tab).
    pub fn gpu_card(&self, ui: &mut egui::Ui, g: &GpuInfo) {
        kit::panel_with_actions(
            ui,
            &g.name,
            |ui| {
                let enabled = self.ctl.settings.is_gpu_enabled(g);
                kit::badge(
                    ui,
                    if enabled {
                        tr("Enabled")
                    } else {
                        tr("Disabled")
                    },
                    if enabled { GREEN } else { Color32::GRAY },
                );
                kit::badge(ui, g.kind.label(), Color32::LIGHT_BLUE);
                kit::badge(ui, g.vendor.label(), Color32::LIGHT_GRAY);
            },
            |ui| {
                let width = ui.available_width();
                match g.vram_used {
                    Some(used) if g.vram_total > 0 => kit::meter(
                        ui,
                        used as f32 / g.vram_total as f32,
                        format!(
                            "VRAM {} / {}",
                            format_bytes(used),
                            format_bytes(g.vram_total)
                        ),
                        width,
                    ),
                    _ => {
                        ui.label(format!("VRAM {}", format_bytes(g.vram_total)));
                    }
                }
                ui.horizontal(|ui| {
                    if let Some(u) = g.utilization {
                        ui.label(RichText::new(format!("{u:.0}%")).size(20.0).strong());
                        ui.label(RichText::new(tr("utilisation")).weak());
                        ui.add_space(16.0);
                    }
                    if let Some(t) = g.temperature {
                        ui.label(RichText::new(format!("{t:.0}°C")).size(20.0).strong());
                        ui.label(RichText::new(tr("temperature")).weak());
                        ui.add_space(16.0);
                    }
                    if let Some(driver) = &g.driver {
                        ui.label(RichText::new(trf!("Driver {driver}", driver = driver)).weak());
                    }
                });
            },
        );
    }
}
