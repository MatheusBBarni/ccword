use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::store::Store;

const PLUGIN_JSON: &str = r#"{
  "name": "ccword",
  "description": "Learn submitted Claude Code prompts into the local ccword n-gram store. Does not complete text.",
  "version": "0.1.0"
}
"#;

const HOOKS_JSON: &str = r#"{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "__CCWORD__",
            "args": ["hook"],
            "async": true,
            "timeout": 5
          }
        ]
      }
    ]
  }
}
"#;

/// Hook entry. Always exits 0. Never prints the prompt or a decision.
pub fn run_hook() -> Result<i32> {
    let mut raw = String::new();
    io::stdin().read_to_string(&mut raw)?;
    if std::env::var_os("CCWORD_WRAPPER").is_some() {
        return Ok(0);
    }
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return Ok(0);
    };
    if value.get("hook_event_name").and_then(Value::as_str) != Some("UserPromptSubmit") {
        return Ok(0);
    }
    let Some(prompt) = value.get("prompt").and_then(Value::as_str) else {
        return Ok(0);
    };
    let paths = Paths::system()?;
    let config = Config::load(&paths);
    if !config.learning {
        return Ok(0);
    }
    if let Ok(mut store) = Store::open(&paths) {
        let _ = store.learn_prompt(prompt, crate::config::now_ms(), config.half_life_days);
    }
    Ok(0)
}

pub fn hook_status(paths: &Paths) -> Result<String> {
    let plugin = paths.plugin_dir().join("hooks/hooks.json").is_file();
    let settings = settings_path();
    let installed = settings
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok())
        .is_some_and(|text| text.contains("\"hook\"") && text.contains("ccword"));
    Ok(format!(
        "plugin files: {}\nuser settings hook: {}\nsettings: {}",
        if plugin { "present" } else { "absent" },
        if installed { "present" } else { "absent" },
        settings
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(HOME is unset)".to_string())
    ))
}

pub fn install_hook(paths: &Paths, write_settings: bool) -> Result<String> {
    let exe = std::env::current_exe()?;
    let root = paths.plugin_dir();
    fs::create_dir_all(root.join(".claude-plugin"))?;
    fs::create_dir_all(root.join("hooks"))?;
    fs::write(root.join(".claude-plugin/plugin.json"), PLUGIN_JSON)?;
    let hooks = HOOKS_JSON.replace("__CCWORD__", &exe.display().to_string());
    fs::write(root.join("hooks/hooks.json"), hooks)?;
    let mut msg = format!(
        "plugin: {}\nload for one session with: claude --plugin-dir {}",
        root.display(),
        root.display()
    );
    if write_settings {
        let settings = settings_path().ok_or_else(|| Error::Message("HOME is not set".into()))?;
        merge_user_hook(&settings, &exe)?;
        msg.push_str("\nwrote the UserPromptSubmit hook into ");
        msg.push_str(&settings.display().to_string());
    }
    Ok(msg)
}

pub fn uninstall_hook(paths: &Paths) -> Result<String> {
    let plugin = paths.plugin_dir();
    if plugin.exists() {
        fs::remove_dir_all(&plugin)?;
    }
    if let Some(settings) = settings_path() {
        if settings.is_file() {
            remove_user_hook(&settings)?;
        }
    }
    Ok("removed the ccword plugin and its user hook".to_string())
}

fn settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".claude/settings.json"))
}

fn merge_user_hook(path: &Path, exe: &Path) -> Result<()> {
    let existing = if path.is_file() {
        let text = fs::read_to_string(path)?;
        if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&text)?
        }
    } else {
        json!({})
    };
    let mut root = existing.as_object().cloned().ok_or_else(|| {
        Error::Message(format!(
            "{} is not a JSON object; refusing to rewrite it",
            path.display()
        ))
    })?;
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or_else(|| Error::Message("settings hooks field is not an object".into()))?;
    let event = hooks_obj
        .entry("UserPromptSubmit")
        .or_insert_with(|| json!([]));
    let groups = event
        .as_array_mut()
        .ok_or_else(|| Error::Message("UserPromptSubmit is not an array".into()))?;
    let command = exe.display().to_string();
    if !groups.iter().any(|group| group_has_ccword(group, &command)) {
        groups.push(json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "args": ["hook"],
                "async": true,
                "timeout": 5
            }]
        }));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(root))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.write_all(b"\n")?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn remove_user_hook(path: &Path) -> Result<()> {
    let text = fs::read_to_string(path)?;
    let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
        return Ok(());
    };
    let Some(groups) = value
        .get_mut("hooks")
        .and_then(|h| h.get_mut("UserPromptSubmit"))
        .and_then(Value::as_array_mut)
    else {
        return Ok(());
    };
    groups.retain(|group| !group_has_ccword(group, "ccword"));
    let text = serde_json::to_string_pretty(&value)?;
    fs::write(path, format!("{text}\n"))?;
    Ok(())
}

fn group_has_ccword(group: &Value, needle: &str) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| {
            hooks.iter().any(|hook| {
                let command = hook.get("command").and_then(Value::as_str).unwrap_or("");
                let args = hook.get("args").and_then(Value::as_array);
                let is_hook =
                    args.is_some_and(|args| args.iter().any(|arg| arg.as_str() == Some("hook")));
                is_hook && (command.contains(needle) || command.ends_with("ccword"))
            })
        })
}

pub fn claude_version(claude: &Path) -> String {
    match Command::new(claude).arg("--version").output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Ok(out) => format!("exited {}", out.status),
        Err(err) => format!("unavailable ({err})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_json_ignores_other_events_and_does_not_echo_the_prompt() {
        let value: Value =
            serde_json::from_str(r#"{"hook_event_name":"Stop","prompt":"secret prompt"}"#).unwrap();
        assert_ne!(value["hook_event_name"], "UserPromptSubmit");
        assert!(value["prompt"].as_str().unwrap().contains("secret"));
    }

    #[test]
    fn install_merge_adds_one_async_hook_and_uninstall_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        fs::write(&settings, "{}\n").unwrap();
        let exe = dir.path().join("ccword");
        fs::write(&exe, b"bin").unwrap();
        merge_user_hook(&settings, &exe).unwrap();
        merge_user_hook(&settings, &exe).unwrap();
        let text = fs::read_to_string(&settings).unwrap();
        assert_eq!(text.matches("\"hook\"").count(), 1);
        assert!(text.contains("\"async\": true"));
        assert!(!text.contains("secret"));
        remove_user_hook(&settings).unwrap();
        let text = fs::read_to_string(&settings).unwrap();
        assert!(!text.contains("UserPromptSubmit") || !text.contains("ccword"));
    }
}
