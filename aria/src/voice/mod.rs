//! Her voice and her ears.
//!
//! | Module | What |
//! |---|---|
//! | [`tts`] | Edge neural voices, the clip cache, playback |
//! | [`lipsync`] | measures the playing audio for the mouth |
//! | [`mic`] | microphone capture on the chosen input device |
//! | [`stt`] | speech-to-text |
//! | [`listener`] | hands-free listening (always-open mic, wake word) |
//!
//! Mic modes: **click-to-talk** toggles a recording with the mic button or Ctrl+Shift+M;
//! **push-to-talk** records while Ctrl+Shift+Space is held; **always-open** submits every phrase
//! the listener hears. The wake word works in any mode.

pub mod lipsync;
pub mod listener;
pub mod mic;
pub mod stt;
pub mod tts;

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

/// Current utterance number. Playback of an older one is abandoned.
static SPEECH_SEQ: AtomicI64 = AtomicI64::new(0);
static SPEAKING: AtomicBool = AtomicBool::new(false);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static RECORDING: AtomicBool = AtomicBool::new(false);

pub use lipsync::mouth_levels;
pub use mic::{start_recording, stop_recording_wav};
pub use stt::transcribe_wav;
pub use tts::{precache, speak_async};

/// True while her voice is actually playing.
pub fn is_speaking() -> bool {
    SPEAKING.load(Ordering::SeqCst)
}

/// True while a click- or push-to-talk recording is open.
pub fn is_recording() -> bool {
    RECORDING.load(Ordering::SeqCst)
}

/// Cuts off whatever she is saying.
pub fn stop_speaking() {
    SPEECH_SEQ.fetch_add(1, Ordering::SeqCst);
    SPEAKING.store(false, Ordering::SeqCst);
}

pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    stop_speaking();
}
