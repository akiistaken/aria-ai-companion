//! Voice: Edge neural TTS with a disk cache, rodio playback, microphone VAD capture and
//! speech-to-text.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use msedge_tts::tts::client::connect;
use msedge_tts::tts::SpeechConfig;

pub struct VoicePreset {
    pub key: &'static str,
    pub label: &'static str,
    pub edge_voice: &'static str,
    /// (pitch offset in Hz, rate offset in %)
    pub pitch_idle: i32,
    pub pitch_talking: i32,
    pub pitch_annoyed: i32,
    pub pitch_walking: i32,
    pub rate_idle: i32,
    pub rate_talking: i32,
    pub rate_annoyed: i32,
    pub rate_walking: i32,
}

pub static VOICES: &[VoicePreset] = &[
    VoicePreset {
        key: "cloned_tsundere_1",
        label: "Tsundere #1 — soft & flustered",
        edge_voice: "en-US-AvaMultilingualNeural",
        pitch_idle: 16,
        pitch_talking: 19,
        pitch_annoyed: 24,
        pitch_walking: 18,
        rate_idle: 10,
        rate_talking: 12,
        rate_annoyed: 16,
        rate_walking: 12,
    },
    VoicePreset {
        key: "cloned_tsundere_2",
        label: "Tsundere/Yandere #2 — intense & playful",
        edge_voice: "en-US-EmmaMultilingualNeural",
        pitch_idle: 18,
        pitch_talking: 22,
        pitch_annoyed: 27,
        pitch_walking: 20,
        rate_idle: 10,
        rate_talking: 13,
        rate_annoyed: 18,
        rate_walking: 14,
    },
    VoicePreset {
        key: "cloned_tsundere_3",
        label: "Tsundere #3 — sassy & high-energy",
        edge_voice: "en-US-AvaMultilingualNeural",
        pitch_idle: 17,
        pitch_talking: 20,
        pitch_annoyed: 25,
        pitch_walking: 19,
        rate_idle: 11,
        rate_talking: 13,
        rate_annoyed: 17,
        rate_walking: 13,
    },
    VoicePreset {
        key: "cloned_tsundere_4",
        label: "Wingman #4 — confident & punchy",
        edge_voice: "en-US-EmmaMultilingualNeural",
        pitch_idle: 19,
        pitch_talking: 22,
        pitch_annoyed: 26,
        pitch_walking: 20,
        rate_idle: 11,
        rate_talking: 14,
        rate_annoyed: 18,
        rate_walking: 14,
    },
    VoicePreset {
        key: "nanami_jp",
        label: "Japanese seiyuu accent (Nanami)",
        edge_voice: "ja-JP-NanamiNeural",
        pitch_idle: 6,
        pitch_talking: 9,
        pitch_annoyed: 15,
        pitch_walking: 8,
        rate_idle: 5,
        rate_talking: 8,
        rate_annoyed: 13,
        rate_walking: 8,
    },
];

pub fn preset(key: &str) -> &'static VoicePreset {
    VOICES.iter().find(|v| v.key == key).unwrap_or(&VOICES[0])
}

fn cache_dir() -> PathBuf {
    let dir = crate::config::data_dir().join("tts_cache");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Current speech sequence number. Playback of an older sequence is abandoned.
static SPEECH_SEQ: AtomicI64 = AtomicI64::new(0);
static SPEAKING: AtomicBool = AtomicBool::new(false);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static RECORDING: AtomicBool = AtomicBool::new(false);

pub fn is_speaking() -> bool {
    SPEAKING.load(Ordering::SeqCst)
}

pub fn is_recording() -> bool {
    RECORDING.load(Ordering::SeqCst)
}

pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    SPEAKING.store(false, Ordering::SeqCst);
}

// ---------------------------------------------------------------- lip-sync envelope

/// Mouth shape of the audio playing right now, published by [`LipSync`] as `f32` bits.
static MOUTH_AA: AtomicU32 = AtomicU32::new(0);
static MOUTH_OH: AtomicU32 = AtomicU32::new(0);

