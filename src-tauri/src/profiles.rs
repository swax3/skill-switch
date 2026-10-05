//! Profiles tab backend: named snapshots of skill states (`{ name, skills: { "<skill
//! folder name>": "active" | "manual" | "disabled" } }`) so a whole set of skill
//! states can be applied in one click. Same app-data config-store pattern as
//! omniroute.rs (`config_path`/`read_config`/`write_config`, own lock, own file).
//!
//! This module only stores and validates profiles — it never touches a skill
//! folder or settings.json itself. Applying a profile is done skill-by-skill from
//! the frontend via the existing `set_skill_enabled`/`set_skill_manual_only`
//! commands (see App.tsx's `applyState`), so profiles never drift from how a
//! single skill's state is actually changed.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

const PROFILES_FILE: &str = "profiles.json";
const VALID_STATES: [&str; 3] = ["active", "manual", "disabled"];

/// Serializes read-modify-write access to profiles.json. Own mutex, independent of
/// SETTINGS_LOCK/OMNI_LOCK — a different file, no reason to serialize against those.
static PROFILES_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    name: String,
    /// BTreeMap so the file's key order is stable/diffable across writes.
    skills: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct ProfilesConfig {
    profiles: Vec<Profile>,
}

// ---------------------------------------------------------------------------
// App-owned config (%LOCALAPPDATA%\<identifier>\profiles.json) — same shape as
// omniroute.rs's config store.
// ---------------------------------------------------------------------------

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("Konnte lokalen App-Datenordner nicht finden: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("Konnte App-Datenordner nicht anlegen: {e}"))?;
    Ok(dir.join(PROFILES_FILE))
}

fn read_config(app: &AppHandle) -> Result<ProfilesConfig, String> {
    let path = config_path(app)?;
    match fs::read_to_string(&path) {
        // Corrupt file must error, never silently reset — that would drop the
        // user's whole profile list without telling them (same reasoning as
        // omniroute.rs's read_config).
        Ok(s) => serde_json::from_str(&s)
            .map_err(|e| format!("profiles.json ist kein gültiges JSON: {e}")),
        Err(_) => Ok(ProfilesConfig::default()),
    }
}

fn write_config(app: &AppHandle, cfg: &ProfilesConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let text = serde_json::to_string_pretty(cfg)
        .map_err(|e| format!("Konnte Profile nicht serialisieren: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("Konnte Profile nicht schreiben: {e}"))
}

// ---------------------------------------------------------------------------
// Pure logic (unit-testable, no I/O)
// ---------------------------------------------------------------------------

/// Inserts or replaces a profile. Trims the name, rejects an empty name or an
/// empty skill map, rejects any state other than the three valid ones, and
/// replaces an existing profile with the same name case-insensitively in place
/// (keeps its position; a new name is appended).
fn upsert(mut profiles: Vec<Profile>, name: &str, skills: BTreeMap<String, String>) -> Result<Vec<Profile>, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Profilname darf nicht leer sein.".to_string());
    }
    if skills.is_empty() {
        return Err("Profil enthält keine Skills.".to_string());
    }
    if let Some((skill, state)) = skills.iter().find(|(_, v)| !VALID_STATES.contains(&v.as_str())) {
        return Err(format!("Ungültiger Zustand \"{state}\" für Skill \"{skill}\"."));
    }

    let profile = Profile { name: trimmed.to_string(), skills };
    match profiles.iter().position(|p| p.name.eq_ignore_ascii_case(trimmed)) {
        Some(pos) => profiles[pos] = profile,
        None => profiles.push(profile),
    }
    Ok(profiles)
}

/// Case-insensitive; a no-op if no profile with that name exists.
fn remove(mut profiles: Vec<Profile>, name: &str) -> Vec<Profile> {
    let trimmed = name.trim();
    profiles.retain(|p| !p.name.eq_ignore_ascii_case(trimmed));
    profiles
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_profiles(app: AppHandle) -> Result<Vec<Profile>, String> {
    Ok(read_config(&app)?.profiles)
}

#[tauri::command]
pub fn save_profile(app: AppHandle, name: String, skills: HashMap<String, String>) -> Result<Vec<Profile>, String> {
    let _guard = PROFILES_LOCK.lock().map_err(|_| "Interner Sperr-Fehler.".to_string())?;
    let cfg = read_config(&app)?;
    let profiles = upsert(cfg.profiles, &name, skills.into_iter().collect())?;
    write_config(&app, &ProfilesConfig { profiles: profiles.clone() })?;
    Ok(profiles)
}

#[tauri::command]
pub fn delete_profile(app: AppHandle, name: String) -> Result<Vec<Profile>, String> {
    let _guard = PROFILES_LOCK.lock().map_err(|_| "Interner Sperr-Fehler.".to_string())?;
    let cfg = read_config(&app)?;
    let profiles = remove(cfg.profiles, &name);
    write_config(&app, &ProfilesConfig { profiles: profiles.clone() })?;
    Ok(profiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skills(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn upsert_adds_new_profile() {
        let profiles = upsert(Vec::new(), "Website-Projekt", skills(&[("humanizer", "active")])).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Website-Projekt");
        assert_eq!(profiles[0].skills["humanizer"], "active");
    }

    #[test]
    fn upsert_trims_name() {
        let profiles = upsert(Vec::new(), "  Spaces  ", skills(&[("a", "active")])).unwrap();
        assert_eq!(profiles[0].name, "Spaces");
    }

    #[test]
    fn upsert_rejects_empty_name() {
        assert!(upsert(Vec::new(), "   ", skills(&[("a", "active")])).is_err());
    }

    #[test]
    fn upsert_rejects_empty_skill_map() {
        assert!(upsert(Vec::new(), "Leer", BTreeMap::new()).is_err());
    }

    #[test]
    fn upsert_rejects_invalid_state() {
        assert!(upsert(Vec::new(), "Foo", skills(&[("a", "off")])).is_err());
    }

    #[test]
    fn upsert_replaces_existing_case_insensitively_and_keeps_position() {
        let profiles = upsert(Vec::new(), "Website-Projekt", skills(&[("a", "active")])).unwrap();
        let profiles = upsert(profiles, "Minimal", skills(&[("b", "disabled")])).unwrap();
        let profiles = upsert(profiles, "WEBSITE-projekt", skills(&[("a", "manual")])).unwrap();

        assert_eq!(profiles.len(), 2, "same name (any case) must replace, not add a third profile");
        assert_eq!(profiles[0].name, "WEBSITE-projekt", "position is kept, only the entry is replaced");
        assert_eq!(profiles[0].skills["a"], "manual");
        assert_eq!(profiles[1].name, "Minimal");
    }

    #[test]
    fn remove_is_case_insensitive_and_noop_if_absent() {
        let profiles = upsert(Vec::new(), "Website-Projekt", skills(&[("a", "active")])).unwrap();
        let profiles = remove(profiles, "website-projekt");
        assert!(profiles.is_empty());

        let profiles = upsert(Vec::new(), "Website-Projekt", skills(&[("a", "active")])).unwrap();
        let profiles = remove(profiles, "Nicht vorhanden");
        assert_eq!(profiles.len(), 1, "removing a name that isn't there must be a no-op");
    }
}
