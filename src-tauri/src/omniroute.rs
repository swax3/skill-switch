//! OmniRoute tab backend: route individual, opt-in "throwaway" project folders
//! through a local OmniRoute proxy (http://localhost:20128 by default) instead of
//! Anthropic directly — CLI/terminal only (see module-level note in lib.rs / CLAUDE.md
//! for why Claude Desktop is architecturally out of scope: Desktop's gateway config
//! is global-per-app, not per-project, so it can never satisfy "never global").
//!
//! Two independent ways to route a session, sharing one config store:
//! - Persistent: patches `<project>\.claude\settings.local.json`'s `env` object.
//!   Applies to any session the user starts themselves in a terminal later.
//! - One-off: `open_omniroute_terminal` spawns a new terminal with the two env vars
//!   set only on that one process — nothing is written to disk at all.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};

use crate::{claude_dir, read_json_file};

const DEFAULT_BASE_URL: &str = "http://localhost:20128/v1";
const OMNI_CONFIG_FILE: &str = "omniroute.json";
const MAX_BACKUPS: usize = 3;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Serializes read-modify-write access to omniroute.json and project settings.local.json
/// files. Separate from lib.rs's SETTINGS_LOCK — different files, no reason to serialize
/// skill-toggling and OmniRoute project-toggling against each other.
static OMNI_LOCK: Mutex<()> = Mutex::new(());

/// Tauri-managed handle to a `omniroute` process this app started, so "Stoppen" can
/// find and kill it. `None` means either never started, or already stopped/exited.
pub struct ProxyState(pub Mutex<Option<Child>>);

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OmniConfig {
    base_url: String,
    api_key: String,
    projects: Vec<String>,
}