/// The current `(aa, oh)` lip-sync levels, both `0..1`. Zero when nothing is playing.
pub fn mouth_levels() -> (f32, f32) {
    (
        f32::from_bits(MOUTH_AA.load(Ordering::Relaxed)),
        f32::from_bits(MOUTH_OH.load(Ordering::Relaxed)),
    )
}

fn publish_mouth(aa: f32, oh: f32) {
    MOUTH_AA.store(aa.to_bits(), Ordering::Relaxed);
    MOUTH_OH.store(oh.to_bits(), Ordering::Relaxed);
}

/// Wraps the TTS decoder and measures it as it plays.
///
/// Every ~20 ms window yields an RMS level (how open the mouth is) and a zero-crossing rate (a
/// cheap stand-in for the formant split the WebView did with an FFT: rounded vowels sit low,
/// open and fricative sounds cross zero often). It also owns the end of speech: `SPEAKING` drops
/// when the audio runs out instead of after a duration guessed from the file size, and a
/// superseded or stopped utterance is cut off mid-sample rather than playing to the end.
struct LipSync<S> {
    inner: S,
    seq: i64,
    window: usize,
    count: usize,
    sum_sq: f32,
    crossings: usize,
    last_positive: bool,
    finished: bool,
}

impl<S: rodio::Source> LipSync<S> {
    fn new(inner: S, seq: i64) -> Self {
        let per_second = inner.sample_rate().get() as usize * inner.channels().get() as usize;
        Self {
            window: (per_second / 50).max(64),
            inner,
            seq,
            count: 0,
            sum_sq: 0.0,
            crossings: 0,
            last_positive: false,
            finished: false,
        }
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        publish_mouth(0.0, 0.0);
        if SPEECH_SEQ.load(Ordering::SeqCst) == self.seq {
            SPEAKING.store(false, Ordering::SeqCst);
        }
    }

    fn flush_window(&mut self) {
        let n = self.count.max(1) as f32;
        let rms = (self.sum_sq / n).sqrt();
        let zcr = self.crossings as f32 / n;
        // Speech RMS from Edge TTS peaks around 0.25; below ~0.02 is breath and room noise.
        let open = ((rms - 0.02) * 5.5).clamp(0.0, 1.0);
        let rounded = (1.0 - zcr * 9.0).clamp(0.0, 1.0);
        publish_mouth(open * (1.0 - 0.45 * rounded), (open * rounded * 0.85).min(0.85));
        self.count = 0;
        self.sum_sq = 0.0;
        self.crossings = 0;
    }
}

impl<S: rodio::Source> Iterator for LipSync<S> {
    type Item = rodio::Sample;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if SHUTTING_DOWN.load(Ordering::Relaxed) || SPEECH_SEQ.load(Ordering::Relaxed) != self.seq {
            self.finish();
            return None;
        }
        let Some(sample) = self.inner.next() else {
            self.finish();
            return None;
        };
        let value = sample as f32;
        self.sum_sq += value * value;
        let positive = value >= 0.0;
        if positive != self.last_positive {
            self.crossings += 1;
            self.last_positive = positive;
        }
        self.count += 1;
        if self.count >= self.window {
            self.flush_window();
        }
        Some(sample)
    }
}

impl<S: rodio::Source> rodio::Source for LipSync<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        self.inner.total_duration()
    }
}

impl<S> Drop for LipSync<S> {
    fn drop(&mut self) {
        if !self.finished {
            publish_mouth(0.0, 0.0);
            if SPEECH_SEQ.load(Ordering::SeqCst) == self.seq {
                SPEAKING.store(false, Ordering::SeqCst);
            }
        }
    }
}

