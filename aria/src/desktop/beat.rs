//! Finding the beat of whatever is playing, so she dances in time.
//!
//! A worker samples the music app's session peak meter at 50 Hz while music plays. From that
//! envelope, [`BeatTracker`] takes the onset strength (rises in loudness), estimates the tempo
//! by autocorrelation over the last eight seconds, and locks the phase with a comb filter. The
//! result ([`Music`]) is the tempo, where the last beat fell, how sure it is, and how loud the
//! music is. Nothing here is fancy DSP; it only needs to be good enough that her steps land on
//! the beat of ordinary pop, EDM, hip-hop and Bollywood tracks.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Samples per second of the envelope.
pub const RATE: f32 = 50.0;
const WINDOW_SECS: f32 = 8.0;
const MIN_BPM: f32 = 70.0;
const MAX_BPM: f32 = 180.0;
/// Tempo prior: most dance music sits near 120 BPM; this resolves half/double-tempo ambiguity.
const PRIOR_BPM: f32 = 120.0;
/// Below this autocorrelation strength the tempo is a guess and is not reported.
const MIN_CONFIDENCE: f32 = 0.12;

/// The beat estimate published to the app.
#[derive(Debug, Clone, Copy)]
pub struct Music {
    pub bpm: f32,
    /// 0..1: how periodic the music is. Ambient or speech scores low.
    pub confidence: f32,
    /// When a beat fell.
    pub beat_at: Instant,
    /// Loudness, 0..1.
    pub energy: f32,
}

impl Music {
    /// Where in the beat `at` falls: 0 on the beat, rising to 1 just before the next.
    pub fn phase_at(&self, at: Instant) -> f32 {
        let period = 60.0 / self.bpm.max(1.0);
        let since = at.saturating_duration_since(self.beat_at).as_secs_f32();
        (since / period).rem_euclid(1.0)
    }
}

/// Envelope in, tempo and phase out. Pure: feed it samples, read estimates.
pub struct BeatTracker {
    /// Onset strength, oldest first.
    onsets: std::collections::VecDeque<f32>,
    last_peak: f32,
    slow: f32,
    pub energy: f32,
}

impl Default for BeatTracker {
    fn default() -> Self {
        Self { onsets: std::collections::VecDeque::new(), last_peak: 0.0, slow: 0.0, energy: 0.0 }
    }
}

impl BeatTracker {
    fn capacity() -> usize {
        (WINDOW_SECS * RATE) as usize
    }

    /// Adds one envelope sample (a peak meter reading, 0..1).
    pub fn push(&mut self, peak: f32) {
        let peak = peak.clamp(0.0, 1.0);
        // Rises above a slow average are onsets; falls and sustained notes are not.
        self.slow += (peak - self.slow) * 0.08;
        let onset = (peak - self.last_peak).max(0.0) + 0.5 * (peak - self.slow).max(0.0);
        self.last_peak = peak;
        // Music session peaks sit around 0.15-0.6; map that onto 0..1.
        self.energy += ((peak * 1.6).min(1.0) - self.energy) * 0.02;
        self.onsets.push_back(onset);
        if self.onsets.len() > Self::capacity() {
            self.onsets.pop_front();
        }
    }

