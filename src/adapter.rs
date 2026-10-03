use vt100::Screen;

use crate::eligibility::{self, Gates, PartialWord};

pub const ADAPTER_ID: &str = "claude-code-v2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerView {
    pub adapter: &'static str,
    pub confident: bool,
    pub buffer: String,
    pub cursor: usize,
    pub at_end: bool,
    pub native_menu: bool,
    pub dialog: bool,
    pub selection: bool,
    pub ime: bool,
    /// 0-based cell where the next typed character will land.
    pub cursor_cell: Option<(u16, u16)>,
    pub term_cursor: (u16, u16),
    pub cols: u16,
    pub observed_version: Option<String>,
}

impl ComposerView {
    pub fn unsupported(screen: &Screen) -> Self {
        Self {
            adapter: "unsupported",
            confident: false,
            buffer: String::new(),
            cursor: 0,
            at_end: false,
            native_menu: false,
            dialog: false,
            selection: false,
            ime: false,
            cursor_cell: None,
            term_cursor: screen.cursor_position(),
            cols: screen.size().1,
            observed_version: version_in(screen),
        }
    }

    pub fn gates(&self) -> Gates {
        Gates {
            confident: self.confident,
            at_end: self.at_end,
            native_menu: self.native_menu,
            dialog: self.dialog,
            selection: self.selection,
            ime: self.ime,
        }
    }

    pub fn partial(&self) -> Option<PartialWord> {
        eligibility::partial_word(&self.buffer, self.gates())
    }
}

pub fn inspect(screen: &Screen) -> ComposerView {
    let (rows, cols) = screen.size();
    let version = version_in(screen);
    let Some(marker) = find_marker(screen) else {
        return ComposerView::unsupported(screen);
    };
    let band = composer_band(screen, marker);
    let dialog = permission_dialog(screen);
    let native_menu = menu_above(screen, band.top) || menu_above(screen, marker.0);
    let extracted = extract_buffer(screen, &band, marker);
    let selection = extracted.inverse_count > 1;
    let term_cursor = screen.cursor_position();
    let cursor_cell = extracted.cursor_cell.or_else(|| {
        let (row, col) = term_cursor;
        let content_col = marker.1 + 2;
        if (band.top..=band.bottom).contains(&row) && col >= content_col {
            Some((row, col))
        } else {
            None
        }
    });
    let on_last_line = cursor_cell.is_some_and(|(row, _)| {
        (row + 1..=band.bottom).all(|later| row_is_blank(screen, later, marker.1 + 2))
    });
    let at_end = cursor_cell.is_some()
        && on_last_line
        && extracted.cursor == extracted.buffer.chars().count();
    let confident = !dialog && cursor_cell.is_some() && !selection;
    ComposerView {
        adapter: ADAPTER_ID,
        confident,
        buffer: extracted.buffer,
        cursor: extracted.cursor,
        at_end,
        native_menu,
        dialog,
        selection,
        ime: extracted.ime,
        cursor_cell,
        term_cursor: screen.cursor_position(),
        cols,
        observed_version: version.or_else(|| {
            let _ = rows;
            None
        }),
    }
}

#[derive(Clone, Copy)]
struct Band {
    top: u16,
    bottom: u16,
}

struct Extracted {
    buffer: String,
    cursor: usize,
    cursor_cell: Option<(u16, u16)>,
    inverse_count: usize,
    ime: bool,
}

fn find_marker(screen: &Screen) -> Option<(u16, u16)> {
    let (rows, cols) = screen.size();
    let start = 0;
    for row in (start..rows).rev() {
        let limit = cols.min(4);
        for col in 0..limit {
            if cell_text(screen, row, col) == "❯" {
                return Some((row, col));
            }
        }
    }
    None
}

fn composer_band(screen: &Screen, marker: (u16, u16)) -> Band {
    let (rows, _) = screen.size();
    let top_rule = (0..marker.0)
        .rev()
        .take(4)
        .find(|row| is_rule(screen, *row));
    let bottom_rule = (marker.0 + 1..rows)
        .take(6)
        .find(|row| is_rule(screen, *row));
    let top = top_rule.map(|row| row + 1).unwrap_or(marker.0);
    let bottom = bottom_rule
        .map(|row| row.saturating_sub(1))
        .unwrap_or(marker.0);
    Band {
        top: top.min(marker.0),
        bottom: bottom.max(marker.0),
    }
}

