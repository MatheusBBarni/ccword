use std::fs::OpenOptions;
use std::io::Write;

use crate::paths::Paths;

const MAX_BYTES: u64 = 1_048_576;

/// Parser state and timing only. Prompt text is never written.
pub fn record(paths: &Paths, enabled: bool, message: &str) {
    if !enabled {
        return;
    }
    if message_has_prompt(message) {
        return;
    }
    let path = paths.debug_log();
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > MAX_BYTES {
            return;
        }
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = paths::restrict(&path);
    let _ = writeln!(file, "{message}");
}

fn message_has_prompt(message: &str) -> bool {
    message.contains("prompt=") || message.contains("buffer=\"")
}

mod paths {
    use std::path::Path;
    pub fn restrict(path: &Path) -> std::io::Result<()> {
        crate::paths::restrict_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_log_records_state_and_skips_prompt_text() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure_root().unwrap();
        record(
            &paths,
            true,
            "adapter=claude-code-v2 hint=yes suffix_chars=2 elapsed_ms=4",
        );
        record(&paths, true, "prompt=secret");
        let text = std::fs::read_to_string(paths.debug_log()).unwrap();
        assert!(text.contains("suffix_chars=2"));
        assert!(!text.contains("secret"));
    }
}
