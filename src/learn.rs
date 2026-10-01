use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use crate::config::now_ms;
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::store::Store;
use crate::tokenize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPreview {
    pub prompts: usize,
    pub tokens: usize,
}

pub fn preview_file(path: &Path) -> Result<ImportPreview> {
    let prompts = read_prompts(path)?;
    let tokens = prompts.iter().map(|p| tokenize::tokenize(p).len()).sum();
    Ok(ImportPreview {
        prompts: prompts.len(),
        tokens,
    })
}

pub fn apply_file(paths: &Paths, path: &Path, half_life_days: f64) -> Result<ImportPreview> {
    let preview = preview_file(path)?;
    let store = Store::open(paths)?;
    if paths.database_file().is_file() {
        store.backup_to(&paths.database_backup())?;
    }
    drop(store);
    let mut store = Store::open(paths)?;
    let now = now_ms();
    for prompt in read_prompts(path)? {
        store.learn_prompt(&prompt, now, half_life_days)?;
    }
    Ok(preview)
}

pub fn undo(paths: &Paths) -> Result<()> {
    let backup = paths.database_backup();
    if !backup.is_file() {
        return Err(Error::Message("no import backup to restore".into()));
    }
    fs::copy(&backup, paths.database_file())?;
    crate::paths::restrict_file(&paths.database_file())?;
    Ok(())
}

pub fn clear(paths: &Paths) -> Result<()> {
    if paths.database_file().is_file() {
        let mut store = Store::open(paths)?;
        store.clear()?;
    }
    let _ = fs::remove_file(paths.database_backup());
    Ok(())
}

fn read_prompts(path: &Path) -> Result<Vec<String>> {
    let file = fs::File::open(path)?;
    let mut prompts = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(prompt) = json_prompt(trimmed) {
            prompts.push(prompt);
            continue;
        }
        prompts.push(trimmed.to_string());
    }
    Ok(prompts)
}

fn json_prompt(line: &str) -> Option<String> {
    if !line.starts_with('{') {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    value
        .get("prompt")
        .or_else(|| value.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::store::predict;
    use crate::tokenize::tokenize;

    #[test]
    fn preview_counts_without_requiring_apply() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("prompts.txt");
        fs::write(&file, "please help\n{\"prompt\":\"show the logs\"}\n").unwrap();
        let preview = preview_file(&file).unwrap();
        assert_eq!(preview.prompts, 2);
        assert!(preview.tokens >= 4);
    }

    #[test]
    fn apply_is_reversible_and_clear_deletes_counts() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("state"));
        let file = dir.path().join("prompts.txt");
        fs::write(&file, "please help\nplease help\n").unwrap();
        apply_file(&paths, &file, 30.0).unwrap();
        let store = Store::open(&paths).unwrap();
        let config = Config {
            min_support: 2,
            min_confidence: 0.1,
            ..Config::default()
        };
        let scored = predict(&store, &tokenize("please"), "he", &config, now_ms())
            .unwrap()
            .unwrap();
        assert_eq!(scored.token, "help");
        drop(store);
        undo(&paths).unwrap();
        let store = Store::open(&paths).unwrap();
        assert_eq!(store.count_rows().unwrap(), 0);
        drop(store);
        apply_file(&paths, &file, 30.0).unwrap();
        clear(&paths).unwrap();
        let store = Store::open(&paths).unwrap();
        assert_eq!(store.count_rows().unwrap(), 0);
    }
}
