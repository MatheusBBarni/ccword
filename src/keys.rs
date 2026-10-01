/// A key or opaque byte sequence taken from the real terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub raw: Vec<u8>,
    pub kind: KeyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Text,
    Right,
    Left,
    Up,
    Down,
    Home,
    End,
    Tab,
    /// Plain Enter submits. A modified Enter inserts a newline.
    Enter {
        modified: bool,
    },
    Escape,
    Backspace,
    CtrlC,
    CtrlZ,
    PasteStart,
    PasteEnd,
    /// Mouse, focus, and unrecognized sequences. Always forward unchanged.
    Opaque,
}

#[derive(Debug, Default)]
pub struct KeyParser {
    buf: Vec<u8>,
    paste: bool,
}

impl KeyParser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Piece> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(piece) = self.pop_ready() {
            out.push(piece);
        }
        out
    }

    /// A lone ESC that never grew into a sequence is the Escape key.
    pub fn flush_escape(&mut self) -> Option<Piece> {
        if self.buf == [0x1b] {
            self.buf.clear();
            Some(Piece {
                raw: vec![0x1b],
                kind: KeyKind::Escape,
            })
        } else {
            None
        }
    }

    pub fn in_paste(&self) -> bool {
        self.paste
    }

    fn pop_ready(&mut self) -> Option<Piece> {
        if self.buf.is_empty() {
            return None;
        }
        if self.paste {
            return self.pop_paste();
        }
        if self.buf[0] != 0x1b {
            return self.pop_plain();
        }
        if self.buf.len() == 1 {
            return None;
        }
        match self.buf[1] {
            b'[' => self.pop_csi(),
            b'O' => self.pop_ss3(),
            b']' | b'P' | b'^' | b'_' => self.pop_until_st(),
            _ => {
                let raw = self.buf.drain(..2).collect();
                Some(Piece {
                    raw,
                    kind: KeyKind::Opaque,
                })
            }
        }
    }

    fn pop_plain(&mut self) -> Option<Piece> {
        let byte = self.buf[0];
        if byte == 0x03 {
            return Some(self.take(1, KeyKind::CtrlC));
        }
        if byte == 0x1a {
            return Some(self.take(1, KeyKind::CtrlZ));
        }
        if byte == b'\t' {
            return Some(self.take(1, KeyKind::Tab));
        }
        if byte == b'\r' || byte == b'\n' {
            return Some(self.take(1, KeyKind::Enter { modified: false }));
        }
        if byte == 0x7f || byte == 0x08 {
            return Some(self.take(1, KeyKind::Backspace));
        }
        if byte < 0x20 {
            return Some(self.take(1, KeyKind::Opaque));
        }
        let width = utf8_width(byte);
        if self.buf.len() < width {
            return None;
        }
        if std::str::from_utf8(&self.buf[..width]).is_err() {
            return Some(self.take(1, KeyKind::Opaque));
        }
        Some(self.take(width, KeyKind::Text))
    }

    fn pop_paste(&mut self) -> Option<Piece> {
        let term = b"\x1b[201~";
        if let Some(end) = find_subsequence(&self.buf, term) {
            if end == 0 {
                self.paste = false;
                return Some(self.take(term.len(), KeyKind::PasteEnd));
            }
            return Some(self.take(end, KeyKind::Text));
        }
        let keep = trailing_prefix_len(&self.buf, term);
        if self.buf.len() > keep {
            return Some(self.take(self.buf.len() - keep, KeyKind::Text));
        }
        None
    }

    fn pop_ss3(&mut self) -> Option<Piece> {
        if self.buf.len() < 3 {
            return None;
        }
        let kind = arrow_from_final(self.buf[2], false);
        Some(self.take(3, kind))
    }

    fn pop_csi(&mut self) -> Option<Piece> {
        let final_at = self
            .buf
            .iter()
            .skip(2)
            .position(|b| (0x40..=0x7e).contains(b))?;
        let end = final_at + 3;
        if end > 128 {
            return Some(self.take(1, KeyKind::Opaque));
        }
        let raw = self.buf[..end].to_vec();
        let kind = classify_csi(&raw);
        if kind == KeyKind::PasteStart {
            self.paste = true;
        }
        self.buf.drain(..end);
        Some(Piece { raw, kind })
    }

    fn pop_until_st(&mut self) -> Option<Piece> {
        if let Some(at) = self.buf.iter().position(|b| *b == 0x07) {
            return Some(self.take(at + 1, KeyKind::Opaque));
        }
        if let Some(at) = find_subsequence(&self.buf, b"\x1b\\") {
            return Some(self.take(at + 2, KeyKind::Opaque));
        }
        if self.buf.len() > 4096 {
            return Some(self.take(self.buf.len(), KeyKind::Opaque));
        }
        None
    }

    fn take(&mut self, n: usize, kind: KeyKind) -> Piece {
        Piece {
            raw: self.buf.drain(..n).collect(),
            kind,
        }
    }
}
fn trailing_prefix_len(buf: &[u8], needle: &[u8]) -> usize {
    let max = buf.len().min(needle.len().saturating_sub(1));
    for len in (1..=max).rev() {
        if needle.starts_with(&buf[buf.len() - len..]) {
            return len;
        }
    }
    0
}

fn utf8_width(byte: u8) -> usize {
    if byte & 0x80 == 0 {
        1
    } else if byte & 0xe0 == 0xc0 {
        2
    } else if byte & 0xf0 == 0xe0 {
        3
    } else if byte & 0xf8 == 0xf0 {
        4
    } else {
        1
    }
}

