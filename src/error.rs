use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Recoverable companion failure.
///
/// Suggestion failures are not errors. Callers disable the hint and keep
/// forwarding input.
#[derive(Debug, Error)]
pub enum Error {
    #[error("could not find the installed claude executable")]
    ClaudeNotFound,
    #[error("refusing to launch ccword recursively ({})", path.display())]
    RecursiveLaunch { path: PathBuf },
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Store(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Pty(#[from] anyhow::Error),
}
pub type Result<T, E = Error> = std::result::Result<T, E>;
