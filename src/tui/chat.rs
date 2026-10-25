//! Chat tab: streamed conversation with a model, answer variants and regeneration.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::kit::{self, AMBER, RED};
use super::{Field, Hit, Tui};
use crate::backend::{ChatEntry, ChatOptions, preferred_model};
use crate::chat_store::ChatConfig;
use crate::util::{CONTEXT_PRESETS, ctx_label};

const TEMPERATURES: [Option<f32>; 7] = [
    None,
    Some(0.0),
    Some(0.2),
    Some(0.5),
    Some(0.8),
    Some(1.0),
    Some(1.5),
];

#[derive(Default)]
pub(super) struct State {
    pub active_id: String,
    pub model: String,
    pub system: String,
    pub ctx: u32,
    /// `None` keeps the model's own default.
    pub temperature: Option<f32>,
    /// Lines scrolled up from the bottom (0 follows the stream).
    pub scroll: usize,
}

impl Tui {
    fn chat_options(&self) -> ChatOptions {
        ChatOptions {
            system: self.chat.system.clone(),
            temperature: self.chat.temperature,
            num_ctx: (self.chat.ctx > 0).then_some(self.chat.ctx),
        }
    }

    pub(super) fn chat_send(&mut self, text: String) {
        if text.trim().is_empty() || self.chat.model.is_empty() {
            return;
        }
        if self.ctl.backend.shared.lock().unwrap().chat.streaming {
            // Keep the text until the current answer is done.
            self.edit(Field::ChatInput, &text);
            return;
        }
        self.chat.scroll = 0;
        let opts = self.chat_options();
        self.ctl.backend.chat_send(text, &self.chat.model, &opts);
        self.edit(Field::ChatInput, "");
    }

    /// Index of the last answer, if the conversation ends with one.
    fn last_answer(&self) -> Option<usize> {
        let s = self.ctl.backend.shared.lock().unwrap();
        let n = s.chat.entries.len();
        matches!(s.chat.entries.last(), Some(ChatEntry::Assistant { .. })).then(|| n - 1)
    }

