//! Settings tab: every setting as an editable row, saved explicitly with `w`.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::kit::{self, AMBER, GREEN};
use super::{Confirm, Editing, Field, Hit, Tui};
use crate::config::{ApiToken, LogSource, Settings, TlsMode};
use crate::util::{generate_token, mask_token};

enum Kind {
    Bool(fn(&mut Settings) -> &mut bool),
    Text(fn(&mut Settings) -> &mut String),
    /// Text shown masked (API token).
    Secret(fn(&mut Settings) -> &mut String),
    Num(fn(&Settings) -> u64, fn(&mut Settings, u64)),
    /// Index into the options, and setter by index.
    Choice(
        &'static [&'static str],
        fn(&Settings) -> usize,
        fn(&mut Settings, usize),
    ),
    /// `KEY=VALUE` lines, edited as one line separated by `;`.
    Lines(fn(&mut Settings) -> &mut String),
    Tokens,
}

struct Item {
    section: &'static str,
    label: &'static str,
    help: &'static str,
    kind: Kind,
}

const KV_TYPES: [&str; 3] = ["f16", "q8_0", "q4_0"];
const TLS_MODES: [TlsMode; 4] = [
    TlsMode::Internal,
    TlsMode::Acme,
    TlsMode::Custom,
    TlsMode::Off,
];
const TLS_LABELS: [&str; 4] = [
    "self-signed",
    "Let's Encrypt",
    "my certificate",
    "off (HTTP)",
];
const SOURCES: [LogSource; 3] = [LogSource::Managed, LogSource::File, LogSource::Journal];
const SOURCE_LABELS: [&str; 3] = ["managed server only", "log file", "systemd journal"];

