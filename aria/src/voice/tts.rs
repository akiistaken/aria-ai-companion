//! Text-to-speech: Edge neural voices, a disk cache, and rodio playback wrapped in lip-sync.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

use msedge_tts::tts::client::connect;
use msedge_tts::tts::SpeechConfig;

use super::lipsync::LipSync;
use super::{SHUTTING_DOWN, SPEAKING, SPEECH_SEQ};

/// Pitch (Hz) and rate (%) offsets for one delivery.
#[derive(Clone, Copy)]
pub struct Delivery {
    pub pitch: i32,
    pub rate: i32,
}

pub struct VoicePreset {
    pub key: &'static str,
    pub label: &'static str,
    pub edge_voice: &'static str,
    pub idle: Delivery,
    pub talking: Delivery,
    pub annoyed: Delivery,
    pub walking: Delivery,
}

const fn d(pitch: i32, rate: i32) -> Delivery {
    Delivery { pitch, rate }
}

pub static VOICES: &[VoicePreset] = &[
    VoicePreset {
        key: "cloned_tsundere_1",
        label: "Tsundere #1 — soft & flustered",
        edge_voice: "en-US-AvaMultilingualNeural",
        idle: d(16, 10),
        talking: d(19, 12),
        annoyed: d(24, 16),
        walking: d(18, 12),
    },
    VoicePreset {
        key: "cloned_tsundere_2",
        label: "Tsundere/Yandere #2 — intense & playful",
        edge_voice: "en-US-EmmaMultilingualNeural",
        idle: d(18, 10),
        talking: d(22, 13),
        annoyed: d(27, 18),
        walking: d(20, 14),
    },
    VoicePreset {
        key: "cloned_tsundere_3",
        label: "Tsundere #3 — sassy & high-energy",
        edge_voice: "en-US-AvaMultilingualNeural",
        idle: d(17, 11),
        talking: d(20, 13),
        annoyed: d(25, 17),
        walking: d(19, 13),
    },
    VoicePreset {
        key: "cloned_tsundere_4",
        label: "Wingman #4 — confident & punchy",
        edge_voice: "en-US-EmmaMultilingualNeural",
        idle: d(19, 11),
        talking: d(22, 14),
        annoyed: d(26, 18),
        walking: d(20, 14),
    },
    VoicePreset {
        key: "nanami_jp",
        label: "Japanese seiyuu accent (Nanami)",
        edge_voice: "ja-JP-NanamiNeural",
        idle: d(6, 5),
        talking: d(9, 8),
        annoyed: d(15, 13),
        walking: d(8, 8),
    },
];

pub fn preset(key: &str) -> &'static VoicePreset {
    VOICES.iter().find(|v| v.key == key).unwrap_or(&VOICES[0])
}

impl VoicePreset {
    /// The full-body emotions borrow the closest of the four deliveries.
    fn delivery(&self, emotion: &str) -> Delivery {
        match emotion {
            "idle" | "sad" | "cry" | "sleepy" | "thinking" | "shy" | "hum" => self.idle,
            "annoyed" | "angry" => self.annoyed,
            "walking" => self.walking,
            _ => self.talking,
        }
    }
}

