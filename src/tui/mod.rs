//! Terminal front-end (`llegion-tui`): the same core as the desktop app, drawn with ratatui.
//! Each tab lives in its own module as additional `impl Tui` methods (draw, keys, hints).

mod chat;
mod dashboard;
mod gpu;
mod input;
mod kit;
mod logs;
mod models;
mod performance;
mod settings;

use std::cell::RefCell;
use std::io::stdout;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, TableState, Tabs, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::config::Settings;
use crate::control::Control;
use crate::ferron;
use crate::util::Repaint;
use input::LineEdit;
use kit::{AMBER, GREEN, RED};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Dashboard,
    Models,
    Chat,
    Performance,
    Gpu,
    Logs,
    Settings,
}

impl Tab {
    const ALL: [Tab; 7] = [
        Tab::Dashboard,
        Tab::Models,
        Tab::Chat,
        Tab::Performance,
        Tab::Gpu,
        Tab::Logs,
        Tab::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Models => "Models",
            Tab::Chat => "Chat",
            Tab::Performance => "Performance",
            Tab::Gpu => "GPU",
            Tab::Logs => "Logs",
            Tab::Settings => "Settings",
        }
    }
}

/// What the line being edited is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    ModelFilter,
    PullName,
    ChatInput,
    ChatSystem,
    BenchPrompt,
    LogFilter,
    /// Index into the settings field list.
    Setting(usize),
    TokenLabel,
}

impl Field {
    fn prompt(self) -> &'static str {
        match self {
            Field::ModelFilter | Field::LogFilter => "Filter",
            Field::PullName => "Pull model (e.g. qwen3:8b)",
            Field::ChatInput => "Message",
            Field::ChatSystem => "System prompt",
            Field::BenchPrompt => "Benchmark prompt",
            Field::Setting(_) => "Value",
            Field::TokenLabel => "New token label",
        }
    }
}

struct Editing {
    field: Field,
    line: LineEdit,
    /// Shows bullets instead of the text (API tokens).
    masked: bool,
}

/// What a click on a region recorded while drawing does.
#[derive(Debug, Clone, Copy)]
enum Hit {
    Tab(Tab),
    /// Same as pressing this key (footer hints, chat input box).
    Key(KeyEvent),
    /// Row of the current tab's list (or of its open popup).
    Row(usize),
    /// Popup background: swallows clicks so they don't reach what is underneath.
    Block,
}

/// Key for a footer hint label, when it is a single key.
fn hint_key(label: &str) -> Option<KeyEvent> {
    let code = match label {
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Space" => KeyCode::Char(' '),
        "End" => KeyCode::End,
        "F5" => KeyCode::F(5),
        l if l.chars().count() == 1 => KeyCode::Char(l.chars().next()?),
        _ => return None,
    };
    Some(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Action waiting for a y/n answer.
enum Confirm {
    DeleteModel(String),
    DeleteToken(usize),
    ResetSettings,
}

pub struct Tui {
    ctl: Control,
    /// Settings being edited in the Settings / GPU tabs.
    draft: Settings,
    tab: Tab,
    editing: Option<Editing>,
    confirm: Option<Confirm>,
    help: bool,
    quit: bool,
    models: models::State,
    chat: chat::State,
    bench: performance::State,
    gpu_sel: usize,
    logs: logs::State,
    settings: settings::State,
    /// Clickable regions of the last frame; later entries are on top.
    hits: RefCell<Vec<(Rect, Hit)>>,
}

/// Runs the terminal UI until the user quits.
pub fn run() -> anyhow::Result<()> {
    let dirty = Arc::new(AtomicBool::new(true));
    let flag = dirty.clone();
    let repaint: Repaint = Arc::new(move |_| flag.store(true, Ordering::Relaxed));
    let mut tui = Tui::new(Settings::load(), repaint);
    let mut terminal = ratatui::try_init()?;
    let _ = execute!(stdout(), EnableBracketedPaste, EnableMouseCapture);
    // ratatui's own hook restores the terminal; also turn mouse reporting off on a panic.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableBracketedPaste, DisableMouseCapture);
        hook(info);
    }));
    let res = tui.event_loop(&mut terminal, &dirty);
    let _ = execute!(stdout(), DisableBracketedPaste, DisableMouseCapture);
    ratatui::restore();
    res
}

impl Tui {
    fn new(settings: Settings, repaint: Repaint) -> Self {
        let mut ctl = Control::new(settings.clone(), repaint);
        ctl.start();
        Self {
            ctl,
            draft: settings,
            tab: Tab::Dashboard,
            editing: None,
            confirm: None,
            help: false,
            quit: false,
            models: Default::default(),
            chat: Default::default(),
            bench: Default::default(),
            gpu_sel: 0,
            logs: Default::default(),
            settings: Default::default(),
            hits: RefCell::new(Vec::new()),
        }
    }