/// Strips markdown, emoji and stutter letters so TTS never spells them out.
pub fn format_speech(text: &str, voice_key: &str) -> String {
    // Compiled once: this runs for every spoken line.
    static CODE_RE: OnceLock<regex::Regex> = OnceLock::new();
    static LINK_RE: OnceLock<regex::Regex> = OnceLock::new();
    static STUTTER_RE: OnceLock<regex::Regex> = OnceLock::new();
    static BAKA_RE: OnceLock<regex::Regex> = OnceLock::new();
    let code_re = CODE_RE.get_or_init(|| regex::Regex::new(r"(?s)```.*?```").expect("valid regex"));
    let link_re =
        LINK_RE.get_or_init(|| regex::Regex::new(r"\[([^\]]+)\]\([^\)]+\)").expect("valid regex"));

    let mut s = text.to_string();
    // Fenced code blocks.
    s = code_re.replace_all(&s, "Here is the code you requested.").to_string();
    // Markdown links -> label.
    s = link_re.replace_all(&s, "$1").to_string();
    s = s.replace('`', "").replace('*', "").replace('_', " ");

    // Emoji and pictographs.
    s = s
        .chars()
        .filter(|c| {
            let cp = *c as u32;
            !((0x1F300..=0x1FAFF).contains(&cp)
                || (0x2600..=0x27BF).contains(&cp)
                || (0xFE00..=0xFE0F).contains(&cp)
                || (0x1F000..=0x1F2FF).contains(&cp)
                || cp == 0x200D)
        })
        .collect();

    let is_jp = voice_key == "nanami_jp";
    let baka = if is_jp { "Baka" } else { "baaka" };

    // Collapse stuttered first letters: "B-Baka", "I-It's", "F-Fine". Rust's regex engine has no
    // backreferences, so the repeated letter is detected by comparing the two captured letters.
    let stutter = STUTTER_RE.get_or_init(|| {
        regex::Regex::new(r"\b([A-Za-z])-([A-Za-z])([A-Za-z]{1,})\b").expect("valid regex")
    });
    s = stutter
        .replace_all(&s, |caps: &regex::Captures| {
            let first = caps[1].to_ascii_lowercase();
            let second = caps[2].to_ascii_lowercase();
            if first == second {
                let mut out = String::from(&caps[2]);
                out.push_str(&caps[3]);
                out
            } else {
                format!("{}-{}{}", &caps[1], &caps[2], &caps[3])
            }
        })
        .to_string();
    let baka_re = BAKA_RE.get_or_init(|| regex::Regex::new(r"(?i)\bbaka\b").expect("valid regex"));
    s = baka_re.replace_all(&s, baka).to_string();

    let cleaned = s.split_whitespace().collect::<Vec<_>>().join(" ");

    // Keep the first couple of sentences for fast synthesis. (`regex` has no look-behind, which
    // the old `(?<=[.!?])\s+` split needed: it panicked on every reply over 160 characters.)
    if cleaned.chars().count() > 160 {
        let mut trimmed = String::new();
        for part in split_sentences(&cleaned) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if trimmed.is_empty() || trimmed.chars().count() + part.chars().count() <= 160 {
                if !trimmed.is_empty() {
                    trimmed.push(' ');
                }
                trimmed.push_str(part);
            } else {
                break;
            }
        }
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    cleaned
}

/// Splits after `.`, `!` or `?` followed by whitespace, keeping the punctuation.
fn split_sentences(text: &str) -> impl Iterator<Item = &str> {
    let mut start = 0;
    let mut pieces = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?') {
            if let Some(&(_, next)) = chars.peek() {
                if next.is_whitespace() {
                    let end = i + c.len_utf8();
                    pieces.push(&text[start..end]);
                    start = end;
                }
            }
        }
    }
    pieces.push(&text[start..]);
    pieces.into_iter()
}

fn cache_file(spoken: &str, voice_key: &str, emotion: &str) -> PathBuf {
    let digest = md5_hex(format!("{voice_key}|{emotion}|{spoken}").as_bytes());
    cache_dir().join(format!("cached_{voice_key}_{}.mp3", &digest[..16]))
}

