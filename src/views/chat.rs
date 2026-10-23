use std::time::Duration;

use eframe::egui::{self, RichText};

use crate::app::{AMBER, App, GREEN, RED};
use crate::backend::{ChatEntry, ChatOptions, Reply};
use crate::ollama::LocalModel;
use crate::views::kit;

/// What a click in the message list asks for, applied once the list is drawn.
enum Action {
    Edit(usize),
    /// Streams a (new variant of the) answer at this index.
    Reply {
        at: usize,
        model: String,
    },
    Select {
        at: usize,
        variant: usize,
    },
}

/// Menu listing every downloaded model; returns the one picked.
fn model_menu(
    ui: &mut egui::Ui,
    label: &str,
    models: &[LocalModel],
    current: &str,
) -> Option<String> {
    let mut picked = None;
    ui.menu_button(label, |ui| {
        for m in models {
            let mut text = RichText::new(&m.name).monospace();
            if m.name == current {
                text = text.strong();
            }
            if ui.button(text).clicked() {
                picked = Some(m.name.clone());
                ui.close();
            }
        }
    });
    picked
}

fn stats_line(r: &Reply) -> Option<String> {
    let ttft = r
        .first_token
        .map(|d| format!("first token {:.2}s", d.as_secs_f64()));
    if let Some(st) = &r.stats {
        let mut parts = vec![
            format!("{:.1} tok/s", st.gen_tps()),
            format!("{} tokens", st.eval_count),
        ];
        parts.extend(ttft);
        parts.push(format!(
            "prompt {} tokens at {:.0} tok/s",
            st.prompt_eval_count,
            st.prompt_tps()
        ));
        if st.load_duration > 50_000_000 {
            parts.push(format!("load {:.2}s", st.load_duration as f64 / 1e9));
        }
        parts.push(format!("total {:.2}s", st.total_duration as f64 / 1e9));
        return Some(parts.join(" · "));
    }
    let mut parts = Vec::new();
    if let Some(tps) = r.live_tps() {
        parts.push(format!("~{tps:.1} tok/s"));
    }
    if r.chunks > 0 {
        parts.push(format!("{} tokens", r.chunks));
    }
    parts.extend(ttft);
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn transcript(entries: &[ChatEntry]) -> String {
    entries
        .iter()
        .filter_map(|e| match e {
            ChatEntry::User(text) => Some(format!("**You:**\n{text}")),
            ChatEntry::Assistant { variants, selected } => variants
                .get(*selected)
                .map(|r| format!("**{}:**\n{}", r.model, r.content)),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

impl App {
    fn chat_options(&self) -> ChatOptions {
        ChatOptions {
            system: self.chat_system.clone(),
            temperature: self.chat_temperature,
            num_ctx: (self.chat_ctx > 0).then_some(self.chat_ctx),
        }
    }

    pub fn chat_tab(&mut self, ui: &mut egui::Ui) {
        let (entries, streaming, models, running) = {
            let s = self.backend.shared.lock().unwrap();
            (
                s.chat.entries.clone(),
                s.chat.streaming,
                s.models.clone(),
                s.running.clone(),
            )
        };
        // Default to a model already in memory, so the first answer comes quickly.
        if !models.iter().any(|m| m.name == self.chat_model) {
            self.chat_model = running
                .iter()
                .find(|r| models.iter().any(|m| m.name == r.name))
                .map(|r| r.name.clone())
                .or_else(|| models.first().map(|m| m.name.clone()))
                .unwrap_or_default();
        }

        ui.horizontal(|ui| {
            ui.heading("Chat");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !entries.is_empty(),
                        egui::Button::new(ic!(NEW_CHAT, "New chat")),
                    )
                    .clicked()
                {
                    self.backend.shared.lock().unwrap().chat.clear();
                }
                if ui
                    .add_enabled(
                        !entries.is_empty(),
                        egui::Button::new(ic!(COPY, "Copy chat")),
                    )
                    .clicked()
                {
                    ui.ctx().copy_text(transcript(&entries));
                }
            });
        });
        ui.add_space(8.0);

        kit::panel(ui, ic!(TUNE, "Model & options"), None, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Model");
                egui::ComboBox::from_id_salt("chat_model")
                    .selected_text(if self.chat_model.is_empty() {
                        "No models"
                    } else {
                        &self.chat_model
                    })
                    .width(300.0)
                    .show_ui(ui, |ui| {
                        for m in &models {
                            ui.selectable_value(&mut self.chat_model, m.name.clone(), &m.name);
                        }
                    });
                if let Some(r) = running.iter().find(|r| r.name == self.chat_model) {
                    kit::badge(ui, format!("loaded · {}", r.processor()), GREEN);
                } else if !self.chat_model.is_empty() {
                    kit::badge(ui, "not loaded", AMBER)
                        .on_hover_text("The first answer includes the model load time.");
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Context");
                crate::views::models::context_picker(ui, "chat_ctx", &mut self.chat_ctx);
                ui.add_space(16.0);
                let mut custom = self.chat_temperature.is_some();
                if ui.checkbox(&mut custom, "Temperature").changed() {
                    self.chat_temperature = custom.then_some(0.7);
                }
                match &mut self.chat_temperature {
                    Some(t) => {
                        ui.add(egui::Slider::new(t, 0.0..=2.0).step_by(0.05));
                    }
                    None => {
                        ui.label(RichText::new("model default").weak());
                    }
                }
            });
            egui::CollapsingHeader::new("System prompt")
                .id_salt("chat_system")
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.chat_system)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY)
                            .hint_text("Optional instructions sent before the conversation"),
                    );
                });
        });

        egui::Panel::bottom("chat_input")
            .resizable(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                top: 12,
                ..Default::default()
            }))
            .show(ui, |ui| self.chat_input_row(ui, streaming));

        let mut action = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ui, |ui| {
                if entries.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        let hint = if models.is_empty() {
                            "No models downloaded yet. Pull one in the Models tab."
                        } else {
                            "Send a message to start. Under each message you can copy it, edit it, \
                             or regenerate the answer with another model to compare speed and quality."
                        };
                        ui.label(RichText::new(hint).weak());
                    });
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for (i, entry) in entries.iter().enumerate() {
                            let a = match entry {
                                ChatEntry::User(text) => {
                                    user_message(ui, i, text, &models, &self.chat_model, streaming)
                                }
                                ChatEntry::Assistant { variants, selected } => {
                                    assistant_message(ui, i, variants, *selected, &models, streaming)
                                }
                            };
                            action = action.take().or(a);
                        }
                    });
            });

        if streaming {
            // Keeps the elapsed time and live speed moving between chunks.
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        match action {
            Some(Action::Edit(at)) => {
                let mut s = self.backend.shared.lock().unwrap();
                if let Some(ChatEntry::User(text)) = s.chat.entries.get(at) {
                    self.chat_input = text.clone();
                    s.chat.entries.truncate(at);
                }
                drop(s);
                ui.memory_mut(|m| m.request_focus(egui::Id::unique("chat_input")));
            }
            Some(Action::Reply { at, model }) => {
                let opts = self.chat_options();
                self.backend.chat_reply(at, &model, &opts);
            }
            Some(Action::Select { at, variant }) => {
                let mut s = self.backend.shared.lock().unwrap();
                if let Some(ChatEntry::Assistant { variants, selected }) =
                    s.chat.entries.get_mut(at)
                    && variant < variants.len()
                {
                    *selected = variant;
                }
            }
            None => {}
        }
    }

    fn chat_input_row(&mut self, ui: &mut egui::Ui, streaming: bool) {
        let id = egui::Id::unique("chat_input");
        // Enter sends, Shift+Enter inserts a new line.
        let enter = ui.memory(|m| m.has_focus(id))
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.chat_input)
                    .id(id)
                    .desired_rows(3)
                    .desired_width(ui.available_width() - 130.0)
                    .hint_text("Message… (Enter to send, Shift+Enter for a new line)"),
            );
            if streaming {
                if ui.button(ic!(STOP, "Stop")).clicked() {
                    self.backend.shared.lock().unwrap().chat.stop();
                }
                return;
            }
            let can_send = !self.chat_input.trim().is_empty() && !self.chat_model.is_empty();
            let clicked = ui
                .add_enabled(can_send, egui::Button::new(ic!(SEND, "Send")))
                .clicked();
            if can_send && (clicked || enter) {
                let text = std::mem::take(&mut self.chat_input).trim().to_string();
                let opts = self.chat_options();
                self.backend.chat_send(text, &self.chat_model, &opts);
                ui.memory_mut(|m| m.request_focus(id));
            }
        });
    }
}

