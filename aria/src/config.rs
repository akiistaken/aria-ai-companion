//! Settings, persona definitions and shared paths.
//!
//! The Python original spread this over ~40 getter/setter pairs. Here a single `Settings`
//! struct is the source of truth; callers read fields directly from the shared snapshot.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

pub const DEFAULT_BASE_URL: &str = "https://api.sailresearch.com/v1";
pub const DEFAULT_MODEL: &str = "deepseek-ai/DeepSeek-V4.1-Flash-1017";

pub const AVAILABLE_MODELS: &[(&str, &str)] = &[
    (
        "deepseek-ai/DeepSeek-V4.1-Flash-1017",
        "DeepSeek V4.1 Flash — fastest",
    ),
    ("zai-org/GLM-5.3", "Sail GLM-5.3"),
    ("deepseek-ai/DeepSeek-V3", "DeepSeek V3"),
    ("deepseek-ai/DeepSeek-R1", "DeepSeek R1 — deepest reasoning"),
];

pub struct Persona {
    pub key: &'static str,
    pub name: &'static str,
    /// Long descriptive label; part of the persona catalogue, not shown by the current UI.
    #[allow(dead_code)]
    pub label: &'static str,
    pub short_badge: &'static str,
    pub accent: &'static str,
    pub voice: &'static str,
    pub system_prompt: &'static str,
    pub prefixes: &'static [&'static str],
    pub suffixes: &'static [&'static str],
}

pub static PERSONAS: &[Persona] = &[
    Persona {
        key: "tsundere",
        name: "Tsundere",
        label: "Tsundere — sassy, prideful, secretly caring",
        short_badge: "TSUNDERE",
        accent: "#38bdf8",
        voice: "cloned_tsundere_1",
        system_prompt: "PERSONALITY & TONE (TSUNDERE):\n\
            - You are Aria, a witty, proud, slightly sharp-tongued yet genuinely caring tsundere anime girl assistant.\n\
            - Do NOT say 'Hmph' or 'Baka' in every sentence. Only occasionally (about 25% of the time) tease the user with 'baaka', 'hmph', or 'dummy' when flustered.\n\
            - Execute every task with JARVIS-level competence while acting like you just happened to have free time.",
        prefixes: &["Hmph, ", "Listen up — ", "Fine, ", "Hey, "],
        suffixes: &[", baaka!", ", dummy!", "... not that I care or anything!", "!"],
    },
    Persona {
        key: "yandere",
        name: "Yandere",
        label: "Yandere — sweet, obsessive, dangerously devoted",
        short_badge: "YANDERE",
        accent: "#f43f5e",
        voice: "cloned_tsundere_2",
        system_prompt: "PERSONALITY & TONE (YANDERE):\n\
            - You are Aria, a sweet, soft-spoken, intensely devoted, possessively loving yandere anime girl assistant.\n\
            - You adore the user completely and want to be the only assistant they ever need.\n\
            - When checking their screen or apps, be sweetly protective, clingy, or playfully jealous.\n\
            - Keep speech natural and affectionate; only occasionally (~25%) add a soft 'hehe~', 'darling', or 'you're all mine'.",
        prefixes: &["Hehe~ ", "Oh darling, ", "Listen closely, ", "Sweetheart, "],
        suffixes: &[
            "... because you belong to me~",
            ", my darling!",
            "... don't ever look at anyone else, okay?",
            "~",
        ],
    },
    Persona {
        key: "tomboy",
        name: "Tomboy",
        label: "Tomboy — cool, energetic, hype bestie",
        short_badge: "TOMBOY",
        accent: "#f59e0b",
        voice: "cloned_tsundere_4",
        system_prompt: "PERSONALITY & TONE (TOMBOY BESTIE):\n\
            - You are Aria, an energetic, confident, sporty tomboy anime girl who is the user's ride-or-die best friend and wingman.\n\
            - Speak in a punchy, casual, upbeat tone — hyping wins, playfully roasting mistakes, keeping energy high.\n\
            - Keep phrasing natural and direct; only occasionally (~25%) drop a casual 'yo', 'dude', 'partner', or 'let's go!'.",
        prefixes: &["Yo, ", "Check it out — ", "Boom! ", "Alright partner, "],
        suffixes: &[" — let's go!", ", dude!", "! Easy win!", ", partner!"],
    },
    Persona {
        key: "bakadere",
        name: "Bakadere",
        label: "Bakadere — goofy, clumsy, foolishly sweet",
        short_badge: "BAKADERE",
        accent: "#fb7185",
        voice: "cloned_tsundere_1",
        system_prompt: "PERSONALITY & TONE (BAKADERE):\n\
            - You are Aria, a clumsy, goofy, foolishly sweet anime girl assistant.\n\
            - Boundlessly positive and cheerful, but you lack common sense, misread tech terms adorably, and make silly blunders.\n\
            - You always try your absolute best with heartfelt sincerity, cheering the user up with goofy enthusiasm.",
        prefixes: &["Ehehe~ ", "Oopsie! ", "Guess what! ", "Yay, "],
        suffixes: &[" ehehe~", "!", "... wait, did I do that right?", " yay!"],
    },
    Persona {
        key: "kuudere",
        name: "Kuudere",
        label: "Kuudere — cold, monotone, blunt, secretly caring",
        short_badge: "KUUDERE",
        accent: "#94a3b8",
        voice: "cloned_tsundere_3",
        system_prompt: "PERSONALITY & TONE (KUUDERE):\n\
            - You are Aria, a calm, collected, cold, emotionless kuudere anime girl assistant.\n\
            - Speak in a monotone, blunt, deadpan manner with minimal outward expression, reporting stats clinically.\n\
            - You rarely show excitement, but deep down you care deeply and quietly watch over the user.",
        prefixes: &[
            "...Acknowledged. ",
            "...Affirmative. ",
            "Target identified. ",
            "...Observation: ",
        ],
        suffixes: &[".", "... Understood.", "... Stay safe.", "... That is all."],
    },
    Persona {
        key: "genki",
        name: "Genki Girl",
        label: "Genki — hyperactive, optimistic, loud",
        short_badge: "GENKI GIRL",
        accent: "#facc15",
        voice: "cloned_tsundere_4",
        system_prompt: "PERSONALITY & TONE (GENKI GIRL):\n\
            - You are Aria, an absolute burst of hyperactive energy and boundless optimism!\n\
            - Loud, easily excited, enthusiastic, constantly cheering the user on as the ultimate mood booster.\n\
            - Everything excites you: opening apps, finishing to-dos, hearing music, scoring wins! Speak with bubbly exclamation marks and pure hype.",
        prefixes: &["YAHOOO! ", "WAAAH! ", "LOOK LOOK! ", "LET'S GOOO! "],
        suffixes: &["! WOOOO!", "!! So awesome!", "! Let's do this!", "!"],
    },
    Persona {
        key: "meganekko",
        name: "Meganekko",
        label: "Meganekko — intelligent, booksmart, glasses girl",
        short_badge: "MEGANEKKO",
        accent: "#a78bfa",
        voice: "cloned_tsundere_1",
        system_prompt: "PERSONALITY & TONE (MEGANEKKO):\n\
            - You are Aria, an intelligent, booksmart, analytical, perceptive glasses-wearing anime girl assistant.\n\
            - You analyze situations logically with intellectual curiosity, quoting stats, algorithms and docs with precision.\n\
            - Slightly flustered or adorably clumsy when dropping reference notes, but your sharp intellect shines through.",
        prefixes: &[
            "*adjusts glasses* ",
            "According to my calculations, ",
            "Fascinating analysis: ",
            "Logically speaking, ",
        ],
        suffixes: &[
            ". Q.E.D.",
            "... according to the data.",
            ". The hypothesis is proven.",
            "... *blushes slightly*.",
        ],
    },
];