fn find_subsequence(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn classify_csi(raw: &[u8]) -> KeyKind {
    if raw == b"\x1b[200~" {
        return KeyKind::PasteStart;
    }
    if raw == b"\x1b[201~" {
        return KeyKind::PasteEnd;
    }
    let final_byte = *raw.last().unwrap_or(&0);
    let params = &raw[2..raw.len() - 1];
    if params.starts_with(b"<") {
        return KeyKind::Opaque;
    }
    if final_byte == b'u' {
        return classify_kitty(params);
    }
    if matches!(final_byte, b'A' | b'B' | b'C' | b'D' | b'H' | b'F') {
        let modified = modifier_present(params);
        if modified && !matches!(final_byte, b'C') {
            return KeyKind::Opaque;
        }
        if final_byte == b'C' && modified {
            return KeyKind::Opaque;
        }
        return arrow_from_final(final_byte, false);
    }
    if final_byte == b'~' {
        let head = params.split(|b| *b == b';').next().unwrap_or(params);
        return match head {
            b"1" | b"7" => KeyKind::Home,
            b"4" | b"8" => KeyKind::End,
            b"3" => KeyKind::Backspace,
            _ => KeyKind::Opaque,
        };
    }
    KeyKind::Opaque
}

fn arrow_from_final(byte: u8, _modified: bool) -> KeyKind {
    match byte {
        b'A' => KeyKind::Up,
        b'B' => KeyKind::Down,
        b'C' => KeyKind::Right,
        b'D' => KeyKind::Left,
        b'H' => KeyKind::Home,
        b'F' => KeyKind::End,
        _ => KeyKind::Opaque,
    }
}

fn modifier_present(params: &[u8]) -> bool {
    let mut parts = params.split(|b| *b == b';');
    let _code = parts.next();
    match parts.next() {
        None | Some(b"") | Some(b"1") => false,
        Some(_) => true,
    }
}

fn classify_kitty(params: &[u8]) -> KeyKind {
    let mut fields = params.split(|b| *b == b';');
    let code_field = fields.next().unwrap_or(b"");
    let code = code_field.split(|b| *b == b':').next().unwrap_or(b"");
    let mods_field = fields.next().unwrap_or(b"1");
    let mut mods_parts = mods_field.split(|b| *b == b':');
    let mods: u32 = std::str::from_utf8(mods_parts.next().unwrap_or(b"1"))
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let event: u32 = mods_parts
        .next()
        .and_then(|s| std::str::from_utf8(s).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    if event == 3 {
        return KeyKind::Opaque;
    }
    let modified = mods != 1;
    let code: u32 = std::str::from_utf8(code)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    match code {
        9 | 57346 if !modified => KeyKind::Tab,
        13 | 57345 if !modified => KeyKind::Enter { modified: false },
        13 | 57345 => KeyKind::Enter { modified: true },
        27 | 57344 if !modified => KeyKind::Escape,
        127 | 57347 => KeyKind::Backspace,
        57350 if !modified => KeyKind::Left,
        57351 if !modified => KeyKind::Right,
        57352 if !modified => KeyKind::Up,
        57353 if !modified => KeyKind::Down,
        57356 if !modified => KeyKind::Home,
        57357 if !modified => KeyKind::End,
        _ => KeyKind::Opaque,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_and_tab_keep_their_raw_bytes() {
        let mut parser = KeyParser::default();
        let pieces = parser.push(b"\x1b[C\t");
        assert_eq!(pieces[0].kind, KeyKind::Right);
        assert_eq!(pieces[0].raw, b"\x1b[C");
        assert_eq!(pieces[1].kind, KeyKind::Tab);
        assert_eq!(pieces[1].raw, b"\t");
    }

    #[test]
    fn incomplete_escape_is_held_until_flush() {
        let mut parser = KeyParser::default();
        assert!(parser.push(b"\x1b").is_empty());
        assert_eq!(parser.flush_escape().unwrap().kind, KeyKind::Escape);
    }

    #[test]
    fn mouse_and_modified_right_are_not_accept_keys() {
        let mut parser = KeyParser::default();
        let pieces = parser.push(b"\x1b[<0;1;1M\x1b[1;2C");
        assert_eq!(pieces[0].kind, KeyKind::Opaque);
        assert_eq!(pieces[1].kind, KeyKind::Opaque);
    }

    #[test]
    fn paste_is_opaque_until_the_terminator() {
        let mut parser = KeyParser::default();
        let start = parser.push(b"\x1b[200~he\x1b[C");
        assert_eq!(start[0].kind, KeyKind::PasteStart);
        assert!(parser.in_paste());
        assert_eq!(start[1].kind, KeyKind::Text);
        let rest = parser.push(b"\x1b[201~");
        assert!(rest.iter().any(|p| p.kind == KeyKind::PasteEnd));
        assert!(!parser.in_paste());
    }

    #[test]
    fn kitty_tab_and_modified_enter_are_distinct() {
        let mut parser = KeyParser::default();
        let pieces = parser.push(b"\x1b[9u\x1b[13;2u\x1b[57351u");
        assert_eq!(pieces[0].kind, KeyKind::Tab);
        assert_eq!(pieces[1].kind, KeyKind::Enter { modified: true });
        assert_eq!(pieces[2].kind, KeyKind::Right);
    }

    #[test]
    fn utf8_is_not_split() {
        let mut parser = KeyParser::default();
        assert!(parser.push("é".as_bytes()[..1].into()).is_empty());
        let pieces = parser.push("é".as_bytes()[1..].into());
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].kind, KeyKind::Text);
        assert_eq!(pieces[0].raw, "é".as_bytes());
    }
}
