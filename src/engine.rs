use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use vt100::Parser;

use crate::adapter::{self, ComposerView};
use crate::ghost::{self, Overlay, SavedCell};
use crate::keys::{KeyKind, KeyParser, Piece};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserAction {
    Forward(Vec<u8>),
    /// Bytes for the child. The original key is not included.
    Insert(Vec<u8>),
    Suspend,
}

pub struct Engine {
    parser: Parser,
    keys: KeyParser,
    view: ComposerView,
    overlay: Option<Overlay>,
    hint_suffix: Option<String>,
    hint_fp: u64,
    sync_depth: u32,
    sync_tail: Vec<u8>,
    append_space: bool,
}

impl Engine {
    pub fn new(rows: u16, cols: u16, append_space: bool) -> Self {
        let parser = Parser::new(rows.max(1), cols.max(1), 0);
        let view = adapter::inspect(parser.screen());
        Self {
            parser,
            keys: KeyParser::default(),
            view,
            overlay: None,
            hint_suffix: None,
            hint_fp: 0,
            sync_depth: 0,
            sync_tail: Vec::new(),
            append_space,
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows.max(1), cols.max(1));
        self.view = adapter::inspect(self.parser.screen());
        self.overlay = None;
    }

    pub fn view(&self) -> &ComposerView {
        &self.view
    }

    pub fn hint_suffix(&self) -> Option<&str> {
        self.hint_suffix.as_deref()
    }

    pub fn sync_idle(&self) -> bool {
        self.sync_depth == 0
    }