fn is_rule(screen: &Screen, row: u16) -> bool {
    let cols = screen.size().1;
    if cols == 0 {
        return false;
    }
    let mut rules = 0;
    for col in 0..cols {
        let text = cell_text(screen, row, col);
        if text == "─" || text == "-" {
            rules += 1;
        }
    }
    rules * 10 >= cols as usize * 8
}

fn extract_buffer(screen: &Screen, band: &Band, marker: (u16, u16)) -> Extracted {
    let cols = screen.size().1;
    let mut buffer = String::new();
    let mut cursor = 0;
    let mut cursor_cell = None;
    let mut inverse_count = 0;
    let mut ime = false;
    let mut seen_cursor = false;
    let content_col = marker.1 + 2;
    for row in band.top..=band.bottom {
        if row > band.top && !screen.row_wrapped(row.saturating_sub(1)) && !buffer.is_empty() {
            buffer.push('\n');
            if !seen_cursor {
                cursor += 1;
            }
        }
        let mut skipped_gap = false;
        for col in content_col..cols {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let text = cell.contents();
            let blank = text.is_empty() || text == " " || text == "\u{00a0}";
            if cell.inverse() {
                inverse_count += 1;
                if cursor_cell.is_none() {
                    cursor_cell = Some((row, col));
                    seen_cursor = true;
                }
                if blank {
                    skipped_gap = !buffer.is_empty();
                }
                continue;
            }
            if blank {
                if !seen_cursor && !buffer.is_empty() {
                    skipped_gap = true;
                }
                continue;
            }
            if cell.dim() && !seen_cursor && buffer.trim().is_empty() {
                continue;
            }
            if cell.underline() {
                ime = true;
            }
            if skipped_gap && !buffer.ends_with(' ') {
                buffer.push(' ');
                cursor += 1;
            }
            skipped_gap = false;
            if !seen_cursor {
                buffer.push_str(text);
                cursor += text.chars().count();
            }
        }
    }
    Extracted {
        buffer: buffer.trim_end().to_string(),
        cursor: cursor.min(buffer.trim_end().chars().count()),
        cursor_cell,
        inverse_count,
        ime,
    }
}

fn row_is_blank(screen: &Screen, row: u16, start: u16) -> bool {
    let cols = screen.size().1;
    (start..cols).all(|col| {
        let text = cell_text(screen, row, col);
        text.is_empty() || text == " " || text == "\u{00a0}"
    })
}

fn menu_above(screen: &Screen, top: u16) -> bool {
    let start = top.saturating_sub(20);
    let mut commands = 0;
    for row in start..top {
        let text = row_string(screen, row);
        let trimmed = text.trim();
        if trimmed.starts_with('/')
            && trimmed
                .chars()
                .nth(1)
                .is_some_and(|ch| ch.is_ascii_alphanumeric())
        {
            commands += 1;
        }
    }
    commands >= 2
}

fn permission_dialog(screen: &Screen) -> bool {
    let text = screen.contents();
    let lower = text.to_ascii_lowercase();
    let asks = lower.contains("do you want")
        || lower.contains("allow") && (lower.contains("deny") || lower.contains("don't ask"));
    asks && (text.contains('╭') || text.contains('┌') || text.contains('│'))
}

fn version_in(screen: &Screen) -> Option<String> {
    let text = screen.contents();
    let marker = "Claude Code v";
    let start = text.find(marker)?;
    let rest = &text[start + marker.len()..];
    let version: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

fn cell_text(screen: &Screen, row: u16, col: u16) -> String {
    screen
        .cell(row, col)
        .map(|cell| cell.contents().to_string())
        .unwrap_or_default()
}

fn row_string(screen: &Screen, row: u16) -> String {
    let cols = screen.size().1;
    let mut out = String::new();
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            continue;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        out.push_str(cell.contents());
    }
    out
}

