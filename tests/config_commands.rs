use std::process::Command;

#[test]
fn config_set_auto_round_trips_without_learning() {
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ccword"))
            .env("HOME", home.path())
            .args(args)
            .output()
            .unwrap()
    };
    let set = run(&["ctl", "config", "set", "mode", "auto"]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    assert!(String::from_utf8_lossy(&set.stdout).contains("mode=auto"));
    let show = run(&["ctl", "config", "show"]);
    assert!(show.status.success());
    let text = String::from_utf8(show.stdout).unwrap();
    assert!(text.contains("mode = \"auto\""), "{text}");
    assert!(text.contains("learning = false"), "{text}");
}

#[test]
fn bare_config_requires_terminal_without_writing() {
    let home = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ccword"))
            .env("HOME", home.path())
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["ctl", "config", "set", "mode", "apple"])
        .status
        .success());
    let file = home
        .path()
        .join("Library/Application Support/ccword/config.toml");
    let before = std::fs::read(&file).unwrap();
    let result = run(&["ctl", "config"]);
    assert!(!result.status.success());
    let error = String::from_utf8(result.stderr).unwrap();
    assert!(error.contains("terminal required"), "{error}");
    assert_eq!(std::fs::read(&file).unwrap(), before);
    let show = run(&["ctl", "config", "show"]);
    assert!(show.status.success());
    assert!(String::from_utf8_lossy(&show.stdout).contains("mode = \"apple\""));
}

#[test]
fn existing_modes_and_auto_diagnostic_keep_script_contract() {
    use ccword::config::{now_ms, Config, Mode};
    use ccword::paths::Paths;
    use ccword::store::Store;
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("Library/Application Support/ccword"));
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ccword"))
            .env("HOME", home.path())
            .args(args)
            .output()
            .unwrap()
    };
    for mode in ["off", "apple", "ngram"] {
        std::fs::create_dir_all(paths.root()).unwrap();
        std::fs::write(paths.config_file(), format!(
            "mode = \"{mode}\"\nlanguage = \"\"\nlearning = false\nmin_confidence = 0.15\nmin_support = 2\nhalf_life_days = 30.0\ndebug = false\nright_arrow_appends_space = true\n"
        )).unwrap();
        assert_eq!(Config::load(&paths).mode.as_str(), mode);
    }
    Config {
        mode: Mode::Auto,
        learning: false,
        ..Config::default()
    }
    .save(&paths)
    .unwrap();
    let mut store = Store::open(&paths).unwrap();
    for _ in 0..2 {
        store.learn_prompt("please helper", now_ms(), 30.0).unwrap();
    }
    drop(store);
    let result = run(&["ctl", "complete", "--mode", "auto", "please he"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), "lper");
    let before = std::fs::read(paths.config_file()).unwrap();
    let invalid = run(&["ctl", "config", "set", "mode", "smol"]);
    assert!(!invalid.status.success());
    assert_eq!(std::fs::read(paths.config_file()).unwrap(), before);
    let invalid = run(&["ctl", "config", "set", "half-life-days", "NaN"]);
    assert!(!invalid.status.success());
    assert_eq!(std::fs::read(paths.config_file()).unwrap(), before);
}
