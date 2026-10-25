use eframe::egui::{self, RichText};

use crate::app::App;
use crate::views::kit;

const LAUNCHERS: &[(&str, &str, &str)] = &[
    ("claude", "Claude Code", "Anthropic's coding agent"),
    ("codex", "Codex", "OpenAI's coding agent"),
    ("opencode", "OpenCode", "Open source coding agent"),
    ("pi", "Pi", "Minimal coding agent toolkit"),
    ("omp", "OMP", "AI coding agent with IDE integration"),
    ("hermes", "Hermes", "Personal AI agent"),
    ("openclaw", "OpenClaw", "Personal AI assistant"),
    ("copilot", "Copilot CLI", "GitHub's coding agent"),
    ("cline", "Cline", "Autonomous coding agent"),
    ("droid", "Droid", "Factory's coding agent"),
];

impl App {
    pub fn launcher_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Launch with Ollama");
        ui.label(RichText::new("Choose an agent to see its launch command.").weak());
        ui.add_space(12.0);
        ui.columns(2, |columns| {
            kit::panel(&mut columns[0], "Agents", None, |ui| {
                for (key, name, description) in LAUNCHERS {
                    if ui
                        .selectable_label(self.launcher_selected == *key, *name)
                        .on_hover_text(*description)
                        .clicked()
                    {
                        self.launcher_selected = (*key).into();
                    }
                }
            });
            let command = format!("ollama launch {}", self.launcher_selected);
            kit::panel(&mut columns[1], "Command", None, |ui| {
                ui.label("Run this from the project directory:");
                ui.add_space(8.0);
                ui.code(&command);
                if ui.button("Copy command").clicked() {
                    ui.ctx().copy_text(command.clone());
                }
            });
        });
    }
}