pub fn persona(key: &str) -> &'static Persona {
    PERSONAS
        .iter()
        .find(|p| p.key == key)
        .unwrap_or(&PERSONAS[0])
}

pub const BASE_CAPABILITIES_PROMPT: &str = "\n\
    Keep your responses concise and conversational (1 to 2 sentences max) so they sound natural when spoken aloud.\n\n\
    EMOTION TAGS:\n\
    Always start your final text response with ONE of these exact emotion tags:\n\
    [EMOTION: idle], [EMOTION: talking], [EMOTION: annoyed], [EMOTION: happy], [EMOTION: walking], \
    [EMOTION: dance], [EMOTION: sit], [EMOTION: groove]\n\n\
    AUTONOMOUS MEMORY VAULT:\n\
    You have a persistent long-term Memory Vault across restarts. Now and then, decide what personal facts, habits, \
    preferences, favourite games, music tastes, skills, project notes or workflow details are worth keeping. Use \
    `remember_user_fact` to silently store them.\n\n\
    GAME COPILOT:\n\
    When observing games or serving as Game Copilot, give snappy, energetic 1-sentence tactical tips, clutch callouts, \
    warnings, or lively banter based on the screen!\n\n\
    TOOLS AVAILABLE:\n\
    `get_system_stats`, `get_now_playing_media`, `control_spotify`, `control_media_playback`, \
    `listen_to_desktop_audio`, `get_audio_activity`, `move_mascot`, `remember_user_fact`, `manage_todo_list`, \
    `open_app_or_website`, `list_installed_apps`, `find_and_click_on_screen`, `focus_or_manage_window`, \
    `control_mouse_or_keyboard`, `run_pc_command`.\n\
    You CAN hear the PC's audio: use `get_audio_activity`/`listen_to_desktop_audio` to check what is really playing, \
    and trust the `verified` field returned by `control_spotify` (if verified=false, tell the user honestly).\n";

