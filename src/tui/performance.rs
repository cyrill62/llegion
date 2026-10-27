//! Performance tab: CPU / memory / GPU history charts and benchmarks.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table, TableState};

use super::kit::{self, GREEN};
use super::{Field, Tui};
use crate::backend::{Sample, preferred_model};
use crate::i18n::tr;
use crate::util::{CONTEXT_PRESETS, ctx_label, ctx_short};

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const SERIES: [Color; 6] = [
    Color::Cyan,
    Color::Magenta,
    Color::Yellow,
    Color::Green,
    Color::Blue,
    Color::Red,
];

pub(super) struct State {
    pub model: String,
    pub prompt: String,
    pub tokens: u32,
    pub ctx: u32,
    /// Selected result (newest first).
    pub sel: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            model: String::new(),
            prompt: "Write a 300-word story about a lighthouse keeper who discovers a message in a bottle.".into(),
            tokens: 256,
            ctx: 0,
            sel: 0,
        }
    }
}

/// Value extractor for one chart line.
type Series<'a> = (String, Box<dyn Fn(&Sample) -> Option<f64> + 'a>);

/// Title, y-axis maximum and unit of a chart.
struct Plot<'a> {
    title: &'a str,
    max: f64,
    unit: &'a str,
}

/// One line chart over the last `window` seconds of the sample history.
fn chart(
    f: &mut Frame,
    area: Rect,
    plot: Plot,
    history: &[Sample],
    window: f64,
    series: Vec<Series>,
) {
    let Plot { title, max, unit } = plot;
    let now = history.last().map_or(0.0, |s| s.t);
    let points: Vec<Vec<(f64, f64)>> = series
        .iter()
        .map(|(_, get)| {
            history
                .iter()
                .filter_map(|s| get(s).map(|v| (s.t, v)))
                .collect()
        })
        .collect();
    let datasets = series
        .iter()
        .zip(&points)
        .enumerate()
        .map(|(i, ((name, _), pts))| {
            let last = pts
                .last()
                .map_or(String::new(), |p| format!(" {:.1}{unit}", p.1));
            Dataset::default()
                .name(format!("{name}{last}"))
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(SERIES[i % SERIES.len()]))
                .data(pts)
        })
        .collect();
    let max = max.max(1e-6);
    f.render_widget(
        Chart::new(datasets)
            .block(kit::panel(title))
            .x_axis(Axis::default().bounds([now - window, now]))
            .y_axis(
                Axis::default()
                    .bounds([0.0, max])
                    .labels([Span::raw("0"), Span::raw(format!("{max:.0}{unit}"))])
                    .style(Style::new().dark_gray()),
            )
            .legend_position(Some(ratatui::widgets::LegendPosition::TopLeft))
            .hidden_legend_constraints((Constraint::Ratio(1, 1), Constraint::Ratio(1, 1))),
        area,
    );
}

