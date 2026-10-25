use std::time::Duration;

use eframe::egui::{self, RichText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use crate::app::{AMBER, App, GREEN, RED};
use crate::backend::{ChatEntry, ChatOptions, Reply, preferred_model};
use crate::chat_store::{ChatConfig, ChatSummary};
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

fn chat_groups(chats: &[ChatSummary], current: &str) -> Vec<String> {
    let mut groups: Vec<_> = chats
        .iter()
        .map(|chat| chat.group.clone())
        .filter(|group| !group.is_empty())
        .collect();
    if !current.is_empty() {
        groups.push(current.to_owned());
    }
    groups.sort();
    groups.dedup();
    groups
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
        if !self.ctl.settings.ui.chat_warning_acknowledged {
            kit::panel(ui, ic!(WARNING, "Chat storage warning"), None, |ui| {
                ui.label("Chat history is saved locally without encryption. Anyone with access to this account can read it.");
                ui.label(crate::config::chat_store_path().display().to_string());
                if ui.button("I understand").clicked() {
                    self.ctl.settings.ui.chat_warning_acknowledged = true;
                    self.draft.ui.chat_warning_acknowledged = true;
                    if let Err(e) = self.ctl.settings.save() {
                        self.ctl.logs.app_error(format!(
                            "Failed to save chat warning acknowledgement: {e:#}"
                        ));
                    }
                }
            });
            return;
        }
        let (entries, streaming, models, running, saved_chats, active_id, active_group, config) = {
            let s = self.ctl.backend.shared.lock().unwrap();
            (
                s.chat.entries.clone(),
                s.chat.streaming,
                s.models.clone(),
                s.running.clone(),
                s.saved_chats.clone(),
                s.chat_id.clone(),
                s.chat_group.clone(),
                s.chat_config.clone(),
            )
        };
        if self.chat_active_id != active_id {
            self.chat_active_id.clone_from(&active_id);
            self.chat_model.clone_from(&config.model);
            self.chat_system.clone_from(&config.system);
            self.chat_ctx = config.context;
            self.chat_temperature = config.temperature;
            self.chat_markdown = config.markdown;
        }
        self.chat_model = preferred_model(&self.chat_model, &models, &running);

        let groups = chat_groups(&saved_chats, &active_group);
        let mut switched_chat = false;
        let mut delete_chat = None;
        ui.horizontal(|ui| {
            ui.heading("Chat");
            ui.scope(|ui| {
                ui.style_mut().always_scroll_the_only_direction = true;
                egui::ScrollArea::horizontal()
                    .id_salt("chat_history_tabs")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mut group_order = vec![String::new()];
                            group_order.extend(groups.iter().cloned());
                            let found_active = saved_chats.iter().any(|chat| chat.id == active_id);
                            for group in group_order {
                                if !group.is_empty() {
                                    let collapsed = self.collapsed_chat_groups.contains(&group);
                                    if ui
                                        .selectable_label(
                                            !collapsed,
                                            format!(
                                                "{} {group}",
                                                if collapsed { "▸" } else { "▾" }
                                            ),
                                        )
                                        .clicked()
                                    {
                                        if collapsed {
                                            self.collapsed_chat_groups.remove(&group);
                                        } else {
                                            self.collapsed_chat_groups.insert(group.clone());
                                        }
                                    }
                                    if collapsed {
                                        continue;
                                    }
                                }
                                for chat in saved_chats.iter().filter(|chat| chat.group == group) {
                                    if self.chat_rename_id.as_deref() == Some(&chat.id) {
                                        let response = ui.add(
                                            egui::TextEdit::singleline(&mut self.chat_rename_text)
                                                .desired_width(130.0),
                                        );
                                        response.request_focus();
                                        let commit = response.lost_focus()
                                            || (response.has_focus()
                                                && ui.input(|input| {
                                                    input.key_pressed(egui::Key::Enter)
                                                }));
                                        if commit {
                                            let result = self.ctl.backend.rename_saved_chat(
                                                &chat.id,
                                                self.chat_rename_text.clone(),
                                            );
                                            if let Err(e) = result {
                                                self.ctl.logs.app_error(format!(
                                                    "Failed to rename chat: {e:#}"
                                                ));
                                            }
                                            self.chat_rename_id = None;
                                        }
                                    } else {
                                        let response =
                                            ui.selectable_label(chat.id == active_id, &chat.title);
                                        if response.double_clicked() {
                                            self.chat_rename_id = Some(chat.id.clone());
                                            self.chat_rename_text.clone_from(&chat.title);
                                        } else if response.clicked() && chat.id != active_id {
                                            match self.ctl.backend.load_saved_chat(&chat.id) {
                                                Ok(()) => switched_chat = true,
                                                Err(e) => self.ctl.logs.app_error(format!(
                                                    "Failed to load saved chat: {e:#}"
                                                )),
                                            }
                                        }
                                    }
                                    if ui
                                        .small_button("×")
                                        .on_hover_text("Close and delete chat")
                                        .clicked()
                                    {
                                        delete_chat = Some(chat.id.clone());
                                    }
                                    ui.menu_button("⋯", |ui| {
                                        ui.label("Move to group");
                                        if ui.button("Ungroup").clicked()
                                            && let Err(e) = self
                                                .ctl
                                                .backend
                                                .set_saved_chat_group(&chat.id, String::new())
                                        {
                                            self.ctl
                                                .logs
                                                .app_error(format!("Failed to move chat: {e:#}"));
                                        }
                                        for target in &groups {
                                            if ui.button(target).clicked()
                                                && let Err(e) = self
                                                    .ctl
                                                    .backend
                                                    .set_saved_chat_group(&chat.id, target.clone())
                                            {
                                                self.ctl.logs.app_error(format!(
                                                    "Failed to move chat: {e:#}"
                                                ));
                                            }
                                        }
                                    });
                                }
                                if !found_active && group == active_group {
                                    let _ = ui.selectable_label(true, "New chat");
                                    if ui
                                        .small_button("×")
                                        .on_hover_text("Close and delete chat")
                                        .clicked()
                                    {
                                        delete_chat = Some(active_id.clone());
                                    }
                                }
                            }
                        });
                    });
            });
            ui.menu_button("+ Group", |ui| {
                ui.text_edit_singleline(&mut self.chat_group_name);
                if ui.button("Group current chat").clicked() {
                    let name = self.chat_group_name.trim().to_owned();
                    if !name.is_empty() {
                        self.ctl.backend.set_current_chat_group(name);
                    }
                    ui.close();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(ic!(NEW_CHAT, "New chat")).clicked() {
                    self.ctl.backend.new_chat();
                    switched_chat = true;
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
        if let Some(id) = delete_chat {
            if id == active_id {
                switched_chat = true;
            }
            if let Err(e) = self.ctl.backend.delete_saved_chat(&id) {
                self.ctl
                    .logs
                    .app_error(format!("Failed to delete chat: {e:#}"));
            }
        }
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
                ui.checkbox(&mut self.chat_markdown, "Render Markdown")
                    .on_hover_text("Off shows the raw text the model sent");
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
        if !switched_chat {
            self.ctl.backend.set_chat_config(ChatConfig {
                model: self.chat_model.clone(),
                system: self.chat_system.clone(),
                context: self.chat_ctx,
                temperature: self.chat_temperature,
                markdown: self.chat_markdown,
            });
        }

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
                                ChatEntry::Assistant { variants, selected } => assistant_message(
                                    ui,
                                    i,
                                    variants,
                                    *selected,
                                    &models,
                                    streaming,
                                    self.chat_markdown.then_some(&mut self.markdown_cache),
                                ),
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
                let mut s = self.ctl.backend.shared.lock().unwrap();
                if let Some(ChatEntry::User(text)) = s.chat.entries.get(at) {
                    self.chat_input = text.clone();
                    s.chat.entries.truncate(at);
                }
                drop(s);
                self.ctl.backend.save_current_chat();
                ui.memory_mut(|m| m.request_focus(egui::Id::unique("chat_input")));
            }
            Some(Action::Reply { at, model }) => {
                let opts = self.chat_options();
                self.ctl.backend.chat_reply(at, &model, &opts);
            }
            Some(Action::Select { at, variant }) => {
                let mut s = self.ctl.backend.shared.lock().unwrap();
                if let Some(ChatEntry::Assistant { variants, selected }) =
                    s.chat.entries.get_mut(at)
                    && variant < variants.len()
                {
                    *selected = variant;
                }
                drop(s);
                self.ctl.backend.save_current_chat();
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
                    self.ctl.backend.shared.lock().unwrap().chat.stop();
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
                self.ctl.backend.chat_send(text, &self.chat_model, &opts);
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
    markdown: Option<&mut CommonMarkCache>,
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
            match markdown {
                // Images stay unloaded: no loaders are installed and `file://` is never implied.
                Some(cache) => {
                    ui.push_id(("chat_md", at, selected), |ui| {
                        CommonMarkViewer::new()
                            .explicit_image_uri_scheme(true)
                            .show(ui, cache, r.content.trim_start());
                    })
                    .response
                }
                None => ui.label(r.content.trim_start()),
            };
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
        if let Some(line) = r.stats_line() {
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
