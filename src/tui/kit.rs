//! Shared terminal widgets: colors, panels, text bars and popups.

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear};

pub const GREEN: Color = Color::Green;
pub const RED: Color = Color::Red;
pub const AMBER: Color = Color::Yellow;
pub const ACCENT: Color = Color::Cyan;

pub fn panel(title: &str) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dark_gray())
        .title(Span::styled(format!(" {title} "), Style::new().bold()))
}

pub fn selected() -> Style {
    Style::new()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}

pub fn weak(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::new().dark_gray())
}

pub fn label(text: &str) -> Span<'static> {
    Span::styled(format!("{text:<16}"), Style::new().dark_gray())
}

/// `label  value` line.
pub fn kv(name: &str, value: impl Into<Span<'static>>) -> Line<'static> {
    Line::from(vec![label(name), value.into()])
}

/// Green below 70 %, amber below 90 %, red above.
pub fn load_color(frac: f64) -> Color {
    if frac < 0.7 {
        GREEN
    } else if frac < 0.9 {
        AMBER
    } else {
        RED
    }
}

/// Text progress bar such as `██████░░░░`, `width` cells wide.
pub fn bar(frac: f64, width: usize) -> Span<'static> {
    let frac = frac.clamp(0.0, 1.0);
    let full = (frac * width as f64).round() as usize;
    Span::styled(
        format!("{}{}", "█".repeat(full), "░".repeat(width - full)),
        Style::new().fg(load_color(frac)),
    )
}

/// Centered rectangle of at most `w` x `h` cells, cleared for a popup.
pub fn popup(f: &mut Frame, w: u16, h: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(w)])
        .flex(Flex::Center)
        .areas(f.area());
    let [area] = Layout::vertical([Constraint::Length(h)])
        .flex(Flex::Center)
        .areas(area);
    f.render_widget(Clear, area);
    area
}

/// Key hint for the footer: `key` highlighted, then its action.
pub fn hint(key: &str, action: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!(" {key} "), Style::new().black().on_dark_gray()),
        Span::raw(format!(" {action}  ")),
    ]
}

/// Moves a list selection by `delta`, clamped to `len`.
pub fn step(sel: &mut usize, delta: isize, len: usize) {
    *sel = sel.saturating_add_signed(delta).min(len.saturating_sub(1));
}

/// Next item after `current` in `items`, wrapping (first item when not found).
pub fn cycle<T: PartialEq + Clone>(items: &[T], current: &T, back: bool) -> T {
    let n = items.len();
    let i = items.iter().position(|x| x == current);
    let next = match (i, back) {
        (None, _) => 0,
        (Some(i), false) => (i + 1) % n,
        (Some(i), true) => (i + n - 1) % n,
    };
    items[next].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_and_steps() {
        assert_eq!(cycle(&[1, 2, 3], &3, false), 1);
        assert_eq!(cycle(&[1, 2, 3], &1, true), 3);
        assert_eq!(cycle(&[1, 2, 3], &9, false), 1);
        let mut sel = 1;
        step(&mut sel, -5, 4);
        assert_eq!(sel, 0);
        step(&mut sel, 10, 4);
        assert_eq!(sel, 3);
        step(&mut sel, 1, 0);
        assert_eq!(sel, 0);
    }
}
