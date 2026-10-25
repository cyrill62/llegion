//! Logs tab: merged server, proxy and app logs with filters.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::kit::{self, AMBER, RED};
use super::{Field, Tui};
use crate::config::LogSource;
use crate::logs::{Level, LogLine, Origin, line_filter};

const LEVELS: [Level; 4] = [Level::Debug, Level::Info, Level::Warn, Level::Error];

pub(super) struct State {
    pub filter: String,
    pub min_level: Level,
    /// Shown origins: ollama, caddy, app.
    pub origins: [bool; 3],
    pub paused: bool,
    /// Lines scrolled up from the newest (0 follows new lines).
    pub scroll: usize,
    cache: Vec<LogLine>,
    cache_key: Option<(u64, String, Level, [bool; 3])>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            filter: String::new(),
            min_level: Level::Debug,
            origins: [true; 3],
            paused: false,
            scroll: 0,
            cache: Vec::new(),
            cache_key: None,
        }
    }
}

fn source_label(s: LogSource) -> &'static str {
    match s {
        LogSource::Managed => "managed server only",
        LogSource::File => "log file",
        LogSource::Journal => "systemd journal",
    }
}

impl Tui {
    fn refresh_log_cache(&mut self) {
        let v = &self.logs;
        let key = (
            self.ctl.logs.generation(),
            v.filter.clone(),
            v.min_level,
            v.origins,
        );
        if v.paused || v.cache_key.as_ref() == Some(&key) {
            return;
        }
        let lines = self
            .ctl
            .logs
            .snapshot_filtered(line_filter(v.origins, v.min_level, &v.filter));
        // Keep the view still while scrolled up and new lines arrive.
        if self.logs.scroll > 0 {
            self.logs.scroll += lines.len().saturating_sub(self.logs.cache.len());
        }
        self.logs.cache = lines;
        self.logs.cache_key = Some(key);
    }

    pub(super) fn logs_key(&mut self, k: KeyEvent) -> bool {
        let v = &mut self.logs;
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => v.scroll += 1,
            KeyCode::Down | KeyCode::Char('j') => v.scroll = v.scroll.saturating_sub(1),
            KeyCode::PageUp => v.scroll += 20,
            KeyCode::PageDown => v.scroll = v.scroll.saturating_sub(20),
            KeyCode::Home => v.scroll = usize::MAX,
            KeyCode::End => v.scroll = 0,
            KeyCode::Char('/') => {
                let filter = v.filter.clone();
                self.edit(Field::LogFilter, &filter);
            }
            KeyCode::Char('v') | KeyCode::Char('V') => {
                v.min_level = kit::cycle(&LEVELS, &v.min_level, k.code == KeyCode::Char('V'))
            }
            KeyCode::Char('o') => v.origins[0] = !v.origins[0],
            KeyCode::Char('c') => v.origins[1] = !v.origins[1],
            KeyCode::Char('a') => v.origins[2] = !v.origins[2],
            KeyCode::Char('p') => v.paused = !v.paused,
            KeyCode::Char('x') => self.ctl.logs.clear(),
            KeyCode::Char('s') => {
                // Switching sources is harmless; apply just that field immediately.
                let source = kit::cycle(
                    &[LogSource::Managed, LogSource::File, LogSource::Journal],
                    &self.ctl.settings.logs.source,
                    false,
                );
                self.ctl.settings.logs.source = source;
                self.draft.logs.source = source;
                let _ = self.ctl.settings.save();
                self.ctl.sync_log_follower();
            }
            _ => return false,
        }
        true
    }

    pub(super) fn logs_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("/", "filter"),
            ("v", "level"),
            ("o c a", "ollama/caddy/app"),
            ("s", "source"),
            ("p", if self.logs.paused { "resume" } else { "pause" }),
            ("x", "clear"),
            ("End", "follow"),
        ]
    }

    pub(super) fn draw_logs(&mut self, f: &mut Frame, area: Rect) {
        self.refresh_log_cache();
        let [bar, list] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

        let v = &self.logs;
        let toggle = |name: &'static str, on: bool| {
            if on {
                Span::styled(format!("{name} "), Style::new().fg(kit::ACCENT))
            } else {
                Span::styled(format!("{name} "), Style::new().dark_gray().crossed_out())
            }
        };
        let s = &self.ctl.settings.logs;
        let source = match s.source {
            LogSource::Managed => source_label(s.source).to_string(),
            LogSource::File => format!("{} {}", source_label(s.source), s.file_path),
            LogSource::Journal => format!("{} -u {}", source_label(s.source), s.journal_unit),
        };
        let mut spans = vec![
            kit::weak(" source "),
            Span::raw(format!("{source}  ")),
            kit::weak("level ≥ "),
            Span::raw(format!("{}  ", v.min_level.label())),
            toggle("ollama", v.origins[0]),
            toggle("caddy", v.origins[1]),
            toggle("app", v.origins[2]),
        ];
        if !v.filter.is_empty() {
            spans.push(kit::weak(" filter "));
            spans.push(Span::raw(format!("\"{}\"", v.filter)));
        }
        if v.paused {
            spans.push(Span::styled("  paused", Style::new().fg(AMBER).bold()));
        }
        f.render_widget(Line::from(spans), bar);

        let title = format!("{} lines", v.cache.len());
        let block = kit::panel(&title);
        let inner = block.inner(list);
        let height = inner.height as usize;
        let n = v.cache.len();
        self.logs.scroll = self.logs.scroll.min(n.saturating_sub(height));
        let end = n - self.logs.scroll;
        let start = end.saturating_sub(height);
        let lines: Vec<Line> = self.logs.cache[start..end]
            .iter()
            .map(|l| {
                let (origin_color, level_style) = (
                    match l.origin {
                        Origin::Ollama => Color::Cyan,
                        Origin::Caddy => Color::Magenta,
                        Origin::App => Color::Blue,
                    },
                    match l.level {
                        Level::Error => Style::new().fg(RED),
                        Level::Warn => Style::new().fg(AMBER),
                        Level::Info => Style::new(),
                        Level::Debug => Style::new().dark_gray(),
                    },
                );
                Line::from(vec![
                    kit::weak(format!("{} ", l.time)),
                    Span::styled(
                        format!("{:<7}", l.origin.label()),
                        Style::new().fg(origin_color),
                    ),
                    Span::styled(format!("{:<6}", l.level.label()), level_style),
                    Span::styled(l.text.clone(), level_style),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(lines).block(block), list);
    }
}