    /// Tempo in BPM and confidence, once there are four seconds of signal.
    pub fn tempo(&self) -> Option<(f32, f32)> {
        if self.onsets.len() < Self::capacity() / 2 {
            return None;
        }
        let x: Vec<f32> = {
            let mean = self.onsets.iter().sum::<f32>() / self.onsets.len() as f32;
            self.onsets.iter().map(|v| v - mean).collect()
        };
        let energy: f32 = x.iter().map(|v| v * v).sum();
        if energy <= 1e-9 {
            return None;
        }
        let lag_of = |bpm: f32| 60.0 * RATE / bpm;
        let (min_lag, max_lag) = (lag_of(MAX_BPM).floor() as usize, lag_of(MIN_BPM).ceil() as usize);
        let acf = |lag: usize| -> f32 { x.iter().zip(&x[lag..]).map(|(a, b)| a * b).sum::<f32>() / energy };
        let scores: Vec<(usize, f32, f32)> = (min_lag.max(2)..=max_lag.min(x.len() / 2))
            .map(|lag| {
                let raw = acf(lag);
                let bpm = 60.0 * RATE / lag as f32;
                // A log-Gaussian prior around 120 BPM, gentle enough not to override clear beats.
                let prior = (-(bpm / PRIOR_BPM).log2().powi(2) / (2.0 * 0.6f32.powi(2))).exp();
                (lag, raw, raw * (0.6 + 0.4 * prior))
            })
            .collect();
        let best = scores.iter().copied().max_by(|a, b| a.2.total_cmp(&b.2))?;
        let (lag, raw) = (best.0, best.1);
        if raw < MIN_CONFIDENCE {
            return None;
        }
        // Sub-sample refinement from the neighbours.
        let (l, r) = (acf(lag - 1), acf(lag + 1));
        let denom = l - 2.0 * raw + r;
        let shift = if denom.abs() > 1e-6 { (0.5 * (l - r) / denom).clamp(-0.5, 0.5) } else { 0.0 };
        let bpm = 60.0 * RATE / (lag as f32 + shift);
        Some((bpm.clamp(MIN_BPM, MAX_BPM), raw.min(1.0)))
    }

    /// How many samples ago the most recent beat fell, for a given tempo.
    pub fn beat_offset(&self, bpm: f32) -> f32 {
        let period = 60.0 * RATE / bpm;
        let n = self.onsets.len();
        let steps = 32;
        // Comb filter: the phase whose teeth collect the most onset energy.
        let mut best = (0.0f32, f32::MIN);
        for k in 0..steps {
            let offset = period * k as f32 / steps as f32;
            let mut score = 0.0;
            let mut tooth = offset;
            while (tooth as usize) < n {
                let i = n - 1 - tooth as usize;
                // A little width around each tooth.
                score += self.onsets[i] + 0.5 * i.checked_sub(1).map_or(0.0, |j| self.onsets[j]);
                tooth += period;
            }
            if score > best.1 {
                best = (offset, score);
            }
        }
        best.0
    }
}

// ---------------------------------------------------------------------------------- live monitor

static LISTENING: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static LATEST: Mutex<Option<Music>> = Mutex::new(None);
/// The exe of the app playing music, so other apps' sounds (and Aria's own voice) are ignored.
static SOURCE_EXE: Mutex<String> = Mutex::new(String::new());

