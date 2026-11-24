//! Finding the beat of whatever is playing, so she dances in time.
//!
//! A worker samples the music app's session peak meter at 50 Hz while music plays. From that
//! envelope, [`BeatTracker`] takes the onset strength (rises in loudness), estimates the tempo
//! by autocorrelation over the last eight seconds, and locks the phase with a comb filter. The
//! result ([`Music`]) is the tempo, where the last beat fell, how sure it is, and how loud the
//! music is. Nothing here is fancy DSP; it only needs to be good enough that her steps land on
//! the beat of ordinary pop, EDM, hip-hop and Bollywood tracks.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Samples per second of the envelope.
pub const RATE: f32 = 50.0;
const WINDOW_SECS: f32 = 8.0;
const MIN_BPM: f32 = 45.0;
const MAX_BPM: f32 = 180.0;
/// How long a locked tempo survives without a fresh measurement. A quiet bar, a breakdown or a
/// metadata hiccup must not drop her back to a slow sway in the middle of a song.
const HOLD: Duration = Duration::from_secs(8);
/// Tempo prior: most dance music sits near 120 BPM; this resolves half/double-tempo ambiguity.
const PRIOR_BPM: f32 = 120.0;
/// Below this autocorrelation strength the tempo is a guess and is not reported.
pub const MIN_CONFIDENCE: f32 = 0.12;

/// The beat estimate published to the app.
#[derive(Debug, Clone, Copy)]
pub struct Music {
    pub bpm: f32,
    /// 0..1: how periodic the music is. Ambient or speech scores low.
    pub confidence: f32,
    /// When a beat fell.
    pub beat_at: Instant,
    /// Smoothed session level in dBFS, mapped from -48..-6 dB to 0..1.
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
    power: f32,
    /// The loudest power seen recently, for the automatic gain below.
    gain: f32,
    pub energy: f32,
}

impl Default for BeatTracker {
    fn default() -> Self {
        Self { onsets: std::collections::VecDeque::new(), last_peak: 0.0, slow: 0.0, power: 0.0, gain: 0.0, energy: 0.0 }
    }
}

impl BeatTracker {
    fn capacity() -> usize {
        (WINDOW_SECS * RATE) as usize
    }