/// Strips markdown, emoji and stutter letters so TTS never spells them out, and keeps the first
/// couple of sentences of long replies for fast synthesis.
pub fn format_speech(text: &str, voice_key: &str) -> String {
    static CODE_RE: OnceLock<regex::Regex> = OnceLock::new();
    static LINK_RE: OnceLock<regex::Regex> = OnceLock::new();
    static STUTTER_RE: OnceLock<regex::Regex> = OnceLock::new();
    static BAKA_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = |cell: &'static OnceLock<regex::Regex>, pattern: &str| cell.get_or_init(|| regex::Regex::new(pattern).expect("valid regex"));

    let mut s = re(&CODE_RE, r"(?s)```.*?```").replace_all(text, "Here is the code you requested.").to_string();
    s = re(&LINK_RE, r"\[([^\]]+)\]\([^\)]+\)").replace_all(&s, "$1").to_string();
    s = s.replace(['`', '*'], "").replace('_', " ");
    s.retain(|c| {
        let cp = c as u32;
        !((0x1F300..=0x1FAFF).contains(&cp)
            || (0x2600..=0x27BF).contains(&cp)
            || (0xFE00..=0xFE0F).contains(&cp)
            || (0x1F000..=0x1F2FF).contains(&cp)
            || cp == 0x200D)
    });

    // Collapse stuttered first letters ("B-Baka", "I-It's"). The regex crate has no
    // backreferences, so the repeated letter is compared by hand.
    s = re(&STUTTER_RE, r"\b([A-Za-z])-([A-Za-z])([A-Za-z]+)\b")
        .replace_all(&s, |caps: &regex::Captures| {
            if caps[1].eq_ignore_ascii_case(&caps[2]) {
                format!("{}{}", &caps[2], &caps[3])
            } else {
                caps[0].to_string()
            }
        })
        .to_string();
    let baka = if voice_key == "nanami_jp" { "Baka" } else { "baaka" };
    s = re(&BAKA_RE, r"(?i)\bbaka\b").replace_all(&s, baka).to_string();

    let cleaned = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= 160 {
        return cleaned;
    }
    let mut trimmed = String::new();
    for part in split_sentences(&cleaned).map(str::trim).filter(|p| !p.is_empty()) {
        if !trimmed.is_empty() && trimmed.chars().count() + part.chars().count() > 160 {
            break;
        }
        if !trimmed.is_empty() {
            trimmed.push(' ');
        }
        trimmed.push_str(part);
    }
    if trimmed.is_empty() {
        cleaned
    } else {
        trimmed
    }
}

/// Splits after `.`, `!` or `?` followed by whitespace, keeping the punctuation.
fn split_sentences(text: &str) -> impl Iterator<Item = &str> {
    let mut start = 0;
    let mut pieces = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?') && chars.peek().is_some_and(|(_, next)| next.is_whitespace()) {
            let end = i + c.len_utf8();
            pieces.push(&text[start..end]);
            start = end;
        }
    }
    pieces.push(&text[start..]);
    pieces.into_iter()
}

fn cache_file(spoken: &str, voice_key: &str, emotion: &str) -> PathBuf {
    let digest = format!("{:x}", md5::compute(format!("{voice_key}|{emotion}|{spoken}").as_bytes()));
    crate::config::data_subdir("tts_cache").join(format!("cached_{voice_key}_{}.mp3", &digest[..16]))
}

/// Keeps the newest `max_files` clips.
fn prune_cache(max_files: usize) {
    let Ok(entries) = std::fs::read_dir(crate::config::data_subdir("tts_cache")) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "mp3"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= max_files {
        return;
    }
    files.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    for (_, path) in files.into_iter().skip(max_files) {
        let _ = std::fs::remove_file(path);
    }
}

/// Synthesizes with Edge neural TTS into `out_file`.
fn synthesize(text: &str, emotion: &str, voice_key: &str, out_file: &Path) -> bool {
    let preset = preset(voice_key);
    let delivery = preset.delivery(emotion);
    let config = SpeechConfig {
        voice_name: preset.edge_voice.to_string(),
        audio_format: "audio-24khz-48kbitrate-mono-mp3".to_string(),
        pitch: delivery.pitch,
        rate: delivery.rate,
        volume: 0,
    };
    let Ok(mut client) = connect() else {
        log::warn!("edge-tts connection failed");
        return false;
    };
    match client.synthesize(text, &config) {
        Ok(audio) if audio.audio_bytes.len() > 200 => {
            // Written to a temp name first, so a half-written file never looks cached.
            let tmp = out_file.with_extension("tmp.mp3");
            let written = std::fs::File::create(&tmp).and_then(|mut f| f.write_all(&audio.audio_bytes));
            if written.is_ok() && std::fs::rename(&tmp, out_file).is_ok() {
                return true;
            }
            let _ = std::fs::remove_file(&tmp);
            false
        }
        Ok(_) => false,
        Err(e) => {
            log::warn!("edge-tts synthesis failed: {e}");
            false
        }
    }
}