/// Starts or pauses beat tracking. `source_exe` is the music app (`spotify.exe`, `chrome.exe`),
/// or empty for "any app but Aria".
pub fn listen(on: bool, source_exe: &str) {
    *SOURCE_EXE.lock().unwrap_or_else(|e| e.into_inner()) = source_exe.to_lowercase();
    LISTENING.store(on, Ordering::Relaxed);
    if !on {
        *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = None;
        return;
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new().name("aria-beat".into()).spawn(run);
    if spawned.is_err() {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// The latest estimate while music plays.
pub fn latest() -> Option<Music> {
    *LATEST.lock().unwrap_or_else(|e| e.into_inner())
}

fn run() {
    let ours = std::process::id();
    let mut tracker = BeatTracker::default();
    let mut meters: Option<crate::desktop::audio::MeterSet> = None;
    let mut last_estimate = Instant::now();
    let tick = Duration::from_secs_f32(1.0 / RATE);
    while LISTENING.load(Ordering::Relaxed) {
        let started = Instant::now();
        // New players appear and old ones go; re-enumerate the sessions every few seconds.
        if meters.as_ref().is_none_or(|m| m.age() > Duration::from_secs(4)) {
            meters = crate::desktop::audio::MeterSet::open();
        }
        let source = SOURCE_EXE.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let peak = meters.as_ref().map_or(0.0, |m| {
            m.peak(|pid| {
                if pid == ours {
                    return false;
                }
                let exe = crate::desktop::win32::process_name(pid);
                if source.is_empty() { exe != "aria-avatar.exe" } else { exe == source }
            })
        });
        tracker.push(peak);
        if last_estimate.elapsed() >= Duration::from_millis(500) {
            last_estimate = Instant::now();
            let estimate = tracker.tempo().map(|(bpm, confidence)| {
                let ago = tracker.beat_offset(bpm) / RATE;
                Music { bpm, confidence, beat_at: Instant::now() - Duration::from_secs_f32(ago), energy: tracker.energy }
            });
            // Unclear rhythm still reports loudness, at a neutral tempo with no confidence.
            let music = estimate.unwrap_or(Music { bpm: PRIOR_BPM, confidence: 0.0, beat_at: Instant::now(), energy: tracker.energy });
            *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = Some(music);
        }
        std::thread::sleep(tick.saturating_sub(started.elapsed()));
    }
    RUNNING.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kick drum: a sharp peak decaying over 120 ms, every beat, over a little noise.
    fn track(bpm: f32, seconds: f32, offbeat: bool) -> BeatTracker {
        let mut tracker = BeatTracker::default();
        let period = 60.0 / bpm;
        let mut seed = 7u32;
        for i in 0..(seconds * RATE) as usize {
            let t = i as f32 / RATE;
            let since = t.rem_euclid(period);
            let mut peak = 0.15 + 0.5 * (-since / 0.12).exp();
            if offbeat {
                // A hi-hat halfway between kicks, quieter.
                peak += 0.15 * (-(t + period / 2.0).rem_euclid(period) / 0.05).exp();
            }
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            peak += (seed >> 16) as f32 / 65_536.0 * 0.04;
            tracker.push(peak);
        }
        tracker
    }

    #[test]
    fn finds_common_dance_tempos() {
        for bpm in [92.0, 105.0, 120.0, 128.0, 140.0] {
            let (found, confidence) = track(bpm, 10.0, true).tempo().expect("a clear beat");
            assert!((found - bpm).abs() < 3.0, "{bpm} BPM read as {found}");
            assert!(confidence > 0.3, "{bpm}: confidence {confidence}");
        }
    }

    #[test]
    fn locks_onto_the_kick_not_the_offbeat() {
        let bpm = 120.0;
        let tracker = track(bpm, 10.0, true);
        let period = 60.0 * RATE / bpm;
        let n = tracker.onsets.len() as f32;
        // The last kick fell at the last multiple of the period.
        let expected = (n - 1.0).rem_euclid(period);
        let found = tracker.beat_offset(bpm);
        let error = (found - expected).abs().min(period - (found - expected).abs());
        assert!(error <= 2.0, "beat offset {found} vs {expected} samples");
    }

    #[test]
    fn silence_and_noise_have_no_tempo() {
        let mut quiet = BeatTracker::default();
        for _ in 0..500 {
            quiet.push(0.0);
        }
        assert!(quiet.tempo().is_none());
        let mut noise = BeatTracker::default();
        let mut seed = 1u32;
        for _ in 0..500 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            noise.push((seed >> 16) as f32 / 65_536.0 * 0.3);
        }
        assert!(noise.tempo().is_none_or(|(_, c)| c < 0.3), "noise is not a beat");
    }

    #[test]
    fn phase_advances_with_time() {
        let now = Instant::now();
        let music = Music { bpm: 120.0, confidence: 1.0, beat_at: now, energy: 0.5 };
        assert!((music.phase_at(now + Duration::from_millis(250)) - 0.5).abs() < 0.01);
        assert!(music.phase_at(now + Duration::from_millis(500)) < 0.01);
    }
}