fn items() -> Vec<Item> {
    let item = |section, label, help, kind| Item {
        section,
        label,
        help,
        kind,
    };
    vec![
        item(
            "Connection",
            "API URL",
            "Ollama API this app talks to (local, or a remote proxy).",
            Kind::Text(|s| &mut s.connection.base_url),
        ),
        item(
            "Connection",
            "API token",
            "Bearer token sent to a token-protected server.",
            Kind::Secret(|s| &mut s.connection.api_token),
        ),
        item(
            "Connection",
            "Accept invalid certs",
            "For self-signed HTTPS proxies.",
            Kind::Bool(|s| &mut s.connection.accept_invalid_certs),
        ),
        item(
            "Server",
            "Manage server",
            "Start and stop `ollama serve` from this app (S / R keys).",
            Kind::Bool(|s| &mut s.server.manage_server),
        ),
        item(
            "Server",
            "Start automatically",
            "Start the server when this app starts.",
            Kind::Bool(|s| &mut s.server.auto_start),
        ),
        item(
            "Server",
            "Stop on exit",
            "Stop the managed server when this app quits.",
            Kind::Bool(|s| &mut s.server.stop_on_exit),
        ),
        item(
            "Server",
            "Restart on crash",
            "At most 3 times per minute.",
            Kind::Bool(|s| &mut s.server.restart_on_crash),
        ),
        item(
            "Server",
            "ollama binary",
            "Path to the ollama executable.",
            Kind::Text(|s| &mut s.server.ollama_path),
        ),
        item(
            "Server",
            "Bind host",
            "Forced to 127.0.0.1 while the proxy is enabled.",
            Kind::Text(|s| &mut s.server.host),
        ),
        item(
            "Server",
            "Port",
            "",
            Kind::Num(
                |s| s.server.port.into(),
                |s, v| s.server.port = v.min(65535) as u16,
            ),
        ),
        item(
            "Server",
            "Context window",
            "Default for every model (OLLAMA_CONTEXT_LENGTH); 0 = auto. Accepts 32k.",
            Kind::Num(
                |s| s.server.context_length.into(),
                |s, v| s.server.context_length = v.min(1 << 20) as u32,
            ),
        ),
        item(
            "Server",
            "Keep alive",
            "How long idle models stay loaded (OLLAMA_KEEP_ALIVE), e.g. 5m, 1h, -1.",
            Kind::Text(|s| &mut s.server.keep_alive),
        ),
        item(
            "Server",
            "Max loaded models",
            "0 = server default.",
            Kind::Num(
                |s| s.server.max_loaded_models.into(),
                |s, v| s.server.max_loaded_models = v as u32,
            ),
        ),
        item(
            "Server",
            "Parallel requests",
            "Per model (OLLAMA_NUM_PARALLEL); 0 = server default.",
            Kind::Num(
                |s| s.server.num_parallel.into(),
                |s, v| s.server.num_parallel = v as u32,
            ),
        ),
        item(
            "Server",
            "Max queue",
            "0 = server default.",
            Kind::Num(
                |s| s.server.max_queue.into(),
                |s, v| s.server.max_queue = v as u32,
            ),
        ),
        item(
            "Server",
            "Flash attention",
            "Less VRAM for long contexts.",
            Kind::Bool(|s| &mut s.server.flash_attention),
        ),
        item(
            "Server",
            "KV cache type",
            "Quantized cache (needs flash attention): q8_0 halves, q4_0 quarters its size.",
            Kind::Choice(
                &KV_TYPES,
                |s| {
                    KV_TYPES
                        .iter()
                        .position(|k| *k == s.server.kv_cache_type)
                        .unwrap_or(0)
                },
                |s, i| s.server.kv_cache_type = KV_TYPES[i].into(),
            ),
        ),
        item(
            "Server",
            "Models directory",
            "Empty = default (OLLAMA_MODELS).",
            Kind::Text(|s| &mut s.server.models_dir),
        ),
        item(
            "Server",
            "Allowed origins",
            "OLLAMA_ORIGINS, comma separated.",
            Kind::Text(|s| &mut s.server.origins),
        ),
        item(
            "Server",
            "Debug logging",
            "OLLAMA_DEBUG=1.",
            Kind::Bool(|s| &mut s.server.debug_logging),
        ),
        item(
            "Server",
            "Extra environment",
            "KEY=VALUE pairs separated by ';'.",
            Kind::Lines(|s| &mut s.server.extra_env),
        ),
        item(
            "Proxy",
            "Enabled",
            "Caddy in front of Ollama with HTTPS and bearer-token auth (P key).",
            Kind::Bool(|s| &mut s.proxy.enabled),
        ),
        item(
            "Proxy",
            "Start automatically",
            "",
            Kind::Bool(|s| &mut s.proxy.auto_start),
        ),
        item(
            "Proxy",
            "caddy binary",
            "",
            Kind::Text(|s| &mut s.proxy.caddy_path),
        ),
        item(
            "Proxy",
            "Listen host",
            "0.0.0.0 = every interface.",
            Kind::Text(|s| &mut s.proxy.listen_host),
        ),
        item(
            "Proxy",
            "Port",
            "",
            Kind::Num(
                |s| s.proxy.port.into(),
                |s, v| s.proxy.port = v.min(65535) as u16,
            ),
        ),
        item(
            "Proxy",
            "Domain",
            "Host name clients use; must be public for Let's Encrypt.",
            Kind::Text(|s| &mut s.proxy.domain),
        ),
        item(
            "Proxy",
            "TLS",
            "",
            Kind::Choice(
                &TLS_LABELS,
                |s| {
                    TLS_MODES
                        .iter()
                        .position(|m| *m == s.proxy.tls)
                        .unwrap_or(0)
                },
                |s, i| s.proxy.tls = TLS_MODES[i],
            ),
        ),
        item(
            "Proxy",
            "ACME email",
            "For Let's Encrypt.",
            Kind::Text(|s| &mut s.proxy.acme_email),
        ),
        item(
            "Proxy",
            "Certificate file",
            "For \"my certificate\".",
            Kind::Text(|s| &mut s.proxy.cert_path),
        ),
        item(
            "Proxy",
            "Key file",
            "For \"my certificate\".",
            Kind::Text(|s| &mut s.proxy.key_path),
        ),
        item(
            "Proxy",
            "Require token",
            "Reject requests without a valid bearer token.",
            Kind::Bool(|s| &mut s.proxy.require_token),
        ),
        item(
            "Proxy",
            "API tokens",
            "Enter to manage the tokens clients send as `Authorization: Bearer …`.",
            Kind::Tokens,
        ),
        item(
            "Proxy",
            "CORS origin",
            "Allowed browser origin, empty = none.",
            Kind::Text(|s| &mut s.proxy.cors_origin),
        ),
        item(
            "Logs",
            "Source",
            "Where the Logs tab reads the server log from.",
            Kind::Choice(
                &SOURCE_LABELS,
                |s| {
                    SOURCES
                        .iter()
                        .position(|m| *m == s.logs.source)
                        .unwrap_or(0)
                },
                |s, i| s.logs.source = SOURCES[i],
            ),
        ),
        item(
            "Logs",
            "Log file",
            "",
            Kind::Text(|s| &mut s.logs.file_path),
        ),
        item(
            "Logs",
            "Journal unit",
            "",
            Kind::Text(|s| &mut s.logs.journal_unit),
        ),
        item(
            "Logs",
            "Max lines",
            "Kept in memory.",
            Kind::Num(
                |s| s.logs.max_lines as u64,
                |s, v| s.logs.max_lines = v as usize,
            ),
        ),
        item(
            "Interface",
            "Poll interval (ms)",
            "",
            Kind::Num(|s| s.ui.poll_interval_ms, |s, v| s.ui.poll_interval_ms = v),
        ),
        item(
            "Interface",
            "History (s)",
            "Span of the Performance charts.",
            Kind::Num(|s| s.ui.history_seconds, |s, v| s.ui.history_seconds = v),
        ),
    ]
}

