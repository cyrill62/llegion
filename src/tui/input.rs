//! Single-line text editor used for prompts, filters and settings values.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Span;

#[derive(Debug, Default, Clone)]
pub struct LineEdit {
    text: String,
    /// Cursor position in chars.
    cursor: usize,
}

impl LineEdit {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(i, _)| i)
    }

    pub fn insert_str(&mut self, s: &str) {
        // Single line: pasted newlines become spaces.
        let s: String = s
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .collect();
        let at = self.byte_at(self.cursor);
        self.text.insert_str(at, &s);
        self.cursor += s.chars().count();
    }

    /// Applies an editing key; returns false for keys it does not handle.
    pub fn handle(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let len = self.text.chars().count();
        match k.code {
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = len,
            KeyCode::Char('u') if ctrl => {
                let at = self.byte_at(self.cursor);
                self.text.drain(..at);
                self.cursor = 0;
            }
            KeyCode::Char('k') if ctrl => {
                let at = self.byte_at(self.cursor);
                self.text.truncate(at);
            }
            KeyCode::Char('w') if ctrl => {
                let chars: Vec<char> = self.text.chars().collect();
                let mut start = self.cursor;
                while start > 0 && chars[start - 1] == ' ' {
                    start -= 1;
                }
                while start > 0 && chars[start - 1] != ' ' {
                    start -= 1;
                }
                let (a, b) = (self.byte_at(start), self.byte_at(self.cursor));
                self.text.drain(a..b);
                self.cursor = start;
            }
            KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                let at = self.byte_at(self.cursor);
                self.text.insert(at, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte_at(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Delete if self.cursor < len => {
                let at = self.byte_at(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            KeyCode::Backspace | KeyCode::Delete => {}
            _ => return false,
        }
        true
    }

    /// The part of the text that fits in `width` columns, and the cursor column within it.
    pub fn view(&self, width: u16, masked: bool) -> (String, u16) {
        let shown: String = if masked {
            "•".repeat(self.text.chars().count())
        } else {
            self.text.clone()
        };
        let chars: Vec<char> = shown.chars().collect();
        let width = width.max(1) as usize;
        let mut start = 0;
        // Scroll right until the cursor fits, keeping one column for it at the end.
        while Span::raw(chars[start..self.cursor].iter().collect::<String>()).width() >= width {
            start += 1;
        }
        let visible: String = chars[start..].iter().collect();
        let col = Span::raw(chars[start..self.cursor].iter().collect::<String>()).width();
        (visible, col as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyCode;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn edits_at_cursor() {
        let mut e = LineEdit::new("héllo");
        e.handle(key(KeyCode::Left));
        e.handle(key(KeyCode::Backspace));
        e.handle(key(KeyCode::Char('L')));
        assert_eq!(e.text(), "hélLo");
        e.handle(key(KeyCode::Home));
        e.handle(key(KeyCode::Delete));
        assert_eq!(e.text(), "élLo");
        e.handle(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        e.insert_str(" a\nb");
        assert_eq!(e.text(), "élLo a b");
        e.handle(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(e.text(), "élLo a ");
    }

    #[test]
    fn view_scrolls_to_cursor() {
        let e = LineEdit::new("abcdefghij");
        let (text, col) = e.view(4, false);
        assert_eq!(text, "hij");
        assert_eq!(col, 3);
    }
}
