use std::io::{self, IsTerminal};

use crate::apple::WordCompleter;
use crate::config::Config;
use crate::hook::{self, claude_version};
use crate::paths::Paths;
use crate::resolve::resolve_claude;
use crate::store::Store;
use crate::term;

pub fn report(paths: &Paths, apple: &dyn WordCompleter) -> String {
    let config = Config::load(paths);
    let exe = std::env::current_exe().ok();
    let claude = exe.as_deref().and_then(|current| {
        resolve_claude(
            current,
            std::env::var_os("CCWORD_CLAUDE").as_deref(),
            std::env::var_os("PATH").as_deref(),
        )
        .ok()
    });
    let version = claude
        .as_deref()
        .map(claude_version)
        .unwrap_or_else(|| "not found".to_string());
    let stdin_tty = io::stdin().is_terminal();
    let stdout_tty = io::stdout().is_terminal();
    let size = term::terminal_size(libc::STDOUT_FILENO);
    let rows = Store::open(paths)
        .ok()
        .and_then(|store| store.count_rows().ok());
    let languages = apple.languages();
    let lang = if languages.is_empty() {
        "unavailable".to_string()
    } else {
        format!("{} installed", languages.len())
    };
    let preferred = if config.language.is_empty() {
        "system".to_string()
    } else if languages.iter().any(|item| item == &config.language) {
        format!("{} (installed)", config.language)
    } else if !languages.is_empty() {
        format!("{} (not installed)", config.language)
    } else {
        config.language.clone()
    };
    format!(
        "ccword 0.1.0\n\
         claude: {}\n\
         version: {version}\n\
         stdin tty: {stdin_tty}\n\
         stdout tty: {stdout_tty}\n\
         term: {}\n\
         size: {}x{}\n\
         provider: {}\n\
         learning: {}\n\
         apple languages: {lang}\n\
         language override: {preferred}\n\
         store: {}\n\
         ngram rows: {}\n\
         {}\n\
         adapter: claude-code-v2, disabled when the live layout is not recognized\n\
         network: none",
        claude
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".to_string()),
        std::env::var("TERM").unwrap_or_else(|_| "unset".to_string()),
        size.cols,
        size.rows,
        config.mode,
        if config.learning { "on" } else { "off" },
        paths.database_file().display(),
        rows.map(|n| n.to_string())
            .unwrap_or_else(|| "unavailable".to_string()),
        hook::hook_status(paths).unwrap_or_else(|err| err.to_string()),
    )
}
