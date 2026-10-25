//! Dashboard tab: server, proxy and system status, GPUs and running models.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use super::Tui;
use super::kit::{self, AMBER, GREEN, RED};
use crate::caddy;
use crate::ollama::RunningModel;
use crate::util::{ctx_short, format_bytes, format_duration_secs};

impl Tui {
    pub(super) fn dashboard_hints(&self) -> Vec<(&'static str, &'static str)> {
        let mut hints = Vec::new();
        let settings = &self.ctl.settings;
        if settings.server.manage_server {
            if self.ctl.ollama_proc.is_running() {
                hints.extend([("S", "stop server"), ("R", "restart server")]);
            } else {
                hints.push(("S", "start server"));
            }
        }
        if settings.proxy.enabled {
            hints.push((
                "P",
                if self.ctl.caddy_proc.is_running() {
                    "stop proxy"
                } else {
                    "start proxy"
                },
            ));
        }
        hints.push(("F5", "refresh"));
        hints
    }

    pub(super) fn draw_dashboard(&self, f: &mut Frame, area: Rect) {
        let s = self.ctl.backend.shared.lock().unwrap();
        let last = s.history.back().cloned();
        let running = s.running.clone();
        let n_models = s.models.len();
        let disk: u64 = s.models.iter().map(|m| m.size).sum();
        let (gpus, gpus_detected) = (s.gpus.clone(), s.gpus_detected);
        let (connected, version, error) = (s.connected, s.version.clone(), s.error.clone());
        drop(s);

        let gpu_rows = gpus.len().max(1) as u16;
        let [tiles, system, models] = Layout::vertical([
            Constraint::Length(6),
            Constraint::Length(gpu_rows + 4),
            Constraint::Min(4),
        ])
        .areas(area);
        let [conn_area, server_area, proxy_area] =
            Layout::horizontal([Constraint::Ratio(1, 3); 3]).areas(tiles);

        let settings = &self.ctl.settings;
        let mut conn = vec![
            kit::kv("API", Span::raw(settings.connection.base_url.clone())),
            if connected {
                kit::kv(
                    "Status",
                    Span::styled(
                        format!("online · v{}", version.unwrap_or_default()),
                        Style::new().fg(GREEN),
                    ),
                )
            } else {
                kit::kv("Status", Span::styled("unreachable", Style::new().fg(RED)))
            },
            kit::kv(
                "Models",
                Span::raw(format!("{n_models} ({} on disk)", format_bytes(disk))),
            ),
        ];
        if let Some(err) = error.filter(|_| !connected) {
            conn.push(Line::from(kit::weak(err)));
        }
        f.render_widget(
            Paragraph::new(conn).block(kit::panel("Connection")),
            conn_area,
        );

        let p = &self.ctl.ollama_proc;
        let server = if !settings.server.manage_server {
            vec![
                kit::kv("Mode", kit::weak("external")),
                Line::from(kit::weak("Enable \"Manage server\" in Settings")),
                Line::from(kit::weak("to start/stop Ollama from here.")),
            ]
        } else {
            let state = if p.is_running() {
                Span::styled(
                    format!("running · pid {}", p.pid().unwrap_or(0)),
                    Style::new().fg(GREEN),
                )
            } else {
                Span::styled("stopped", Style::new().fg(AMBER))
            };
            let uptime = p
                .started
                .map(|t| format_duration_secs(t.elapsed().as_secs() as i64))
                .unwrap_or_else(|| "-".into());
            let ctx = match settings.server.context_length {
                0 => "server default".into(),
                c => format!("{} tokens", ctx_short(c as u64)),
            };
            vec![
                kit::kv("State", state),
                kit::kv("Uptime", Span::raw(uptime)),
                kit::kv("Context", Span::raw(ctx)),
                Line::from(kit::weak("S start/stop · R restart")),
            ]
        };
        f.render_widget(
            Paragraph::new(server).block(kit::panel("Managed server")),
            server_area,
        );

        let proxy = if !settings.proxy.enabled {
            vec![Line::from(kit::weak("Disabled (Settings → Proxy)."))]
        } else {
            let state = if self.ctl.caddy_proc.is_running() {
                Span::styled("running", Style::new().fg(GREEN))
            } else {
                Span::styled("stopped", Style::new().fg(AMBER))
            };
            vec![
                kit::kv("State", state),
                kit::kv("URL", Span::raw(caddy::public_url(settings))),
                kit::kv("Tokens", Span::raw(settings.proxy.tokens.len().to_string())),
                Line::from(kit::weak("P start/stop")),
            ]
        };
        f.render_widget(
            Paragraph::new(proxy).block(kit::panel("HTTPS proxy")),
            proxy_area,
        );

        let mut sys = Vec::new();
        if let Some(sample) = &last {
            let ram = sample.ram_used as f64 / sample.ram_total.max(1) as f64;
            sys.push(Line::from(vec![
                kit::label("CPU"),
                kit::bar(sample.cpu as f64 / 100.0, 20),
                Span::raw(format!(
                    " {:>5.1}%   ollama {:.1}%",
                    sample.cpu, sample.ollama_cpu
                )),
            ]));
            sys.push(Line::from(vec![
                kit::label("RAM"),
                kit::bar(ram, 20),
                Span::raw(format!(
                    " {} / {}   ollama {}",
                    format_bytes(sample.ram_used),
                    format_bytes(sample.ram_total),
                    format_bytes(sample.ollama_mem)
                )),
            ]));
        }
        if !gpus_detected {
            sys.push(Line::from(kit::weak("Detecting GPUs…")));
        } else if gpus.is_empty() {
            sys.push(Line::from(kit::weak(
                "No GPU detected: Ollama runs on the CPU.",
            )));
        }
        for g in &gpus {
            let mut spans = vec![kit::label(&format!("GPU {}", g.vendor.label()))];
            match g.vram_used {
                Some(used) if g.vram_total > 0 => {
                    spans.push(kit::bar(used as f64 / g.vram_total as f64, 20));
                    spans.push(Span::raw(format!(
                        " {} / {}",
                        format_bytes(used),
                        format_bytes(g.vram_total)
                    )));
                }
                _ => spans.push(Span::raw(format!("{} VRAM", format_bytes(g.vram_total)))),
            }
            if let Some(u) = g.utilization {
                spans.push(Span::raw(format!("   util {u:.0}%")));
            }
            if let Some(t) = g.temperature {
                spans.push(Span::raw(format!("   {t:.0}°C")));
            }
            spans.push(kit::weak(format!("   {}", g.name)));
            sys.push(Line::from(spans));
        }
        f.render_widget(Paragraph::new(sys).block(kit::panel("System")), system);

        running_table(f, models, &running, None, "Running models");
    }
}

