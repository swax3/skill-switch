mod omniroute;
mod profiles;
mod usage;

use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// Serializes read-modify-write access to settings.json across concurrent commands
/// (e.g. two skills toggled to "manual" within the same moment would otherwise race).
static SETTINGS_LOCK: Mutex<()> = Mutex::new(());

// camelCase so struct fields line up with the TypeScript types without a manual mapper.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Skill {
    name: String,
    description: String,
    skill_md_path: Option<String>,
    enabled: bool,
    /// true if `skillOverrides[name] == "user-invocable-only"` in settings.json —
    /// only meaningful while `enabled` (Claude Code only applies overrides to skills
    /// it can actually see under ~/.claude/skills).
    manual_only: bool,
    linked: bool,
    /// "owner/repo" this skill was installed from, if it's tracked in a
    /// `~/.agents/.skill-lock.json` (used by some skill-registry tools). None for
    /// skills placed by hand or via a tool that doesn't write that lock file.
    source: Option<String>,
    /// Clickable https URL for `source` (the lock file's `.git` remote, `.git` stripped).
    source_url: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct McpServerInfo {
    name: String,
    scope: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PluginInfo {
    id: String,
    version: Option<String>,
    enabled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvInfo {
    mcp_servers: Vec<McpServerInfo>,
    plugins: Vec<PluginInfo>,
}

pub(crate) fn claude_dir() -> Result<PathBuf, String> {
    let profile = std::env::var("USERPROFILE")
        .map_err(|_| "USERPROFILE ist nicht gesetzt.".to_string())?;
    Ok(PathBuf::from(profile).join(".claude"))
}

/// Extract the description shown for a skill from a SKILL.md's content.
/// Prefers YAML frontmatter's `description:` key (plain or block-scalar `|`/`>`);
/// falls back to the first non-empty, non-heading line of the body.
fn parse_description(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();

    let mut body_start = 0;
    if lines.first().map(|l| l.trim()) == Some("---") {
        body_start = 1; // malformed/unclosed frontmatter: at least skip the opening "---"
        if let Some(end) = lines.iter().skip(1).position(|l| l.trim() == "---") {
            let end = end + 1; // index in `lines`, relative to skip(1)
            for (i, line) in lines.iter().enumerate().take(end).skip(1) {
                let trimmed = line.trim_start();
                if let Some(rest) = trimmed.strip_prefix("description:") {
                    let rest = rest.trim();
                    if rest.is_empty() || matches!(rest, "|" | ">" | "|-" | ">-" | "|+" | ">+") {
                        // Block scalar: first non-empty indented line after this one.
                        if let Some(block_line) =
                            lines[i + 1..end].iter().find(|l| !l.trim().is_empty())
                        {
                            return block_line.trim().to_string();
                        }
                    } else {
                        return strip_quotes(rest).to_string();
                    }
                }
            }
            body_start = end + 1;
        }
    }

    match lines[body_start..]
        .iter()
        .find(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
    {
        Some(line) => line.trim().to_string(),
        None => "(keine Beschreibung)".to_string(),
    }
}

fn strip_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Best-effort lookup of which GitHub repo each skill was installed from, read from
/// `~/.agents/.skill-lock.json` (the lock file written by skill-registry tools that
/// manage `~/.agents/skills` — the junction target for most of a user's skills).
/// Returns an empty map if that file doesn't exist; that's the normal case for
/// skills installed by hand, and the app just shows them with no known source.
fn read_skill_lock() -> std::collections::HashMap<String, (String, String)> {
    let mut map = std::collections::HashMap::new();
    let Ok(profile) = std::env::var("USERPROFILE") else {
        return map;
    };
    let path = PathBuf::from(profile).join(".agents").join(".skill-lock.json");
    let Some(lock) = read_json_file(&path) else {
        return map;
    };
    let Some(entries) = lock.get("skills").and_then(Value::as_object) else {
        return map;
    };
    for (key, entry) in entries {
        // Lock keys can use "plugin::skill"; the on-disk folder name is "plugin-skill".
        let folder_name = key.replace("::", "-");
        let source = entry.get("source").and_then(Value::as_str).unwrap_or("");
        if source.is_empty() {
            continue;
        }
        let source_url = entry
            .get("sourceUrl")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_end_matches(".git")
            .to_string();
        map.insert(folder_name, (source.to_string(), source_url));
    }
    map
}

fn read_skills_in(
    dir: &Path,
    enabled: bool,
    overrides: &serde_json::Map<String, Value>,
    sources: &std::collections::HashMap<String, (String, String)>,
) -> Vec<Skill> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        // DirEntry::file_type() reports reparse points (junctions/symlinks) as neither
        // dir nor file on Windows, even when they point at a directory. Resolve those
        // explicitly instead of silently dropping every linked skill from the list.
        let is_dir = file_type.is_dir() || (file_type.is_symlink() && path.is_dir());
        if !is_dir {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let linked = fs::symlink_metadata(&path)
            .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(false);
        let manual_only =
            enabled && overrides.get(&name).and_then(Value::as_str) == Some("user-invocable-only");
        let (source, source_url) = match sources.get(&name) {
            Some((s, u)) => (Some(s.clone()), Some(u.clone())),
            None => (None, None),
        };

        let skill_md = path.join("SKILL.md");
        let (description, skill_md_path) = match fs::read_to_string(&skill_md) {
            Ok(content) => (parse_description(&content), Some(skill_md.to_string_lossy().to_string())),
            Err(_) => ("(keine SKILL.md gefunden)".to_string(), None),
        };

        skills.push(Skill {
            name,
            description,
            skill_md_path,
            enabled,
            manual_only,
            linked,
            source,
            source_url,
        });
    }
    skills
}

#[tauri::command]
fn list_skills() -> Result<Vec<Skill>, String> {
    let base = claude_dir()?;
    let disabled_dir = base.join("skills-disabled");
    fs::create_dir_all(&disabled_dir)
        .map_err(|e| format!("Konnte \"skills-disabled\" nicht anlegen: {e}"))?;

    // Best-effort: a missing/corrupt settings.json just means no overrides are shown,
    // it doesn't stop the skill list from loading.
    let overrides = read_settings()
        .ok()
        .and_then(|v| v.get("skillOverrides").cloned())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let sources = read_skill_lock();

    let mut skills = read_skills_in(&base.join("skills"), true, &overrides, &sources);
    skills.extend(read_skills_in(&disabled_dir, false, &overrides, &sources));
    skills.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(skills)
}

fn validate_skill_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains(':')
        || name.contains("..")
    {
        return Err(format!("Ungültiger Skill-Name: \"{name}\"."));
    }
    Ok(())
}

fn translate_move_error(e: &io::Error, name: &str) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => {
            format!("Keine Rechte, um \"{name}\" zu verschieben.")
        }
        _ if e.raw_os_error() == Some(32) => {
            format!("\"{name}\" ist gerade in Benutzung (Editor/Explorer geöffnet?).")
        }
        _ => format!("Verschieben von \"{name}\" fehlgeschlagen: {e}"),
    }
}

