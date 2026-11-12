//! Settings and shared paths.
//!
//! A single [`Settings`] struct is the source of truth. Readers take a cheap snapshot with
//! [`get`]; writers go through [`update`], which clamps the values and saves the file.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

pub const DEFAULT_BASE_URL: &str = "https://api.sailresearch.com/v1";
pub const DEFAULT_MODEL: &str = "deepseek-ai/DeepSeek-V4.1-Flash-1017";

pub const AVAILABLE_MODELS: &[(&str, &str)] = &[
    ("deepseek-ai/DeepSeek-V4.1-Flash-1017", "DeepSeek V4.1 Flash — fastest"),
    ("zai-org/GLM-5.3", "Sail GLM-5.3"),
    ("deepseek-ai/DeepSeek-V3", "DeepSeek V3"),
    ("deepseek-ai/DeepSeek-R1", "DeepSeek R1 — deepest reasoning"),
];

/// How hard the model thinks before answering (`reasoning_effort`). `auto` sends nothing and
/// leaves it to the model's default.
pub const REASONING_LEVELS: &[(&str, &str)] = &[
    ("auto", "Auto — the model's default"),
    ("none", "Off — answer straight away"),
    ("low", "Low — quick thinking"),
    ("medium", "Medium — balanced"),
    ("high", "High — think it through"),
];

pub const MIC_MODES: &[(&str, &str)] = &[
    ("always_open", "Always-Open Microphone — continuous hands-free VAD"),
    ("push_to_talk", "Push-to-Talk — hold Ctrl+Shift+Space while speaking"),
    ("click", "Click-to-Talk — click to start and stop"),
];

pub const BODY_FRAMINGS: &[(&str, &str)] = &[
    ("full", "Full body — head to toe"),
    ("half", "Half body — waist up"),
    ("face", "Face and portrait — close-up"),
];

pub const STAGE_LIGHTING: &[(&str, &str)] = &[
    ("auto", "Auto — synced to time of day"),
    ("day", "Crisp daylight"),
    ("dawn", "Soft rose dawn"),
    ("sunset", "Golden amber sunset"),
    ("night", "Cyberpunk neon night"),
];

fn is_one_of(value: &str, options: &[(&str, &str)]) -> bool {
    options.iter().any(|(key, _)| *key == value)
}

/// Everything persisted in settings.json. Unknown keys from older versions are ignored, and
/// known ones keep their names so an existing settings.json carries over unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub deepseek_api_key: String,
    pub gemini_api_key: String,
    pub base_url: String,
    pub ai_model: String,
    /// One of [`REASONING_LEVELS`].
    pub reasoning: String,

    pub tts_voice: String,
    pub tts_enabled: bool,
    pub wake_word_enabled: bool,
    pub custom_clone_audio_path: String,
    /// One of [`MIC_MODES`].
    pub mic_mode: String,
    pub input_audio_device: String,
    pub output_audio_device: String,

    pub avatar_scale: f32,
    pub avatar_props: serde_json::Map<String, serde_json::Value>,
    pub body_framing: String,
    pub camera_zoom: f32,
    pub camera_y_offset: f32,
    pub custom_vrm_path: String,
    pub stage_lighting: String,

    pub subtitles_enabled: bool,
    pub bubble_offset: Option<[i32; 2]>,
    pub bubble_custom_size: Option<[i32; 2]>,
    pub bubble_font_pt: f32,
    pub input_bar_visible: bool,
    pub capsule_mode: bool,
    pub capsule_autohide: bool,

    pub proactive_enabled: bool,
    pub music_vibe_enabled: bool,
    pub presence_guardian_enabled: bool,
    pub automatic_game_copilot_enabled: bool,
    pub smart_gaming_dodge_enabled: bool,
    pub edge_snapping_enabled: bool,
    pub discord_presence_enabled: bool,
    /// Little things she does on her own when nothing is going on, chosen by the AI.
    pub idle_events_enabled: bool,
    /// Drive the desktop through Cua Driver (background clicks, accessibility trees, window
    /// lists) when it is installed.
    pub cua_enabled: bool,

    pub voice_follows_mood: bool,
    pub preferred_browser: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            deepseek_api_key: String::new(),
            gemini_api_key: String::new(),
            base_url: DEFAULT_BASE_URL.into(),
            ai_model: DEFAULT_MODEL.into(),
            reasoning: "auto".into(),
            tts_voice: "cloned_tsundere_1".into(),
            tts_enabled: true,
            wake_word_enabled: false,
            custom_clone_audio_path: String::new(),
            mic_mode: "click".into(),
            input_audio_device: String::new(),
            output_audio_device: String::new(),
            avatar_scale: 1.0,
            avatar_props: serde_json::Map::new(),
            // Head to toe: she stands on the taskbar, so her feet belong in the picture.
            body_framing: "full".into(),
            camera_zoom: 1.0,
            camera_y_offset: 0.0,
            custom_vrm_path: String::new(),
            stage_lighting: "auto".into(),
            subtitles_enabled: true,
            bubble_offset: None,
            bubble_custom_size: None,
            bubble_font_pt: 9.0,
            input_bar_visible: true,
            capsule_mode: true,
            capsule_autohide: true,
            proactive_enabled: true,
            music_vibe_enabled: true,
            presence_guardian_enabled: true,
            automatic_game_copilot_enabled: true,
            smart_gaming_dodge_enabled: true,
            edge_snapping_enabled: true,
            discord_presence_enabled: false,
            idle_events_enabled: true,
            cua_enabled: true,
            voice_follows_mood: false,
            preferred_browser: String::new(),
        }
    }
}

