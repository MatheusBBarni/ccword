use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;

use crate::apple::AppleCompleter;
use crate::complete::hint_for;
use crate::config::Config;
use crate::doctor;
use crate::eligibility::{partial_word, Gates};
use crate::error::{Error, Result};
use crate::hook;
use crate::learn;
use crate::paths::Paths;

pub(crate) fn flag_command(flag: &str) -> Option<&'static str> {
    match flag {
        "-h" | "--h" | "-help" | "--help" => Some("help"),
        "-v" | "-V" | "--v" | "-version" | "--version" => Some("version"),
        "-u" | "--u" | "-update" | "--update" => Some("update"),
        "-c" | "--c" | "-config" | "--config" => Some("config"),
        _ => None,
    }
}

pub fn run(args: &[OsString]) -> Result<i32> {
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        print_help();
        return Ok(0);
    };
    let cmd = flag_command(cmd).unwrap_or(cmd);
    if cmd == "help" {
        print_help();
        return Ok(0);
    }
    if cmd == "version" {
        println!("ccword {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if cmd == "update" {
        if args.len() != 1 {
            return Err(Error::Message(
                "usage: ccword --update (or ccword ctl update)".into(),
            ));
        }
        return update();
    }
    let paths = Paths::system()?;
    match cmd {
        "doctor" => {
            let apple = AppleCompleter::new();
            println!("{}", doctor::report(&paths, &apple));
            Ok(0)
        }
        "config" => config_cmd(&paths, &args[1..]),
        "learn" => learn_cmd(&paths, &args[1..]),
        "hook" => hook_cmd(&paths, &args[1..]),
        "complete" => complete_cmd(&paths, &args[1..]),
        other => Err(Error::Message(format!(
            "unknown ctl command `{other}`; try `ccword ctl help`"
        ))),
    }
}

fn update() -> Result<i32> {
    let exe = std::env::current_exe()?;
    let bin = exe
        .parent()
        .ok_or_else(|| Error::Message("cannot find installation directory".into()))?;
    if bin.file_name().is_none_or(|name| name != "bin") {
        return Err(Error::Message(
            "update requires an installed ccword in <root>/bin; for a source build, run `git pull --ff-only` and `cargo install --path . --locked --bin ccword --force`".into(),
        ));
    }
    let root = bin
        .parent()
        .ok_or_else(|| Error::Message("cannot find installation root".into()))?;
    println!(
        "Updating ccword from https://github.com/MatheusBBarni/ccword (main) into {}",
        bin.display()
    );
    io::stdout().flush()?;
    let status = std::process::Command::new("cargo")
        .args([
            "install",
            "--git",
            "https://github.com/MatheusBBarni/ccword",
            "--branch",
            "main",
            "--locked",
            "--bin",
            "ccword",
            "--force",
            "--root",
        ])
        .arg(root)
        .arg("ccword")
        .status()
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                Error::Message(
                    "Cargo is required to update ccword; install Rust from https://rustup.rs"
                        .into(),
                )
            } else {
                Error::Message(format!("could not start Cargo update: {err}"))
            }
        })?;
    if !status.success() {
        return Err(Error::Message(format!(
            "ccword update failed: Cargo {status}"
        )));
    }
    println!("ccword updated. Run `ccword ctl version` to inspect the installed version.");
    Ok(0)
}

fn config_cmd(paths: &Paths, args: &[String]) -> Result<i32> {
    match args.first().map(String::as_str) {
        None => {
            if !crate::term::is_tty(libc::STDIN_FILENO) || !crate::term::is_tty(libc::STDOUT_FILENO)
            {
                return Err(Error::Message("terminal required for interactive config; use `ccword ctl config show` or `set` in scripts".into()));
            }
            crate::config_tui::run(paths)
        }
        Some("show") => {
            let config = Config::load(paths);
            println!("{}", toml::to_string_pretty(&config).unwrap_or_default());
            println!("file: {}", paths.config_file().display());
            Ok(0)
        }
        Some("set") => {
            let key = args
                .get(1)
                .ok_or_else(|| Error::Message("missing setting name".into()))?;
            let value = args
                .get(2)
                .ok_or_else(|| Error::Message("missing setting value".into()))?;
            let mut config = Config::load(paths);
            config.set(key, value)?;
            config.save(paths)?;
            println!("{key}={}", display_set(key, &config));
            Ok(0)
        }
        Some(other) => Err(Error::Message(format!("unknown config command `{other}`"))),
    }
}

fn display_set(key: &str, config: &Config) -> String {
    match key {
        "mode" => config.mode.to_string(),
        "language" => {
            if config.language.is_empty() {
                "system".to_string()
            } else {
                config.language.clone()
            }
        }
        "learning" => on_off(config.learning),
        "debug" => on_off(config.debug),
        "min-confidence" | "min_confidence" => config.min_confidence.to_string(),
        "min-support" | "min_support" => config.min_support.to_string(),
        "half-life-days" | "half_life_days" => config.half_life_days.to_string(),
        "right-arrow-appends-space" | "right_arrow_appends_space" => {
            on_off(config.right_arrow_appends_space)
        }
        _ => "ok".to_string(),
    }
}

