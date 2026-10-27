use eframe::egui::{self, RichText};

use crate::app::{AMBER, App};
use crate::config::GpuMode;
use crate::gpu::GpuKind;
use crate::i18n::tr;
use crate::views::kit;

impl App {
    pub fn gpu_tab(&mut self, ui: &mut egui::Ui) {
        let (gpus, detected) = {
            let s = self.ctl.backend.shared.lock().unwrap();
            (s.gpus.clone(), s.gpus_detected)
        };

        ui.horizontal(|ui| {
            ui.heading("GPU");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(ic!(SEARCH, "Re-detect")).clicked() {
                    self.ctl.backend.redetect_gpus();
                }
            });
        });
        ui.add_space(8.0);

        if !detected {
            ui.spinner();
        } else if gpus.is_empty() {
            kit::panel(ui, tr("No GPU detected"), None, |ui| {
                ui.label(RichText::new(tr("Ollama will run on the CPU.")).weak());
            });
        }
        for (i, g) in gpus.iter().enumerate() {
            self.gpu_card(ui, g);
            // Per-device switch, only meaningful in Custom mode.
            if self.draft.gpu.mode == GpuMode::Custom {
                let key = g.key();
                let forced_off = self.draft.gpu.disable_igpu && g.kind == GpuKind::Integrated;
                let mut enabled = !forced_off && !self.draft.gpu.disabled_devices.contains(&key);
                ui.add_enabled_ui(!forced_off, |ui| {
                    if kit::switch(ui, &mut enabled, &format!("Use GPU #{i} ({})", g.name))
                        .changed()
                    {
                        self.draft.gpu.disabled_devices.retain(|k| k != &key);
                        if !enabled {
                            self.draft.gpu.disabled_devices.push(key);
                        }
                    }
                });
                ui.add_space(12.0);
            }
        }

        kit::panel(
            ui,
            ic!(SETTINGS, "Configuration"),
            Some(tr("Changes take effect when the Ollama server restarts.")),
            |ui| {
                kit::setting(
                    ui,
                    tr("Mode"),
                    Some(tr("Which devices Ollama may use")),
                    |ui| {
                        kit::choice(
                            ui,
                            &mut self.draft.gpu.mode,
                            &[
                                (GpuMode::Auto, tr("Auto")),
                                (GpuMode::Custom, tr("Custom")),
                                (GpuMode::CpuOnly, tr("CPU only")),
                            ],
                        );
                    },
                );
                ui.add_enabled_ui(self.draft.gpu.mode != GpuMode::CpuOnly, |ui| {
                    kit::setting(
                        ui,
                        tr("Disable integrated GPUs"),
                        Some(tr(
                            "iGPUs share system RAM and are often slower than the CPU",
                        )),
                        |ui| kit::switch(ui, &mut self.draft.gpu.disable_igpu, ""),
                    );
                    kit::setting(
                        ui,
                        tr("Vulkan backend"),
                        Some(tr("OLLAMA_VULKAN=1, for Intel and some AMD (experimental)")),
                        |ui| kit::switch(ui, &mut self.draft.gpu.enable_vulkan, ""),
                    );
                });
                ui.add_space(8.0);
                self.save_buttons(ui);
            },
        );

        kit::panel(
            ui,
            tr("Resulting environment"),
            Some(tr(
                "Variables passed to `ollama serve`. For a system service, copy them from Settings → Server.",
            )),
            |ui| {
                let env = self.draft.gpu_env(&gpus);
                if env.is_empty() {
                    ui.label(RichText::new(tr("None — Ollama defaults.")).weak());
                } else {
                    let text: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    kit::code_block(ui, &text.join("\n"));
                }
                if self.draft.gpu.mode == GpuMode::CpuOnly {
                    ui.label(RichText::new(tr("All GPUs are hidden from Ollama.")).color(AMBER));
                }
            },
        );
    }
}