fn message_frame(ui: &mut egui::Ui, user: bool, body: impl FnOnce(&mut egui::Ui)) {
    let mut frame = kit::card_frame(ui).inner_margin(14.0);
    if user {
        frame = frame.fill(ui.visuals().selection.bg_fill.gamma_multiply(0.15));
    }
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui);
    });
    ui.add_space(8.0);
}

fn user_message(
    ui: &mut egui::Ui,
    at: usize,
    text: &str,
    models: &[LocalModel],
    current: &str,
    streaming: bool,
) -> Option<Action> {
    let mut action = None;
    message_frame(ui, true, |ui| {
        ui.label(RichText::new(ic!(PERSON, "You")).strong());
        ui.label(text);
        ui.horizontal(|ui| {
            if ui.small_button(ic!(COPY, "Copy")).clicked() {
                ui.ctx().copy_text(text.to_string());
            }
            ui.add_enabled_ui(!streaming, |ui| {
                if ui
                    .small_button(ic!(EDIT, "Edit"))
                    .on_hover_text(
                        "Put this message back in the input and remove it and what follows",
                    )
                    .clicked()
                {
                    action = Some(Action::Edit(at));
                }
                if let Some(model) = model_menu(ui, ic!(SEND, "Resend to…"), models, current) {
                    action = Some(Action::Reply { at: at + 1, model });
                }
            });
        });
    });
    action
}

