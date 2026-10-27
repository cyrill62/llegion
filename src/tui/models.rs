//! Models tab: downloaded models, load/unload, pull, delete and details.

use std::sync::atomic::Ordering;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Gauge, Paragraph, Row, Table, TableState, Wrap};

use super::kit::{self, GREEN, RED};
use super::{Confirm, Field, Hit, Tab, Tui};
use crate::i18n::tr;
use crate::ollama::LocalModel;
use crate::util::{CONTEXT_PRESETS, ctx_label, format_bytes};

#[derive(Default)]
pub(super) struct State {
    pub filter: String,
    pub sel: usize,
    /// Context size for "load" (0 = server default).
    pub load_ctx: u32,
    /// Model whose details popup is open, and its scroll offset.
    pub info: Option<String>,
    pub info_scroll: u16,
}

impl Tui {
    fn filtered_models(&self) -> Vec<LocalModel> {
        let filter = self.models.filter.to_lowercase();
        self.ctl
            .backend
            .shared
            .lock()
            .unwrap()
            .models
            .iter()
            .filter(|m| filter.is_empty() || m.name.to_lowercase().contains(&filter))
            .cloned()
            .collect()
    }

    fn selected_model(&self) -> Option<String> {
        self.filtered_models()
            .get(self.models.sel)
            .map(|m| m.name.clone())
    }

