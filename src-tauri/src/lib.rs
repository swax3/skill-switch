use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

// camelCase so struct fields line up with the TypeScript types without a manual mapper.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Skill {
    name: String,
    description: String,
    skill_md_path: Option<String>,
    enabled: bool,
    linked: bool,
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

fn claude_dir() -> Result<PathBuf, String> {
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

fn read_skills_in(dir: &Path, enabled: bool) -> Vec<Skill> {
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
            linked,
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

    let mut skills = read_skills_in(&base.join("skills"), true);
    skills.extend(read_skills_in(&disabled_dir, false));
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

    fs::rename(&src, &dst).map_err(|e| translate_move_error(&e, &name))
}

fn read_json_file(path: &Path) -> Option<Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
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
        .invoke_handler(tauri::generate_handler![list_skills, set_skill_enabled, read_env])
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
}
