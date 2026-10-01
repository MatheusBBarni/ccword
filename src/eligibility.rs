use crate::tokenize::{self, Token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialWord {
    pub prefix: String,
    pub start: usize,
    pub preceding: Vec<Token>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gates {
    pub confident: bool,
    pub at_end: bool,
    pub native_menu: bool,
    pub dialog: bool,
    pub selection: bool,
    pub ime: bool,
}

impl Gates {
    pub fn open(self) -> bool {
        self.confident
            && self.at_end
            && !self.native_menu
            && !self.dialog
            && !self.selection
            && !self.ime
    }
}

/// Return the trailing alphabetic prefix when a hint is allowed.
pub fn partial_word(buffer: &str, gates: Gates) -> Option<PartialWord> {
    if !gates.open() {
        return None;
    }
    if buffer.chars().any(|ch| ch == '\n' || ch == '\r') {
        let last = buffer.rsplit(['\n', '\r']).next().unwrap_or(buffer);
        if last != buffer && !last_line_is_end(buffer) {
            return None;
        }
    }
    let token = current_token(buffer);
    if suppressed_token(token) || line_is_special(buffer) {
        return None;
    }
    if odd_backticks(buffer) {
        return None;
    }
    let prefix = trailing_word(token)?;
    if prefix.chars().count() < 2 {
        return None;
    }
    if prefix.chars().any(|ch| ch.is_numeric()) {
        return None;
    }
    let start = buffer.chars().count() - prefix.chars().count();
    let preceding = tokenize::context_before_partial(buffer, start);
    Some(PartialWord {
        prefix,
        start,
        preceding,
    })
}

fn last_line_is_end(buffer: &str) -> bool {
    !buffer.ends_with('\n') && !buffer.ends_with('\r')
}

fn current_token(buffer: &str) -> &str {
    let line = buffer.rsplit(['\n', '\r']).next().unwrap_or(buffer);
    line.rsplit(char::is_whitespace).next().unwrap_or(line)
}

fn line_is_special(buffer: &str) -> bool {
    buffer.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with('/') || trimmed.starts_with('!') || trimmed.starts_with('>')
    }) || buffer.trim_start().starts_with('@')
}

fn suppressed_token(token: &str) -> bool {
    if token.is_empty() {
        return true;
    }
    let first = token.chars().next().unwrap();
    if matches!(first, '/' | '!' | '@' | '-' | '`' | '#' | '$' | '>') {
        return true;
    }
    token
        .chars()
        .any(|ch| matches!(ch, '/' | '\\' | ':' | '.' | '_' | '@' | '`' | '='))
}

fn odd_backticks(buffer: &str) -> bool {
    buffer.chars().filter(|ch| *ch == '`').count() % 2 == 1
}

fn trailing_word(token: &str) -> Option<String> {
    let chars: Vec<char> = token.chars().collect();
    if chars.is_empty()
        || !chars
            .iter()
            .all(|ch| tokenize::is_word_char(*ch) || *ch == '\'')
    {
        let mut end = chars.len();
        while end > 0 && tokenize::is_word_char(chars[end - 1]) {
            end -= 1;
        }
        let word: String = chars[end..].iter().collect();
        if word.chars().count() >= 2 && word.chars().all(tokenize::is_word_char) {
            return Some(word);
        }
        return None;
    }
    Some(token.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> Gates {
        Gates {
            confident: true,
            at_end: true,
            native_menu: false,
            dialog: false,
            selection: false,
            ime: false,
        }
    }

    #[test]
    fn accepts_a_two_letter_natural_prefix() {
        let word = partial_word("please he", open()).unwrap();
        assert_eq!(word.prefix, "he");
        assert_eq!(word.preceding[0].normalized, "please");
    }

    #[test]
    fn rejects_short_closed_and_uncertain_states() {
        assert!(partial_word("h", open()).is_none());
        let mut gates = open();
        gates.at_end = false;
        assert!(partial_word("help", gates).is_none());
        gates = open();
        gates.confident = false;
        assert!(partial_word("help", gates).is_none());
        gates = open();
        gates.native_menu = true;
        assert!(partial_word("help", gates).is_none());
        gates = open();
        gates.ime = true;
        assert!(partial_word("help", gates).is_none());
        gates = open();
        gates.selection = true;
        assert!(partial_word("help", gates).is_none());
        gates = open();
        gates.dialog = true;
        assert!(partial_word("help", gates).is_none());
    }

    #[test]
    fn suppresses_commands_paths_code_and_shell() {
        let gates = open();
        for sample in [
            "/help",
            "run /status",
            "@file",
            "see @readme",
            "src/main",
            "https://example.com/he",
            "--help",
            "!ls",
            "use `code",
            "foo_bar",
            "item42",
            "v1",
            "obj.method",
        ] {
            assert!(partial_word(sample, gates).is_none(), "{sample}");
        }
    }

    #[test]
    fn multiline_away_from_the_last_line_is_rejected_by_the_gate() {
        let mut gates = open();
        gates.at_end = false;
        assert!(partial_word("help me\nstill he", gates).is_none());
    }
}
