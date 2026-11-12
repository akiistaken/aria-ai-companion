//! Lip-sync: measures the TTS audio as it plays and publishes how open her mouth is.

use std::sync::atomic::{AtomicU32, Ordering};

use super::{SPEAKING, SPEECH_SEQ, SHUTTING_DOWN};

/// Mouth shape of the audio playing right now, as `f32` bits.
static MOUTH_AA: AtomicU32 = AtomicU32::new(0);
static MOUTH_OH: AtomicU32 = AtomicU32::new(0);

/// The current `(aa, oh)` lip-sync levels, both `0..1`. Zero when nothing is playing.
pub fn mouth_levels() -> (f32, f32) {
    (f32::from_bits(MOUTH_AA.load(Ordering::Relaxed)), f32::from_bits(MOUTH_OH.load(Ordering::Relaxed)))
}

fn publish(aa: f32, oh: f32) {
    MOUTH_AA.store(aa.to_bits(), Ordering::Relaxed);
    MOUTH_OH.store(oh.to_bits(), Ordering::Relaxed);
}

/// Wraps the TTS decoder and measures it as it plays.
///
/// Every ~20 ms window yields an RMS level (how open the mouth is) and a zero-crossing rate (a
/// cheap stand-in for a formant split: rounded vowels sit low, open and fricative sounds cross
/// zero often). It also owns the end of speech: `SPEAKING` drops when the audio runs out, and a
/// superseded or stopped utterance is cut off mid-sample rather than playing to the end.
pub struct LipSync<S> {
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
    pub fn new(inner: S, seq: i64) -> Self {
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
        if std::mem::replace(&mut self.finished, true) {
            return;
        }
        publish(0.0, 0.0);
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
        publish(open * (1.0 - 0.45 * rounded), (open * rounded * 0.85).min(0.85));
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
        let value = sample;
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
            publish(0.0, 0.0);
            if SPEECH_SEQ.load(Ordering::SeqCst) == self.seq {
                SPEAKING.store(false, Ordering::SeqCst);
            }
        }
    }
}
