use unicode_normalization::UnicodeNormalization;

/// A word taken from a submitted prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub normalized: String,
    pub display: String,
}

/// Split `prompt` into alphabetic words.
///
/// Apostrophes inside a word are kept (`don't`). Digits, paths, and
/// punctuation break the word. The original surface form is retained for
/// rendering; lookup uses NFC lowercase.
pub fn tokenize(prompt: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    for raw in prompt.split_whitespace() {
        if !is_natural_span(raw) {
            continue;
        }
        tokenize_span(raw, &mut tokens);
    }
    tokens
}

fn is_natural_span(raw: &str) -> bool {
    !raw.contains(['/', '\\', ':', '_', '@', '=', '#'])
        && !raw.starts_with('-')
        && !raw.chars().any(|ch| ch.is_ascii_digit())
}

fn tokenize_span(raw: &str, tokens: &mut Vec<Token>) {
    let mut display = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if !is_word_char(ch) {
            continue;
        }
        display.push(ch);
        while let Some(&next) = chars.peek() {
            if next == '\'' || next == '\u{2019}' {
                let mut look = chars.clone();
                look.next();
                if look.peek().is_some_and(|c| is_word_char(*c)) {
                    display.push(chars.next().unwrap());
                    continue;
                }
            }
            if is_word_char(next) {
                display.push(chars.next().unwrap());
                continue;
            }
            break;
        }
        push_token(tokens, &display);
        display.clear();
    }
}

fn push_token(tokens: &mut Vec<Token>, display: &str) {
    if display.chars().count() < 2 {
        return;
    }
    let normalized = normalize(display);
    if normalized.chars().count() < 2 {
        return;
    }
    tokens.push(Token {
        normalized,
        display: display.to_string(),
    });
}

pub fn normalize(text: &str) -> String {
    text.nfc().collect::<String>().to_lowercase()
}

pub fn is_word_char(ch: char) -> bool {
    ch.is_alphabetic()
}

/// Words before the trailing partial word, for context backoff.
pub fn context_before_partial(prompt: &str, partial_start: usize) -> Vec<Token> {
    let head: String = prompt.chars().take(partial_start).collect();
    tokenize(&head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_words_and_keeps_internal_apostrophes() {
        let tokens = tokenize("Please don't rewrite the URL https://example.com or foo_bar");
        let norms: Vec<_> = tokens.iter().map(|t| t.normalized.as_str()).collect();
        assert_eq!(
            norms,
            vec!["please", "don't", "rewrite", "the", "url", "or"]
        );
    }

    #[test]
    fn normalizes_case_and_compatibility_without_losing_display() {
        let tokens = tokenize("Café HELP");
        assert_eq!(tokens[0].normalized, "café");
        assert_eq!(tokens[0].display, "Café");
        assert_eq!(tokens[1].display, "HELP");
        assert_eq!(tokens[1].normalized, "help");
    }

    #[test]
    fn context_stops_before_the_partial_word() {
        let prompt = "please he";
        let ctx = context_before_partial(prompt, 7);
        assert_eq!(ctx.len(), 1);
        assert_eq!(ctx[0].normalized, "please");
    }
}