    pub fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.view.buffer.hash(&mut hasher);
        self.view.cursor.hash(&mut hasher);
        self.view.at_end.hash(&mut hasher);
        self.view.native_menu.hash(&mut hasher);
        self.view.dialog.hash(&mut hasher);
        self.view.cursor_cell.hash(&mut hasher);
        hasher.finish()
    }

    pub fn set_append_space(&mut self, append_space: bool) {
        self.append_space = append_space;
    }

    pub fn cached_suffix(&self) -> Option<&str> {
        if self.hint_suffix.is_some() && self.hint_fp == self.fingerprint() {
            self.hint_suffix.as_deref()
        } else {
            None
        }
    }

    /// Child bytes are returned unchanged. A restore, if any, is a prefix the
    /// real terminal needs before those bytes.
    pub fn on_child(&mut self, bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let restore = self.take_restore();
        self.note_sync(bytes);
        self.parser.process(bytes);
        self.view = adapter::inspect(self.parser.screen());
        if self.hint_fp != self.fingerprint() {
            self.hint_suffix = None;
        }
        (restore, bytes.to_vec())
    }

    pub fn on_user(&mut self, bytes: &[u8]) -> Vec<UserAction> {
        let mut actions = Vec::new();
        for piece in self.keys.push(bytes) {
            actions.push(self.decide(piece));
        }
        actions
    }

    pub fn flush_escape(&mut self) -> Option<UserAction> {
        self.keys.flush_escape().map(|piece| self.decide(piece))
    }

    pub fn remember_hint(&mut self, suffix: &str) {
        self.hint_suffix = Some(suffix.to_string());
        self.hint_fp = self.fingerprint();
    }

    pub fn clear_hint(&mut self) {
        self.hint_suffix = None;
        self.hint_fp = 0;
    }

    /// Draw a ghost only on the real terminal. Returns no child bytes.
    pub fn overlay_bytes(&mut self, suffix: &str) -> Option<Vec<u8>> {
        let suffix = ghost::fit_suffix(suffix, self.view.cursor_cell?.1, self.view.cols);
        if suffix.is_empty() {
            return None;
        }
        let (row, col) = self.view.cursor_cell?;
        let mut saved = Vec::new();
        let mut col_cursor = col;
        for ch in suffix.chars() {
            if col_cursor >= self.view.cols {
                break;
            }
            saved.push(
                adapter::cell_at(self.parser.screen(), row, col_cursor).unwrap_or(SavedCell {
                    row,
                    col: col_cursor,
                    text: " ".to_string(),
                    inverse: false,
                    dim: false,
                }),
            );
            col_cursor += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
        }
        if saved.is_empty() {
            return None;
        }
        let overlay = Overlay {
            saved,
            suffix,
            cursor: self.view.term_cursor,
            reveal_cursor: !self.parser.screen().hide_cursor(),
        };
        let bytes = ghost::draw(&overlay);
        self.overlay = Some(overlay);
        Some(bytes)
    }

    fn decide(&mut self, piece: Piece) -> UserAction {
        let accept = self.hint_suffix.is_some()
            && self.view.confident
            && self.view.at_end
            && !self.view.native_menu
            && !self.view.dialog
            && !self.keys.in_paste();
        match piece.kind {
            KeyKind::Right if accept => self.insert(true),
            KeyKind::Tab if accept => self.insert(false),
            KeyKind::CtrlZ if !self.keys.in_paste() => UserAction::Suspend,
            _ => {
                self.hint_suffix = None;
                UserAction::Forward(piece.raw)
            }
        }
    }

    fn insert(&mut self, space: bool) -> UserAction {
        let mut bytes = self.hint_suffix.take().unwrap_or_default().into_bytes();
        if space && self.append_space {
            bytes.push(b' ');
        }
        self.hint_fp = 0;
        UserAction::Insert(bytes)
    }

    fn take_restore(&mut self) -> Vec<u8> {
        self.overlay
            .take()
            .map(|overlay| ghost::restore(&overlay))
            .unwrap_or_default()
    }

    fn note_sync(&mut self, bytes: &[u8]) {
        self.sync_tail.extend_from_slice(bytes);
        let begin = b"\x1b[?2026h";
        let end = b"\x1b[?2026l";
        let mut i = 0;
        while i < self.sync_tail.len() {
            if self.sync_tail[i..].starts_with(begin) {
                self.sync_depth = self.sync_depth.saturating_add(1);
                i += begin.len();
                continue;
            }
            if self.sync_tail[i..].starts_with(end) {
                self.sync_depth = self.sync_depth.saturating_sub(1);
                i += end.len();
                continue;
            }
            i += 1;
        }
        let keep = begin.len() - 1;
        if self.sync_tail.len() > keep {
            self.sync_tail.drain(..self.sync_tail.len() - keep);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        Engine::new(24, 80, true)
    }

    fn with_prompt(text: &str) -> Engine {
        let mut engine = engine();
        let ansi = format!(
            "\x1b[10;1H{rule}\x1b[11;1H❯ {text}\x1b[11;{}H\x1b[7m \x1b[27m\x1b[12;1H{rule}",
            3 + text.chars().count() + 1,
            rule = "─".repeat(80),
        );
        let _ = engine.on_child(ansi.as_bytes());
        engine
    }

    #[test]
    fn child_bytes_pass_through_unchanged() {
        let mut engine = engine();
        let (restore, child) = engine.on_child(b"\x1b[31mhello\x1b[0m");
        assert!(restore.is_empty());
        assert_eq!(child, b"\x1b[31mhello\x1b[0m");
    }

    #[test]
    fn right_and_tab_forward_when_no_hint_is_visible() {
        let mut engine = with_prompt("he");
        let actions = engine.on_user(b"\x1b[C\t");
        assert_eq!(actions[0], UserAction::Forward(b"\x1b[C".to_vec()));
        assert_eq!(actions[1], UserAction::Forward(b"\t".to_vec()));
    }

    #[test]
    fn accept_inserts_only_the_suffix_and_optional_space() {
        let mut engine = with_prompt("he");
        engine.remember_hint("lp");
        let right = engine.on_user(b"\x1b[C");
        assert_eq!(right, vec![UserAction::Insert(b"lp ".to_vec())]);
        engine.remember_hint("lp");
        let tab = engine.on_user(b"\t");
        assert_eq!(tab, vec![UserAction::Insert(b"lp".to_vec())]);
    }

    #[test]
    fn enter_forwards_the_key_and_drops_the_hint() {
        let mut engine = with_prompt("he");
        engine.remember_hint("lp");
        let actions = engine.on_user(b"\r");
        assert_eq!(actions, vec![UserAction::Forward(b"\r".to_vec())]);
        assert!(engine.hint_suffix().is_none());
    }

    #[test]
    fn native_menu_does_not_steal_tab() {
        let mut engine = engine();
        let ansi = format!(
            "\x1b[8;1H/add-dir Add\n/agents Manage\n\x1b[10;1H{rule}\x1b[11;1H❯ /\x1b[11;4H\x1b[7m \x1b[27m\x1b[12;1H{rule}",
            rule = "─".repeat(80),
        );
        let _ = engine.on_child(ansi.as_bytes());
        engine.remember_hint("lp");
        let actions = engine.on_user(b"\t");
        assert_eq!(actions, vec![UserAction::Forward(b"\t".to_vec())]);
    }

    #[test]
    fn overlay_bytes_are_not_part_of_child_output() {
        let mut engine = with_prompt("he");
        let drawn = engine.overlay_bytes("lp").unwrap();
        assert!(drawn.windows(2).any(|w| w == b"lp"));
        let (restore, child) = engine.on_child(b"redraw");
        assert!(!restore.is_empty());
        assert_eq!(child, b"redraw");
    }

    #[test]
    fn cached_suffix_survives_a_redraw_of_the_same_word() {
        let mut engine = with_prompt("product");
        engine.remember_hint("ion");
        let same = format!(
            "\x1b[10;1H{rule}\x1b[11;1H❯ product\x1b[11;11H\x1b[7m \x1b[27m\x1b[12;1H{rule}",
            rule = "─".repeat(80),
        );
        let _ = engine.on_child(same.as_bytes());
        assert_eq!(engine.cached_suffix(), Some("ion"));
        let suffix = engine.cached_suffix().unwrap().to_string();
        assert!(engine.overlay_bytes(&suffix).is_some());
    }




}
