//! Hands-free listening: an energy VAD that waits for speech, records the phrase and hands the
//! transcript on.
//!
//! It runs while the mic mode is **Always-Open** (every phrase is a request) or the **wake word**
//! is on ("hey Aria, …" in any mode). [`apply`] is called whenever those settings or the input
//! device change; it starts, stops or restarts the listener to match, so a change in the
//! Control Center takes effect immediately.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::mic::{self, Capture};
use super::{RECORDING, SHUTTING_DOWN, SPEAKING};

/// Speech is louder than this RMS (16-bit samples).
const SPEECH_RMS: f64 = 580.0;
/// A phrase ends after this much quiet.
const END_SILENCE: Duration = Duration::from_millis(800);
const MAX_PHRASE: Duration = Duration::from_millis(8000);
const MIN_PHRASE: Duration = Duration::from_millis(350);
const STEP: Duration = Duration::from_millis(100);

const WAKE_WORDS: &[&str] = &["hey aria", "hi aria", "okay aria", "ok aria", "yo aria", "aria"];

/// Bumped on every [`apply`]; a listener thread exits when its generation is stale.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// The configuration the running listener was started for.
static RUNNING_FOR: Mutex<Option<Config>> = Mutex::new(None);

#[derive(Clone, PartialEq, Debug)]
struct Config {
    always_open: bool,
    wake_word: bool,
    device: String,
}

/// What to do with a heard phrase: the text to submit.
pub type OnPhrase = fn(String);

/// Starts, stops or restarts the listener to match the current settings.
pub fn apply(on_phrase: OnPhrase) {
    let s = crate::config::get();
    let wanted = Config { always_open: s.mic_mode == "always_open", wake_word: s.wake_word_enabled, device: s.input_audio_device };
    let needed = wanted.always_open || wanted.wake_word;
    let mut running = RUNNING_FOR.lock().unwrap_or_else(|e| e.into_inner());
    if needed && running.as_ref() == Some(&wanted) {
        return;
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    if !needed {
        if running.take().is_some() {
            log::info!("hands-free listening off");
        }
        return;
    }
    log::info!(
        "hands-free listening on ({}, device: {})",
        if wanted.always_open { "always open" } else { "wake word" },
        if wanted.device.is_empty() { "default" } else { &wanted.device }
    );
    *running = Some(wanted.clone());
    let spawned = std::thread::Builder::new()
        .name("aria-listener".into())
        .spawn(move || run(generation, wanted, on_phrase));
    if let Err(e) = spawned {
        log::warn!("could not start the listener: {e}");
        *running = None;
    }
}

fn current(generation: u64) -> bool {
    GENERATION.load(Ordering::SeqCst) == generation && !SHUTTING_DOWN.load(Ordering::SeqCst)
}

fn run(generation: u64, config: Config, on_phrase: OnPhrase) {
    let capture = match Capture::open() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("hands-free listening unavailable: {e}");
            let mut running = RUNNING_FOR.lock().unwrap_or_else(|e| e.into_inner());
            // Only forget it if no newer listener has been asked for meanwhile, so the next
            // `apply` (a device change, say) tries again.
            if GENERATION.load(Ordering::SeqCst) == generation {
                *running = None;
            }
            return;
        }
    };
    let step_samples = (capture.source_rate as f64 * STEP.as_secs_f64()) as usize;
    while current(generation) {
        std::thread::sleep(STEP);
        // Her own voice and push/click recordings are not requests.
        if RECORDING.load(Ordering::SeqCst) || SPEAKING.load(Ordering::SeqCst) {
            capture.clear();
            continue;
        }
        let block = capture.take(None);
        if mic::rms(&block) < SPEECH_RMS {
            continue;
        }
        let phrase = record_phrase(&capture, block, step_samples, generation);
        if phrase.len() < (capture.source_rate as f64 * MIN_PHRASE.as_secs_f64()) as usize {
            continue;
        }
        let samples = mic::resample(&phrase, capture.source_rate, mic::SAMPLE_RATE);
        let text = super::stt::transcribe_wav(&crate::desktop::audio::wav_bytes(&samples, mic::SAMPLE_RATE));
        if let Some(request) = to_request(&text, config.always_open) {
            log::info!("heard: {request}");
            on_phrase(request);
        }
    }
}

/// Keeps recording until the speaker pauses or the phrase gets too long.
fn record_phrase(capture: &Capture, first: Vec<i16>, step_samples: usize, generation: u64) -> Vec<i16> {
    let mut phrase = first;
    let started = Instant::now();
    let mut quiet_since: Option<Instant> = None;
    while current(generation) && started.elapsed() < MAX_PHRASE {
        std::thread::sleep(STEP);
        let block = capture.take(Some(step_samples * 4));
        let loud = mic::rms(&block) >= SPEECH_RMS;
        phrase.extend(block);
        match (loud, quiet_since) {
            (true, _) => quiet_since = None,
            (false, None) => quiet_since = Some(Instant::now()),
            (false, Some(since)) if since.elapsed() >= END_SILENCE => break,
            _ => {}
        }
    }
    phrase
}

/// The request in a transcript: everything, in always-open mode; what follows the wake word
/// otherwise ("hey Aria" alone is a greeting).
fn to_request(transcript: &str, always_open: bool) -> Option<String> {
    let text = transcript.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_lowercase();
    for wake in WAKE_WORDS {
        if let Some(pos) = lower.find(wake) {
            let after = text[pos + wake.len()..].trim_matches(|c: char| " ,.!?:;-".contains(c)).trim();
            return Some(if after.is_empty() { "Hey Aria!".to_string() } else { after.to_string() });
        }
    }
    always_open.then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_words_and_always_open() {
        assert_eq!(to_request("Hey Aria, open Spotify", false).as_deref(), Some("open Spotify"));
        assert_eq!(to_request("hey aria", false).as_deref(), Some("Hey Aria!"));
        assert_eq!(to_request("what's the weather", false), None, "no wake word, not always open");
        assert_eq!(to_request("what's the weather", true).as_deref(), Some("what's the weather"));
        assert_eq!(to_request("  ", true), None);
    }
}
