use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::error::Result;

const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

/// Companion state lives here, not in Claude's settings directory.
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn system() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
        Ok(Self::new(
            PathBuf::from(home).join("Library/Application Support/ccword"),
        ))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn database_file(&self) -> PathBuf {
        self.root.join("ngrams.sqlite")
    }

    pub fn database_backup(&self) -> PathBuf {
        self.root.join("ngrams.sqlite.bak")
    }

    pub fn debug_log(&self) -> PathBuf {
        self.root.join("debug.log")
    }

    pub fn plugin_dir(&self) -> PathBuf {
        self.root.join("plugin")
    }

    pub fn ensure_root(&self) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        restrict_dir(&self.root)?;
        Ok(())
    }
}

pub fn restrict_dir(path: &Path) -> io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(DIR_MODE);
    fs::set_permissions(path, perms)
}

pub fn restrict_file(path: &Path) -> io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(FILE_MODE);
    fs::set_permissions(path, perms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_root_creates_private_directory() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("ccword"));
        paths.ensure_root().unwrap();
        let mode = fs::metadata(paths.root()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