#[tauri::command]
fn set_skill_enabled(name: String, enabled: bool) -> Result<(), String> {
    validate_skill_name(&name)?;
    let base = claude_dir()?;
    let skills_dir = base.join("skills");
    let disabled_dir = base.join("skills-disabled");
    fs::create_dir_all(&disabled_dir)
        .map_err(|e| format!("Konnte \"skills-disabled\" nicht anlegen: {e}"))?;

    let (src_dir, dst_dir) = if enabled {
        (&disabled_dir, &skills_dir)
    } else {
        (&skills_dir, &disabled_dir)
    };
    let src = src_dir.join(&name);
    let dst = dst_dir.join(&name);

    // Defense in depth: the validated name must not have escaped its parent.
    if src.parent() != Some(src_dir.as_path()) || dst.parent() != Some(dst_dir.as_path()) {
        return Err(format!("Ungültiger Skill-Name: \"{name}\"."));
    }

    if fs::symlink_metadata(&src).is_err() {
        return Err(format!("\"{name}\" wurde nicht gefunden. Vielleicht wurde es bereits verschoben."));
    }
    if fs::symlink_metadata(&dst).is_ok() {
        return Err(format!(
            "Im Zielordner existiert bereits \"{name}\". Abgebrochen, damit nichts überschrieben wird."
        ));
    }

    fs::rename(&src, &dst).map_err(|e| translate_move_error(&e, &name))?;

    if !enabled {
        // The skill no longer exists under ~/.claude/skills, so any override for it is
        // inert. Best-effort tidy-up: don't fail the (already successful) move over it.
        let _ = set_skill_override(&name, None);
    }
    Ok(())
}

