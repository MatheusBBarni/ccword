use ccword::apple::WordCompleter;
use ccword::complete::hint_for;
use ccword::config::{now_ms, Config, Mode};
use ccword::eligibility::{partial_word, Gates};
use ccword::store::Store;

struct Apple(&'static str);
impl WordCompleter for Apple {
    fn complete_word(&self, _: &str, _: usize, _: usize, _: Option<&str>) -> Option<String> {
        Some(self.0.into())
    }
}

fn partial(prompt: &str) -> ccword::eligibility::PartialWord {
    partial_word(
        prompt,
        Gates {
            confident: true,
            at_end: true,
            native_menu: false,
            dialog: false,
            selection: false,
            ime: false,
        },
    )
    .unwrap()
}

#[test]
fn auto_prefers_eligible_learned_word() {
    let mut store = Store::open_in_memory().unwrap();
    let now = now_ms();
    for _ in 0..2 {
        store.learn_prompt("please helper", now, 30.0).unwrap();
    }
    let config = Config {
        mode: Mode::Auto,
        ..Config::default()
    };
    let result = hint_for(
        &config,
        Some(&store),
        &Apple("lp"),
        "please he",
        &partial("please he"),
    );
    assert_eq!(result.unwrap().suffix, "lper");
}

#[test]
fn auto_uses_apple_when_learned_word_is_missing() {
    let store = Store::open_in_memory().unwrap();
    let config = Config {
        mode: Mode::Auto,
        ..Config::default()
    };
    let result = hint_for(&config, Some(&store), &Apple("lp"), "he", &partial("he"));
    assert_eq!(result.unwrap().suffix, "lp");
}

#[test]
fn auto_returns_no_hint_without_eligible_sources() {
    let mut store = Store::open_in_memory().unwrap();
    store.learn_prompt("please helper", now_ms(), 30.0).unwrap();
    let config = Config {
        mode: Mode::Auto,
        ..Config::default()
    };
    assert!(hint_for(
        &config,
        Some(&store),
        &ccword::apple::Unavailable,
        "please he",
        &partial("please he")
    )
    .is_none());
    assert!(hint_for(
        &config,
        None,
        &ccword::apple::Unavailable,
        "please he",
        &partial("please he")
    )
    .is_none());
}