fn learn_cmd(paths: &Paths, args: &[String]) -> Result<i32> {
    let mut config = Config::load(paths);
    match args.first().map(String::as_str) {
        Some("status") | None => {
            let rows = crate::store::Store::open(paths)
                .ok()
                .and_then(|s| s.count_rows().ok());
            println!(
                "learning: {}\nstore: {}\nrows: {}",
                on_off(config.learning),
                paths.database_file().display(),
                rows.map(|n| n.to_string())
                    .unwrap_or_else(|| "0".to_string())
            );
            Ok(0)
        }
        Some("path") => {
            println!("{}", paths.database_file().display());
            Ok(0)
        }
        Some("pause") => {
            config.learning = false;
            config.save(paths)?;
            println!("learning=off");
            Ok(0)
        }
        Some("resume") => {
            config.learning = true;
            config.save(paths)?;
            println!("learning=on");
            Ok(0)
        }
        Some("clear") => {
            learn::clear(paths)?;
            println!("deleted learned n-gram counts");
            Ok(0)
        }
        Some("undo") => {
            learn::undo(paths)?;
            println!("restored the pre-import database");
            Ok(0)
        }
        Some("import") => import_cmd(paths, &args[1..], config.half_life_days),
        Some(other) => Err(Error::Message(format!("unknown learn command `{other}`"))),
    }
}

fn import_cmd(paths: &Paths, args: &[String], half_life_days: f64) -> Result<i32> {
    let apply = args.iter().any(|a| a == "--apply");
    let preview = args.iter().any(|a| a == "--preview") || !apply;
    let file = args.iter().find(|a| !a.starts_with('-')).ok_or_else(|| {
        Error::Message("usage: ccword ctl learn import --preview|--apply <file>".into())
    })?;
    let path = Path::new(file);
    if preview && !apply {
        let stats = learn::preview_file(path)?;
        println!(
            "prompts: {}\ntokens: {}\nstore unchanged. Re-run with --apply to write counts.",
            stats.prompts, stats.tokens
        );
        return Ok(0);
    }
    let stats = learn::apply_file(paths, path, half_life_days)?;
    println!(
        "learned {} prompts ({} tokens). Undo with `ccword ctl learn undo`.",
        stats.prompts, stats.tokens
    );
    Ok(0)
}

fn hook_cmd(paths: &Paths, args: &[String]) -> Result<i32> {
    match args.first().map(String::as_str) {
        Some("status") | None => {
            println!("{}", hook::hook_status(paths)?);
            Ok(0)
        }
        Some("install") => {
            let write = args.iter().any(|a| a == "--write-settings");
            println!("{}", hook::install_hook(paths, write)?);
            Ok(0)
        }
        Some("uninstall") => {
            println!("{}", hook::uninstall_hook(paths)?);
            Ok(0)
        }
        Some(other) => Err(Error::Message(format!("unknown hook command `{other}`"))),
    }
}

fn complete_cmd(paths: &Paths, args: &[String]) -> Result<i32> {
    let mut config = Config::load(paths);
    let mut prompt = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| Error::Message("missing mode".into()))?;
                config.set("mode", value)?;
                i += 2;
            }
            "--language" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| Error::Message("missing language".into()))?;
                config.set("language", value)?;
                i += 2;
            }
            other => {
                if !prompt.is_empty() {
                    prompt.push(' ');
                }
                prompt.push_str(other);
                i += 1;
            }
        }
    }
    if prompt.is_empty() {
        return Err(Error::Message(
            "usage: ccword ctl complete [--mode off|auto|apple|ngram] <prompt>".into(),
        ));
    }
    let gates = Gates {
        confident: true,
        at_end: true,
        native_menu: false,
        dialog: false,
        selection: false,
        ime: false,
    };
    let Some(partial) = partial_word(&prompt, gates) else {
        return Ok(0);
    };
    let store = crate::store::Store::open(paths).ok();
    let apple = AppleCompleter::new();
    if let Some(hint) = hint_for(&config, store.as_ref(), &apple, &prompt, &partial) {
        let mut out = io::stdout().lock();
        writeln!(out, "{}", hint.suffix)?;
    }
    Ok(0)
}

fn on_off(value: bool) -> String {
    if value {
        "on".into()
    } else {
        "off".into()
    }
}

fn print_help() {
    println!(
        "\
ccword launches the installed claude binary and can show one local word hint.

  ccword [claude arguments]     launch Claude Code
  ccword -h, --help             show ccword help
  ccword -v, --version          show ccword version
  ccword -u, --update           install latest main using Cargo (network required)
  ccword -c, --config           open the settings TUI (terminal required)
  ccword ctl config              edit all settings in a terminal
  ccword ctl config show
  ccword ctl config set <key> <value>
  ccword ctl doctor
  ccword ctl learn status|pause|resume|clear|path
  ccword ctl learn import --preview|--apply <file>
  ccword ctl learn undo
  ccword ctl hook status|install [--write-settings]|uninstall
  ccword ctl complete [--mode off|auto|apple|ngram] <prompt>
  ccword ctl version
  ccword ctl update

Aliases: --h / -help, --v / -version / -V, --u / -update, --c / -config.
Root flags act only when used alone; use `ccword -- --help` for Claude help.
Update requires an installation in <root>/bin and replaces only ccword there.

Settings keys: mode (off, auto, apple, ngram), language, learning, min-confidence,
min-support, half-life-days, debug, right-arrow-appends-space.

Interactive config: Tab/Shift-Tab or Up/Down moves between fields.
Click Language or press Space on it to choose installed languages.
Enter uses a language choice, then Enter saves settings; Escape backs out or cancels.

Right Arrow accepts the hint and adds a space. Tab accepts it without a space.
Enter submits only the text you typed or accepted. Hints stay on this Mac."
    );
}