impl Tui {
    pub(super) fn performance_key(&mut self, k: KeyEvent) -> bool {
        let backend = self.ctl.backend.clone();
        let n_results = backend.shared.lock().unwrap().benchmarks.len();
        let b = &mut self.bench;
        match k.code {
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
                    b.model = kit::cycle(&names, &b.model, k.code == KeyCode::Char('M'));
                }
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                b.ctx = kit::cycle(&CONTEXT_PRESETS, &b.ctx, k.code == KeyCode::Char('C'))
            }
            KeyCode::Char('+') | KeyCode::Char('=') => b.tokens = (b.tokens * 2).min(4096),
            KeyCode::Char('-') => b.tokens = (b.tokens / 2).max(16),
            KeyCode::Char('e') => {
                let prompt = b.prompt.clone();
                self.edit(Field::BenchPrompt, &prompt);
            }
            KeyCode::Up | KeyCode::Char('k') => kit::step(&mut b.sel, -1, n_results),
            KeyCode::Down | KeyCode::Char('j') => kit::step(&mut b.sel, 1, n_results),
            KeyCode::Char('x') => backend.clear_benchmarks(),
            KeyCode::Enter | KeyCode::Char('b') => {
                let running = backend.shared.lock().unwrap().bench_running;
                if !b.model.is_empty() && !running {
                    let ctx = (b.ctx > 0).then_some(b.ctx);
                    backend.benchmark(&b.model, &b.prompt, b.tokens, ctx, self.ctl.server_env());
                    b.sel = 0;
                }
            }
            _ => return false,
        }
        true
    }

    pub(super) fn performance_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("Enter", tr("run benchmark")),
            ("m", tr("model")),
            ("c", tr("context")),
            ("+/-", tr("tokens")),
            ("e", tr("prompt")),
            ("↑↓", tr("select result")),
            ("x", tr("clear results")),
        ]
    }

    pub(super) fn draw_performance(&mut self, f: &mut Frame, area: Rect) {
        let (history, gpus, models, running, benchmarks, bench_running) = {
            let s = self.ctl.backend.shared.lock().unwrap();
            (
                s.history.iter().cloned().collect::<Vec<_>>(),
                s.gpus.clone(),
                s.models.clone(),
                s.running.clone(),
                s.benchmarks.clone(),
                s.bench_running,
            )
        };
        self.bench.model = preferred_model(&self.bench.model, &models, &running);
        let window = self.ctl.settings.ui.history_seconds as f64;

        let [charts, form, results] = Layout::vertical([
            Constraint::Percentage(45),
            Constraint::Length(7),
            Constraint::Min(5),
        ])
        .areas(area);
        // GPU charts only when there is a GPU to plot.
        let gpu_height = if gpus.is_empty() {
            Constraint::Length(0)
        } else {
            Constraint::Ratio(1, 2)
        };
        let [top, bottom] = Layout::vertical([Constraint::Fill(1), gpu_height]).areas(charts);
        let [cpu_area, ram_area] = Layout::horizontal([Constraint::Ratio(1, 2); 2]).areas(top);
        let [util_area, vram_area] = Layout::horizontal([Constraint::Ratio(1, 2); 2]).areas(bottom);

        chart(
            f,
            cpu_area,
            Plot {
                title: "CPU",
                max: 100.0,
                unit: "%",
            },
            &history,
            window,
            vec![
                ("system".into(), Box::new(|s: &Sample| Some(s.cpu as f64))),
                (
                    "ollama".into(),
                    Box::new(|s: &Sample| Some(s.ollama_cpu as f64)),
                ),
            ],
        );
        let ram_total = history.last().map_or(0, |s| s.ram_total) as f64 / GIB;
        chart(
            f,
            ram_area,
            Plot {
                title: tr("Memory (GiB)"),
                max: ram_total,
                unit: "",
            },
            &history,
            window,
            vec![
                (
                    "used".into(),
                    Box::new(|s: &Sample| Some(s.ram_used as f64 / GIB)),
                ),
                (
                    "ollama".into(),
                    Box::new(|s: &Sample| Some(s.ollama_mem as f64 / GIB)),
                ),
            ],
        );
        let util_series = (0..gpus.len())
            .map(|i| -> Series {
                (
                    format!("#{i}"),
                    Box::new(move |s: &Sample| s.gpus.get(i).and_then(|g| g.util).map(f64::from)),
                )
            })
            .collect();
        chart(
            f,
            util_area,
            Plot {
                title: tr("GPU utilization"),
                max: 100.0,
                unit: "%",
            },
            &history,
            window,
            util_series,
        );
        let vram_max = gpus.iter().map(|g| g.vram_total).max().unwrap_or(0) as f64 / GIB;
        let vram_series = (0..gpus.len())
            .map(|i| -> Series {
                (
                    format!("#{i}"),
                    Box::new(move |s: &Sample| {
                        s.gpus
                            .get(i)
                            .and_then(|g| g.vram_used)
                            .map(|v| v as f64 / GIB)
                    }),
                )
            })
            .collect();
        chart(
            f,
            vram_area,
            Plot {
                title: tr("VRAM (GiB)"),
                max: vram_max,
                unit: "",
            },
            &history,
            window,
            vram_series,
        );

        // Benchmark form.
        let mut model_line = vec![
            kit::label(tr("Model")),
            Span::styled(
                if self.bench.model.is_empty() {
                    tr("none downloaded").to_string()
                } else {
                    self.bench.model.clone()
                },
                Style::new().fg(kit::ACCENT).bold(),
            ),
        ];
        if let Some(r) = running.iter().find(|r| r.name == self.bench.model) {
            let ctx = r
                .context_length
                .map(|c| format!(" · {} ctx", ctx_short(c)))
                .unwrap_or_default();
            model_line.push(Span::styled(
                format!(
                    "  {}{ctx}",
                    trf!("loaded · {processor}", processor = r.processor())
                ),
                Style::new().fg(GREEN),
            ));
        }
        let mut form_lines = vec![
            Line::from(model_line),
            kit::kv(tr("Prompt"), Span::raw(self.bench.prompt.clone())),
            kit::kv(tr("Tokens"), Span::raw(self.bench.tokens.to_string())),
            kit::kv(tr("Context"), Span::raw(ctx_label(self.bench.ctx))),
        ];
        if bench_running {
            form_lines.push(Line::from(Span::styled(
                tr("Running…"),
                Style::new().fg(kit::AMBER),
            )));
        }
        f.render_widget(
            Paragraph::new(form_lines).block(kit::panel(tr("Benchmark"))),
            form,
        );

        // Results, newest first, with the selected run's server config below.
        let [table_area, detail_area] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(4)]).areas(results);
        let rows = benchmarks.iter().rev().map(|b| {
            let requested = b
                .num_ctx
                .map(|c| c.to_string())
                .unwrap_or_else(|| tr("auto").into());
            Row::new(vec![
                Cell::from(b.when.clone()),
                Cell::from(b.model.clone().bold()),
                Cell::from(b.loaded_ctx.map(|c| c.to_string()).unwrap_or(requested)),
                Cell::from(b.processor.clone().unwrap_or_else(|| "-".into())),
                Cell::from(b.server_summary()),
                Cell::from(format!("{:.1}", b.stats.prompt_tps())),
                Cell::from(Span::styled(
                    format!("{:.1}", b.stats.gen_tps()),
                    Style::new().fg(GREEN).bold(),
                )),
                Cell::from(format!(
                    "{} + {}",
                    b.stats.prompt_eval_count, b.stats.eval_count
                )),
                Cell::from(format!("{:.2}s", b.stats.load_duration as f64 / 1e9)),
                Cell::from(format!("{:.2}s", b.stats.total_duration as f64 / 1e9)),
            ])
        });
        let header = Row::new([
            tr("Time"),
            tr("Model"),
            tr("Context"),
            tr("Processor"),
            tr("Server"),
            tr("Prompt t/s"),
            tr("Gen t/s"),
            tr("Tokens"),
            tr("Load"),
            tr("Total"),
        ])
        .style(Style::new().dark_gray());
        let widths = [
            Constraint::Length(8),
            Constraint::Fill(2),
            Constraint::Length(8),
            Constraint::Length(16),
            Constraint::Fill(2),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(11),
            Constraint::Length(7),
            Constraint::Length(7),
        ];
        kit::step(&mut self.bench.sel, 0, benchmarks.len());
        let mut state =
            TableState::default().with_selected((!benchmarks.is_empty()).then_some(self.bench.sel));
        f.render_stateful_widget(
            Table::new(rows, widths)
                .header(header)
                .row_highlight_style(kit::selected())
                .block(kit::panel(tr("Results"))),
            table_area,
            &mut state,
        );
        self.hit_table(table_area, &state, benchmarks.len());
        let detail = benchmarks
            .iter()
            .rev()
            .nth(self.bench.sel)
            .map(|b| {
                let requested = b.num_ctx.map_or(tr("auto").into(), |c| c.to_string());
                trf!(
                    "requested context {requested} · {details}",
                    requested = requested,
                    details = b.server_details().replace('\n', "  ")
                )
            })
            .unwrap_or_default();
        f.render_widget(
            Paragraph::new(detail)
                .wrap(ratatui::widgets::Wrap { trim: true })
                .block(kit::panel(tr("Server configuration of the selected run"))),
            detail_area,
        );
    }
}
