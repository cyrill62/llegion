//! GPU tab: detected devices and which of them Ollama may use.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use super::Tui;
use super::kit::{self, AMBER, GREEN, RED};
use crate::config::GpuMode;
use crate::gpu::GpuKind;
use crate::i18n::tr;
use crate::util::format_bytes;

impl Tui {
    pub(super) fn gpu_key(&mut self, k: KeyEvent) -> bool {
        let gpus = self.ctl.backend.shared.lock().unwrap().gpus.clone();
        let g = &mut self.draft.gpu;
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => kit::step(&mut self.gpu_sel, -1, gpus.len()),
            KeyCode::Down | KeyCode::Char('j') => kit::step(&mut self.gpu_sel, 1, gpus.len()),
            KeyCode::Char('m') => {
                g.mode = kit::cycle(
                    &[GpuMode::Auto, GpuMode::Custom, GpuMode::CpuOnly],
                    &g.mode,
                    false,
                )
            }
            KeyCode::Char('i') => g.disable_igpu = !g.disable_igpu,
            KeyCode::Char('v') => g.enable_vulkan = !g.enable_vulkan,
            KeyCode::Char('d') => self.ctl.backend.redetect_gpus(),
            KeyCode::Char(' ') if g.mode == GpuMode::Custom => {
                if let Some(gpu) = gpus.get(self.gpu_sel) {
                    let key = gpu.key();
                    if g.disabled_devices.contains(&key) {
                        g.disabled_devices.retain(|k| k != &key);
                    } else {
                        g.disabled_devices.push(key);
                    }
                }
            }
            _ => return self.save_key(k),
        }
        true
    }

    pub(super) fn gpu_hints(&self) -> Vec<(&'static str, &'static str)> {
        let mut hints = vec![("m", tr("mode"))];
        if self.draft.gpu.mode == GpuMode::Custom {
            hints.push(("Space", tr("use device")));
        }
        hints.extend([
            ("i", tr("integrated GPUs")),
            ("v", tr("Vulkan")),
            ("d", tr("re-detect")),
        ]);
        hints.extend(self.save_hints());
        hints
    }

    pub(super) fn draw_gpu(&mut self, f: &mut Frame, area: Rect) {
        let (gpus, detected) = {
            let s = self.ctl.backend.shared.lock().unwrap();
            (s.gpus.clone(), s.gpus_detected)
        };
        kit::step(&mut self.gpu_sel, 0, gpus.len());
        let [list_area, config_area, env_area] = Layout::vertical([
            Constraint::Length(gpus.len().max(1) as u16 + 3),
            Constraint::Length(5),
            Constraint::Min(3),
        ])
        .areas(area);

        if !detected || gpus.is_empty() {
            let text = if detected {
                tr("No GPU detected: Ollama will run on the CPU.")
            } else {
                tr("Detecting GPUs…")
            };
            f.render_widget(
                Paragraph::new(kit::weak(text)).block(kit::panel(tr("Devices"))),
                list_area,
            );
        } else {
            let rows = gpus.iter().enumerate().map(|(i, g)| {
                let enabled = self.draft.is_gpu_enabled(g);
                let use_cell = if enabled {
                    Span::styled(tr("yes"), Style::new().fg(GREEN))
                } else {
                    Span::styled(tr("no"), Style::new().fg(RED))
                };
                let vram = match g.vram_used {
                    Some(used) => {
                        format!("{} / {}", format_bytes(used), format_bytes(g.vram_total))
                    }
                    None => format_bytes(g.vram_total),
                };
                Row::new(vec![
                    Cell::from(format!("#{i}")),
                    Cell::from(use_cell),
                    Cell::from(g.vendor.label()),
                    Cell::from(g.name.clone().bold()),
                    Cell::from(g.kind.label()),
                    Cell::from(vram),
                    Cell::from(g.utilization.map_or("-".into(), |u| format!("{u:.0}%"))),
                    Cell::from(g.temperature.map_or("-".into(), |t| format!("{t:.0}°C"))),
                    Cell::from(g.driver.clone().unwrap_or_else(|| "-".into())),
                ])
            });
            let header = Row::new([
                "",
                tr("Use"),
                tr("Vendor"),
                tr("Name"),
                tr("Kind"),
                "VRAM",
                tr("Util"),
                tr("Temp"),
                tr("Driver"),
            ])
            .style(Style::new().dark_gray());
            let widths = [
                Constraint::Length(3),
                Constraint::Length(4),
                Constraint::Length(8),
                Constraint::Fill(2),
                Constraint::Length(10),
                Constraint::Length(20),
                Constraint::Length(5),
                Constraint::Length(6),
                Constraint::Fill(1),
            ];
            let mut state = TableState::default().with_selected(Some(self.gpu_sel));
            f.render_stateful_widget(
                Table::new(rows, widths)
                    .header(header)
                    .row_highlight_style(kit::selected())
                    .block(kit::panel(tr("Devices"))),
                list_area,
                &mut state,
            );
            self.hit_table(list_area, &state, gpus.len());
        }

        let g = &self.draft.gpu;
        let on_off = |b: bool| if b { tr("on") } else { tr("off") };
        let mode = match g.mode {
            GpuMode::Auto => tr("Auto"),
            GpuMode::Custom => tr("Custom (Space toggles the selected device)"),
            GpuMode::CpuOnly => tr("CPU only"),
        };
        let config = vec![
            kit::kv(tr("Mode"), Span::raw(mode)),
            kit::kv(
                tr("Disable iGPUs"),
                Span::raw(format!(
                    "{}  {}",
                    on_off(g.disable_igpu),
                    tr("(integrated GPUs share system RAM and are often slower than the CPU)")
                )),
            ),
            kit::kv(
                "Vulkan",
                Span::raw(format!(
                    "{}  {}",
                    on_off(g.enable_vulkan),
                    tr("(OLLAMA_VULKAN=1, for Intel and some AMD)")
                )),
            ),
        ];
        f.render_widget(
            Paragraph::new(config).block(kit::panel(tr(
                "Configuration · applies when the server restarts",
            ))),
            config_area,
        );

        let env = self.draft.gpu_env(&gpus);
        let mut lines: Vec<Line> = env
            .iter()
            .map(|(k, v)| Line::raw(format!("{k}={v}")))
            .collect();
        if lines.is_empty() {
            lines.push(Line::from(kit::weak(tr("None — Ollama defaults."))));
        }
        if g.mode == GpuMode::CpuOnly {
            lines.push(Line::from(Span::styled(
                tr("All GPUs are hidden from Ollama."),
                Style::new().fg(AMBER),
            )));
        }
        if gpus.iter().any(|g| g.kind == GpuKind::Integrated) && self.draft.gpu.disable_igpu {
            lines.push(Line::from(kit::weak(tr("Integrated GPUs are disabled."))));
        }
        f.render_widget(
            Paragraph::new(lines).block(kit::panel(tr("Resulting environment"))),
            env_area,
        );
    }
}
