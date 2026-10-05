//! Context-warning hook: a `UserPromptSubmit` hook that runs a bundled PowerShell
//! script (see `hooks/context-hook.ps1`) which prints a `systemMessage` — shown to
//! the user, never injected into the model's context, never blocking — once a
//! session's context crosses 850k tokens. See CLAUDE.md's "## Kontext-Hook".

use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use crate::omniroute::rotate_backup;
use crate::{read_settings, settings_path, write_settings, SETTINGS_LOCK};

const SCRIPT_CONTENT: &str = include_str!("../hooks/context-hook.ps1");
const SCRIPT_FILE_NAME: &str = "context-hook.ps1";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    installed: bool,
    script_path: String,
    settings_path: String,
}

fn script_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("Konnte lokalen App-Datenordner nicht finden: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("Konnte App-Datenordner nicht anlegen: {e}"))?;
    Ok(dir.join(SCRIPT_FILE_NAME))
}

/// Overwrites the script file with the currently embedded version — called on every
/// install so an app update's script changes propagate to already-installed hooks.
/// `hooks/context-hook.ps1` is saved with a UTF-8 BOM (kept as-is by `include_str!`
/// and written straight through here): Windows PowerShell 5.1 falls back to the
/// system ANSI codepage for a script with no BOM, mangling the message's "≈"/"—".
fn write_script(app: &AppHandle) -> Result<PathBuf, String> {
    let path = script_path(app)?;
    fs::write(&path, SCRIPT_CONTENT).map_err(|e| format!("Konnte Hook-Skript nicht schreiben: {e}"))?;
    Ok(path)
}

/// True if any of `args` (a hook command's `args` array) is our hook script,
/// matched by shape rather than the full current path: a string ending in
/// `context-hook.ps1`, case-insensitive (`/` vs `\` doesn't affect a suffix match).
/// This way an entry left over from an older install location still counts as ours.
fn args_reference_script(args: &[Value]) -> bool {
    args.iter()
        .any(|a| a.as_str().is_some_and(|s| s.to_lowercase().ends_with("context-hook.ps1")))
}

fn hooks_matching_script(hooks: &[Value]) -> bool {
    hooks
        .iter()
        .any(|h| h.get("args").and_then(Value::as_array).is_some_and(|a| args_reference_script(a)))
}

fn hook_installed(settings: &Value) -> bool {
    settings
        .get("hooks")
        .and_then(|h| h.get("UserPromptSubmit"))
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.get("hooks").and_then(Value::as_array).is_some_and(|hs| hooks_matching_script(hs)))
        })
}

fn build_hook_entry(script: &str) -> Value {
    serde_json::json!({
        "hooks": [{
            "type": "command",
            "command": "powershell.exe",
            "args": ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script],
            "timeout": 10
        }]
    })
}

/// Pure JSON patch (unit-testable, no I/O) — same shape as `apply_skill_override`/
/// `apply_route`. Adds exactly one `hooks.UserPromptSubmit` entry for our script
/// (idempotent) or removes only that entry, leaving every other hook/key untouched.
pub(crate) fn apply_context_hook(mut settings: Value, script: &str, enable: bool) -> Result<Value, String> {
    let obj = settings
        .as_object_mut()
        .ok_or_else(|| "settings.json hat kein Objekt auf oberster Ebene.".to_string())?;

    if enable {
        let hooks = obj
            .entry("hooks")
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        let hooks_obj = hooks
            .as_object_mut()
            .ok_or_else(|| "\"hooks\" in settings.json ist kein Objekt.".to_string())?;
        let ups = hooks_obj
            .entry("UserPromptSubmit")
            .or_insert_with(|| Value::Array(Vec::new()));
        let ups_arr = ups
            .as_array_mut()
            .ok_or_else(|| "\"hooks.UserPromptSubmit\" in settings.json ist kein Array.".to_string())?;

        let already_present = ups_arr
            .iter()
            .any(|entry| entry.get("hooks").and_then(Value::as_array).is_some_and(|hs| hooks_matching_script(hs)));
        if !already_present {
            ups_arr.push(build_hook_entry(script));
        }
    } else if let Some(hooks_obj) = obj.get_mut("hooks").and_then(Value::as_object_mut) {
        if let Some(ups_arr) = hooks_obj.get_mut("UserPromptSubmit").and_then(Value::as_array_mut) {
            for entry in ups_arr.iter_mut() {
                if let Some(inner) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
                    inner.retain(|h| !h.get("args").and_then(Value::as_array).is_some_and(|a| args_reference_script(a)));
                }
            }
            ups_arr.retain(|entry| entry.get("hooks").and_then(Value::as_array).map(|a| !a.is_empty()).unwrap_or(true));
            if ups_arr.is_empty() {
                hooks_obj.remove("UserPromptSubmit");
            }
        }
        if hooks_obj.is_empty() {
            obj.remove("hooks");
        }
    }
    Ok(settings)
}

