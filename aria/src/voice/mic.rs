//! The microphone: one capture stream feeding 16-bit mono samples into a shared buffer.
//!
//! Click-to-talk, push-to-talk and the hands-free listener all record through [`Capture`], on
//! the input device chosen in the Control Center.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::RECORDING;

pub const SAMPLE_RATE: u32 = 16_000;

/// The input device picked in the Control Center, or the system default.
pub fn input_device() -> Option<cpal::Device> {
    let host = cpal::default_host();
    let wanted = crate::config::get().input_audio_device;
    if !wanted.trim().is_empty() {
        if let Some(device) = host.input_devices().ok()?.find(|d| d.to_string() == wanted) {
            return Some(device);
        }
        log::warn!("microphone '{wanted}' not found; using the default input");
    }
    host.default_input_device()
}

/// A live capture stream. Samples accumulate until taken; dropping it stops the microphone.
pub struct Capture {
    _stream: cpal::Stream,
    buffer: Arc<Mutex<Vec<i16>>>,
    pub source_rate: u32,
}

impl Capture {
    pub fn open() -> Result<Capture, String> {
        let device = input_device().ok_or("no microphone device")?;
        let config = device.default_input_config().map_err(|e| format!("microphone config failed: {e}"))?.config();
        let source_rate = config.sample_rate;
        let channels = (config.channels as usize).max(1);
        let buffer = Arc::new(Mutex::new(Vec::with_capacity(source_rate as usize * 4)));
        let sink = buffer.clone();
        let stream = device
            .build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut buf = sink.lock().unwrap_or_else(|e| e.into_inner());
                    for frame in data.chunks(channels) {
                        let avg = frame.iter().sum::<f32>() / channels as f32;
                        buf.push((avg.clamp(-1.0, 1.0) * 32767.0) as i16);
                    }
                    // Never more than a minute of audio, whatever happens to the reader.
                    let cap = source_rate as usize * 60;
                    if buf.len() > cap {
                        let excess = buf.len() - cap;
                        buf.drain(..excess);
                    }
                },
                |err| log::warn!("microphone stream error: {err}"),
                None,
            )
            .map_err(|e| format!("could not open the microphone: {e}"))?;
        stream.play().map_err(|e| format!("could not start the microphone: {e}"))?;
        Ok(Capture { _stream: stream, buffer, source_rate })
    }

    /// Takes up to `max` samples captured so far (all of them when `max` is `None`).
    pub fn take(&self, max: Option<usize>) -> Vec<i16> {
        let mut buf = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        let n = max.map_or(buf.len(), |m| m.min(buf.len()));
        buf.drain(..n).collect()
    }

    pub fn clear(&self) {
        self.buffer.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

static ACTIVE: Mutex<Option<Capture>> = Mutex::new(None);

/// Starts recording a phrase (click-to-talk or push-to-talk).
pub fn start_recording() -> Result<(), String> {
    if RECORDING.load(Ordering::SeqCst) {
        return Ok(());
    }
    super::stop_speaking();
    let capture = Capture::open()?;
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(capture);
    RECORDING.store(true, Ordering::SeqCst);
    Ok(())
}

/// Stops recording and returns 16 kHz mono WAV, or `None` when under a quarter second was heard.
pub fn stop_recording_wav() -> Option<Vec<u8>> {
    if !RECORDING.swap(false, Ordering::SeqCst) {
        return None;
    }
    let capture = ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    let raw = capture.take(None);
    let samples = resample(&raw, capture.source_rate, SAMPLE_RATE);
    (samples.len() >= SAMPLE_RATE as usize / 4).then(|| crate::desktop::audio::wav_bytes(&samples, SAMPLE_RATE))
}

/// Linear resampling of 16-bit mono (box filter for whole-number ratios).
pub fn resample(input: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    if from % to == 0 {
        let factor = (from / to) as usize;
        return input
            .chunks_exact(factor)
            .map(|w| (w.iter().map(|s| *s as i32).sum::<i32>() / factor as i32) as i16)
            .collect();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let src = i as f64 * ratio;
            let idx = (src as usize).min(input.len() - 1);
            let next = (idx + 1).min(input.len() - 1);
            let frac = src - idx as f64;
            (input[idx] as f64 + (input[next] as f64 - input[idx] as f64) * frac) as i16
        })
        .collect()
}

/// Root-mean-square level of a block of samples.
pub fn rms(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_duration() {
        assert_eq!(resample(&vec![100; 48_000], 48_000, 16_000).len(), 16_000);
        let out = resample(&vec![100; 44_100], 44_100, 16_000);
        assert!((out.len() as i32 - 16_000).abs() <= 1, "{}", out.len());
        assert!(out.iter().all(|s| *s == 100));
    }

    #[test]
    fn rms_of_silence_and_tone() {
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(rms(&[0; 10]), 0.0);
        assert!((rms(&[1000, -1000, 1000, -1000]) - 1000.0).abs() < 1e-9);
    }
}
