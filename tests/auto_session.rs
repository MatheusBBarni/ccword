mod support;

use ccword::config::{now_ms, Config, Mode};
use ccword::paths::Paths;
use ccword::store::Store;
use std::fs;
use std::time::Duration;
use support::Terminal;

#[test]
fn auto_uses_existing_history_without_enabling_learning() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("Library/Application Support/ccword"));
    let config = Config {
        mode: Mode::Auto,
        learning: false,
        ..Config::default()
    };
    config.save(&paths).unwrap();
    let mut store = Store::open(&paths).unwrap();
    for _ in 0..2 {
        store.learn_prompt("please helper", now_ms(), 30.0).unwrap();
    }
    let rows = store.count_rows().unwrap();
    drop(store);
    let log = home.path().join("submitted.log");
    let fake = std::path::Path::new(env!("CARGO_BIN_EXE_ccword-fake-tui"));
    let script = home.path().join("fake-claude.sh");
    fs::write(&script, format!("#!/bin/sh\nexec '{}'\n", fake.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let mut term = Terminal::spawn(
        &[],
        home.path(),
        24,
        80,
        &[("CCWORD_CLAUDE", &script), ("CCWORD_FAKE_LOG", &log)],
    );
    term.until("❯");
    term.send(b"please he");
    term.until("helper");
    term.send(b"\r");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let text = fs::read_to_string(&log).unwrap_or_default();
        if text.contains("SUBMITTED:please he") {
            assert!(!text.contains("SUBMITTED:please helper"), "{text}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "missing typed submission: {text}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let saved = Config::load(&paths);
    assert!(!saved.learning);
    assert_eq!(Store::open(&paths).unwrap().count_rows().unwrap(), rows);
}