fn apply_hook_setting(app: &AppHandle, script: &str, enable: bool) -> Result<(), String> {
    let _guard = SETTINGS_LOCK.lock().map_err(|_| "Interner Sperr-Fehler.".to_string())?;

    let path = settings_path()?;
    let (original_text, file_existed) = match fs::read_to_string(&path) {
        Ok(s) => (s, true),
        Err(_) => ("{}".to_string(), false),
    };
    let original_value: Value = serde_json::from_str(&original_text)
        .map_err(|e| format!("settings.json ist kein gültiges JSON: {e}"))?;

    let patched = apply_context_hook(original_value.clone(), script, enable)?;
    if patched == original_value {
        return Ok(()); // idempotent no-op: nothing to write, no backup churn
    }

    if file_existed {
        // Best-effort, same as OmniRoute: a backup failure warns, never blocks the write.
        if let Some(warning) = rotate_backup(app, "settings", "settings.json", &original_text) {
            log::warn!("Kontext-Hook: {warning}");
        }
    }

    write_settings(&patched)
}

#[tauri::command]
pub fn context_hook_status(app: AppHandle) -> Result<HookStatus, String> {
    let script = script_path(&app)?.to_string_lossy().to_string();
    let settings = read_settings().unwrap_or(Value::Object(serde_json::Map::new()));
    Ok(HookStatus {
        installed: hook_installed(&settings),
        script_path: script,
        settings_path: settings_path()?.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub fn install_context_hook(app: AppHandle) -> Result<(), String> {
    let script = write_script(&app)?.to_string_lossy().to_string();
    apply_hook_setting(&app, &script, true)
}

/// Removes only our settings.json entry. The script file is left in place under app
/// data — it's inert without the hook entry, and re-installing later needs it anyway.
#[tauri::command]
pub fn uninstall_context_hook(app: AppHandle) -> Result<(), String> {
    let script = script_path(&app)?.to_string_lossy().to_string();
    apply_hook_setting(&app, &script, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "C:\\Users\\test\\AppData\\Local\\com.skillswitch.app\\context-hook.ps1";

    #[test]
    fn adds_once_and_is_idempotent() {
        let s = apply_context_hook(serde_json::json!({}), SCRIPT, true).unwrap();
        assert_eq!(s["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 1);
        let s2 = apply_context_hook(s.clone(), SCRIPT, true).unwrap();
        assert_eq!(s2, s, "installing twice must not add a second entry");
        assert_eq!(s2["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn preserves_foreign_hook_and_other_top_level_keys() {
        let settings = serde_json::json!({
            "theme": "dark",
            "hooks": {
                "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "other.exe", "args": ["-x"] }] }]
            }
        });
        let s = apply_context_hook(settings, SCRIPT, true).unwrap();
        assert_eq!(s["theme"], "dark");
        assert_eq!(s["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 2);
        assert_eq!(s["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"], "other.exe");
    }

    #[test]
    fn removes_only_ours() {
        let settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": [
                    { "hooks": [{ "type": "command", "command": "other.exe", "args": ["-x"] }] },
                    { "hooks": [{ "type": "command", "command": "powershell.exe", "args": ["-File", SCRIPT] }] }
                ]
            }
        });
        let s = apply_context_hook(settings, SCRIPT, false).unwrap();
        let ups = s["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(ups.len(), 1, "only the foreign entry should remain");
        assert_eq!(ups[0]["hooks"][0]["command"], "other.exe");
    }

    #[test]
    fn removing_last_entry_cleans_up_empty_containers() {
        let s = apply_context_hook(serde_json::json!({ "theme": "dark" }), SCRIPT, true).unwrap();
        let s2 = apply_context_hook(s, SCRIPT, false).unwrap();
        assert!(s2.get("hooks").is_none(), "empty hooks object should be dropped");
        assert_eq!(s2["theme"], "dark");
    }

    #[test]
    fn matches_script_path_case_insensitively() {
        let settings = serde_json::json!({
            "hooks": { "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "powershell.exe", "args": ["-File", SCRIPT.to_uppercase()] }] }] }
        });
        assert!(hook_installed(&settings));
        let s2 = apply_context_hook(settings, SCRIPT, false).unwrap();
        assert!(s2.get("hooks").is_none());
    }

    #[test]
    fn matches_by_shape_even_at_an_old_install_location() {
        let settings = serde_json::json!({
            "hooks": { "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "powershell.exe", "args": ["-File", "C:\\Old\\Dir\\context-hook.ps1"] }] }] }
        });
        assert!(hook_installed(&settings), "old-location entry should still count as installed");
        // Re-installing at the new SCRIPT path must not add a second entry.
        let s2 = apply_context_hook(settings, SCRIPT, true).unwrap();
        assert_eq!(s2["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 1);
        assert_eq!(s2["hooks"]["UserPromptSubmit"][0]["hooks"][0]["args"][1], "C:\\Old\\Dir\\context-hook.ps1");
    }

    #[test]
    fn does_not_match_a_foreign_script_by_shape() {
        let settings = serde_json::json!({
            "hooks": { "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "powershell.exe", "args": ["-File", "C:\\Some\\Dir\\other.ps1"] }] }] }
        });
        assert!(!hook_installed(&settings));
    }

    #[test]
    fn rejects_non_object_root() {
        assert!(apply_context_hook(serde_json::json!([1]), SCRIPT, true).is_err());
    }
}