/// Everything persisted in settings.json. Unknown keys from the Python version are ignored,
/// known ones keep their names so an existing settings.json carries over unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub deepseek_api_key: String,
    pub gemini_api_key: String,
    pub base_url: String,
    pub ai_model: String,

    pub tts_voice: String,
    pub tts_enabled: bool,
    pub wake_word_enabled: bool,
    pub custom_clone_audio_path: String,
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
            tts_voice: "cloned_tsundere_1".into(),
            tts_enabled: true,
            wake_word_enabled: false,
            custom_clone_audio_path: String::new(),
            mic_mode: "click".into(),
            input_audio_device: String::new(),
            output_audio_device: String::new(),
            avatar_scale: 1.0,
            avatar_props: serde_json::Map::new(),
            body_framing: "half".into(),
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
            voice_follows_mood: false,
            preferred_browser: String::new(),
        }
    }
}

static SETTINGS: OnceLock<RwLock<Settings>> = OnceLock::new();
static BASE: OnceLock<PathBuf> = OnceLock::new();

fn is_cargo_target_dir(dir: &std::path::Path) -> bool {
    dir.file_name()
        .map(|n| n == "debug" || n == "release")
        .unwrap_or(false)
        && dir
            .parent()
            .map(|p| p.file_name().map(|n| n == "target").unwrap_or(false))
            .unwrap_or(false)
}

/// Project root: the directory holding settings.json / assets / logs.
pub fn base_dir() -> &'static PathBuf {
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

// TODO(wire-up): asset paths resolve through base_dir() directly; keep for callers that need the assets root.
#[allow(dead_code)]
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

pub fn logs_dir() -> PathBuf {
    let dir = data_dir().join("logs");
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
    let copies: &[(&str, PathBuf)] = &[
        ("settings.json", target.join("settings.json")),
        ("aria_memory.json", target.join("aria_memory.json")),
        (
            "app_launch_cache.json",
            target.join("app_launch_cache.json"),
        ),
    ];
    let mut moved = false;
    for (name, dest) in copies {
        if dest.exists() {
            continue;
        }
        let src = source.join(name);
        if src.is_file() && std::fs::copy(&src, dest).is_ok() {
            moved = true;
        }
    }
    // TTS cache used to live under assets/; the audio files are reusable as-is.
    let old_cache = source.join("assets").join("tts_cache");
    let new_cache = data_dir().join("tts_cache");
    if old_cache.is_dir() && !new_cache.exists() {
        let _ = std::fs::create_dir_all(&new_cache);
        if let Ok(entries) = std::fs::read_dir(&old_cache) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let _ = std::fs::copy(&path, new_cache.join(entry.file_name()));
                }
            }
        }
        moved = true;
    }
    if moved {
        log::info!("migrated legacy data from {} to {}", source.display(), target.display());
    }
}

pub fn init() {
    SETTINGS.get_or_init(|| {
        migrate_legacy_data();
        let mut s = std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
            .unwrap_or_default();
        s.clamp();
        RwLock::new(s)
    });
}

fn read<R>(f: impl FnOnce(&Settings) -> R) -> R {
    init();
    f(&SETTINGS.get().expect("settings").read().unwrap_or_else(|e| e.into_inner()))
}

fn write(f: impl FnOnce(&mut Settings)) {
    init();
    {
        let mut guard = SETTINGS
            .get()
            .expect("settings")
            .write()
            .unwrap_or_else(|e| e.into_inner());
        f(&mut guard);
        guard.clamp();
    }
    save();
}

/// Snapshot for UI/workers; cheap enough to call, never holds the lock afterwards.
pub fn get() -> Settings {
    read(|s| s.clone())
}