    fn event_loop(
        &mut self,
        terminal: &mut DefaultTerminal,
        dirty: &AtomicBool,
    ) -> anyhow::Result<()> {
        let mut last_draw: Option<Instant> = None;
        let mut input = true;
        while !self.quit {
            self.ctl.tick();
            // Redraw on input, on background updates, and every second for timers and toasts.
            if input
                || dirty.swap(false, Ordering::Relaxed)
                || last_draw.is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
            {
                terminal.draw(|f| self.draw(f))?;
                last_draw = Some(Instant::now());
            }
            input = false;
            if event::poll(Duration::from_millis(100))? {
                input = true;
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
                    Event::Paste(text) => {
                        if let Some(e) = &mut self.editing {
                            e.line.insert_str(&text);
                            self.live_edit();
                        }
                    }
                    Event::Mouse(m) => input = self.on_mouse(m),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// GPU selection only reaches a server this app starts.
    fn tab_enabled(&self, tab: Tab) -> bool {
        tab != Tab::Gpu || self.draft.server.manage_server
    }

    fn go(&mut self, tab: Tab) {
        if self.tab_enabled(tab) {
            self.tab = tab;
        }
    }

    fn draft_dirty(&self) -> bool {
        self.draft != self.ctl.settings
    }

    // ------------------------------------------------------------------
    // Input

    fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.editing.is_some() {
            self.edit_key(k);
            return;
        }
        if let Some(c) = self.confirm.take() {
            if matches!(k.code, KeyCode::Char('y' | 'Y')) {
                self.confirmed(c);
            }
            return;
        }
        if self.help {
            self.help = false;
            return;
        }
        let handled = match self.tab {
            Tab::Dashboard => false,
            Tab::Models => self.models_key(k),
            Tab::Chat => self.chat_key(k),
            Tab::Performance => self.performance_key(k),
            Tab::Gpu => self.gpu_key(k),
            Tab::Logs => self.logs_key(k),
            Tab::Settings => self.settings_key(k),
        };
        if handled {
            return;
        }
        let tab_index = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Tab | KeyCode::BackTab => {
                let n = Tab::ALL.len();
                let step = if k.code == KeyCode::Tab { 1 } else { n - 1 };
                let mut i = (tab_index + step) % n;
                while !self.tab_enabled(Tab::ALL[i]) {
                    i = (i + step) % n;
                }
                self.tab = Tab::ALL[i];
            }
            KeyCode::Char(c @ '1'..='7') => self.go(Tab::ALL[c as usize - '1' as usize]),
            KeyCode::F(5) => self.ctl.backend.refresh_now(),
            KeyCode::Char('S') if self.ctl.settings.server.manage_server => {
                if self.ctl.ollama_proc.is_running() {
                    self.ctl.stop_ollama();
                } else {
                    self.ctl.start_ollama();
                }
            }
            KeyCode::Char('R') if self.ctl.ollama_proc.is_running() => self.ctl.restart_ollama(),
            KeyCode::Char('P') if self.ctl.settings.proxy.enabled => {
                if self.ctl.ferron_proc.is_running() {
                    self.ctl.stop_ferron();
                } else {
                    self.ctl.start_ferron();
                }
            }
            _ => {}
        }
    }

    /// Handles a mouse event; returns whether it may have changed something.
    fn on_mouse(&mut self, m: MouseEvent) -> bool {
        match m.kind {
            MouseEventKind::ScrollUp => self.scroll(-3),
            MouseEventKind::ScrollDown => self.scroll(3),
            MouseEventKind::Down(MouseButton::Left) => {
                if self.help {
                    self.help = false;
                    return true;
                }
                let hit = self
                    .hits
                    .borrow()
                    .iter()
                    .rev()
                    .find(|(r, _)| r.contains((m.column, m.row).into()))
                    .map(|(_, h)| *h);
                match hit {
                    Some(Hit::Key(k)) => self.on_key(k),
                    // While typing or asked y/n, only the footer keys respond.
                    _ if self.editing.is_some() || self.confirm.is_some() => {}
                    Some(Hit::Tab(tab)) => self.go(tab),
                    Some(Hit::Row(i)) => self.click_row(i),
                    Some(Hit::Block) => {}
                    // A click on empty space closes the open popup.
                    None => match self.tab {
                        Tab::Models => self.models.info = None,
                        Tab::Settings => self.settings.tokens_open = false,
                        _ => {}
                    },
                }
            }
            _ => return false,
        }
        true
    }

    /// Wheel: scrolls the text views, moves the selection in lists.
    fn scroll(&mut self, delta: isize) {
        let up = delta < 0;
        let n = delta.unsigned_abs();
        match self.tab {
            Tab::Chat if up => self.chat.scroll += n,
            Tab::Chat => self.chat.scroll = self.chat.scroll.saturating_sub(n),
            Tab::Logs if up => self.logs.scroll += n,
            Tab::Logs => self.logs.scroll = self.logs.scroll.saturating_sub(n),
            Tab::Models if self.models.info.is_some() => {
                let s = &mut self.models.info_scroll;
                *s = s.saturating_add_signed(delta as i16);
            }
            // Lists clamp their selection when drawn.
            Tab::Models => kit::step(&mut self.models.sel, delta.signum(), usize::MAX),
            Tab::Performance => kit::step(&mut self.bench.sel, delta.signum(), usize::MAX),
            Tab::Gpu => kit::step(&mut self.gpu_sel, delta.signum(), usize::MAX),
            Tab::Settings if self.settings.tokens_open => {
                kit::step(&mut self.settings.token_sel, delta.signum(), usize::MAX)
            }
            Tab::Settings => kit::step(&mut self.settings.sel, delta.signum(), usize::MAX),
            Tab::Dashboard => {}
        }
    }

    /// Click on a list row: selects it; a click on the selected row opens or edits it.
    /// A click outside an open popup closes it.
    fn click_row(&mut self, i: usize) {
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        match self.tab {
            Tab::Models if self.models.info.is_some() => self.models.info = None,
            Tab::Models if self.models.sel == i => {
                self.models_key(enter);
            }
            Tab::Models => self.models.sel = i,
            Tab::Performance => self.bench.sel = i,
            Tab::Gpu => self.gpu_sel = i,
            Tab::Settings if self.settings.tokens_open => self.settings.token_sel = i,
            Tab::Settings if self.settings.sel == i => {
                self.settings_key(enter);
            }
            Tab::Settings => self.settings.sel = i,
            Tab::Dashboard | Tab::Chat | Tab::Logs => {}
        }
    }

    fn hit(&self, area: Rect, hit: Hit) {
        self.hits.borrow_mut().push((area, hit));
    }

    /// Records one `Hit::Row` per visible row of a list drawn in `rows`
    /// (one line per row, `offset` rows scrolled out at the top).
    fn hit_rows(&self, rows: Rect, offset: usize, count: usize) {
        for (line, i) in (offset..count).enumerate() {
            if line >= rows.height as usize {
                break;
            }
            let y = rows.y + line as u16;
            self.hit(Rect::new(rows.x, y, rows.width, 1), Hit::Row(i));
        }
    }

    /// Row hits for a table drawn in a `kit::panel` with a one-line header.
    fn hit_table(&self, area: Rect, state: &TableState, count: usize) {
        let inner = area.inner(Margin::new(1, 1));
        let rows = Rect {
            y: inner.y + 1,
            height: inner.height.saturating_sub(1),
            ..inner
        };
        self.hit_rows(rows, state.offset(), count);
    }

    fn edit(&mut self, field: Field, text: &str) {
        self.editing = Some(Editing {
            field,
            line: LineEdit::new(text),
            masked: false,
        });
    }

    fn edit_key(&mut self, k: KeyEvent) {
        let Some(e) = &mut self.editing else { return };
        match k.code {
            KeyCode::Enter => {
                let e = self.editing.take().unwrap();
                self.commit(e.field, e.line.text().to_string());
            }
            KeyCode::Esc => {
                let e = self.editing.take().unwrap();
                // Filters apply live; Esc clears them.
                match e.field {
                    Field::ModelFilter => self.models.filter.clear(),
                    Field::LogFilter => self.logs.filter.clear(),
                    _ => {}
                }
            }
            _ => {
                if e.line.handle(k) {
                    self.live_edit();
                }
            }
        }
    }

    /// Filters follow the text while it is typed.
    fn live_edit(&mut self) {
        let Some(e) = &self.editing else { return };
        match e.field {
            Field::ModelFilter => {
                self.models.filter = e.line.text().to_string();
                self.models.sel = 0;
            }
            Field::LogFilter => self.logs.filter = e.line.text().to_string(),
            _ => {}
        }
    }

    fn commit(&mut self, field: Field, text: String) {
        match field {
            Field::ModelFilter => self.models.filter = text,
            Field::LogFilter => self.logs.filter = text,
            Field::PullName => {
                if !text.trim().is_empty() {
                    self.ctl.backend.pull(text.trim());
                }
            }
            Field::ChatInput => self.chat_send(text),
            Field::ChatSystem => self.chat.system = text,
            Field::BenchPrompt => self.bench.prompt = text,
            Field::Setting(i) => self.set_setting(i, text),
            Field::TokenLabel => self.add_token(text),
        }
    }

    fn confirmed(&mut self, c: Confirm) {
        match c {
            Confirm::DeleteModel(name) => self.ctl.backend.delete(&name),
            Confirm::DeleteToken(i) => {
                if i < self.draft.proxy.tokens.len() {
                    self.draft.proxy.tokens.remove(i);
                }
            }
            Confirm::ResetSettings => self.draft = Settings::default(),
        }
    }

    // ------------------------------------------------------------------
    // Drawing

    fn draw(&mut self, f: &mut Frame) {
        self.hits.borrow_mut().clear();
        if !self.tab_enabled(self.tab) {
            self.tab = Tab::Dashboard;
        }
        let [top, tabs, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(f.area());
        self.top_bar(f, top);
        let titles = Tab::ALL.iter().enumerate().map(|(i, t)| {
            let title = Line::raw(format!("{} {}", i + 1, t.label()));
            if self.tab_enabled(*t) {
                title
            } else {
                title.dark_gray().crossed_out()
            }
        });
        let selected = Tab::ALL.iter().position(|t| *t == self.tab);
        // Same geometry as `Tabs`: one space of padding each side, one-space divider.
        let mut x = tabs.x;
        for (i, t) in Tab::ALL.iter().enumerate() {
            let w = Span::raw(format!("{} {}", i + 1, t.label())).width() as u16 + 2;
            self.hit(Rect::new(x, tabs.y, w, 1).intersection(tabs), Hit::Tab(*t));
            x += w + 1;
        }
        f.render_widget(
            Tabs::new(titles)
                .select(selected)
                .highlight_style(Style::new().fg(kit::ACCENT).bold().reversed())
                .divider(" "),
            tabs,
        );
        match self.tab {
            Tab::Dashboard => self.draw_dashboard(f, body),
            Tab::Models => self.draw_models(f, body),
            Tab::Chat => self.draw_chat(f, body),
            Tab::Performance => self.draw_performance(f, body),
            Tab::Gpu => self.draw_gpu(f, body),
            Tab::Logs => self.draw_logs(f, body),
            Tab::Settings => self.draw_settings(f, body),
        }
        self.toasts(f, body);
        self.footer(f, footer);
        if self.help {
            self.draw_help(f);
        }
    }

    fn top_bar(&self, f: &mut Frame, area: Rect) {
        let (connected, version) = {
            let s = self.ctl.backend.shared.lock().unwrap();
            (s.connected, s.version.clone())
        };
        let mut spans = vec![Span::styled(
            " Llegion ",
            Style::new().bold().fg(kit::ACCENT),
        )];
        spans.push(if connected {
            Span::styled(
                format!("● Connected v{} ", version.unwrap_or_default()),
                Style::new().fg(GREEN),
            )
        } else {
            Span::styled("● Disconnected ", Style::new().fg(RED))
        });
        spans.push(kit::weak(format!(
            "{}  ",
            self.ctl.settings.connection.base_url
        )));
        let p = &self.ctl.ollama_proc;
        spans.push(if !self.ctl.settings.server.manage_server {
            kit::weak("external server  ")
        } else if p.is_running() {
            Span::styled(
                format!("server running · pid {}  ", p.pid().unwrap_or(0)),
                Style::new().fg(GREEN),
            )
        } else {
            Span::styled("server stopped  ", Style::new().fg(AMBER))
        });
        if self.ctl.settings.proxy.enabled {
            spans.push(if self.ctl.ferron_proc.is_running() {
                Span::styled(
                    format!("proxy on {}  ", ferron::public_url(&self.ctl.settings)),
                    Style::new().fg(GREEN),
                )
            } else {
                Span::styled("proxy off  ", Style::new().fg(AMBER))
            });
        }
        if self.draft_dirty() {
            spans.push(Span::styled("● unsaved settings", Style::new().fg(AMBER)));
        }
        f.render_widget(Line::from(spans), area);
    }

    fn footer(&self, f: &mut Frame, area: Rect) {
        if let Some(e) = &self.editing
            && e.field != Field::ChatInput
        {
            let prompt = format!(" {}: ", e.field.prompt());
            let width = area.width.saturating_sub(prompt.len() as u16 + 1);
            let (text, col) = e.line.view(width, e.masked);
            f.render_widget(
                Line::from(vec![
                    Span::styled(prompt.clone(), Style::new().fg(kit::ACCENT).bold()),
                    Span::raw(text),
                ]),
                area,
            );
            f.set_cursor_position((area.x + prompt.len() as u16 + col, area.y));
            return;
        }
        let mut spans = Vec::new();
        let hints = if let Some(c) = &self.confirm {
            let question = match c {
                Confirm::DeleteModel(name) => format!("Delete {name} from disk?"),
                Confirm::DeleteToken(i) => format!(
                    "Delete token \"{}\"?",
                    self.draft.proxy.tokens.get(*i).map_or("", |t| &t.label)
                ),
                Confirm::ResetSettings => "Reset all settings to defaults (not saved yet)?".into(),
            };
            spans.push(Span::styled(
                format!(" {question}  "),
                Style::new().fg(AMBER).bold(),
            ));
            vec![("y", "yes"), ("n", "no")]
        } else if self.editing.is_some() {
            vec![("Enter", "send"), ("Esc", "stop typing")]
        } else {
            let mut hints = match self.tab {
                Tab::Dashboard => self.dashboard_hints(),
                Tab::Models => self.models_hints(),
                Tab::Chat => self.chat_hints(),
                Tab::Performance => self.performance_hints(),
                Tab::Gpu => self.gpu_hints(),
                Tab::Logs => self.logs_hints(),
                Tab::Settings => self.settings_hints(),
            };
            hints.extend([("?", "help"), ("q", "quit")]);
            hints
        };
        let mut x = area.x + spans.iter().map(|s| s.width() as u16).sum::<u16>();
        for (key, action) in hints {
            let hint = kit::hint(key, action);
            let w = hint.iter().map(|s| s.width() as u16).sum::<u16>();
            if let Some(k) = hint_key(key) {
                self.hit(Rect::new(x, area.y, w, 1).intersection(area), Hit::Key(k));
            }
            x += w;
            spans.extend(hint);
        }
        f.render_widget(Line::from(spans), area);
    }

    fn toasts(&self, f: &mut Frame, body: Rect) {
        let notices = {
            let mut s = self.ctl.backend.shared.lock().unwrap();
            s.notices
                .retain(|n| n.at.elapsed() < Duration::from_secs(6));
            s.notices.clone()
        };
        let width = body.width.min(60);
        let mut y = body.bottom();
        for n in notices.iter().rev().take(3) {
            let lines = (n.text.chars().count() as u16 / width.saturating_sub(4).max(1)) + 3;
            if y < body.y + lines {
                break;
            }
            y -= lines;
            let area = Rect::new(body.right().saturating_sub(width), y, width, lines);
            let color = if n.error { RED } else { GREEN };
            f.render_widget(ratatui::widgets::Clear, area);
            f.render_widget(
                Paragraph::new(n.text.as_str())
                    .wrap(Wrap { trim: true })
                    .style(Style::new().fg(color))
                    .block(kit::panel(if n.error { "Error" } else { "Info" }).border_style(color)),
                area,
            );
        }
    }

    fn draw_help(&self, f: &mut Frame) {
        let keys: &[(&str, &str)] = &[
            ("Tab / Shift+Tab, 1-7", "switch tab"),
            ("↑ ↓ / j k, PgUp PgDn", "move / scroll"),
            ("S", "start / stop the managed server"),
            ("R", "restart the managed server"),
            ("P", "start / stop the HTTPS proxy"),
            ("F5", "refresh now"),
            ("Enter / Esc", "confirm / cancel when typing"),
            ("Ctrl+U, Ctrl+W", "clear line / delete word when typing"),
            ("q, Ctrl+C", "quit"),
            ("Mouse", "click tabs, rows and key hints; wheel scrolls"),
            ("Shift+drag", "select text (most terminals)"),
        ];
        let mut lines: Vec<Line> = keys
            .iter()
            .map(|(k, v)| {
                Line::from(vec![
                    Span::styled(format!("{k:<24}"), Style::new().fg(kit::ACCENT)),
                    Span::raw(*v),
                ])
            })
            .collect();
        lines.push(Line::raw(""));
        lines.push(Line::from(kit::weak(
            "Tab-specific keys are listed at the bottom of the screen.",
        )));
        let area = kit::popup(f, 76, lines.len() as u16 + 2);
        self.hit(area, Hit::Block);
        f.render_widget(Paragraph::new(lines).block(kit::panel("Keys")), area);
    }
}