    pub(super) fn models_key(&mut self, k: KeyEvent) -> bool {
        if self.models.info.is_some() {
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.models.info_scroll = self.models.info_scroll.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => self.models.info_scroll += 1,
                KeyCode::PageUp => {
                    self.models.info_scroll = self.models.info_scroll.saturating_sub(10)
                }
                KeyCode::PageDown => self.models.info_scroll += 10,
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('i') => self.models.info = None,
                _ => return false,
            }
            return true;
        }
        let len = self.filtered_models().len();
        let sel = self.selected_model();
        let backend = &self.ctl.backend;
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => kit::step(&mut self.models.sel, -1, len),
            KeyCode::Down | KeyCode::Char('j') => kit::step(&mut self.models.sel, 1, len),
            KeyCode::PageUp => kit::step(&mut self.models.sel, -10, len),
            KeyCode::PageDown => kit::step(&mut self.models.sel, 10, len),
            KeyCode::Home => self.models.sel = 0,
            KeyCode::End => self.models.sel = len.saturating_sub(1),
            KeyCode::Char('/') => self.edit(Field::ModelFilter, &self.models.filter.clone()),
            KeyCode::Char('p') => self.edit(Field::PullName, ""),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.models.load_ctx = kit::cycle(
                    &CONTEXT_PRESETS,
                    &self.models.load_ctx,
                    k.code == KeyCode::Char('C'),
                )
            }
            KeyCode::Char('x') => {
                let mut s = backend.shared.lock().unwrap();
                for p in s.pulls.values() {
                    p.cancel.store(true, Ordering::Relaxed);
                }
                s.pulls.retain(|_, p| !p.done);
            }
            _ => {
                let Some(name) = sel else { return false };
                match k.code {
                    KeyCode::Enter | KeyCode::Char('i') => {
                        backend.show(&name);
                        self.models.info = Some(name);
                        self.models.info_scroll = 0;
                    }
                    KeyCode::Char('l') => {
                        let ctx = (self.models.load_ctx > 0).then_some(self.models.load_ctx);
                        backend.load(&name, ctx);
                    }
                    KeyCode::Char('u') => backend.unload(&name),
                    KeyCode::Char('d') => self.confirm = Some(Confirm::DeleteModel(name)),
                    KeyCode::Char('b') => {
                        self.bench.model = name;
                        self.tab = Tab::Performance;
                    }
                    KeyCode::Char('t') => {
                        self.chat.model = name;
                        self.tab = Tab::Chat;
                    }
                    _ => return false,
                }
            }
        }
        true
    }

    pub(super) fn models_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.models.info.is_some() {
            return vec![("↑↓", tr("scroll")), ("Esc", tr("close"))];
        }
        vec![
            ("Enter", tr("details")),
            ("l", tr("load")),
            ("u", tr("unload")),
            ("c", tr("load context")),
            ("p", tr("pull")),
            ("x", tr("cancel/clear pulls")),
            ("d", tr("delete")),
            ("t", tr("chat")),
            ("b", tr("benchmark")),
            ("/", tr("filter")),
        ]
    }

    pub(super) fn draw_models(&mut self, f: &mut Frame, area: Rect) {
        let (running, pulls, busy) = {
            let s = self.ctl.backend.shared.lock().unwrap();
            (s.running.clone(), s.pulls.clone(), s.busy.clone())
        };
        let models = self.filtered_models();
        kit::step(&mut self.models.sel, 0, models.len());

        let running_h = running.len().max(1) as u16 + 3;
        let pulls_h = if pulls.is_empty() {
            0
        } else {
            pulls.len() as u16 + 2
        };
        let [run_area, pull_area, list_area] = Layout::vertical([
            Constraint::Length(running_h),
            Constraint::Length(pulls_h),
            Constraint::Min(5),
        ])
        .areas(area);
        super::dashboard::running_table(f, run_area, &running, None, tr("Running models"));

        if !pulls.is_empty() {
            let block = kit::panel(tr("Pulls"));
            let inner = block.inner(pull_area);
            f.render_widget(block, pull_area);
            for (i, (name, p)) in pulls.iter().enumerate() {
                let row = Rect::new(inner.x, inner.y + i as u16, inner.width, 1);
                let [name_area, bar_area] =
                    Layout::horizontal([Constraint::Length(28), Constraint::Min(10)]).areas(row);
                f.render_widget(Span::raw(name.clone()).bold(), name_area);
                if let Some(err) = &p.error {
                    f.render_widget(Span::styled(err.clone(), Style::new().fg(RED)), bar_area);
                } else if p.done {
                    f.render_widget(Span::styled(tr("done"), Style::new().fg(GREEN)), bar_area);
                } else {
                    let ratio = if p.total > 0 {
                        p.completed as f64 / p.total as f64
                    } else {
                        0.0
                    };
                    let label = if p.total > 0 {
                        format!(
                            "{} — {} / {}",
                            p.status,
                            format_bytes(p.completed),
                            format_bytes(p.total)
                        )
                    } else {
                        p.status.clone()
                    };
                    f.render_widget(
                        Gauge::default()
                            .ratio(ratio.clamp(0.0, 1.0))
                            .label(label)
                            .gauge_style(Style::new().fg(kit::ACCENT)),
                        bar_area,
                    );
                }
            }
        }

        let rows = models.iter().map(|m| {
            let state = if busy.contains(&m.name) {
                Span::styled(tr("busy…"), Style::new().fg(kit::AMBER))
            } else if running.iter().any(|r| r.name == m.name) {
                Span::styled(tr("loaded"), Style::new().fg(GREEN))
            } else {
                Span::raw("")
            };
            Row::new(vec![
                Cell::from(m.name.clone().bold()),
                Cell::from(m.details.parameter_size.clone()),
                Cell::from(m.details.quantization_level.clone()),
                Cell::from(m.details.family.clone()),
                Cell::from(format_bytes(m.size)),
                Cell::from(
                    m.modified_at
                        .get(..10)
                        .unwrap_or(&m.modified_at)
                        .to_string(),
                ),
                Cell::from(state),
            ])
        });
        let header = Row::new([
            tr("Name"),
            tr("Params"),
            tr("Quant"),
            tr("Family"),
            tr("Size"),
            tr("Modified"),
            "",
        ])
        .style(Style::new().dark_gray());
        let widths = [
            Constraint::Fill(3),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(11),
            Constraint::Length(7),
        ];
        let mut title = trf!(
            "Downloaded models · load context: {context}",
            context = ctx_label(self.models.load_ctx)
        );
        if !self.models.filter.is_empty() {
            title.push_str(&format!(
                " · {}",
                trf!("filter \"{filter}\"", filter = self.models.filter)
            ));
        }
        let mut state = TableState::default().with_selected(Some(self.models.sel));
        f.render_stateful_widget(
            Table::new(rows, widths)
                .header(header)
                .row_highlight_style(kit::selected())
                .block(kit::panel(&title)),
            list_area,
            &mut state,
        );
        self.hit_table(list_area, &state, models.len());

        self.draw_info(f);
    }

    fn draw_info(&self, f: &mut Frame) {
        let Some(name) = &self.models.info else {
            return;
        };
        let detail = self
            .ctl
            .backend
            .shared
            .lock()
            .unwrap()
            .details
            .get(name)
            .cloned();
        let lines: Vec<Line> = match detail {
            None => vec![Line::from(kit::weak(tr("Loading…")))],
            Some(Err(e)) => vec![Line::from(Span::styled(e, Style::new().fg(RED)))],
            Some(Ok(d)) => {
                let mut lines = vec![
                    kit::kv(tr("Family"), Span::raw(d.details.family.clone())),
                    kit::kv(
                        tr("Parameters"),
                        Span::raw(d.details.parameter_size.clone()),
                    ),
                    kit::kv(
                        tr("Quantization"),
                        Span::raw(d.details.quantization_level.clone()),
                    ),
                    kit::kv(tr("Format"), Span::raw(d.details.format.clone())),
                    kit::kv(
                        tr("Native context"),
                        Span::raw(
                            d.native_context_length()
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "-".into()),
                        ),
                    ),
                    kit::kv(tr("Capabilities"), Span::raw(d.capabilities.join(", "))),
                ];
                let model_info = d
                    .model_info
                    .iter()
                    .map(|(k, v)| (k, v.to_string()))
                    .filter(|(_, v)| v.len() < 200)
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                for (title, content) in [
                    (tr("Parameters"), d.parameters.as_str()),
                    (tr("Template"), d.template.as_str()),
                    (tr("Model info"), model_info.as_str()),
                    (tr("License"), d.license.as_str()),
                ] {
                    if content.trim().is_empty() {
                        continue;
                    }
                    lines.push(Line::raw(""));
                    lines.push(Line::from(Span::styled(
                        title,
                        Style::new().fg(kit::ACCENT).bold(),
                    )));
                    lines.extend(content.lines().map(|l| Line::raw(l.to_string())));
                }
                lines
            }
        };
        let area = kit::popup(
            f,
            f.area().width.saturating_sub(8).min(110),
            f.area().height.saturating_sub(4),
        );
        self.hit(area, Hit::Block);
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((self.models.info_scroll, 0))
                .block(kit::panel(name)),
            area,
        );
    }
}
