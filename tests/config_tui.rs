mod support;

use ccword::config::{Config, Mode};
use ccword::paths::Paths;
use support::Terminal;

#[test]
fn screen_displays_all_saved_settings() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("Library/Application Support/ccword"));
    Config {
        mode: Mode::Apple,
        language: String::new(),
        learning: true,
        min_confidence: 0.35,
        min_support: 4,
        half_life_days: 14.0,
        debug: true,
        right_arrow_appends_space: false,
    }
    .save(&paths)
    .unwrap();
    let mut term = Terminal::spawn(&["ctl", "config"], home.path(), 24, 80, &[]);
    let view = term.until("Right Arrow appends space");
    for label in [
        "Mode",
        "Language",
        "Learning",
        "Min confidence",
        "Min support",
        "Half-life days",
        "Debug",
        "Right Arrow appends space",
    ] {
        assert!(view.contains(label), "missing {label}: {view}");
    }
    for value in ["apple", "system default", "on", "0.35", "4", "14", "off"] {
        assert!(view.contains(value), "missing {value}: {view}");
    }
    term.send(b"\x1b");
    assert_eq!(term.child.wait().unwrap().exit_code(), 0);
}

#[test]
fn keyboard_edits_cancel_or_save_as_one_change() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("Library/Application Support/ccword"));
    Config::default().save(&paths).unwrap();
    let original = std::fs::read(paths.config_file()).unwrap();
    for save in [false, true] {
        let mut term = Terminal::spawn(&["ctl", "config"], home.path(), 24, 80, &[]);
        term.until("Mode: off");
        term.send(b" ");
        term.until("Mode: auto");
        term.send(b"\t");
        term.send(b"en");
        term.until("Language: en");
        term.send(b"\t ");
        term.until("Learning: on");
        term.send(b"\t\x15");
        term.send(b"0.4");
        term.until("Min confidence: 0.4");
        term.send(b"\t\x15");
        term.send(b"3");
        term.until("Min support: 3");
        term.send(b"\t\x15");
        term.send(b"12");
        term.until("Half-life days: 12");
        term.send(b"\t ");
        term.until("Debug: on");
        term.send(b"\t ");
        term.until("Right Arrow appends space: off");
        term.send(b"\x1b[Z");
        term.until("> Debug: on");
        term.send(b"\x1b[B");
        term.until("> Right Arrow appends space: off");
        term.send(if save { b"\r" } else { b"\x1b" });
        assert_eq!(term.child.wait().unwrap().exit_code(), 0);
        let flags = term.master.get_termios().unwrap().local_flags.bits();
        assert_ne!(flags & libc::ICANON, 0);
        assert_ne!(flags & libc::ECHO, 0);
        if !save {
            assert_eq!(std::fs::read(paths.config_file()).unwrap(), original);
        }
    }
    let saved: toml::Value =
        toml::from_str(&std::fs::read_to_string(paths.config_file()).unwrap()).unwrap();
    assert_eq!(saved["mode"].as_str(), Some("auto"));
    assert_eq!(saved["language"].as_str(), Some("en"));
    assert_eq!(saved["learning"].as_bool(), Some(true));
    assert_eq!(saved["min_confidence"].as_float(), Some(0.4));
    assert_eq!(saved["min_support"].as_integer(), Some(3));
    assert_eq!(saved["half_life_days"].as_float(), Some(12.0));
    assert_eq!(saved["debug"].as_bool(), Some(true));
    assert_eq!(saved["right_arrow_appends_space"].as_bool(), Some(false));
    let mut reopened = Terminal::spawn(&["ctl", "config"], home.path(), 24, 80, &[]);
    reopened.until("Mode: auto");
    reopened.send(b"\x1b");
    assert_eq!(reopened.child.wait().unwrap().exit_code(), 0);
}

#[test]
fn invalid_numeric_fields_show_errors_and_do_not_save() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("Library/Application Support/ccword"));
    Config::default().save(&paths).unwrap();
    let original = std::fs::read(paths.config_file()).unwrap();
    for (field, value, label) in [
        (3, "1.2", "Min confidence"),
        (4, "-1", "Min support"),
        (5, "0", "Half-life days"),
        (5, "NaN", "Half-life days"),
        (5, "inf", "Half-life days"),
    ] {
        let mut term = Terminal::spawn(&["ctl", "config"], home.path(), 24, 80, &[]);
        term.until("Mode: off");
        for _ in 0..field {
            term.send(b"\t");
        }
        term.send(b"\x15");
        term.send(value.as_bytes());
        term.until(&format!("{label}: {value}"));
        term.send(b"\r");
        let view = term.until(&format!("{label}: {value}  Error:"));
        assert!(view.contains("Error:"), "{view}");
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), original);
        term.send(b"\x1b");
        assert_eq!(term.child.wait().unwrap().exit_code(), 0);
    }
}

#[test]
fn small_terminal_shows_size_message_without_writing() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("Library/Application Support/ccword"));
    Config::default().save(&paths).unwrap();
    let before = std::fs::read(paths.config_file()).unwrap();
    let mut term = Terminal::spawn(&["ctl", "config"], home.path(), 12, 35, &[]);
    let view = term.until("Terminal too small");
    assert!(view.contains("minimum"), "{view}");
    term.send(b"\x1b");
    assert_eq!(term.child.wait().unwrap().exit_code(), 0);
    assert_eq!(std::fs::read(paths.config_file()).unwrap(), before);
}

#[test]
fn failed_save_restores_terminal_and_keeps_config() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path().join("Library/Application Support/ccword"));
    Config::default().save(&paths).unwrap();
    let original = std::fs::read(paths.config_file()).unwrap();
    std::fs::create_dir(paths.root().join("config.tmp")).unwrap();
    let mut term = Terminal::spawn(&["ctl", "config"], home.path(), 24, 80, &[]);
    term.until("Mode: off");
    term.send(b" ");
    term.until("Mode: auto");
    term.send(b"\r");
    assert_ne!(term.child.wait().unwrap().exit_code(), 0);
    assert_eq!(std::fs::read(paths.config_file()).unwrap(), original);
    let attrs = term.master.get_termios().unwrap();
    assert_ne!(attrs.local_flags.bits() & libc::ICANON, 0);
    assert_ne!(attrs.local_flags.bits() & libc::ECHO, 0);
}

#[test]
fn raw_mode_restores_after_unwind() {
    use portable_pty::{native_pty_system, PtySize};
    use std::os::fd::AsRawFd;
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(pair.master.tty_name().unwrap())
        .unwrap();
    let fd = tty.as_raw_fd();
    let flags = |fd| {
        let mut attrs = std::mem::MaybeUninit::<libc::termios>::uninit();
        assert_eq!(unsafe { libc::tcgetattr(fd, attrs.as_mut_ptr()) }, 0);
        let attrs = unsafe { attrs.assume_init() };
        // macOS may add the kernel-owned 0x20000000 lflag on tcsetattr.
        (
            attrs.c_iflag,
            attrs.c_oflag,
            attrs.c_cflag,
            attrs.c_lflag & !0x20000000,
            attrs.c_cc,
        )
    };
    let before = flags(fd);
    let caught = std::panic::catch_unwind(|| {
        let _guard = ccword::term::RawMode::enter(fd).unwrap();
        assert_ne!(flags(fd).3, before.3);
        panic!("unwind terminal guard");
    });
    assert!(caught.is_err());
    assert_eq!(flags(fd), before);
}