impl Settings {
    fn clamp(&mut self) {
        self.avatar_scale = self.avatar_scale.clamp(0.5, 2.5);
        self.camera_zoom = self.camera_zoom.clamp(0.4, 3.0);
        self.camera_y_offset = self.camera_y_offset.clamp(-0.8, 0.8);
        self.bubble_font_pt = self.bubble_font_pt.clamp(6.0, 22.0);
        if let Some([w, h]) = self.bubble_custom_size {
            self.bubble_custom_size = Some([w.max(90), h.max(44)]);
        }
        if !is_one_of(&self.mic_mode, MIC_MODES) {
            self.mic_mode = "click".into();
        }
        if !is_one_of(&self.body_framing, BODY_FRAMINGS) {
            self.body_framing = "full".into();
        }
        if !is_one_of(&self.stage_lighting, STAGE_LIGHTING) {
            self.stage_lighting = "auto".into();
        }
        if !is_one_of(&self.reasoning, REASONING_LEVELS) {
            self.reasoning = "auto".into();
        }
        if self.base_url.trim().is_empty() {
            self.base_url = DEFAULT_BASE_URL.into();
        }
        if self.ai_model.trim().is_empty() {
            self.ai_model = DEFAULT_MODEL.into();
        }
    }
}

// ---------------------------------------------------------------------------------- paths

fn is_cargo_target_dir(dir: &std::path::Path) -> bool {
    dir.file_name().is_some_and(|n| n == "debug" || n == "release")
        && dir.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "target")
}

/// Project root: the directory holding `assets`.
pub fn base_dir() -> &'static PathBuf {
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    BASE.get_or_init(|| {
        if let Ok(p) = std::env::var("ARIA_HOME") {
            let p = PathBuf::from(p);
            if p.is_dir() {
                return p;
            }
        }

        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd);
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut dir = exe.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            if is_cargo_target_dir(&dir) {
                if let Some(root) = dir.parent().and_then(|p| p.parent()) {
                    candidates.push(root.to_path_buf());
                }
            }
            for _ in 0..4 {
                candidates.push(dir.clone());
                match dir.parent() {
                    Some(parent) => dir = parent.to_path_buf(),
                    None => break,
                }
            }
        }

        // The project root is the directory that actually holds the app's data, not merely the
        // one the binary happens to sit under.
        candidates
            .iter()
            .find(|c| c.join("settings.json").exists() && c.join("assets").is_dir())
            .or_else(|| candidates.iter().find(|c| c.join("assets").is_dir()))
            .or_else(|| candidates.iter().find(|c| c.join("settings.json").exists()))
            .cloned()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    })
}

pub fn assets_dir() -> PathBuf {
    base_dir().join("assets")
}