    pub(super) fn chat_key(&mut self, k: KeyEvent) -> bool {
        if !self.ctl.settings.ui.chat_warning_acknowledged {
            if k.code == KeyCode::Enter {
                self.ctl.settings.ui.chat_warning_acknowledged = true;
                self.draft.ui.chat_warning_acknowledged = true;
                if let Err(e) = self.ctl.settings.save() {
                    self.ctl.logs.app_error(format!(
                        "Failed to save chat warning acknowledgement: {e:#}"
                    ));
                }
            }
            return true;
        }
        let backend = self.ctl.backend.clone();
        match k.code {
            KeyCode::Enter | KeyCode::Char('i') => self.edit(Field::ChatInput, ""),
            KeyCode::Esc => backend.shared.lock().unwrap().chat.stop(),
            KeyCode::Char('n') => {
                backend.new_chat();
                self.chat.scroll = 0;
            }
            KeyCode::Char('o' | 'O') => {
                let (chats, current) = {
                    let s = backend.shared.lock().unwrap();
                    (s.saved_chats.clone(), s.chat_id.clone())
                };
                if !chats.is_empty() {
                    let at = chats.iter().position(|c| c.id == current);
                    let next = match (at, k.code) {
                        (Some(i), KeyCode::Char('O')) => (i + chats.len() - 1) % chats.len(),
                        (Some(i), _) => (i + 1) % chats.len(),
                        (None, _) => 0,
                    };
                    if let Err(e) = backend.load_saved_chat(&chats[next].id) {
                        backend.shared.lock().unwrap().chat_storage_error = Some(format!("{e:#}"));
                    }
                }
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                let names: Vec<String> = backend
                    .shared
                    .lock()
                    .unwrap()
                    .models
                    .iter()
                    .map(|m| m.name.clone())
                    .collect();
                if !names.is_empty() {
                    self.chat.model =
                        kit::cycle(&names, &self.chat.model, k.code == KeyCode::Char('M'));
                }
            }
            KeyCode::Char('s') => self.edit(Field::ChatSystem, &self.chat.system.clone()),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.chat.ctx = kit::cycle(
                    &CONTEXT_PRESETS,
                    &self.chat.ctx,
                    k.code == KeyCode::Char('C'),
                )
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.chat.temperature = kit::cycle(
                    &TEMPERATURES,
                    &self.chat.temperature,
                    k.code == KeyCode::Char('T'),
                )
            }
            KeyCode::Char('g') => {
                if let Some(at) = self.last_answer() {
                    let opts = self.chat_options();
                    backend.chat_reply(at, &self.chat.model, &opts);
                    self.chat.scroll = 0;
                }
            }
            KeyCode::Char('[') | KeyCode::Char(']') => {
                let mut s = backend.shared.lock().unwrap();
                if !s.chat.streaming
                    && let Some(ChatEntry::Assistant { variants, selected }) =
                        s.chat.entries.last_mut()
                {
                    *selected = if k.code == KeyCode::Char('[') {
                        selected.saturating_sub(1)
                    } else {
                        (*selected + 1).min(variants.len() - 1)
                    };
                }
                drop(s);
                backend.save_current_chat();
            }
            KeyCode::Up | KeyCode::Char('k') => self.chat.scroll += 1,
            KeyCode::Down | KeyCode::Char('j') => {
                self.chat.scroll = self.chat.scroll.saturating_sub(1)
            }
            KeyCode::PageUp => self.chat.scroll += 10,
            KeyCode::PageDown => self.chat.scroll = self.chat.scroll.saturating_sub(10),
            KeyCode::End => self.chat.scroll = 0,
            _ => return false,
        }
        true
    }

    pub(super) fn chat_hints(&self) -> Vec<(&'static str, &'static str)> {
        if !self.ctl.settings.ui.chat_warning_acknowledged {
            return vec![("Enter", "acknowledge chat storage warning")];
        }
        vec![
            ("Enter", "type"),
            ("Esc", "stop"),
            ("m", "model"),
            ("g", "regenerate"),
            ("[ ]", "variant"),
            ("s", "system"),
            ("c", "context"),
            ("t", "temperature"),
            ("n", "new chat"),
            ("o/O", "older/newer chat"),
            ("↑↓", "scroll"),
        ]
    }

    pub(super) fn draw_chat(&mut self, f: &mut Frame, area: Rect) {
        let (entries, streaming, models, running, id, config) = {
            let s = self.ctl.backend.shared.lock().unwrap();
            (
                s.chat.entries.clone(),
                s.chat.streaming,
                s.models.clone(),
                s.running.clone(),
                s.chat_id.clone(),
                s.chat_config.clone(),
            )
        };
        if self.chat.active_id != id {
            self.chat.active_id = id;
            self.chat.model = config.model.clone();
            self.chat.system = config.system.clone();
            self.chat.ctx = config.context;
            self.chat.temperature = config.temperature;
        }
        self.chat.model = preferred_model(&self.chat.model, &models, &running);
        self.ctl.backend.set_chat_config(ChatConfig {
            model: self.chat.model.clone(),
            system: self.chat.system.clone(),
            context: self.chat.ctx,
            temperature: self.chat.temperature,
            markdown: config.markdown,
        });

        let [opts_area, msgs_area, input_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .areas(area);

        let temp = self
            .chat
            .temperature
            .map(|t| format!("{t:.1}"))
            .unwrap_or_else(|| "model default".into());
        let mut opts = vec![
            kit::weak(" model "),
            Span::styled(
                if self.chat.model.is_empty() {
                    "none downloaded".to_string()
                } else {
                    self.chat.model.clone()
                },
                Style::new().fg(kit::ACCENT).bold(),
            ),
            kit::weak("  context "),
            Span::raw(ctx_label(self.chat.ctx)),
            kit::weak("  temperature "),
            Span::raw(temp),
        ];
        if !self.chat.system.is_empty() {
            opts.push(kit::weak("  system "));
            opts.push(Span::raw(self.chat.system.clone()));
        }
        f.render_widget(Line::from(opts), opts_area);

        let mut lines: Vec<Line> = Vec::new();
        if !self.ctl.settings.ui.chat_warning_acknowledged {
            lines.push(Line::from(Span::styled(
                "Chat history is stored without encryption.",
                Style::new().fg(AMBER).bold(),
            )));
            lines.push(Line::raw("Anyone with access to this account can read it."));
            lines.push(Line::raw(
                crate::config::chat_store_path().display().to_string(),
            ));
            lines.push(Line::from(kit::weak("Press Enter to continue.")));
        }
        for e in &entries {
            match e {
                ChatEntry::User(text) => {
                    lines.push(Line::from(Span::styled(
                        "You",
                        Style::new().fg(kit::ACCENT).bold(),
                    )));
                    lines.extend(text.lines().map(|l| Line::raw(l.to_string())));
                }
                ChatEntry::Assistant { variants, selected } => {
                    let Some(r) = variants.get(*selected) else {
                        continue;
                    };
                    let mut head = vec![Span::styled(
                        r.model.clone(),
                        Style::new().fg(kit::GREEN).bold(),
                    )];
                    if variants.len() > 1 {
                        head.push(kit::weak(format!("  {}/{}", selected + 1, variants.len())));
                    }
                    lines.push(Line::from(head));
                    for l in r.thinking.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("│ {l}"),
                            Style::new().dark_gray().italic(),
                        )));
                    }
                    lines.extend(r.content.lines().map(|l| Line::raw(l.to_string())));
                    if !r.done && r.content.is_empty() && r.thinking.is_empty() {
                        lines.push(Line::from(kit::weak("…")));
                    }
                    if let Some(err) = &r.error {
                        lines.push(Line::from(Span::styled(err.clone(), Style::new().fg(RED))));
                    } else if r.stopped {
                        lines.push(Line::from(Span::styled("stopped", Style::new().fg(AMBER))));
                    }
                    if let Some(stats) = r.stats_line() {
                        lines.push(Line::from(kit::weak(stats)));
                    }
                }
            }
            lines.push(Line::raw(""));
        }
        if entries.is_empty() && self.ctl.settings.ui.chat_warning_acknowledged {
            lines.push(Line::from(kit::weak(
                "Press Enter to type a message. Answers stream here.",
            )));
        }
        let block = kit::panel(if streaming {
            "Conversation · streaming"
        } else {
            "Conversation"
        });
        let inner = block.inner(msgs_area);
        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        let total = para.line_count(inner.width);
        let max_offset = total.saturating_sub(inner.height as usize);
        self.chat.scroll = self.chat.scroll.min(max_offset);
        let top = (max_offset - self.chat.scroll) as u16;
        f.render_widget(para.scroll((top, 0)).block(block), msgs_area);

        let editing = self
            .editing
            .as_ref()
            .filter(|e| e.field == Field::ChatInput);
        let input_block = kit::panel("Message").border_style(if editing.is_some() {
            Style::new().fg(kit::ACCENT)
        } else {
            Style::new().dark_gray()
        });
        let inner = input_block.inner(input_area);
        f.render_widget(input_block, input_area);
        if editing.is_none() {
            self.hit(
                input_area,
                Hit::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            );
        }
        match editing {
            Some(e) => {
                let (text, col) = e.line.view(inner.width, false);
                f.render_widget(Span::raw(text), inner);
                f.set_cursor_position((inner.x + col, inner.y));
            }
            None => f.render_widget(kit::weak("Enter to type"), inner),
        }
    }
}
