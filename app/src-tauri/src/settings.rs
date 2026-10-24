// Persists everything on the setup screen except the API key (which lives in
// OS secure storage — see secrets.rs, never here) and the transient
// in-session hint feed (which the brief §6 requires to reset on close).
//
// Plain JSON in the app data dir is appropriate here specifically because
// none of these fields are secrets — unlike the API key, there's no reason
// to route them through Keychain/Credential Manager.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub provider: String,
    pub model: String,
    pub interview_context: String,
    /// Company card name (profiles/companies/<name>.md); empty = none.
    pub company: String,
    /// Knowledge profile (profiles/<name>/bot/); empty = first one on disk.
    pub profile: String,
    pub stt_language: String,
    pub capture_exclusion: bool,
    /// UI-wide text scale (1.0 = 100%, the CSS root font-size baseline),
    /// adjusted via Cmd/Ctrl +/-/0 rather than a settings-screen control —
    /// see docs/PROMPT-FONT-SIZE-OPTION.md for why. Persisted so a user who
    /// needed a larger size once doesn't have to redo it every launch.
    pub zoom_level: f64,
    /// Result of the last explicit "Check / enable" click in the Permissions
    /// section — `None` means never checked yet. There's no live preflight
    /// API for this TCC entry (see `audio::check_audio_permission`), so
    /// unlike Screen Recording this can't be silently re-verified on every
    /// launch; persisting the last known result at least avoids the setup
    /// screen looking like it "forgot" a permission that was actually
    /// granted last time (docs/TODO.md, found live 2026-07-30). The UI
    /// labels this as "last checked", since it can go stale if revoked in
    /// System Settings since.
    pub audio_permission_granted: Option<bool>,
    /// Set true the first time the user successfully enters listening mode —
    /// gates the "Send feedback" link on the setup screen, which stays
    /// hidden until there's been at least one real session to have an
    /// opinion about. Never reset back to false.
    pub has_used_app: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: "openai".to_string(),
            model: String::new(),
            interview_context: String::new(),
            company: String::new(),
            profile: String::new(),
            stt_language: "en".to_string(),
            capture_exclusion: true,
            zoom_level: 1.0,
            audio_permission_granted: None,
            has_used_app: false,
        }
    }
}

fn settings_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve the app data directory: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}

/// Reads saved settings from disk, using `resource` layer of defaults for a
/// first run or a corrupted file — a broken settings file should degrade to
/// "starts fresh," never block the app from opening at all.
#[tauri::command]
pub fn load_settings(app: AppHandle) -> Settings {
    let Ok(path) = settings_path(&app) else {
        return Settings::default();
    };
    let Ok(data) = std::fs::read_to_string(&path) else {
        return Settings::default();
    };
    serde_json::from_str(&data).unwrap_or_else(|e| {
        eprintln!("settings.json is corrupted, starting fresh: {e}");
        Settings::default()
    })
}

#[tauri::command]
pub fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let path = settings_path(&app)?;
    let data = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, data).map_err(|e| e.to_string())
}