fn md5_hex(data: &[u8]) -> String {
    format!("{:x}", md5::compute(data))
}

/// Keeps the audio cache bounded.
fn prune_cache(max_files: usize) {
    let Ok(entries) = std::fs::read_dir(cache_dir()) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "mp3").unwrap_or(false))
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.path()))
        })
        .collect();
    if files.len() <= max_files {
        return;
    }
    files.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    for (_, path) in files.into_iter().skip(max_files) {
        let _ = std::fs::remove_file(path);
    }
}

/// Synthesizes with Edge neural TTS into `out_file`. Returns true on success.
fn synthesize(text: &str, emotion: &str, voice_key: &str, out_file: &Path) -> bool {
    let preset = preset(voice_key);
    let emo = match emotion {
        "idle" | "talking" | "annoyed" | "walking" => emotion,
        _ => "talking",
    };
    let (pitch, rate) = match emo {
        "idle" => (preset.pitch_idle, preset.rate_idle),
        "annoyed" => (preset.pitch_annoyed, preset.rate_annoyed),
        "walking" => (preset.pitch_walking, preset.rate_walking),
        _ => (preset.pitch_talking, preset.rate_talking),
    };

    let config = SpeechConfig {
        voice_name: preset.edge_voice.to_string(),
        audio_format: "audio-24khz-48kbitrate-mono-mp3".to_string(),
        pitch,
        rate,
        volume: 0,
    };

    let Ok(mut client) = connect() else {
        log::warn!("edge-tts connection failed");
        return false;
    };
    match client.synthesize(text, &config) {
        Ok(audio) => {
            if audio.audio_bytes.len() <= 200 {
                return false;
            }
            let tmp = out_file.with_extension("tmp.mp3");
            let written = std::fs::File::create(&tmp)
                .and_then(|mut f| f.write_all(&audio.audio_bytes));
            if written.is_ok() {
                std::fs::rename(&tmp, out_file).is_ok()
            } else {
                let _ = std::fs::remove_file(&tmp);
                false
            }
        }
        Err(e) => {
            log::warn!("edge-tts synthesis failed: {e}");
            false
        }
    }
}

static OUTPUT_STREAM: OnceLock<Mutex<Option<rodio::MixerDeviceSink>>> = OnceLock::new();

fn output_stream() -> &'static Mutex<Option<rodio::MixerDeviceSink>> {
    OUTPUT_STREAM.get_or_init(|| Mutex::new(None))
}

/// Starts playback of one utterance. The [`LipSync`] wrapper clears `SPEAKING` when it ends.
fn play_file(path: &Path, seq: i64) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let reader = std::io::BufReader::new(file);
    let Ok(source) = rodio::Decoder::try_from(reader) else {
        log::warn!("could not decode {}", path.display());
        return false;
    };
    let source = LipSync::new(source, seq);
    let mut guard = output_stream().lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(handle) => *guard = Some(handle),
            Err(e) => {
                log::warn!("no audio output device: {e}");
                return false;
            }
        }
    }
    if let Some(handle) = guard.as_ref() {
        handle.mixer().add(source);
        return true;
    }
    false
}

/// Speaks text asynchronously; supersedes any in-flight utterance.
pub fn speak_async(text: &str, emotion: &str) {
    if SHUTTING_DOWN.load(Ordering::SeqCst) || !crate::config::get().tts_enabled {
        return;
    }
    let voice_key = crate::config::get().tts_voice;
    let spoken = format_speech(text, &voice_key);
    if spoken.is_empty() {
        return;
    }
    let seq = SPEECH_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    SPEAKING.store(true, Ordering::SeqCst);
    let emotion = emotion.to_string();

    std::thread::spawn(move || {
        let cached = cache_file(&spoken, &voice_key, &emotion);
        let path = if cached.exists()
            && std::fs::metadata(&cached).map(|m| m.len() > 500).unwrap_or(false)
        {
            cached
        } else if synthesize(&spoken, &emotion, &voice_key, &cached) {
            prune_cache(40);
            cached
        } else {
            SPEAKING.store(false, Ordering::SeqCst);
            return;
        };

        if SHUTTING_DOWN.load(Ordering::SeqCst) || SPEECH_SEQ.load(Ordering::SeqCst) != seq {
            return;
        }
        // On success the lip-sync source clears `SPEAKING` when the audio actually ends.
        if !play_file(&path, seq) && SPEECH_SEQ.load(Ordering::SeqCst) == seq {
            SPEAKING.store(false, Ordering::SeqCst);
        }
    });
}

