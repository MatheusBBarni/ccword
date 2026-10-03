use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::paths::{self, Paths};

pub const PREDICTION_DEADLINE_MS: u64 = 30;

/// Local completion modes. Scores are never blended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Off,
    Auto,
    Ngram,
    Apple,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "auto" => Some(Self::Auto),
            "ngram" | "n-gram" => Some(Self::Ngram),
            "apple" => Some(Self::Apple),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Auto => "auto",
            Self::Ngram => "ngram",
            Self::Apple => "apple",
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub mode: Mode,
    /// Empty means the macOS spelling language.
    pub language: String,
    pub learning: bool,
    pub min_confidence: f64,
    pub min_support: u64,
    pub half_life_days: f64,
    pub debug: bool,
    /// Right Arrow accepts and appends one ASCII space. Tab never does.
    pub right_arrow_appends_space: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            language: String::new(),
            learning: false,
            min_confidence: 0.15,
            min_support: 2,
            half_life_days: 30.0,
            debug: false,
            right_arrow_appends_space: true,
        }
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Self {
        let path = paths.config_file();
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|_| Self::default()),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        paths.ensure_root()?;
        let path = paths.config_file();
        write_private(
            &path,
            &toml::to_string_pretty(self)
                .map_err(|err| Error::Message(format!("could not encode config: {err}")))?,
        )
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "mode" => {
                self.mode = Mode::parse(value).ok_or_else(|| {
                    Error::Message("mode must be off, auto, ngram, or apple".to_string())
                })?;
            }
            "language" => {
                self.language = if value == "system" || value == "-" {
                    String::new()
                } else {
                    value.to_string()
                };
            }
            "learning" => self.learning = parse_bool(value)?,
            "min-confidence" | "min_confidence" => {
                self.min_confidence = parse_unit(value, "min-confidence")?;
            }
            "min-support" | "min_support" => {
                self.min_support = value.parse().map_err(|_| {
                    Error::Message("min-support must be a non-negative integer".to_string())
                })?;
            }
            "half-life-days" | "half_life_days" => {
                let days: f64 = value.parse().map_err(|_| {
                    Error::Message("half-life-days must be a positive number".to_string())
                })?;
                if !days.is_finite() || days <= 0.0 {
                    return Err(Error::Message(
                        "half-life-days must be a positive number".to_string(),
                    ));
                }
                self.half_life_days = days;
            }
            "debug" => self.debug = parse_bool(value)?,
            "right-arrow-appends-space" | "right_arrow_appends_space" => {
                self.right_arrow_appends_space = parse_bool(value)?;
            }
            _ => {
                return Err(Error::Message(format!(
                    "unknown setting `{key}`; expected mode, language, learning, min-confidence, min-support, half-life-days, debug, right-arrow-appends-space"
                )));
            }
        }
        Ok(())
    }
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Ok(true),
        "0" | "false" | "off" | "no" => Ok(false),
        _ => Err(Error::Message("expected on or off".to_string())),
    }
}

fn parse_unit(value: &str, name: &str) -> Result<f64> {
    let parsed: f64 = value
        .parse()
        .map_err(|_| Error::Message(format!("{name} must be a number between 0 and 1")))?;
    if !(0.0..=1.0).contains(&parsed) {
        return Err(Error::Message(format!(
            "{name} must be a number between 0 and 1"
        )));
    }
    Ok(parsed)
}

pub fn write_private(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        paths::restrict_dir(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }
    paths::restrict_file(&tmp)?;
    fs::rename(&tmp, path)?;
    paths::restrict_file(path)?;
    Ok(())
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn corrupt_config_fails_open_to_off() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        fs::create_dir_all(paths.root()).unwrap();
        fs::write(paths.config_file(), "mode = [").unwrap();
        let config = Config::load(&paths);
        assert_eq!(config.mode, Mode::Off);
        assert!(!config.learning);
    }

    #[test]
    fn set_rejects_unknown_mode_and_persists_apple() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let mut config = Config::default();
        assert!(config.set("mode", "smol").is_err());
        config.set("mode", "apple").unwrap();
        config.set("learning", "on").unwrap();
        config.save(&paths).unwrap();
        let loaded = Config::load(&paths);
        assert_eq!(loaded.mode, Mode::Apple);
        assert!(loaded.learning);
        let mode = fs::metadata(paths.config_file())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
