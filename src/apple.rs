use crate::ngram::suffix_for;
use crate::utf16::char_range_to_utf16;

/// One dictionary completion. No correction and no mutation of the prompt.
pub trait WordCompleter: Send {
    fn complete_word(
        &self,
        text: &str,
        start_chars: usize,
        end_chars: usize,
        language: Option<&str>,
    ) -> Option<String>;

    fn languages(&self) -> Vec<String> {
        Vec::new()
    }
}

pub struct Unavailable;

impl WordCompleter for Unavailable {
    fn complete_word(&self, _: &str, _: usize, _: usize, _: Option<&str>) -> Option<String> {
        None
    }
}

#[cfg(target_os = "macos")]
pub struct AppleCompleter {
    tag: objc2::ffi::NSInteger,
}

#[cfg(target_os = "macos")]
impl AppleCompleter {
    pub fn new() -> Self {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
        }
        let tag = objc2_app_kit::NSSpellChecker::uniqueSpellDocumentTag();
        Self { tag }
    }
}

#[cfg(target_os = "macos")]
impl Default for AppleCompleter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "macos")]
impl WordCompleter for AppleCompleter {
    fn complete_word(
        &self,
        text: &str,
        start_chars: usize,
        end_chars: usize,
        language: Option<&str>,
    ) -> Option<String> {
        use objc2_foundation::{NSArray, NSString};
        let (location, length) = char_range_to_utf16(text, start_chars, end_chars)?;
        if length == 0 {
            return None;
        }
        let prefix: String = text
            .chars()
            .skip(start_chars)
            .take(end_chars - start_chars)
            .collect();
        let ns_text = NSString::from_str(text);
        let lang = language
            .filter(|lang| !lang.is_empty())
            .map(NSString::from_str);
        if let Some(lang) = lang.as_ref() {
            if !self
                .languages()
                .iter()
                .any(|known| known == &lang.to_string())
            {
                return None;
            }
        }
        let range = objc2_foundation::NSRange {
            location: location as _,
            length: length as _,
        };
        let checker = objc2_app_kit::NSSpellChecker::sharedSpellChecker();
        let words: Option<objc2::rc::Retained<NSArray<NSString>>> = checker
            .completionsForPartialWordRange_inString_language_inSpellDocumentWithTag(
                range,
                &ns_text,
                lang.as_deref(),
                self.tag,
            );
        let words = words?;
        for index in 0..words.count() {
            let candidate = words.objectAtIndex(index).to_string();
            if candidate.chars().any(char::is_whitespace) {
                continue;
            }
            if let Some(suffix) = suffix_for(&candidate, &prefix) {
                return Some(suffix);
            }
        }
        None
    }

    fn languages(&self) -> Vec<String> {
        use objc2_foundation::{NSArray, NSString};
        let checker = objc2_app_kit::NSSpellChecker::sharedSpellChecker();
        let langs: objc2::rc::Retained<NSArray<NSString>> = checker.availableLanguages();
        (0..langs.count())
            .map(|i| langs.objectAtIndex(i).to_string())
            .collect()
    }
}

#[cfg(not(target_os = "macos"))]
pub struct AppleCompleter;

#[cfg(not(target_os = "macos"))]
impl AppleCompleter {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(target_os = "macos"))]
impl WordCompleter for AppleCompleter {
    fn complete_word(&self, _: &str, _: usize, _: usize, _: Option<&str>) -> Option<String> {
        None
    }
}

pub fn first_valid_suffix(prefix: &str, candidates: &[String]) -> Option<String> {
    candidates.iter().find_map(|candidate| {
        if candidate.chars().any(char::is_whitespace) {
            return None;
        }
        suffix_for(candidate, prefix)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_first_prefix_match_and_rejects_the_rest() {
        let got = first_valid_suffix(
            "he",
            &["held".to_string(), "help".to_string(), "he".to_string()],
        );
        assert_eq!(got.as_deref(), Some("ld"));
    }

    #[test]
    fn empty_candidate_list_shows_nothing() {
        assert!(first_valid_suffix("he", &[]).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn spell_checker_returns_a_suffix_or_nothing_for_he() {
        let apple = AppleCompleter::new();
        let suffix = apple.complete_word("he", 0, 2, None);
        if let Some(suffix) = suffix {
            assert!(!suffix.is_empty());
            assert!(!suffix.chars().any(char::is_whitespace));
            let full = format!("he{suffix}");
            assert!(full.to_lowercase().starts_with("he"));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unsupported_language_returns_nothing() {
        let apple = AppleCompleter::new();
        assert!(apple.complete_word("he", 0, 2, Some("zz-ZZ")).is_none());
    }
}