impl Item {
    /// Options of `ollama serve` and of the proxy in front of it, meaningless for a server started elsewhere.
    fn managed_only(&self) -> bool {
        match self.section {
            "Server" => self.label != "Manage server",
            "Proxy" => true,
            _ => false,
        }
    }
}

/// "32k" → 32768, "8192" → 8192.
fn parse_num(text: &str) -> Option<u64> {
    let t = text.trim().to_lowercase();
    match t.strip_suffix('k') {
        Some(n) => n.trim().parse::<u64>().ok().map(|n| n * 1024),
        None => t.parse().ok(),
    }
}

#[derive(Default)]
pub(super) struct State {
    pub sel: usize,
    /// The API tokens popup is open, with its selection.
    pub tokens_open: bool,
    pub token_sel: usize,
    pub reveal: bool,
}

impl Tui {
    /// Save keys shared by the Settings and GPU tabs.
    pub(super) fn save_key(&mut self, k: KeyEvent) -> bool {
        match k.code {
            KeyCode::Char('w') => self.ctl.apply(self.draft.clone(), false),
            KeyCode::Char('W') => self.ctl.apply(self.draft.clone(), true),
            KeyCode::Char('u') => self.draft = self.ctl.settings.clone(),
            _ => return false,
        }
        true
    }

