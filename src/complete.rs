use std::time::Instant;

use crate::apple::WordCompleter;
use crate::config::{Config, Mode, PREDICTION_DEADLINE_MS};
use crate::eligibility::PartialWord;
use crate::ngram::suffix_for;
use crate::store::{self, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub suffix: String,
    pub elapsed_ms: u128,
}

pub fn hint_for(
    config: &Config,
    store: Option<&Store>,
    apple: &dyn WordCompleter,
    buffer: &str,
    partial: &PartialWord,
) -> Option<Hint> {
    if config.mode == Mode::Off {
        return None;
    }
    let started = Instant::now();
    let learned = || {
        let store = store?;
        let scored = store::predict(
            store,
            &partial.preceding,
            &partial.prefix,
            config,
            crate::config::now_ms(),
        )
        .ok()??;
        suffix_for(&scored.display, &partial.prefix).filter(|suffix| !suffix.is_empty())
    };
    let system = || {
        let language = if config.language.is_empty() {
            None
        } else {
            Some(config.language.as_str())
        };
        let end = partial.start + partial.prefix.chars().count();
        apple
            .complete_word(buffer, partial.start, end, language)
            .filter(|suffix| !suffix.is_empty())
    };
    let suffix = match config.mode {
        Mode::Off => None,
        Mode::Ngram => learned(),
        Mode::Apple => system(),
        Mode::Auto => learned().or_else(system),
    }?;
    let elapsed_ms = started.elapsed().as_millis();
    let _deadline = PREDICTION_DEADLINE_MS;
    Some(Hint { suffix, elapsed_ms })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple::Unavailable;
    use crate::eligibility::{partial_word, Gates};

    struct Scripted(Option<String>);

    impl WordCompleter for Scripted {
        fn complete_word(&self, _: &str, _: usize, _: usize, _: Option<&str>) -> Option<String> {
            self.0.clone()
        }
    }

    fn gates() -> Gates {
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
    fn off_mode_never_asks_the_provider() {
        let config = Config::default();
        let partial = partial_word("he", gates()).unwrap();
        assert!(hint_for(&config, None, &Scripted(Some("lp".into())), "he", &partial).is_none());
    }

    #[test]
    fn apple_mode_returns_the_provider_suffix_only() {
        let config = Config {
            mode: Mode::Apple,
            ..Config::default()
        };
        let partial = partial_word("he", gates()).unwrap();
        let hint = hint_for(&config, None, &Scripted(Some("lp".into())), "he", &partial).unwrap();
        assert_eq!(hint.suffix, "lp");
    }

    #[test]
    fn ngram_mode_ignores_the_apple_provider() {
        let mut store = Store::open_in_memory().unwrap();
        store.learn_prompt("please help", 0, 30.0).unwrap();
        store.learn_prompt("please help", 1, 30.0).unwrap();
        let config = Config {
            mode: Mode::Ngram,
            min_support: 2,
            min_confidence: 0.15,
            ..Config::default()
        };
        let partial = partial_word("please he", gates()).unwrap();
        let hint = hint_for(&config, Some(&store), &Unavailable, "please he", &partial).unwrap();
        assert_eq!(hint.suffix, "lp");
    }
}