fn assistant_message(
    ui: &mut egui::Ui,
    at: usize,
    variants: &[Reply],
    selected: usize,
    models: &[LocalModel],
    streaming: bool,
) -> Option<Action> {
    let r = variants.get(selected)?;
    let mut action = None;
    message_frame(ui, false, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon!(BOT)).strong());
            // Separate label: the monospace font has no icon glyphs.
            ui.label(RichText::new(&r.model).strong().monospace());
            if variants.len() > 1 {
                ui.add_enabled_ui(!streaming, |ui| {
                    if ui
                        .add_enabled(selected > 0, egui::Button::new(icon!(PREV)).small())
                        .clicked()
                    {
                        action = Some(Action::Select {
                            at,
                            variant: selected - 1,
                        });
                    }
                    ui.label(format!("{}/{}", selected + 1, variants.len()));
                    if ui
                        .add_enabled(
                            selected + 1 < variants.len(),
                            egui::Button::new(icon!(NEXT)).small(),
                        )
                        .clicked()
                    {
                        action = Some(Action::Select {
                            at,
                            variant: selected + 1,
                        });
                    }
                });
            }
        });
        if !r.thinking.is_empty() {
            egui::CollapsingHeader::new(RichText::new(ic!(THINK, "Thinking")).weak())
                .id_salt(("chat_think", at, selected))
                .show(ui, |ui| {
                    ui.label(RichText::new(r.thinking.trim()).weak().italics());
                });
        }
        if !r.content.is_empty() {
            ui.label(r.content.trim_start());
        } else if !r.done {
            ui.horizontal(|ui| {
                ui.spinner();
                let what = if r.thinking.is_empty() {
                    "Waiting for the first token"
                } else {
                    "Thinking"
                };
                ui.label(
                    RichText::new(format!("{what}… {:.1}s", r.started.elapsed().as_secs_f64()))
                        .weak(),
                );
            });
        }
        if let Some(err) = &r.error {
            ui.label(RichText::new(format!("{} {err}", icon!(WARNING))).color(RED));
        }
        if r.stopped {
            ui.label(RichText::new("Stopped").color(AMBER));
        }
        if let Some(line) = stats_line(r) {
            ui.label(
                RichText::new(format!("{}  {line}", icon!(SPEED)))
                    .small()
                    .weak(),
            );
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !r.content.is_empty(),
                    egui::Button::new(ic!(COPY, "Copy")).small(),
                )
                .clicked()
            {
                ui.ctx().copy_text(r.content.trim().to_string());
            }
            ui.add_enabled_ui(!streaming, |ui| {
                if let Some(model) =
                    model_menu(ui, ic!(REGENERATE, "Regenerate with…"), models, &r.model)
                {
                    action = Some(Action::Reply { at, model });
                }
            })
            .response
            .on_hover_text(
                "Adds another answer here (use ‹ › to compare); later messages are removed",
            );
        });
    });
    action
}