pub(crate) fn read_json_file(path: &Path) -> Option<Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(claude_dir()?.join("settings.json"))
}

fn read_settings() -> Result<Value, String> {
    let path = settings_path()?;
    match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s)
            .map_err(|e| format!("settings.json ist kein gültiges JSON: {e}")),
        Err(_) => Ok(Value::Object(serde_json::Map::new())), // missing file: start fresh
    }
}

fn write_settings(value: &Value) -> Result<(), String> {
    let path = settings_path()?;
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| format!("Konnte settings.json nicht serialisieren: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("Konnte settings.json nicht schreiben: {e}"))
}

/// Pure JSON patch: set or remove `skillOverrides[name]`, leaving every other key
/// untouched, and dropping the (now-empty) `skillOverrides` object if it was the last entry.
fn apply_skill_override(mut settings: Value, name: &str, value: Option<&str>) -> Result<Value, String> {
    let obj = settings
        .as_object_mut()
        .ok_or_else(|| "settings.json hat kein Objekt auf oberster Ebene.".to_string())?;
    let overrides = obj
        .entry("skillOverrides")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let overrides_obj = overrides
        .as_object_mut()
        .ok_or_else(|| "\"skillOverrides\" in settings.json ist kein Objekt.".to_string())?;

    match value {
        Some(v) => {
            overrides_obj.insert(name.to_string(), Value::String(v.to_string()));
        }
        None => {
            overrides_obj.remove(name);
        }
    }
    if overrides_obj.is_empty() {
        obj.remove("skillOverrides");
    }
    Ok(settings)
}

fn set_skill_override(name: &str, value: Option<&str>) -> Result<(), String> {
    let _guard = SETTINGS_LOCK
        .lock()
        .map_err(|_| "Interner Sperr-Fehler.".to_string())?;
    let settings = read_settings()?;
    let settings = apply_skill_override(settings, name, value)?;
    write_settings(&settings)
}

#[tauri::command]
fn set_skill_manual_only(name: String, manual_only: bool) -> Result<(), String> {
    validate_skill_name(&name)?;
    let skill_path = claude_dir()?.join("skills").join(&name);
    if fs::symlink_metadata(&skill_path).is_err() {
        return Err(format!(
            "\"{name}\" ist nicht aktiv. \"Nur manuell\" gilt nur für aktive Skills."
        ));
    }
    set_skill_override(&name, manual_only.then_some("user-invocable-only"))
}

#[tauri::command]
fn read_env() -> Result<EnvInfo, String> {
    let profile =
        std::env::var("USERPROFILE").map_err(|_| "USERPROFILE ist nicht gesetzt.".to_string())?;
    let profile = PathBuf::from(profile);
    let base = claude_dir()?;

    let mut mcp_servers = Vec::new();
    if let Some(config) = read_json_file(&profile.join(".claude.json")) {
        if let Some(global) = config.get("mcpServers").and_then(Value::as_object) {
            for name in global.keys() {
                mcp_servers.push(McpServerInfo {
                    name: name.clone(),
                    scope: "Global".to_string(),
                });
            }
        }
        if let Some(projects) = config.get("projects").and_then(Value::as_object) {
            for (project_path, project) in projects {
                if let Some(servers) = project.get("mcpServers").and_then(Value::as_object) {
                    for name in servers.keys() {
                        mcp_servers.push(McpServerInfo {
                            name: name.clone(),
                            scope: project_path.clone(),
                        });
                    }
                }
            }
        }
    }

    let enabled_plugins = read_json_file(&base.join("settings.json"))
        .and_then(|v| v.get("enabledPlugins").cloned())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    let mut plugins = Vec::new();
    if let Some(installed) = read_json_file(&base.join("plugins").join("installed_plugins.json"))
    {
        if let Some(map) = installed.get("plugins").and_then(Value::as_object) {
            for (id, records) in map {
                let version = records
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|r| r.get("version"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let enabled = enabled_plugins
                    .get(id)
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                plugins.push(PluginInfo {
                    id: id.clone(),
                    version,
                    enabled,
                });
            }
        }
    }
    plugins.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));

    Ok(EnvInfo {
        mcp_servers,
        plugins,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_shell::init())
        .manage(omniroute::ProxyState(Mutex::new(None)))
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_skills,
            set_skill_enabled,
            set_skill_manual_only,
            read_env,
            omniroute::read_omniroute,
            omniroute::set_omniroute_config,
            omniroute::add_omniroute_project,
            omniroute::remove_omniroute_project,
            omniroute::set_project_route,
            omniroute::set_gateway_discovery,
            omniroute::add_gitignore_entry,
            omniroute::omniroute_status,
            omniroute::start_omniroute,
            omniroute::stop_omniroute,
            omniroute::open_omniroute_terminal,
            omniroute::check_global_route,
            omniroute::test_omniroute_connection,
            usage::analyze_usage,
            profiles::list_profiles,
            profiles::save_profile,
            profiles::delete_profile
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_plain_frontmatter() {
        let content = "---\nname: foo\ndescription: Ein Test-Skill.\n---\n\nBody text.\n";
        assert_eq!(parse_description(content), "Ein Test-Skill.");
    }

    #[test]
    fn description_quoted_frontmatter() {
        let content = "---\ndescription: \"Ein Test-Skill.\"\n---\nBody\n";
        assert_eq!(parse_description(content), "Ein Test-Skill.");
    }

    #[test]
    fn description_block_scalar_pipe() {
        let content = "---\nname: humanizer\ndescription: |\n  Rewrite AI-sounding text so it reads naturally.\n  Second line.\nlicense: MIT\n---\nBody\n";
        assert_eq!(
            parse_description(content),
            "Rewrite AI-sounding text so it reads naturally."
        );
    }

    #[test]
    fn description_block_scalar_folded() {
        let content = "---\ndescription: >\n  Folded description here.\n---\nBody\n";
        assert_eq!(parse_description(content), "Folded description here.");
    }

    #[test]
    fn description_no_frontmatter_uses_body() {
        let content = "# Heading\n\nFirst real line of text.\nMore text.\n";
        assert_eq!(parse_description(content), "First real line of text.");
    }

    #[test]
    fn description_frontmatter_without_key_falls_back_to_body() {
        let content = "---\nname: foo\n---\n# Heading\nActual first line.\n";
        assert_eq!(parse_description(content), "Actual first line.");
    }

    #[test]
    fn skill_name_validation_rejects_traversal() {
        assert!(validate_skill_name("../evil").is_err());
        assert!(validate_skill_name("a/b").is_err());
        assert!(validate_skill_name("a\\b").is_err());
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("normal-skill").is_ok());
    }

    #[test]
    fn override_sets_and_clears_key_without_touching_siblings() {
        let settings = serde_json::json!({ "theme": "dark" });
        let s2 = apply_skill_override(settings, "foo", Some("user-invocable-only")).unwrap();
        assert_eq!(s2["skillOverrides"]["foo"], "user-invocable-only");
        assert_eq!(s2["theme"], "dark");

        let s3 = apply_skill_override(s2, "foo", None).unwrap();
        assert!(s3.get("skillOverrides").is_none(), "empty skillOverrides should be dropped");
        assert_eq!(s3["theme"], "dark");
    }

    #[test]
    fn override_preserves_other_skill_overrides() {
        let settings = serde_json::json!({ "skillOverrides": { "bar": "off" } });
        let s2 = apply_skill_override(settings, "foo", Some("user-invocable-only")).unwrap();
        assert_eq!(s2["skillOverrides"]["foo"], "user-invocable-only");
        assert_eq!(s2["skillOverrides"]["bar"], "off");

        let s3 = apply_skill_override(s2, "foo", None).unwrap();
        assert!(s3["skillOverrides"].get("foo").is_none());
        assert_eq!(s3["skillOverrides"]["bar"], "off", "removing one entry must not drop siblings");
    }

    #[test]
    fn override_rejects_non_object_settings_root() {
        assert!(apply_skill_override(serde_json::json!([1, 2]), "foo", Some("off")).is_err());
    }
}