impl Default for OmniConfig {
    fn default() -> Self {
        OmniConfig {
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: String::new(),
            projects: Vec::new(),
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProjectRoute {
    path: String,
    /// Folder still exists and is a directory.
    exists: bool,
    /// env.ANTHROPIC_BASE_URL is present (and non-empty) in that project's
    /// settings.local.json right now — read fresh from disk, not from client state.
    active: bool,
    /// The base URL actually on disk, so the UI can flag drift from the configured one.
    base_url: Option<String>,
    has_token: bool,
    gitignore_ok: bool,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OmniState {
    base_url: String,
    api_key_set: bool,
    projects: Vec<ProjectRoute>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalRouteWarning {
    settings_json: Option<String>,
    user_env: Option<String>,
    machine_env: Option<String>,
}

// ---------------------------------------------------------------------------
// App-owned config (%LOCALAPPDATA%\<identifier>\omniroute.json) — Local, not
// Roaming, so the API key doesn't sync to a domain profile server.
// ---------------------------------------------------------------------------

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("Konnte lokalen App-Datenordner nicht finden: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("Konnte App-Datenordner nicht anlegen: {e}"))?;
    Ok(dir.join(OMNI_CONFIG_FILE))
}

fn read_config(app: &AppHandle) -> Result<OmniConfig, String> {
    let path = config_path(app)?;
    match fs::read_to_string(&path) {
        // Corrupt file must error, never silently reset — that would drop the
        // user's whole project list without telling them.
        Ok(s) => serde_json::from_str(&s)
            .map_err(|e| format!("omniroute.json ist kein gültiges JSON: {e}")),
        Err(_) => Ok(OmniConfig::default()),
    }
}

fn write_config(app: &AppHandle, cfg: &OmniConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let text = serde_json::to_string_pretty(cfg)
        .map_err(|e| format!("Konnte Konfiguration nicht serialisieren: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("Konnte Konfiguration nicht schreiben: {e}"))
}

// ---------------------------------------------------------------------------
// Path validation — the project folder is arbitrary (anywhere on disk), so
// validate_skill_name's character blocklist doesn't apply (it would reject any
// normal Windows absolute path). Validate structurally instead.
// ---------------------------------------------------------------------------

fn validate_project_path(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Projektpfad ist leer.".to_string());
    }
    let p = PathBuf::from(trimmed);
    if !p.is_absolute() {
        return Err(format!("\"{trimmed}\" ist kein absoluter Pfad."));
    }
    // Reject ".." *segments*, not a substring match — a folder legitimately named
    // "my..folder" must not be rejected.
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("\"{trimmed}\" enthält \"..\" – nicht erlaubt."));
    }
    let meta = fs::metadata(&p)
        .map_err(|_| format!("\"{trimmed}\" existiert nicht oder ist nicht erreichbar."))?;
    if !meta.is_dir() {
        return Err(format!("\"{trimmed}\" ist kein Ordner."));
    }
    Ok(p)
}

/// Dedupe/lookup key only — never persisted or shown.
fn normalize_key(p: &Path) -> String {
    p.to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

fn ensure_claude_dir(project: &Path) -> Result<(), String> {
    let dir = project.join(".claude");
    match fs::metadata(&dir) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err("In diesem Projekt ist \".claude\" eine Datei, kein Ordner.".to_string()),
        Err(_) => fs::create_dir_all(&dir)
            .map_err(|e| format!("Konnte \".claude\" nicht anlegen: {e}")),
    }
}

fn project_settings_path(project: &Path) -> PathBuf {
    project.join(".claude").join("settings.local.json")
}

// ---------------------------------------------------------------------------
// Pure JSON patch (unit-testable, no I/O) — same shape as apply_skill_override.
// ---------------------------------------------------------------------------

fn apply_route(mut settings: Value, route: Option<(&str, &str)>) -> Result<Value, String> {
    let obj = settings
        .as_object_mut()
        .ok_or_else(|| "settings.local.json hat kein Objekt auf oberster Ebene.".to_string())?;
    let env = obj
        .entry("env")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let env_obj = env
        .as_object_mut()
        .ok_or_else(|| "\"env\" in settings.local.json ist kein Objekt.".to_string())?;

    match route {
        Some((base_url, api_key)) => {
            env_obj.insert(
                "ANTHROPIC_BASE_URL".to_string(),
                Value::String(base_url.to_string()),
            );
            env_obj.insert(
                "ANTHROPIC_AUTH_TOKEN".to_string(),
                Value::String(api_key.to_string()),
            );
        }
        None => {
            env_obj.remove("ANTHROPIC_BASE_URL");
            env_obj.remove("ANTHROPIC_AUTH_TOKEN");
        }
    }
    if env_obj.is_empty() {
        obj.remove("env");
    }
    Ok(settings)
}

// ---------------------------------------------------------------------------
// Backups — centralized under the app's own data dir, never in the project
// (no git noise, no unignored plaintext-key file sitting in the repo). Only
// taken when a file already existed; capped at MAX_BACKUPS newest per project.
// ---------------------------------------------------------------------------

fn backup_dir_for(app: &AppHandle, project: &Path) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("Konnte lokalen App-Datenordner nicht finden: {e}"))?;
    let mut hasher = DefaultHasher::new();
    normalize_key(project).hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    let dir = base.join("backups").join(key);
    fs::create_dir_all(&dir).map_err(|e| format!("Konnte Backup-Ordner nicht anlegen: {e}"))?;
    Ok(dir)
}

/// Best-effort: returns a warning message on failure, never blocks the caller.
fn rotate_backup(app: &AppHandle, project: &Path, content: &str) -> Option<String> {
    let dir = match backup_dir_for(app, project) {
        Ok(d) => d,
        Err(e) => return Some(e),
    };
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file = dir.join(format!("settings.local.json.{ts}.bak"));
    if let Err(e) = fs::write(&file, content) {
        return Some(format!("Backup konnte nicht geschrieben werden: {e}"));
    }
    let Ok(read_dir) = fs::read_dir(&dir) else {
        return None;
    };
    let mut entries: Vec<PathBuf> = read_dir.flatten().map(|e| e.path()).collect();
    entries.sort(); // timestamp-prefixed names sort chronologically
    if entries.len() > MAX_BACKUPS {
        for old in &entries[..entries.len() - MAX_BACKUPS] {
            let _ = fs::remove_file(old);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// .gitignore — an offer, never a blocker for toggling the route on.
// ---------------------------------------------------------------------------

fn check_gitignore(project: &Path) -> bool {
    let Ok(content) = fs::read_to_string(project.join(".gitignore")) else {
        return false;
    };
    content.lines().map(str::trim).any(|l| {
        matches!(
            l,
            ".claude/settings.local.json" | "settings.local.json" | "*.local.json" | ".claude/" | ".claude"
        )
    })
}

fn add_gitignore_line(project: &Path) -> Result<(), String> {
    if check_gitignore(project) {
        return Ok(()); // idempotent
    }
    let path = project.join(".gitignore");
    let needs_leading_newline = match fs::read_to_string(&path) {
        Ok(c) => !c.is_empty() && !c.ends_with('\n'),
        Err(_) => false,
    };
    let mut line = String::new();
    if needs_leading_newline {
        line.push('\n');
    }
    line.push_str(".claude/settings.local.json\n");

    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Konnte .gitignore nicht öffnen: {e}"))?;
    f.write_all(line.as_bytes())
        .map_err(|e| format!("Konnte .gitignore nicht schreiben: {e}"))
}

// ---------------------------------------------------------------------------
// Reading on-disk state
// ---------------------------------------------------------------------------

fn read_project_route(raw: &str) -> ProjectRoute {
    let path = PathBuf::from(raw.trim());
    let exists = fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false);
    if !exists {
        return ProjectRoute {
            path: raw.to_string(),
            exists: false,
            active: false,
            base_url: None,
            has_token: false,
            gitignore_ok: false,
            error: None,
        };
    }
    let gitignore_ok = check_gitignore(&path);
    let content = match fs::read_to_string(project_settings_path(&path)) {
        Ok(c) => c,
        Err(_) => {
            return ProjectRoute {
                path: raw.to_string(),
                exists: true,
                active: false,
                base_url: None,
                has_token: false,
                gitignore_ok,
                error: None,
            };
        }
    };
    match serde_json::from_str::<Value>(&content) {
        Ok(v) => {
            let env = v.get("env").and_then(Value::as_object);
            let base_url = env
                .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let has_token = env
                .and_then(|e| e.get("ANTHROPIC_AUTH_TOKEN"))
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty());
            ProjectRoute {
                path: raw.to_string(),
                exists: true,
                active: base_url.is_some(),
                base_url,
                has_token,
                gitignore_ok,
                error: None,
            }
        }
        Err(e) => ProjectRoute {
            path: raw.to_string(),
            exists: true,
            active: false,
            base_url: None,
            has_token: false,
            gitignore_ok,
            error: Some(format!("settings.local.json ist kein gültiges JSON: {e}")),
        },
    }
}

fn extract_host_port(base_url: &str) -> Result<(String, u16), String> {
    let url = url::Url::parse(base_url).map_err(|e| format!("Ungültige Basis-URL: {e}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| "Basis-URL hat keinen Host.".to_string())?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "Basis-URL hat keinen Port.".to_string())?;
    Ok((host, port))
}

fn is_port_reachable(host: &str, port: u16) -> bool {
    let Ok(addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    addrs
        .into_iter()
        .any(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn read_omniroute(app: AppHandle) -> Result<OmniState, String> {
    let cfg = read_config(&app)?;
    let projects = cfg.projects.iter().map(|p| read_project_route(p)).collect();
    Ok(OmniState {
        base_url: cfg.base_url,
        api_key_set: !cfg.api_key.is_empty(),
        projects,
    })
}

#[tauri::command]
pub fn set_omniroute_config(
    app: AppHandle,
    base_url: String,
    api_key: Option<String>,
) -> Result<(), String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("Basis-URL darf nicht leer sein.".to_string());
    }
    let _guard = OMNI_LOCK.lock().map_err(|_| "Interner Sperr-Fehler.".to_string())?;
    let mut cfg = read_config(&app)?;
    cfg.base_url = trimmed.to_string();
    if let Some(key) = api_key {
        if !key.is_empty() {
            cfg.api_key = key;
        }
    }
    write_config(&app, &cfg)
}

#[tauri::command]
pub fn add_omniroute_project(app: AppHandle, path: String) -> Result<(), String> {
    let project = validate_project_path(&path)?;
    let _guard = OMNI_LOCK.lock().map_err(|_| "Interner Sperr-Fehler.".to_string())?;
    let mut cfg = read_config(&app)?;
    let key = normalize_key(&project);
    if cfg.projects.iter().any(|p| normalize_key(Path::new(p)) == key) {
        return Err("Dieses Projekt ist bereits in der Liste.".to_string());
    }
    cfg.projects.push(project.to_string_lossy().to_string());
    write_config(&app, &cfg)
}

#[tauri::command]
pub fn remove_omniroute_project(app: AppHandle, path: String) -> Result<(), String> {
    let _guard = OMNI_LOCK.lock().map_err(|_| "Interner Sperr-Fehler.".to_string())?;
    let mut cfg = read_config(&app)?;
    let key = normalize_key(Path::new(path.trim()));

    if let Some(existing) = cfg
        .projects
        .iter()
        .find(|p| normalize_key(Path::new(p.as_str())) == key)
    {
        if read_project_route(existing).active {
            return Err("Erst deaktivieren, dann aus der Liste entfernen.".to_string());
        }
    }
    cfg.projects
        .retain(|p| normalize_key(Path::new(p.as_str())) != key);
    write_config(&app, &cfg)
}

/// `Ok(None)` = clean success (or true no-op if nothing actually changed).
/// `Ok(Some(warning))` = the route was written, but the backup and/or gitignore
/// check has something the user should see. `Err` = nothing was written.
#[tauri::command]
pub fn set_project_route(app: AppHandle, path: String, active: bool) -> Result<Option<String>, String> {
    let project = validate_project_path(&path)?;

    let cfg = read_config(&app)?;
    if active && cfg.api_key.is_empty() {
        return Err("Kein OmniRoute-API-Key hinterlegt. Erst in den Einstellungen eintragen.".to_string());
    }

    let _guard = OMNI_LOCK.lock().map_err(|_| "Interner Sperr-Fehler.".to_string())?;

    ensure_claude_dir(&project)?;
    let settings_path = project_settings_path(&project);
    let (original_text, file_existed) = match fs::read_to_string(&settings_path) {
        Ok(s) => (s, true),
        Err(_) => ("{}".to_string(), false),
    };
    let original_value: Value = serde_json::from_str(&original_text)
        .map_err(|e| format!("settings.local.json ist kein gültiges JSON: {e}"))?;

    let route = if active {
        Some((cfg.base_url.as_str(), cfg.api_key.as_str()))
    } else {
        None
    };
    let patched = apply_route(original_value.clone(), route)?;

    if patched == original_value {
        return Ok(None); // true no-op: no write, no backup churn
    }

    let mut warning = if file_existed {
        rotate_backup(&app, &project, &original_text)
    } else {
        None
    };

    let text = serde_json::to_string_pretty(&patched)
        .map_err(|e| format!("Konnte settings.local.json nicht serialisieren: {e}"))?;
    fs::write(&settings_path, text)
        .map_err(|e| format!("Konnte settings.local.json nicht schreiben: {e}"))?;

    if active && !check_gitignore(&project) {
        let git_msg = "Diese Datei enthält deinen API-Key im Klartext und ist nicht in der .gitignore erfasst.";
        warning = Some(match warning {
            Some(w) => format!("{w} {git_msg}"),
            None => git_msg.to_string(),
        });
    }

    Ok(warning)
}

#[tauri::command]
pub fn add_gitignore_entry(path: String) -> Result<(), String> {
    let project = validate_project_path(&path)?;
    add_gitignore_line(&project)
}

#[tauri::command(async)]
pub fn omniroute_status(app: AppHandle) -> Result<bool, String> {
    let cfg = read_config(&app)?;
    let (host, port) = extract_host_port(&cfg.base_url)?;
    Ok(is_port_reachable(&host, port))
}

#[tauri::command(async)]
pub fn start_omniroute(app: AppHandle, state: State<ProxyState>) -> Result<(), String> {
    let cfg = read_config(&app)?;
    let (host, port) = extract_host_port(&cfg.base_url)?;

    if is_port_reachable(&host, port) {
        return Err(format!("Auf Port {port} läuft bereits ein Dienst."));
    }

    let found_on_path = Command::new("where")
        .arg("omniroute")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !found_on_path {
        return Err(
            "\"omniroute\" wurde nicht gefunden. Ist es global installiert (npm install -g omniroute) und liegt es im PATH?".to_string(),
        );
    }

    // omniroute is an npm-global .cmd shim; CreateProcess doesn't apply PATHEXT,
    // so it must be launched through cmd.exe. Stdio piped to null so an unread
    // pipe never fills and hangs the child.
    let child = Command::new("cmd")
        .args(["/C", "omniroute"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Konnte omniroute nicht starten: {e}"))?;

    let mut guard = state.0.lock().map_err(|_| "Interner Sperr-Fehler.".to_string())?;
    *guard = Some(child);
    Ok(())
}

#[tauri::command(async)]
pub fn stop_omniroute(state: State<ProxyState>) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|_| "Interner Sperr-Fehler.".to_string())?;
    let Some(mut child) = guard.take() else {
        return Err(
            "Kein von Skill Switch gestarteter Dienst bekannt. Falls trotzdem etwas auf dem Port läuft, im Terminal beenden.".to_string(),
        );
    };
    if matches!(child.try_wait(), Ok(Some(_))) {
        return Ok(()); // already exited on its own
    }
    // Kill the whole tree: the Child handle is cmd.exe's, and killing only that
    // would orphan the node.exe grandchild it launched.
    let pid = child.id();
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = child.wait();
    Ok(())
}

/// Opens a new terminal in `path` with the OmniRoute env vars set only for that
/// one process, then runs `claude`. Nothing is written to disk — this is the
/// route for Desktop-or-terminal use without touching any settings file.
#[tauri::command(async)]
pub fn open_omniroute_terminal(app: AppHandle, path: String) -> Result<(), String> {
    let project = validate_project_path(&path)?;
    let cfg = read_config(&app)?;
    if cfg.api_key.is_empty() {
        return Err("Kein OmniRoute-API-Key hinterlegt. Erst in den Einstellungen eintragen.".to_string());
    }

    let has_wt = Command::new("where")
        .arg("wt")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if has_wt {
        Command::new("wt")
            .env("ANTHROPIC_BASE_URL", &cfg.base_url)
            .env("ANTHROPIC_AUTH_TOKEN", &cfg.api_key)
            .args(["-d", &project.to_string_lossy(), "cmd", "/k", "claude"])
            .spawn()
            .map_err(|e| format!("Konnte Windows Terminal nicht starten: {e}"))?;
    } else {
        Command::new("cmd")
            .env("ANTHROPIC_BASE_URL", &cfg.base_url)
            .env("ANTHROPIC_AUTH_TOKEN", &cfg.api_key)
            .args(["/K", "claude"])
            .current_dir(&project)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map_err(|e| format!("Konnte Terminal nicht starten: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn check_global_route() -> Result<GlobalRouteWarning, String> {
    let settings_json = read_json_file(&claude_dir()?.join("settings.json")).and_then(|v| {
        v.get("env")
            .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });

    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let user_env = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .ok()
        .and_then(|k| k.get_value::<String, _>("ANTHROPIC_BASE_URL").ok())
        .filter(|s| !s.is_empty());

    let machine_env = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
        .ok()
        .and_then(|k| k.get_value::<String, _>("ANTHROPIC_BASE_URL").ok())
        .filter(|s| !s.is_empty());

    Ok(GlobalRouteWarning {
        settings_json,
        user_env,
        machine_env,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_sets_and_clears_both_keys_without_touching_siblings() {
        let settings = serde_json::json!({ "otherSetting": true, "env": { "OTHER_VAR": "keep-me" } });
        let s2 = apply_route(settings, Some(("http://localhost:20128/v1", "sk-test"))).unwrap();
        assert_eq!(s2["env"]["ANTHROPIC_BASE_URL"], "http://localhost:20128/v1");
        assert_eq!(s2["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-test");
        assert_eq!(s2["env"]["OTHER_VAR"], "keep-me");
        assert_eq!(s2["otherSetting"], true);

        let s3 = apply_route(s2, None).unwrap();
        assert!(s3["env"].get("ANTHROPIC_BASE_URL").is_none());
        assert!(s3["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
        assert_eq!(s3["env"]["OTHER_VAR"], "keep-me", "unrelated env var must survive disabling the route");
    }

    #[test]
    fn route_drops_env_object_when_it_becomes_empty() {
        let settings = serde_json::json!({});
        let s2 = apply_route(settings, Some(("http://x", "k"))).unwrap();
        assert!(s2.get("env").is_some());
        let s3 = apply_route(s2, None).unwrap();
        assert!(s3.get("env").is_none(), "empty env object should be dropped");
    }

    #[test]
    fn route_rejects_non_object_root_and_non_object_env() {
        assert!(apply_route(serde_json::json!([1]), Some(("u", "k"))).is_err());
        assert!(apply_route(serde_json::json!({ "env": "not-an-object" }), Some(("u", "k"))).is_err());
    }

    #[test]
    fn project_path_rejects_relative_and_traversal_but_accepts_dotdot_in_a_name() {
        assert!(validate_project_path("").is_err());
        assert!(validate_project_path("relative\\path").is_err());
        assert!(validate_project_path("C:foo").is_err());
        assert!(validate_project_path("C:\\..\\evil").is_err());
        // A folder literally named "my..folder" must NOT be rejected by a naive
        // substring check on "..". It doesn't exist on disk in this test, so it
        // still errors — but via the "does not exist" path, not path validation.
        match validate_project_path("C:\\Users\\nobody\\my..folder") {
            Err(msg) => assert!(msg.contains("existiert nicht"), "should fail on existence, not on \"..\": {msg}"),
            Ok(_) => panic!("should not exist"),
        }
    }

    #[test]
    fn normalize_key_collapses_trailing_separators_and_case() {
        assert_eq!(normalize_key(Path::new("C:\\Foo\\Bar\\")), normalize_key(Path::new("c:\\foo\\bar")));
    }

    #[test]
    fn extract_host_port_reads_configured_port() {
        assert_eq!(extract_host_port("http://localhost:20128/v1").unwrap(), ("localhost".to_string(), 20128));
        assert_eq!(extract_host_port("http://localhost:20128").unwrap(), ("localhost".to_string(), 20128));
        assert!(extract_host_port("not a url").is_err());
    }

    #[test]
    fn gitignore_check_recognizes_common_patterns() {
        // Pure function over lines(), no filesystem — using check_gitignore directly
        // would need a real file, so this exercises the same matching logic inline.
        let matches = |line: &str| {
            matches!(
                line.trim(),
                ".claude/settings.local.json" | "settings.local.json" | "*.local.json" | ".claude/" | ".claude"
            )
        };
        assert!(matches(".claude/settings.local.json"));
        assert!(matches("*.local.json"));
        assert!(!matches("node_modules/"));
    }
}