/// Loaded models with memory placement and time to unload.
pub(super) fn running_table(
    f: &mut Frame,
    area: Rect,
    running: &[RunningModel],
    selected: Option<usize>,
    title: &str,
) {
    if running.is_empty() {
        f.render_widget(
            Paragraph::new(kit::weak("No model is currently loaded in memory."))
                .block(kit::panel(title)),
            area,
        );
        return;
    }
    let now = chrono::Utc::now();
    let rows = running.iter().enumerate().map(|(i, m)| {
        let expires = chrono::DateTime::parse_from_rfc3339(&m.expires_at)
            .map(|t| format_duration_secs((t.with_timezone(&chrono::Utc) - now).num_seconds()))
            .unwrap_or_else(|_| "-".into());
        let processor = m.processor();
        let color = if processor.contains("CPU") {
            AMBER
        } else {
            GREEN
        };
        let row = Row::new(vec![
            Cell::from(m.name.clone().bold()),
            Cell::from(format_bytes(m.size)),
            Cell::from(format_bytes(m.size_vram)),
            Cell::from(Span::styled(processor, Style::new().fg(color))),
            Cell::from(
                m.context_length
                    .map(ctx_short)
                    .unwrap_or_else(|| "-".into()),
            ),
            Cell::from(expires),
        ]);
        if selected == Some(i) {
            row.style(kit::selected())
        } else {
            row
        }
    });
    let header = Row::new([
        "Model",
        "Size",
        "VRAM",
        "Processor",
        "Context",
        "Unloads in",
    ])
    .style(Style::new().dark_gray());
    let widths = [
        Constraint::Fill(3),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(16),
        Constraint::Length(8),
        Constraint::Length(10),
    ];
    f.render_widget(
        Table::new(rows, widths)
            .header(header)
            .block(kit::panel(title)),
        area,
    );
}
