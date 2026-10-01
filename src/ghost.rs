use unicode_width::UnicodeWidthStr;

/// Cells overwritten by a ghost suffix, so a later child redraw can be preceded
/// by an exact restore. The companion never sends these bytes to Claude.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCell {
    pub row: u16,
    pub col: u16,
    pub text: String,
    pub inverse: bool,
    pub dim: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    pub saved: Vec<SavedCell>,
    pub suffix: String,
    pub cursor: (u16, u16),
    pub reveal_cursor: bool,
}

/// Truncate `suffix` so it fits on the cursor row.
pub fn fit_suffix(suffix: &str, col: u16, cols: u16) -> String {
    if cols == 0 || col >= cols {
        return String::new();
    }
    let budget = (cols - col) as usize;
    let mut out = String::new();
    let mut used = 0;
    for ch in suffix.chars() {
        let width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4])).max(1);
        if used + width > budget {
            break;
        }
        out.push(ch);
        used += width;
    }
    out
}

pub fn draw(overlay: &Overlay) -> Vec<u8> {
    let Some(first) = overlay.saved.first() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    cup(&mut out, first.row, first.col);
    out.extend_from_slice(b"\x1b[?25l\x1b[0;2;38;5;245m");
    out.extend_from_slice(overlay.suffix.as_bytes());
    out.extend_from_slice(b"\x1b[0m");
    cup(&mut out, overlay.cursor.0, overlay.cursor.1);
    out
}

pub fn restore(overlay: &Overlay) -> Vec<u8> {
    let mut out = Vec::new();
    for cell in &overlay.saved {
        cup(&mut out, cell.row, cell.col);
        out.extend_from_slice(b"\x1b[0m");
        if cell.inverse {
            out.extend_from_slice(b"\x1b[7m");
        }
        if cell.dim {
            out.extend_from_slice(b"\x1b[2m");
        }
        if cell.text.is_empty() {
            out.push(b' ');
        } else {
            out.extend_from_slice(cell.text.as_bytes());
        }
        out.extend_from_slice(b"\x1b[0m");
    }
    if overlay.reveal_cursor {
        out.extend_from_slice(b"\x1b[?25h");
    }
    cup(&mut out, overlay.cursor.0, overlay.cursor.1);
    out
}

fn cup(out: &mut Vec<u8>, row: u16, col: u16) {
    let _ = std::io::Write::write_fmt(out, format_args!("\x1b[{};{}H", row + 1, col + 1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_is_dim_and_returns_the_cursor() {
        let overlay = Overlay {
            saved: vec![SavedCell {
                row: 10,
                col: 4,
                text: " ".to_string(),
                inverse: true,
                dim: false,
            }],
            suffix: "lp".to_string(),
            cursor: (36, 2),
            reveal_cursor: true,
        };
        let bytes = draw(&overlay);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\x1b[0;2;38;5;245m"));
        assert!(text.contains("lp"));
        assert!(text.contains("\x1b[37;3H"));
        assert!(!text.contains("\x1b[4m"));
    }

    #[test]
    fn restore_rewrites_the_saved_inverse_cell() {
        let overlay = Overlay {
            saved: vec![SavedCell {
                row: 10,
                col: 4,
                text: " ".to_string(),
                inverse: true,
                dim: false,
            }],
            suffix: "lp".to_string(),
            cursor: (11, 0),
            reveal_cursor: true,
        };
        let text = String::from_utf8(restore(&overlay)).unwrap();
        assert!(text.contains("\x1b[7m"));
        assert!(text.contains("\x1b[12;1H"));
    }

    #[test]
    fn suffix_stops_at_the_last_column() {
        assert_eq!(fit_suffix("lp", 78, 80), "lp");
        assert_eq!(fit_suffix("hello", 78, 80), "he");
        assert!(fit_suffix("x", 80, 80).is_empty());
    }
}