/// Pre-warms the cache with the lines most likely to be spoken on startup.
pub fn precache(lines: &[(String, String)]) {
    let voice_key = crate::config::get().tts_voice;
    let lines: Vec<(String, String)> = lines.to_vec();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        for (raw, emotion) in lines {
            if SHUTTING_DOWN.load(Ordering::SeqCst) {
                break;
            }
            let spoken = format_speech(&raw, &voice_key);
            if spoken.is_empty() {
                continue;
            }
            let target = cache_file(&spoken, &voice_key, &emotion);
            if !target.exists() {
                synthesize(&spoken, &emotion, &voice_key, &target);
            }
            std::thread::sleep(std::time::Duration::from_millis(350));
        }
    });
}

// ---------------------------------------------------------------- microphone

static MIC_FRAMES: OnceLock<Mutex<Vec<i16>>> = OnceLock::new();

fn mic_frames() -> &'static Mutex<Vec<i16>> {
    MIC_FRAMES.get_or_init(|| Mutex::new(Vec::new()))
}

struct RecordingHandle {
    _stream: cpal::Stream,
}

static RECORDING_HANDLE: OnceLock<Mutex<Option<RecordingHandle>>> = OnceLock::new();

fn recording_handle() -> &'static Mutex<Option<RecordingHandle>> {
    RECORDING_HANDLE.get_or_init(|| Mutex::new(None))
}

const SAMPLE_RATE: u32 = 16_000;

fn input_device() -> Option<cpal::Device> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let wanted = crate::config::get().input_audio_device;
    if !wanted.trim().is_empty() {
        if let Ok(devices) = host.input_devices() {
            for device in devices {
                if device.to_string() == wanted {
                    return Some(device);
                }
            }
        }
    }
    host.default_input_device()
}

/// Starts capturing 16 kHz mono frames. Returns false when no microphone is available.
pub fn start_recording() -> bool {
    use cpal::traits::{DeviceTrait, StreamTrait};
    if RECORDING.load(Ordering::SeqCst) {
        return false;
    }
    stop_speaking();
    mic_frames().lock().unwrap_or_else(|e| e.into_inner()).clear();

    let Some(device) = input_device() else {
        log::warn!("no microphone device");
        return false;
    };
    let config = match device.default_input_config() {
        Ok(c) => c.config(),
        Err(e) => {
            log::warn!("microphone config failed: {e}");
            return false;
        }
    };
    let source_rate = config.sample_rate;
    let channels = config.channels as usize;

    let frames = mic_frames();
    let stream = match device.build_input_stream(
        config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if !RECORDING.load(Ordering::SeqCst) {
                return;
            }
            let mut buf = frames.lock().unwrap_or_else(|e| e.into_inner());
            for frame in data.chunks(channels) {
                let avg: f32 = frame.iter().sum::<f32>() / channels as f32;
                buf.push((avg.clamp(-1.0, 1.0) * 32767.0) as i16);
            }
        },
        |err| log::warn!("microphone stream error: {err}"),
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("could not build mic stream: {e}");
            return false;
        }
    };
    if stream.play().is_err() {
        return false;
    }

    *recording_handle().lock().unwrap_or_else(|e| e.into_inner()) = Some(RecordingHandle { _stream: stream });
    MIC_SOURCE_RATE.store(source_rate as i64, Ordering::SeqCst);
    RECORDING.store(true, Ordering::SeqCst);
    true
}