/// Writable runtime data: settings, memory, logs, caches.
///
/// This is `%APPDATA%\Aria` so the app's state never lands in the repo or next to the exe.
pub fn data_dir() -> &'static PathBuf {
    static DATA: OnceLock<PathBuf> = OnceLock::new();
    DATA.get_or_init(|| {
        let dir = match std::env::var("APPDATA") {
            Ok(appdata) if !appdata.trim().is_empty() => PathBuf::from(appdata).join("Aria"),
            _ => base_dir().join("data"),
        };
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
}

/// A subdirectory of [`data_dir`], created on first use.
pub fn data_subdir(name: &str) -> PathBuf {
    let dir = data_dir().join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

/// Copies pre-migration files out of the old repo-root location once.
///
/// The old layout kept everything next to the sources; the new one keeps it in `data_dir()`.
/// The originals stay behind (not deleted) so a failed migration never loses user data.
fn migrate_legacy_data() {
    let target = data_dir();
    let source = base_dir();
    if source == target {
        return;
    }
    let mut moved = false;
    for name in ["settings.json", "aria_memory.json", "app_launch_cache.json"] {
        let dest = target.join(name);
        let src = source.join(name);
        if !dest.exists() && src.is_file() && std::fs::copy(&src, &dest).is_ok() {
            moved = true;
        }
    }
    // The TTS cache used to live under assets/; the audio files are reusable as-is.
    let old_cache = source.join("assets").join("tts_cache");
    let new_cache = data_dir().join("tts_cache");
    if old_cache.is_dir() && !new_cache.exists() {
        let _ = std::fs::create_dir_all(&new_cache);
        for entry in std::fs::read_dir(&old_cache).into_iter().flatten().flatten() {
            if entry.path().is_file() {
                let _ = std::fs::copy(entry.path(), new_cache.join(entry.file_name()));
            }
        }
        moved = true;
    }
    if moved {
        log::info!("migrated legacy data from {} to {}", source.display(), target.display());
    }
}

// ---------------------------------------------------------------------------------- store

static SETTINGS: OnceLock<RwLock<Settings>> = OnceLock::new();

fn store() -> &'static RwLock<Settings> {
    SETTINGS.get_or_init(|| {
        migrate_legacy_data();
        let mut s = std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
            .unwrap_or_default();
        s.clamp();
        RwLock::new(s)
    })
}

/// Loads the settings (and migrates old data) up front, instead of on first use.
pub fn init() {
    store();
}

/// Snapshot for UI and workers; never holds the lock afterwards.
pub fn get() -> Settings {
    store().read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Changes the settings, clamps them and saves the file.
pub fn update(f: impl FnOnce(&mut Settings)) {
    {
        let mut guard = store().write().unwrap_or_else(|e| e.into_inner());
        f(&mut guard);
        guard.clamp();
    }
    save();
}

/// Persists the current in-memory settings.
pub fn save() {
    let path = settings_path();
    let written = serde_json::to_string_pretty(&get()).is_ok_and(|text| std::fs::write(&path, text).is_ok());
    if !written {
        log::warn!("could not write {}", path.display());
    }
}

// ---------------------------------------------------------------------------------- AI endpoint

fn first_env(vars: &[&str]) -> Option<String> {
    vars.iter()
        .filter_map(|var| std::env::var(var).ok())
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
}

/// The API key: an environment variable wins over the saved one.
pub fn api_key() -> String {
    if let Some(key) = first_env(&["DEEPSEEK_API_KEY", "SAIL_API_KEY", "OPENAI_API_KEY", "GEMINI_API_KEY"]) {
        return key;
    }
    let s = get();
    [s.deepseek_api_key, s.gemini_api_key]
        .into_iter()
        .map(|k| k.trim().to_string())
        .find(|k| !k.is_empty())
        .unwrap_or_default()
}

pub fn base_url() -> String {
    first_env(&["DEEPSEEK_BASE_URL", "SAIL_BASE_URL", "OPENAI_BASE_URL"]).unwrap_or_else(|| get().base_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamping_repairs_bad_values() {
        let mut s = Settings {
            mic_mode: "telepathy".into(),
            reasoning: "galaxy-brain".into(),
            body_framing: "".into(),
            avatar_scale: 40.0,
            base_url: " ".into(),
            ..Settings::default()
        };
        s.clamp();
        assert_eq!(s.mic_mode, "click");
        assert_eq!(s.reasoning, "auto");
        assert_eq!(s.body_framing, "full");
        assert_eq!(s.avatar_scale, 2.5);
        assert_eq!(s.base_url, DEFAULT_BASE_URL);
    }

    #[test]
    fn old_settings_files_still_load() {
        // A file from before `reasoning`, `cua_enabled` and `idle_events_enabled` existed.
        let s: Settings = serde_json::from_str(r#"{"mic_mode":"always_open","ai_model":"x"}"#).unwrap();
        assert_eq!(s.mic_mode, "always_open");
        assert_eq!(s.reasoning, "auto");
        assert!(s.cua_enabled && s.idle_events_enabled);
    }
}
