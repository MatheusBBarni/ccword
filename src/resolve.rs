use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Find the installed `claude` binary without resolving back to ccword.
pub fn resolve_claude(
    current_exe: &Path,
    explicit: Option<&std::ffi::OsStr>,
    path_env: Option<&std::ffi::OsStr>,
) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_with_home(current_exe, explicit, path_env, home.as_deref(), true)
}

pub fn resolve_with_home(
    current_exe: &Path,
    explicit: Option<&std::ffi::OsStr>,
    path_env: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
    common_locations: bool,
) -> Result<PathBuf> {
    let self_canon = fs::canonicalize(current_exe).unwrap_or_else(|_| current_exe.to_path_buf());
    if let Some(explicit) = explicit {
        let candidate = PathBuf::from(explicit);
        return accept(&candidate, &self_canon);
    }
    if let Some(path_env) = path_env {
        for dir in std::env::split_paths(path_env) {
            let candidate = dir.join("claude");
            if let Ok(found) = accept(&candidate, &self_canon) {
                return Ok(found);
            }
        }
    }
    if let Some(home) = home {
        let home = PathBuf::from(home);
        for rel in [".local/bin/claude", ".claude/local/claude"] {
            let candidate = home.join(rel);
            if let Ok(found) = accept(&candidate, &self_canon) {
                return Ok(found);
            }
        }
    }
    if common_locations {
        for candidate in [
            "/opt/homebrew/bin/claude",
            "/usr/local/bin/claude",
            "/usr/bin/claude",
        ] {
            if let Ok(found) = accept(Path::new(candidate), &self_canon) {
                return Ok(found);
            }
        }
    }
    Err(Error::ClaudeNotFound)
}

fn accept(candidate: &Path, self_canon: &Path) -> Result<PathBuf> {
    if !candidate.is_file() {
        return Err(Error::ClaudeNotFound);
    }
    let canon = fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf());
    if same_file(&canon, self_canon) || is_companion_name(&canon) {
        return Err(Error::RecursiveLaunch { path: canon });
    }
    Ok(canon)
}

fn is_companion_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name == "ccword" || name.starts_with("ccword-")
}

fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(left), Ok(right)) => {
            use std::os::unix::fs::MetadataExt;
            left.dev() == right.dev() && left.ino() == right.ino()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_the_companion_and_finds_claude() {
        let dir = tempfile::tempdir().unwrap();
        let companion = dir.path().join("bin/ccword");
        let claude = dir.path().join("bin/claude");
        fs::create_dir_all(companion.parent().unwrap()).unwrap();
        fs::write(&companion, b"#!/bin/sh\n").unwrap();
        fs::write(&claude, b"#!/bin/sh\n").unwrap();
        let path = companion.parent().unwrap().as_os_str();
        let found = resolve_claude(&companion, None, Some(path)).unwrap();
        assert_eq!(found.file_name().unwrap(), "claude");
    }

    #[test]
    fn explicit_self_path_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let companion = dir.path().join("ccword");
        fs::write(&companion, b"bin").unwrap();
        let err = resolve_claude(&companion, Some(companion.as_os_str()), None).unwrap_err();
        assert!(matches!(err, Error::RecursiveLaunch { .. }));
    }

    #[test]
    fn missing_claude_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let companion = dir.path().join("ccword");
        fs::write(&companion, b"bin").unwrap();
        let err = resolve_with_home(&companion, None, Some(dir.path().as_os_str()), None, false)
            .unwrap_err();
        assert!(matches!(err, Error::ClaudeNotFound));
    }
}