static MIC_SOURCE_RATE: AtomicI64 = AtomicI64::new(SAMPLE_RATE as i64);

pub fn stop_speaking() {
    SPEECH_SEQ.fetch_add(1, Ordering::SeqCst);
    SPEAKING.store(false, Ordering::SeqCst);
}

/// Stops capture and returns 16 kHz mono WAV bytes, or None when nothing usable was recorded.
pub fn stop_recording_wav() -> Option<Vec<u8>> {
    if !RECORDING.load(Ordering::SeqCst) {
        return None;
    }
    RECORDING.store(false, Ordering::SeqCst);
    *recording_handle().lock().unwrap_or_else(|e| e.into_inner()) = None;

    let raw = {
        let mut frames = mic_frames().lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *frames)
    };
    if raw.is_empty() {
        return None;
    }
    let source_rate = MIC_SOURCE_RATE.load(Ordering::SeqCst) as u32;
    let samples = resample(&raw, source_rate, SAMPLE_RATE);
    if samples.len() < (SAMPLE_RATE as usize) / 4 {
        return None;
    }
    Some(crate::audio::wav_bytes(&samples, SAMPLE_RATE))
}

fn resample(input: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    if from % to == 0 {
        let factor = (from / to) as usize;
        let usable = input.len() / factor * factor;
        return input[..usable]
            .chunks_exact(factor)
            .map(|w| (w.iter().map(|s| *s as i32).sum::<i32>() / factor as i32) as i16)
            .collect();
    }
    let out_len = (input.len() as f64 * to as f64 / from as f64) as usize;
    (0..out_len)
        .map(|i| {
            let src = i as f64 * from as f64 / to as f64;
            let idx = (src as usize).min(input.len() - 1);
            input[idx]
        })
        .collect()
}