    /// Adds one envelope sample (a peak meter reading, 0..1).
    pub fn push(&mut self, peak: f32) {
        let peak = if peak.is_finite() { peak.clamp(0.0, 1.0) } else { 0.0 };
        // Rises above a slow average are onsets; falls and sustained notes are not.
        self.slow += (peak - self.slow) * 0.08;
        let onset = (peak - self.last_peak).max(0.0) + 0.5 * (peak - self.slow).max(0.0);
        self.last_peak = peak;
        // Session meters supply peaks, so this is a digital level estimate, not acoustic SPL.
        // Quick attack and a longer release preserve accents without flickering the face.
        let power = peak * peak;
        self.power += (power - self.power) * if power > self.power { 0.22 } else { 0.045 };
        // Track the loudest the song has been recently. Session meters are uncalibrated: what a
        // quiet master delivers at -40 dBFS a loud one delivers at -14, and judging both on
        // absolute level is why some songs never reached the point of dancing.
        self.gain += (power - self.gain) * if power > self.gain { 0.35 } else { 0.0025 };
        let dbfs = 10.0 * self.power.max(1e-12).log10();
        let absolute = ((dbfs + 54.0) / 48.0).clamp(0.0, 1.0);
        // How loud this is against the song's own recent peak: 1 at its loudest passages.
        let relative = (self.power / self.gain.max(1e-9)).clamp(0.0, 1.0);
        // Absolute level sets whether there is anything to hear at all (so real silence is
        // exactly 0 and a hiss is not "loud"); within the song, its own peak sets the scale,
        // so a quiet master still gets everything from a groove to a full dance.
        self.energy = absolute.sqrt() * (0.35 + 0.65 * relative);
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

/// Real mixes have much weaker autocorrelation than a synthetic kick. Require repeated
/// agreement, rather than rejecting a steady beat just because vocals lower its score.
#[derive(Default)]
struct RhythmLock {
    stable: Option<(f32, f32)>,
    candidate: Option<(f32, u8)>,
    /// How long the tempo has gone unmeasured. Silence, a spoken interlude or a breakdown ends a
    /// rhythm; a quiet bar or a lost estimate does not.
    missing_for: Duration,
}

impl RhythmLock {
    /// `dt` is how much audio the measurement covers; misses accumulate real time, not counts,
    /// so a change in how often estimates arrive cannot change the lock's patience.
    fn update(&mut self, measured: Option<(f32, f32)>, dt: Duration) -> Option<(f32, f32)> {
        if let Some((bpm, confidence)) = measured.filter(|(b, c)| b.is_finite() && *c >= MIN_CONFIDENCE) {
            self.missing_for = Duration::ZERO;
            if let Some((previous, _)) = self.stable {
                if (bpm / previous - 1.0).abs() < 0.08 {
                    self.stable = Some((previous + 0.25 * (bpm - previous), confidence));
                    self.candidate = None;
                    return self.stable;
                }
            }
            let (candidate, count) = match self.candidate {
                Some((previous, count)) if (bpm / previous - 1.0).abs() < 0.08 => ((previous + bpm) * 0.5, count + 1),
                _ => (bpm, 1),
            };
            self.candidate = Some((candidate, count));
            if count >= 3 {
                self.stable = Some((candidate, confidence));
                self.candidate = None;
            }
        } else {
            self.candidate = None;
            self.missing_for += dt;
            // Only genuine quiet ends a rhythm; a breakdown or a failed estimate keeps the tempo.
            if self.missing_for >= HOLD {
                self.stable = None;
            }
        }
        self.stable
    }
}

static LISTENING: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static LATEST: Mutex<Option<Music>> = Mutex::new(None);
/// The exe of the app playing music, so other apps' sounds (and Aria's own voice) are ignored.
static SOURCE: Mutex<(String, String)> = Mutex::new((String::new(), String::new()));
/// Set when the song or the app changed, so the worker starts the new one from a clean slate.
static RESET: AtomicBool = AtomicBool::new(false);
static WAKE: Condvar = Condvar::new();

/// Starts or pauses beat tracking. `source_exe` is the music app (`spotify.exe`, `chrome.exe`),
/// or empty for "any app but Aria". A new track starts the old song's beat history over.
///
/// An empty track label is *not* a new song. Media metadata is queried with a timeout and
/// occasionally comes back blank while the music plays on, so treating that as a new track threw
/// the tempo away mid-song and left her swaying until it locked again.
pub fn listen(on: bool, source_exe: &str, track: &str) {
    let exe = source_exe.to_lowercase();
    let track = track.trim();
    let mut source = SOURCE.lock().unwrap_or_else(|e| e.into_inner());
    let changed = if exe != source.0 {
        true
    } else if track.is_empty() {
        // A blank label means "still the same song, metadata unavailable this poll".
        false
    } else {
        // Extras after the title (a "(Remastered)" suffix appearing late) are not a new song.
        let same = !source.1.is_empty() && (source.1.contains(track) || track.contains(source.1.as_str()));
        !same
    };
    if changed {
        source.1 = if track.is_empty() { String::new() } else { track.to_string() };
        *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = None;
        RESET.store(true, Ordering::SeqCst);
    }
    source.0 = exe;
    LISTENING.store(on, Ordering::Relaxed);
    drop(source);
    WAKE.notify_one();
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
    let mut selected_pids = std::collections::HashSet::new();
    let mut selected_source = String::new();
    let mut last_estimate = Instant::now();
    let mut estimate = None;
    let mut rhythm = RhythmLock::default();
    let tick = Duration::from_secs_f32(1.0 / RATE);
    // Keep one worker alive across pause/resume, avoiding a lost restart while it exits.
    loop {
        if !LISTENING.load(Ordering::Relaxed) {
            tracker = BeatTracker::default();
            rhythm = RhythmLock::default();
            estimate = None;
            meters = None;
            let guard = SOURCE.lock().unwrap_or_else(|e| e.into_inner());
            drop(WAKE.wait_while(guard, |_| !LISTENING.load(Ordering::Relaxed))
                .unwrap_or_else(|e| e.into_inner()));
            last_estimate = Instant::now();
            continue;
        }
        let started = Instant::now();
        let source = SOURCE.lock().unwrap_or_else(|e| e.into_inner()).clone();
        // A new song starts from scratch; a metadata hiccup flags nothing, so the tempo it had
        // found carries over instead of being re-acquired every poll.
        if RESET.swap(false, Ordering::SeqCst) {
            tracker = BeatTracker::default();
            rhythm = RhythmLock::default();
            estimate = None;
            last_estimate = Instant::now();
        }
        // New players appear and old ones go; re-enumerate the sessions every few seconds.
        let refresh = meters.as_ref().is_none_or(|m| m.age() > Duration::from_secs(4));
        if refresh {
            meters = crate::desktop::audio::MeterSet::open();
        }
        if refresh || selected_source != source.0 {
            selected_source.clone_from(&source.0);
            selected_pids.clear();
            if let Some(meters) = meters.as_ref() {
                selected_pids.extend(meters.process_ids().filter(|&pid| {
                    if pid == 0 || pid == ours { return false; }
                    let exe = crate::desktop::win32::process_name(pid);
                    if source.0.is_empty() { exe != "aria-avatar.exe" } else { exe == source.0 }
                }));
            }
        }
        let peak = meters.as_ref().map_or(0.0, |m| m.peak(|pid| selected_pids.contains(&pid)));
        tracker.push(peak);
        if last_estimate.elapsed() >= Duration::from_millis(500) {
            let took = last_estimate.elapsed();
            last_estimate = Instant::now();
            // A quiet passage keeps the tempo it already found: silence ends a rhythm after
            // `HOLD`, but a breath between bars, a breakdown or a spoken intro does not. This
            // used to reset the lock on the first quiet sample, which is why she dropped from
            // dancing to a slow sway at random points in a song.
            estimate = rhythm.update(tracker.tempo(), took).map(|(bpm, confidence)| {
                let ago = tracker.beat_offset(bpm) / RATE;
                (bpm, confidence, started - Duration::from_secs_f32(ago))
            });
        }
        // Publish the level at meter speed; tempo estimation is the slower operation. Before the
        // first lock there is no tempo at all (0) rather than a made-up one: reporting a default
        // 78 BPM left her swaying through fast songs she had not locked onto yet.
        let (bpm, confidence, beat_at) = estimate.unwrap_or((0.0, 0.0, started));
        let music = Music { bpm, confidence, beat_at, energy: tracker.energy };
        let current_source = SOURCE.lock().unwrap_or_else(|e| e.into_inner());
        if LISTENING.load(Ordering::Relaxed) && *current_source == source {
            *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = Some(music);
        }
        drop(current_source);
        std::thread::sleep(tick.saturating_sub(started.elapsed()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "reads the currently playing music and audio meters for twelve seconds"]
    fn live_music_probe() {
        eprintln!("audio sessions: {:?}", crate::desktop::audio::audio_activity(200));
        eprintln!("reading system media sessions...");
        let sessions = crate::desktop::media::sessions();
        eprintln!("media sessions: {sessions:?}");
        let playing = crate::desktop::media::now_playing();
        eprintln!("selected: {playing:?}");
        listen(playing.is_playing, &playing.exe, &playing.label());
        let meters = crate::desktop::audio::MeterSet::open().expect("audio meters available");
        let mut tracker = BeatTracker::default();
        let started = Instant::now();
        let mut samples = 0;
        let mut report = Instant::now();
        while started.elapsed() < Duration::from_secs(12) {
            let tick_start = Instant::now();
            let peak = meters.peak(|pid| crate::desktop::win32::process_name(pid) == playing.exe);
            tracker.push(peak);
            samples += 1;
            if report.elapsed() > Duration::from_secs(1) {
                report = Instant::now();
                eprintln!("{:.1}s peak={peak:.4} level={:.3} tempo={:?} production={:?} samples={samples}", started.elapsed().as_secs_f32(), tracker.energy, tracker.tempo(), latest().map(|m| (m.energy, m.bpm, m.confidence)));
            }
            std::thread::sleep(Duration::from_secs_f32(1.0 / RATE).saturating_sub(tick_start.elapsed()));
        }
        let production = latest();
        listen(false, "", "");
        if playing.is_playing && tracker.energy > 0.1 {
            assert!(production.is_some_and(|m| m.energy > 0.1), "the production worker must receive audible music");
        }
    }

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
    fn real_mix_confidence_can_lock_without_chasing_half_tempo_outliers() {
        let mut rhythm = RhythmLock::default();
        let step = Duration::from_millis(500);
        assert!(rhythm.update(Some((121.0, 0.16)), step).is_none());
        assert!(rhythm.update(Some((120.0, 0.18)), step).is_none());
        assert!((rhythm.update(Some((120.5, 0.17)), step).unwrap().0 - 120.5).abs() < 1.0);
        assert!(rhythm.update(Some((80.0, 0.2)), step).unwrap().0 > 119.0);
        assert!(rhythm.update(None, step).unwrap().0 > 119.0, "a quiet break preserves tempo");
        rhythm.update(Some((65.0, 0.2)), step);
        rhythm.update(Some((64.5, 0.18)), step);
        assert!((rhythm.update(Some((65.0, 0.17)), step).unwrap().0 - 65.0).abs() < 1.0);
        for _ in 0..(HOLD.as_millis() as usize / 500) { rhythm.update(None, step); }
        assert!(rhythm.stable.is_none(), "a lost rhythm eventually releases");
    }

    #[test]
    fn a_dropout_in_the_metadata_does_not_start_the_song_again() {
        // Two seconds without metadata (a slow WinRT query) must not throw the tempo away.
        let mut rhythm = RhythmLock::default();
        let step = Duration::from_millis(500);
        for _ in 0..3 { rhythm.update(Some((128.0, 0.2)), step); }
        assert!(rhythm.stable.is_some());
        for _ in 0..4 {
            let kept = rhythm.update(None, step).expect("the tempo survives a short gap");
            assert!((kept.0 - 128.0).abs() < 2.0);
        }
    }

    #[test]
    fn finds_common_dance_tempos() {
        for bpm in [45.0, 55.0, 65.0, 78.0, 92.0, 105.0, 120.0, 128.0, 140.0, 175.0] {
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

    #[test]
    fn loudness_follows_the_song_and_silence_is_silent() {
        // Within one song, a loud passage reads higher than a quiet one.
        let mut tracker = track(120.0, 8.0, true);
        let loud = tracker.energy;
        for _ in 0..300 { tracker.push(0.0); }
        assert_eq!(tracker.energy, 0.0, "real silence is exactly zero");
        assert!(loud > 0.3, "an audible kick drum reads as audible: {loud}");
        tracker.push(f32::NAN);
        assert!(tracker.energy.is_finite());

        // A quietly mastered song must still register: the same rhythm at a quarter of the
        // level is what used to fall under the dance threshold.
        let quiet = {
            let mut tracker = BeatTracker::default();
            let period = 60.0 / 120.0;
            for i in 0..(8.0 * RATE) as usize {
                let t = i as f32 / RATE;
                let since = t.rem_euclid(period);
                tracker.push(0.25 * (0.15 + 0.5 * (-since / 0.12).exp()));
            }
            tracker.energy
        };
        assert!(quiet > 0.25, "a quiet master still reads as audible: {quiet}");
    }
}