pub fn cell_at(screen: &Screen, row: u16, col: u16) -> Option<crate::ghost::SavedCell> {
    let cell = screen.cell(row, col)?;
    Some(crate::ghost::SavedCell {
        row,
        col,
        text: if cell.contents().is_empty() {
            " ".to_string()
        } else {
            cell.contents().to_string()
        },
        inverse: cell.inverse(),
        dim: cell.dim(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vt100::Parser;

    fn parse(ansi: &str) -> Screen {
        let mut parser = Parser::new(24, 80, 0);
        parser.process(ansi.as_bytes());
        parser.screen().clone()
    }

    fn prompt(text: &str, cursor_col: u16) -> String {
        format!(
            "\x1b[10;1H{rule}\x1b[11;1H❯ {text}\x1b[11;{cursor_col}H\x1b[7m \x1b[27m\x1b[12;1H{rule}",
            rule = "─".repeat(80),
            cursor_col = cursor_col + 1,
        )
    }

    #[test]
    fn reads_the_composer_and_ignores_the_cursor_block() {
        let screen = parse(&prompt("he", 4));
        let view = inspect(&screen);
        assert_eq!(view.adapter, ADAPTER_ID);
        assert!(view.confident);
        assert_eq!(view.buffer, "he");
        assert!(view.at_end);
        assert_eq!(view.partial().unwrap().prefix, "he");
        assert_eq!(view.cursor_cell, Some((10, 4)));
    }

    #[test]
    fn slash_menu_suppresses_the_hint() {
        let mut ansi = String::new();
        ansi.push_str("\x1b[8;1H/add-dir          Add a directory\n");
        ansi.push_str("/agents           Manage agents\n");
        ansi.push_str(&prompt("/", 3));
        let view = inspect(&parse(&ansi));
        assert!(view.native_menu);
        assert!(view.partial().is_none());
    }

    #[test]
    fn permission_dialog_is_not_the_composer() {
        let ansi = format!(
            "\x1b[4;10H╭──────────────╮\x1b[5;10H│ Do you want  │\x1b[6;10H│ Allow  Deny  │\x1b[7;10H╰──────────────╯{}",
            prompt("he", 4)
        );
        let view = inspect(&parse(&ansi));
        assert!(view.dialog);
        assert!(view.partial().is_none());
    }

    #[test]
    fn hardware_cursor_after_he_is_a_confident_composer() {
        let ansi = format!(
            "\x1b[10;1H{rule}\x1b[11;1H❯ he\x1b[12;1H{rule}\x1b[11;5H\x1b[?25h",
            rule = "─".repeat(80),
        );
        let view = inspect(&parse(&ansi));
        assert!(view.confident);
        assert_eq!(view.buffer, "he");
        assert!(view.at_end);
        assert_eq!(view.cursor_cell, Some((10, 4)));
        assert_eq!(view.partial().unwrap().prefix, "he");
    }

    #[test]
    fn second_word_is_eligible_when_the_cursor_is_after_it() {
        let ansi = format!(
            "\x1b[10;1H{rule}\x1b[11;1H❯ product re\x1b[12;1H{rule}\x1b[11;14H\x1b[?25h",
            rule = "─".repeat(80),
        );
        let view = inspect(&parse(&ansi));
        assert_eq!(view.buffer, "product re");
        assert_eq!(view.partial().unwrap().prefix, "re");
    }

    #[test]
    fn a_blank_cell_between_words_stays_a_space() {
        let ansi = format!(
            "\x1b[10;1H{rule}\x1b[11;1H❯ product\x1b[11;12Hreq\x1b[12;1H{rule}\x1b[11;15H\x1b[?25h",
            rule = "─".repeat(80),
        );
        let view = inspect(&parse(&ansi));
        assert_eq!(view.buffer, "product req");
        assert_eq!(view.partial().unwrap().prefix, "req");
    }

    #[test]
    fn unknown_layout_fails_open() {
        let view = inspect(&parse("\x1b[1;1Hjust a transcript line\n"));
        assert!(!view.confident);
        assert_eq!(view.adapter, "unsupported");
        assert!(view.partial().is_none());
    }

    #[test]
    fn version_is_observed_without_enabling_an_unknown_layout() {
        let view = inspect(&parse("\x1b[1;1HClaude Code v9.0.1\nno prompt here\n"));
        assert_eq!(view.observed_version.as_deref(), Some("9.0.1"));
        assert!(!view.confident);
    }
}