/// Recognizes speech in a WAV buffer using Edge's free dictation endpoint.
pub fn transcribe_wav(wav: &[u8]) -> String {
    if wav.is_empty() {
        return String::new();
    }
    // Google's free web-speech endpoint used by the Python original: send the raw PCM.
    let pcm = &wav[44.min(wav.len())..];
    let url = "https://www.google.com/speech-api/v2/recognize?client=chromium&lang=en-US&key=AIzaSyBOti4mM-6x9WDnZIjIeyEU21OpBXqWBgw";
    let result = ureq::post(url)
        .header("Content-Type", "audio/l16; rate=16000")
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(12)))
        .build()
        .send(pcm);

    if let Ok(mut response) = result {
        if let Ok(text) = response.body_mut().read_to_string() {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                if let Some(transcript) = value
                    .get("result")
                    .and_then(|r| r.as_array())
                    .and_then(|r| r.first())
                    .and_then(|r| r.get("alternative"))
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|a| a.get("transcript"))
                    .and_then(|t| t.as_str())
                {
                    let clean = transcript.trim();
                    if !clean.is_empty() {
                        return clean.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

/// Energy VAD loop: listens for speech, transcribes, then hands the text to `on_command`.
pub fn start_wake_word_listener(on_command: fn(String)) {
    std::thread::spawn(move || {
        use cpal::traits::{DeviceTrait, StreamTrait};
        let Some(device) = input_device() else {
            return;
        };
        let Ok(config) = device.default_input_config() else {
            return;
        };
        let config = config.config();
        let source_rate = config.sample_rate;
        let channels = config.channels as usize;

        let chunk_samples = (source_rate as usize / 5).max(1024);
        let buffer: std::sync::Arc<Mutex<Vec<i16>>> =
            std::sync::Arc::new(Mutex::new(Vec::with_capacity(chunk_samples * 8)));
        let buf_clone = buffer.clone();
        let stream = match device.build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mut guard = buf_clone.lock().unwrap_or_else(|e| e.into_inner());
                for frame in data.chunks(channels) {
                    let avg: f32 = frame.iter().sum::<f32>() / channels as f32;
                    guard.push((avg.clamp(-1.0, 1.0) * 32767.0) as i16);
                }
            },
            |err| log::warn!("wake-word mic error: {err}"),
            None,
        ) {
            Ok(s) => s,
            Err(_) => return,
        };
        if stream.play().is_err() {
            return;
        }

        loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if SHUTTING_DOWN.load(Ordering::SeqCst) {
                break;
            }
            let settings = crate::config::get();
            let always_open = settings.mic_mode == "always_open";
            if (!always_open && !settings.wake_word_enabled)
                || RECORDING.load(Ordering::SeqCst)
                || SPEAKING.load(Ordering::SeqCst)
            {
                buffer.lock().unwrap_or_else(|e| e.into_inner()).clear();
                continue;
            }

            let chunk = {
                let mut guard = buffer.lock().unwrap_or_else(|e| e.into_inner());
                let take = guard.len().min(chunk_samples * 3);
                if take == 0 {
                    continue;
                }
                let drained: Vec<i16> = guard.drain(..take).collect();
                drained
            };
            let rms = (chunk.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / chunk.len() as f64).sqrt();
            if rms < 580.0 {
                continue;
            }

            // Collect a phrase: up to ~5.6 s of speech, stopping after ~0.8 s of silence.
            let mut phrase = chunk;
            let mut silent = 0;
            while phrase.len() < chunk_samples * 28 {
                std::thread::sleep(std::time::Duration::from_millis(200));
                let next = {
                    let mut guard = buffer.lock().unwrap_or_else(|e| e.into_inner());
                    let take = guard.len().min(chunk_samples);
                    let drained: Vec<i16> = guard.drain(..take).collect();
                    drained
                };
                if next.is_empty() {
                    silent += 1;
                    if silent >= 4 {
                        break;
                    }
                    continue;
                }
                let level = (next.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / next.len() as f64).sqrt();
                silent = if level > 580.0 { 0 } else { silent + 1 };
                phrase.extend(next);
                if silent >= 4 {
                    break;
                }
            }

            if phrase.len() < chunk_samples * 2 {
                continue;
            }
            let resampled = resample(&phrase, source_rate, SAMPLE_RATE);
            let wav = crate::audio::wav_bytes(&resampled, SAMPLE_RATE);
            let text = transcribe_wav(&wav);
            let text = text.trim().to_string();
            if text.is_empty() {
                continue;
            }

            if always_open {
                on_command(text);
            } else {
                let lower = text.to_lowercase();
                for trigger in ["hey aria", "hi aria", "okay aria", "ok aria", "yo aria", "aria"] {
                    if let Some(pos) = lower.find(trigger) {
                        let after = text[pos + trigger.len()..]
                            .trim_matches(|c: char| " ,.!?:;-".contains(c))
                            .trim()
                            .to_string();
                        on_command(if after.is_empty() { "Hey Aria!".to_string() } else { after });
                        break;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_format_strips_markdown_but_keeps_words() {
        let out = format_speech("[EMOTION: happy] Fine. **Good** job `today`.", "cloned_tsundere_1");
        assert!(out.contains("Good"));
        assert!(!out.contains('*'));
        assert!(!out.contains('`'));
    }

    #[test]
    fn stutter_letters_collapse() {
        let out = format_speech("B-Baka! I-It's fine.", "cloned_tsundere_1");
        assert!(out.to_lowercase().contains("baaka"), "got {out}");
        assert!(out.to_lowercase().contains("it's"), "got {out}");
    }

    #[test]
    fn long_text_is_trimmed_for_synthesis() {
        let long = "This is a sentence. ".repeat(20);
        let out = format_speech(&long, "cloned_tsundere_1");
        assert!(out.chars().count() <= 190, "got {}", out.chars().count());
    }

    #[test]
    fn preset_lookup_falls_back() {
        assert_eq!(preset("nope").key, VOICES[0].key);
        assert_eq!(preset("nanami_jp").edge_voice, "ja-JP-NanamiNeural");
    }
}
