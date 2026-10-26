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

/// Subjects created by the user first, then any other subject a chat is filed under.
fn chat_subjects(created: &[String], chats: &[ChatSummary], current: &str) -> Vec<String> {
    let mut subjects = created.to_vec();
    for subject in chats
        .iter()
        .map(|chat| chat.group.as_str())
        .chain([current])
    {
        if !subject.is_empty() && !subjects.iter().any(|s| s == subject) {
            subjects.push(subject.to_owned());
        }
    }
    subjects
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

        let mut switched_chat = false;
        let mut open = self.chat_subjects_open;
        egui::Panel::left("chat_subjects")
            .resizable(true)
            .default_size(250.0)
            .size_range(170.0..=480.0)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                right: 12,
                ..Default::default()
            }))
            .show_collapsible(ui, &mut open, |ui| {
                switched_chat =
                    self.chat_subjects_panel(ui, &saved_chats, &active_id, &active_group);
            });
        self.chat_subjects_open = open;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 12,
                ..Default::default()
            }))
            .show(ui, |ui| {
                self.chat_main(ui, &entries, streaming, &models, &running, switched_chat)
            });
    }

    /// Left panel: chats filed under subjects; drag a chat onto a subject to move it.
    fn chat_subjects_panel(
        &mut self,
        ui: &mut egui::Ui,
        saved_chats: &[ChatSummary],
        active_id: &str,
        active_group: &str,
    ) -> bool {
        let subjects = chat_subjects(
            &self.ctl.settings.ui.chat_subjects,
            saved_chats,
            active_group,
        );
        let mut switched = false;
        let mut delete_chat = None;
        let mut moved: Option<(String, String)> = None;
        let mut removed_subject = None;
        ui.horizontal(|ui| {
            ui.heading("Subjects");
            ui.menu_button(icon!(ADD), |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.chat_subject_name)
                        .hint_text("Subject name"),
                );
                response.request_focus();
                let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button(ic!(ADD, "Create")).clicked() || enter {
                    let name = self.chat_subject_name.trim().to_owned();
                    if !name.is_empty() && !subjects.contains(&name) {
                        let mut created = self.ctl.settings.ui.chat_subjects.clone();
                        created.push(name);
                        self.save_chat_subjects(created);
                    }
                    self.chat_subject_name.clear();
                    ui.close();
                }
            })
            .response
            .on_hover_text("New subject");
        });
        ui.add_space(6.0);

        let found_active = saved_chats.iter().any(|chat| chat.id == active_id);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                // "" holds the chats without a subject, listed last.
                for subject in subjects.iter().map(String::as_str).chain([""]) {
                    let chats: Vec<&ChatSummary> = saved_chats
                        .iter()
                        .filter(|chat| chat.group == subject)
                        .collect();
                    let new_chat_here = !found_active && active_group == subject;
                    let frame = egui::Frame::new().inner_margin(4.0).corner_radius(6.0);
                    let (_, dropped) = ui.dnd_drop_zone::<String, ()>(frame, |ui| {
                        ui.set_width(ui.available_width());
                        let collapsed =
                            !subject.is_empty() && self.collapsed_chat_subjects.contains(subject);
                        ui.horizontal(|ui| {
                            let title = if subject.is_empty() {
                                "No subject".to_owned()
                            } else {
                                format!("{} {subject}", if collapsed { "▸" } else { "▾" })
                            };
                            let header = ui.add(
                                egui::Button::new(RichText::new(title).strong())
                                    .frame(false)
                                    .truncate(),
                            );
                            if header.clicked() && !subject.is_empty() {
                                if collapsed {
                                    self.collapsed_chat_subjects.remove(subject);
                                } else {
                                    self.collapsed_chat_subjects.insert(subject.to_owned());
                                }
                            }
                            if !subject.is_empty() && chats.is_empty() && !new_chat_here {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button("×")
                                            .on_hover_text("Remove this empty subject")
                                            .clicked()
                                        {
                                            removed_subject = Some(subject.to_owned());
                                        }
                                    },
                                );
                            }
                        });
                        if collapsed {
                            return;
                        }
                        for chat in &chats {
                            self.chat_row(
                                ui,
                                &chat.id,
                                &chat.title,
                                chat.id == active_id,
                                &mut switched,
                                &mut delete_chat,
                            );
                        }
                        if new_chat_here {
                            self.chat_row(
                                ui,
                                active_id,
                                "New chat",
                                true,
                                &mut switched,
                                &mut delete_chat,
                            );
                        }
                        if chats.is_empty() && !new_chat_here {
                            ui.label(RichText::new("Drop chats here").weak().small());
                        }
                    });
                    if let Some(id) = dropped {
                        moved = Some((id.as_ref().clone(), subject.to_owned()));
                    }
                    ui.add_space(4.0);
                }
            });

        if let Some((id, subject)) = moved {
            if id == active_id {
                self.ctl.backend.set_current_chat_group(subject);
            } else if let Err(e) = self.ctl.backend.set_saved_chat_group(&id, subject) {
                self.ctl
                    .logs
                    .app_error(format!("Failed to move chat: {e:#}"));
            }
        }
        if let Some(subject) = removed_subject {
            let mut created = self.ctl.settings.ui.chat_subjects.clone();
            created.retain(|s| *s != subject);
            self.save_chat_subjects(created);
        }
        if let Some(id) = delete_chat {
            if id == active_id {
                switched = true;
            }
            if let Err(e) = self.ctl.backend.delete_saved_chat(&id) {
                self.ctl
                    .logs
                    .app_error(format!("Failed to delete chat: {e:#}"));
            }
        }
        switched
    }

    /// One chat in the subjects panel: click to open, double-click to rename, drag to move.
    fn chat_row(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        title: &str,
        active: bool,
        switched: &mut bool,
        delete_chat: &mut Option<String>,
    ) {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Close and delete chat")
                    .clicked()
                {
                    *delete_chat = Some(id.to_owned());
                }
                if self.chat_rename_id.as_deref() == Some(id) {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.chat_rename_text)
                            .desired_width(ui.available_width()),
                    );
                    response.request_focus();
                    let commit = response.lost_focus()
                        || (response.has_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                    if commit {
                        let result = self
                            .ctl
                            .backend
                            .rename_saved_chat(id, self.chat_rename_text.clone());
                        if let Err(e) = result {
                            self.ctl
                                .logs
                                .app_error(format!("Failed to rename chat: {e:#}"));
                        }
                        self.chat_rename_id = None;
                    }
                    return;
                }
                let response = ui
                    .add(
                        egui::Button::selectable(active, title)
                            .right_text("")
                            .truncate()
                            .min_size(egui::vec2(ui.available_width(), 0.0))
                            // One widget for click and drag: a drag-only layer on top
                            // (`dnd_drag_source`) would swallow the clicks.
                            .sense(egui::Sense::click_and_drag()),
                    )
                    .on_hover_text(title);
                response.dnd_set_drag_payload(id.to_owned());
                if response.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    if let Some(pos) = ui.ctx().pointer_interact_pos() {
                        egui::Area::new(egui::Id::unique("chat_drag_preview"))
                            .order(egui::Order::Tooltip)
                            .fixed_pos(pos + egui::vec2(12.0, 4.0))
                            .interactable(false)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(title));
                            });
                    }
                }
                if response.double_clicked() {
                    self.chat_rename_id = Some(id.to_owned());
                    self.chat_rename_text = title.to_owned();
                } else if response.clicked() && !active {
                    match self.ctl.backend.load_saved_chat(id) {
                        Ok(()) => *switched = true,
                        Err(e) => self
                            .ctl
                            .logs
                            .app_error(format!("Failed to load saved chat: {e:#}")),
                    }
                }
            });
        });
    }

    fn save_chat_subjects(&mut self, subjects: Vec<String>) {
        self.draft.ui.chat_subjects.clone_from(&subjects);
        self.ctl.settings.ui.chat_subjects = subjects;
        if let Err(e) = self.ctl.settings.save() {
            self.ctl
                .logs
                .app_error(format!("Failed to save chat subjects: {e:#}"));
        }
    }

    fn chat_main(
        &mut self,
        ui: &mut egui::Ui,
        entries: &[ChatEntry],
        streaming: bool,
        models: &[LocalModel],
        running: &[crate::ollama::RunningModel],
        mut switched_chat: bool,
    ) {
        ui.horizontal(|ui| {
            let (toggle, tip) = if self.chat_subjects_open {
                (icon!(PANEL_CLOSE), "Hide subjects")
            } else {
                (icon!(PANEL_OPEN), "Show subjects")
            };
            if ui.button(toggle).on_hover_text(tip).clicked() {
                self.chat_subjects_open = !self.chat_subjects_open;
            }
            ui.heading("Chat");
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
                    ui.ctx().copy_text(transcript(entries));
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
                        for m in models {
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
                                    user_message(ui, i, text, models, &self.chat_model, streaming)
                                }
                                ChatEntry::Assistant { variants, selected } => assistant_message(
                                    ui,
                                    i,
                                    variants,
                                    *selected,
                                    models,
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