    pub(super) fn save_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.draft_dirty() {
            vec![
                ("w", "save"),
                ("W", "save & restart"),
                ("u", "undo changes"),
            ]
        } else {
            Vec::new()
        }
    }

    pub(super) fn set_setting(&mut self, i: usize, text: String) {
        let Some(item) = items().into_iter().nth(i) else {
            return;
        };
        let s = &mut self.draft;
        match item.kind {
            Kind::Text(get) | Kind::Secret(get) => *get(s) = text.trim().to_string(),
            Kind::Lines(get) => {
                *get(s) = text
                    .split(';')
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Kind::Num(_, set) => match parse_num(&text) {
                Some(v) => set(s, v),
                None => self.ctl.logs.app_warn(format!("Not a number: {text}")),
            },
            Kind::Bool(_) | Kind::Choice(..) | Kind::Tokens => {}
        }
    }

    pub(super) fn add_token(&mut self, label: String) {
        let label = if label.trim().is_empty() {
            format!("token {}", self.draft.proxy.tokens.len() + 1)
        } else {
            label.trim().to_string()
        };
        self.draft.proxy.tokens.push(ApiToken {
            label,
            token: generate_token(),
        });
        self.settings.token_sel = self.draft.proxy.tokens.len() - 1;
        self.settings.reveal = true;
    }

    fn tokens_key(&mut self, k: KeyEvent) -> bool {
        let n = self.draft.proxy.tokens.len();
        let st = &mut self.settings;
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => kit::step(&mut st.token_sel, -1, n),
            KeyCode::Down | KeyCode::Char('j') => kit::step(&mut st.token_sel, 1, n),
            KeyCode::Char('r') => st.reveal = !st.reveal,
            KeyCode::Char('n') => self.edit(Field::TokenLabel, ""),
            KeyCode::Char('d') if n > 0 => self.confirm = Some(Confirm::DeleteToken(st.token_sel)),
            KeyCode::Esc | KeyCode::Enter => st.tokens_open = false,
            _ => return self.save_key(k),
        }
        true
    }

    pub(super) fn settings_key(&mut self, k: KeyEvent) -> bool {
        if self.settings.tokens_open {
            return self.tokens_key(k);
        }
        let items = items();
        let n = items.len();
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => kit::step(&mut self.settings.sel, -1, n),
            KeyCode::Down | KeyCode::Char('j') => kit::step(&mut self.settings.sel, 1, n),
            KeyCode::PageUp => kit::step(&mut self.settings.sel, -10, n),
            KeyCode::PageDown => kit::step(&mut self.settings.sel, 10, n),
            KeyCode::Home => self.settings.sel = 0,
            KeyCode::End => self.settings.sel = n - 1,
            KeyCode::Char('D') => self.confirm = Some(Confirm::ResetSettings),
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right
                if items[self.settings.sel].managed_only() && !self.draft.server.manage_server => {}
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right => {
                let i = self.settings.sel;
                let back = k.code == KeyCode::Left;
                let s = &mut self.draft;
                match items[i].kind {
                    Kind::Bool(get) => {
                        let v = get(s);
                        *v = !*v;
                    }
                    Kind::Choice(options, get, set) => {
                        let n = options.len();
                        let cur = get(s);
                        set(
                            s,
                            if back {
                                (cur + n - 1) % n
                            } else {
                                (cur + 1) % n
                            },
                        );
                    }
                    _ if matches!(k.code, KeyCode::Left | KeyCode::Right) => return false,
                    Kind::Text(get) => {
                        let v = get(s).clone();
                        self.edit(Field::Setting(i), &v);
                    }
                    Kind::Secret(get) => {
                        let v = get(s).clone();
                        self.edit(Field::Setting(i), &v);
                        if let Some(e) = &mut self.editing {
                            e.masked = !self.settings.reveal;
                        }
                    }
                    Kind::Lines(get) => {
                        let v = get(s).lines().collect::<Vec<_>>().join("; ");
                        self.edit(Field::Setting(i), &v);
                    }
                    Kind::Num(get, _) => {
                        let v = get(s).to_string();
                        self.edit(Field::Setting(i), &v);
                    }
                    Kind::Tokens => {
                        self.settings.tokens_open = true;
                        self.settings.token_sel = 0;
                    }
                }
            }
            KeyCode::Char('r') => self.settings.reveal = !self.settings.reveal,
            _ => return self.save_key(k),
        }
        true
    }

    pub(super) fn settings_hints(&self) -> Vec<(&'static str, &'static str)> {
        let mut hints = if self.settings.tokens_open {
            vec![
                ("n", "new token"),
                ("d", "delete"),
                ("r", "reveal"),
                ("Esc", "close"),
            ]
        } else {
            vec![
                ("Enter", "edit / toggle"),
                ("← →", "change choice"),
                ("r", "reveal secrets"),
                ("D", "defaults"),
            ]
        };
        hints.extend(self.save_hints());
        hints
    }

    fn value_span(&mut self, kind: &Kind) -> Span<'static> {
        let s = &mut self.draft;
        match kind {
            Kind::Bool(get) => {
                if *get(s) {
                    Span::styled("[x] on", Style::new().fg(GREEN))
                } else {
                    Span::styled("[ ] off", Style::new().dark_gray())
                }
            }
            Kind::Text(get) => {
                let v = get(s).clone();
                if v.is_empty() {
                    kit::weak("(empty)")
                } else {
                    Span::raw(v)
                }
            }
            Kind::Secret(get) => {
                let v = get(s).clone();
                if v.is_empty() {
                    kit::weak("(none)")
                } else if self.settings.reveal {
                    Span::raw(v)
                } else {
                    Span::raw(mask_token(&v))
                }
            }
            Kind::Num(get, _) => Span::raw(get(s).to_string()),
            Kind::Choice(options, get, _) => Span::raw(format!("‹ {} ›", options[get(s)])),
            Kind::Lines(get) => {
                let v = get(s).lines().collect::<Vec<_>>().join("; ");
                if v.is_empty() {
                    kit::weak("(none)")
                } else {
                    Span::raw(v)
                }
            }
            Kind::Tokens => Span::raw(format!(
                "{} token(s) — Enter to manage",
                s.proxy.tokens.len()
            )),
        }
    }

    pub(super) fn draw_settings(&mut self, f: &mut Frame, area: Rect) {
        let items = items();
        let [list_area, help_area] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(4)]).areas(area);

        let mut lines = Vec::new();
        // Line of each item, for the selection scroll and the click regions.
        let mut item_lines = Vec::new();
        let mut sel_line = 0;
        let mut section = "";
        for (i, item) in items.iter().enumerate() {
            if item.section != section {
                section = item.section;
                if !lines.is_empty() {
                    lines.push(Line::raw(""));
                }
                lines.push(Line::from(Span::styled(
                    section,
                    Style::new().fg(kit::ACCENT).bold(),
                )));
            }
            let selected = i == self.settings.sel;
            if selected {
                sel_line = lines.len();
            }
            item_lines.push(lines.len());
            let editing =
                matches!(&self.editing, Some(Editing { field: Field::Setting(j), .. }) if *j == i);
            let value = if editing {
                Span::styled("editing below…", Style::new().fg(AMBER))
            } else {
                self.value_span(&item.kind)
            };
            let line = Line::from(vec![Span::raw(format!("  {:<24}", item.label)), value]);
            // Greyed out while the server is not managed by this app.
            let line = if item.managed_only() && !self.draft.server.manage_server {
                Line::from(
                    line.spans
                        .into_iter()
                        .map(|s| s.dark_gray())
                        .collect::<Vec<_>>(),
                )
            } else {
                line
            };
            lines.push(if selected {
                line.style(kit::selected())
            } else {
                line
            });
        }
        let block = kit::panel(if self.draft_dirty() {
            "Settings · unsaved changes (w save, W save & restart services)"
        } else {
            "Settings"
        });
        let height = block.inner(list_area).height as usize;
        // Keep the selection in view, with a little context below it.
        let scroll = (sel_line + 3).saturating_sub(height);
        let inner = block.inner(list_area);
        f.render_widget(
            Paragraph::new(lines)
                .scroll((scroll as u16, 0))
                .block(block),
            list_area,
        );
        for (i, line) in item_lines.into_iter().enumerate() {
            if let Some(y) = line.checked_sub(scroll).filter(|y| *y < height) {
                self.hit(
                    Rect::new(inner.x, inner.y + y as u16, inner.width, 1),
                    Hit::Row(i),
                );
            }
        }

        let item = &items[self.settings.sel];
        let mut help = vec![Line::raw(item.help)];
        if item.managed_only() && !self.draft.server.manage_server {
            help.push(Line::from(Span::styled(
                "Only used for a server started by this app: enable \"Manage server\" first.",
                Style::new().fg(AMBER),
            )));
        } else if item.section == "Server" {
            help.push(Line::from(kit::weak(
                "Server settings take effect when the managed server restarts (W). GPU selection is in the GPU tab.",
            )));
        }
        f.render_widget(
            Paragraph::new(help)
                .wrap(Wrap { trim: true })
                .block(kit::panel(item.label)),
            help_area,
        );

        if self.settings.tokens_open {
            self.draw_tokens(f);
        }
    }

    fn draw_tokens(&self, f: &mut Frame) {
        let tokens = &self.draft.proxy.tokens;
        let mut lines: Vec<Line> = tokens
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let token = if self.settings.reveal {
                    t.token.clone()
                } else {
                    mask_token(&t.token)
                };
                let line = Line::from(vec![
                    Span::raw(format!(" {:<20} ", t.label)),
                    Span::raw(token),
                ]);
                if i == self.settings.token_sel {
                    line.style(kit::selected())
                } else {
                    line
                }
            })
            .collect();
        if lines.is_empty() {
            lines.push(Line::from(kit::weak(
                " No token yet: press n to generate one.",
            )));
        }
        lines.push(Line::raw(""));
        lines.push(Line::from(kit::weak(
            " New tokens are saved with w. Give each client its own token.",
        )));
        let area = kit::popup(f, 100, lines.len() as u16 + 2);
        self.hit(area, Hit::Block);
        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        self.hit_rows(inner, 0, tokens.len());
        f.render_widget(Paragraph::new(lines).block(kit::panel("API tokens")), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numbers_with_k_suffix() {
        assert_eq!(parse_num("32k"), Some(32768));
        assert_eq!(parse_num(" 8192 "), Some(8192));
        assert_eq!(parse_num("abc"), None);
    }

    #[test]
    fn every_choice_maps_back() {
        let mut s = Settings::default();
        for item in items() {
            if let Kind::Choice(options, get, set) = item.kind {
                for i in 0..options.len() {
                    set(&mut s, i);
                    assert_eq!(get(&s), i, "{}", item.label);
                }
            }
        }
    }
}