fn output() -> &'static Mutex<Option<rodio::MixerDeviceSink>> {
    static OUTPUT: OnceLock<Mutex<Option<rodio::MixerDeviceSink>>> = OnceLock::new();
    OUTPUT.get_or_init(|| Mutex::new(None))
}

/// Starts playback of one utterance. [`LipSync`] clears `SPEAKING` when it ends.
fn play_file(path: &Path, seq: i64) -> bool {
    let Ok(file) = std::fs::File::open(path) else { return false };
    let Ok(source) = rodio::Decoder::try_from(std::io::BufReader::new(file)) else {
        log::warn!("could not decode {}", path.display());
        return false;
    };
    let mut guard = output().lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(handle) => *guard = Some(handle),
            Err(e) => {
                log::warn!("no audio output device: {e}");
                return false;
            }
        }
    }
    match guard.as_ref() {
        Some(handle) => {
            handle.mixer().add(LipSync::new(source, seq));
            true
        }
        None => false,
    }
}

/// Speaks text asynchronously, superseding any utterance in flight.
pub fn speak_async(text: &str, emotion: &str) {
    let settings = crate::config::get();
    if SHUTTING_DOWN.load(Ordering::SeqCst) || !settings.tts_enabled {
        return;
    }
    let voice_key = settings.tts_voice;
    let spoken = format_speech(text, &voice_key);
    if spoken.is_empty() {
        return;
    }
    let seq = SPEECH_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    SPEAKING.store(true, Ordering::SeqCst);
    let emotion = emotion.to_string();
    std::thread::spawn(move || {
        let cached = cache_file(&spoken, &voice_key, &emotion);
        let ready = std::fs::metadata(&cached).is_ok_and(|m| m.len() > 500)
            || (synthesize(&spoken, &emotion, &voice_key, &cached) && {
                prune_cache(40);
                true
            });
        let current = SPEECH_SEQ.load(Ordering::SeqCst) == seq;
        if (!ready || SHUTTING_DOWN.load(Ordering::SeqCst) || !current || !play_file(&cached, seq))
            && SPEECH_SEQ.load(Ordering::SeqCst) == seq {
                SPEAKING.store(false, Ordering::SeqCst);
            }
    });
}

/// Pre-warms the cache with the lines most likely to be spoken on startup.
pub fn precache(lines: Vec<(String, String)>) {
    let voice_key = crate::config::get().tts_voice;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        for (raw, emotion) in lines {
            if SHUTTING_DOWN.load(Ordering::SeqCst) {
                break;
            }
            let spoken = format_speech(&raw, &voice_key);
            let target = cache_file(&spoken, &voice_key, &emotion);
            if !spoken.is_empty() && !target.exists() {
                synthesize(&spoken, &emotion, &voice_key, &target);
            }
            std::thread::sleep(std::time::Duration::from_millis(350));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_format_strips_markdown_but_keeps_words() {
        let out = format_speech("[EMOTION: happy] Fine. **Good** job `today`.", "cloned_tsundere_1");
        assert!(out.contains("Good") && !out.contains('*') && !out.contains('`'));
    }

    #[test]
    fn stutter_letters_collapse() {
        let out = format_speech("B-Baka! I-It's fine. A-B test.", "cloned_tsundere_1").to_lowercase();
        assert!(out.contains("baaka") && out.contains("it's") && out.contains("a-b"), "got {out}");
    }

    #[test]
    fn long_text_is_trimmed_for_synthesis() {
        let out = format_speech(&"This is a sentence. ".repeat(20), "cloned_tsundere_1");
        assert!(out.chars().count() <= 190, "got {}", out.chars().count());
    }

    #[test]
    fn preset_lookup_falls_back() {
        assert_eq!(preset("nope").key, VOICES[0].key);
        assert_eq!(preset("nanami_jp").edge_voice, "ja-JP-NanamiNeural");
        assert_eq!(preset("cloned_tsundere_1").delivery("angry").pitch, 24);
    }
}