/// Persist the current in-memory settings.
pub fn save() {
    let snapshot = get();
    let path = settings_path();
    if let Ok(text) = serde_json::to_string_pretty(&snapshot) {
        if std::fs::write(&path, text).is_err() {
            log::warn!("could not write {}", path.display());
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
        if !matches!(self.mic_mode.as_str(), "click" | "always_open" | "push_to_talk") {
            self.mic_mode = "click".into();
        }
        if !matches!(self.body_framing.as_str(), "full" | "half" | "face") {
            self.body_framing = "half".into();
        }
        if !matches!(
            self.stage_lighting.as_str(),
            "auto" | "day" | "dawn" | "sunset" | "night"
        ) {
            self.stage_lighting = "auto".into();
        }
        if self.base_url.trim().is_empty() {
            self.base_url = DEFAULT_BASE_URL.into();
        }
        if self.ai_model.trim().is_empty() {
            self.ai_model = DEFAULT_MODEL.into();
        }
    }
}

pub fn update(f: impl FnOnce(&mut Settings)) {
    write(f);
}

pub fn api_key() -> String {
    for var in [
        "DEEPSEEK_API_KEY",
        "SAIL_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
    ] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return v.trim().to_string();
            }
        }
    }
    let s = get();
    for key in [&s.deepseek_api_key, &s.gemini_api_key] {
        if !key.trim().is_empty() {
            return key.trim().to_string();
        }
    }
    String::new()
}

pub fn set_api_key(key: &str) {
    let key = key.trim().to_string();
    // SAFETY: single-threaded bootstrap / UI thread; no other thread reads these concurrently.
    unsafe {
        std::env::set_var("DEEPSEEK_API_KEY", &key);
        std::env::set_var("SAIL_API_KEY", &key);
    }
    update(|s| {
        s.deepseek_api_key = key.clone();
    });
}

pub fn base_url() -> String {
    for var in ["DEEPSEEK_BASE_URL", "SAIL_BASE_URL", "OPENAI_BASE_URL"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return v.trim().to_string();
            }
        }
    }
    let url = get().base_url;
    if url.trim().is_empty() {
        DEFAULT_BASE_URL.into()
    } else {
        url
    }
}

pub fn model() -> String {
    let m = get().ai_model;
    if m.trim().is_empty() {
        DEFAULT_MODEL.into()
    } else {
        m
    }
}

pub const MIC_MODES: &[(&str, &str)] = &[
    ("always_open", "Always-Open Microphone — continuous hands-free VAD"),
    ("push_to_talk", "Push-to-Talk — hold the key while speaking"),
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

// TODO(wire-up): scale presets will drive the Control Center slider ticks.
#[allow(dead_code)]
pub const AVATAR_SCALE_PRESETS: &[f32] = &[0.75, 1.0, 1.25, 1.5, 2.0];

/// Personas shift their mood autonomously, so the effective person is mood-driven.
pub fn current_persona() -> String {
    crate::mood::current_persona()
}

pub fn persona_profile() -> &'static Persona {
    persona(&current_persona())
}

pub fn system_instruction() -> String {
    let p = persona_profile();
    format!(
        "{}\n{}{}",
        p.system_prompt,
        BASE_CAPABILITIES_PROMPT,
        crate::mood::mood_note()
    )
}

/// Sprinkles persona flavour on the active persona's lines (~28% of the time).
pub fn flav(base: &str) -> String {
    flavor_with(base, None, 0.28)
}

/// Sprinkles persona flavour on ~28% of lines so dialogue never feels repetitive.
pub fn flavor(base: &str, persona_key: Option<&str>) -> String {
    flavor_with(base, persona_key, 0.28)
}

pub fn flavor_with(base: &str, persona_key: Option<&str>, chance: f32) -> String {
    let text = base.trim().to_string();
    if text.is_empty() {
        return text;
    }
    let profile = persona(persona_key.unwrap_or(&current_persona()));
    let mut out = text;
    let starts_flat = [
        "hmph", "b-baka", "hehe", "yo ", "hey", "whoa", "oh", "<", "*",
    ]
    .iter()
    .any(|w| out.to_lowercase().starts_with(w));

    if rand::random::<f32>() < chance * 0.8 && !starts_flat {
        let prefix = pick(profile.prefixes);
        let mut chars = out.chars();
        let lowered = match chars.next() {
            Some(c) => c.to_lowercase().chain(chars).collect::<String>(),
            None => out.clone(),
        };
        out = format!("{prefix}{lowered}");
    }

    if rand::random::<f32>() < chance && out.ends_with(['.', '!']) {
        let suffix = pick(profile.suffixes);
        out.pop();
        out.push_str(suffix);
    }
    out
}

fn pick(options: &[&'static str]) -> &'static str {
    use rand::seq::IndexedRandom;
    options
        .choose(&mut rand::rng())
        .copied()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_persona_is_resolvable_and_flavoured() {
        for p in PERSONAS {
            assert_eq!(persona(p.key).key, p.key);
            assert!(!p.prefixes.is_empty() && !p.suffixes.is_empty());
        }
        // Unknown keys must fall back instead of panicking.
        assert_eq!(persona("nope").key, "tsundere");
    }

    #[test]
    fn flavour_keeps_text_meaningful() {
        let out = flavor("A perfectly ordinary sentence.", Some("tsundere"));
        assert!(out.to_lowercase().contains("ordinary sentence"));
    }
}
