//! Desktop audio awareness: per-app session peak meters and speaker loopback capture.
//!
//! WASAPI requires COM on the calling thread ([`crate::desktop::com::ensure_mta`]). No COM object
//! outlives the call that made it, except inside a [`MeterSet`], which stays on its own thread.

use serde::Serialize;
use std::time::{Duration, Instant};
use wasapi::{AudioMeterInformation, Direction, SampleType, StreamMode, WaveFormat};

const AUDIBLE_PEAK: f32 = 0.002;
const TARGET_RATE: u32 = 16_000;

fn with_com<R>(f: impl FnOnce() -> R) -> R {
    crate::desktop::com::ensure_mta();
    f()
}

/// Peak meters of every playback session, kept open for fast repeated reads (the beat tracker
/// samples them at 50 Hz). Not `Send`: create and use it on one thread.
pub struct MeterSet {
    meters: Vec<(u32, AudioMeterInformation)>,
    opened: Instant,
}

impl MeterSet {
    /// Opens the meters of the default output device's sessions.
    pub fn open() -> Option<MeterSet> {
        with_com(|| {
            let device = wasapi::DeviceEnumerator::new().ok()?.get_default_device(Direction::Render).ok()?;
            let sessions = device.get_iaudiosessionmanager().ok()?.get_audiosessionenumerator().ok()?;
            let mut meters = Vec::new();
            for index in 0..sessions.get_count().unwrap_or(0) {
                let Ok(session) = sessions.get_session(index) else { continue };
                let pid = session.get_process_id().unwrap_or(0);
                if let Ok(meter) = session.get_audiometerinformation() {
                    meters.push((pid, meter));
                }
            }
            Some(MeterSet { meters, opened: Instant::now() })
        })
    }

    /// Seconds since the sessions were enumerated; new players need a fresh set.
    pub fn age(&self) -> Duration {
        self.opened.elapsed()
    }

    /// The loudest peak right now among sessions whose pid passes `keep` (system sounds, pid 0,
    /// are always skipped).
    pub fn peak(&self, keep: impl Fn(u32) -> bool) -> f32 {
        self.meters
            .iter()
            .filter(|(pid, _)| *pid != 0 && keep(*pid))
            .map(|(_, meter)| meter.get_peak_value().unwrap_or(0.0))
            .fold(0.0, f32::max)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AudibleApp {
    pub name: String,
    pub pid: u32,
    pub peak: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioActivity {
    pub available: bool,
    pub is_audible: bool,
    pub apps: Vec<AudibleApp>,
    pub loudest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn unavailable(error: impl Into<String>) -> AudioActivity {
    AudioActivity {
        available: false,
        is_audible: false,
        apps: Vec::new(),
        loudest: String::new(),
        error: Some(error.into()),
    }
}

/// Samples every audio session's peak meter for `sample_ms` and reports who is audible.
pub fn audio_activity(sample_ms: u64) -> AudioActivity {
    with_com(|| {
        let enumerator = match wasapi::DeviceEnumerator::new() {
            Ok(e) => e,
            Err(e) => return unavailable(format!("device enumerator failed: {e}")),
        };
        let device = match enumerator.get_default_device(Direction::Render) {
            Ok(d) => d,
            Err(e) => return unavailable(format!("no default render device: {e}")),
        };
        let manager = match device.get_iaudiosessionmanager() {
            Ok(m) => m,
            Err(e) => return unavailable(format!("session manager failed: {e}")),
        };
        let sessions = match manager.get_audiosessionenumerator() {
            Ok(s) => s,
            Err(e) => return unavailable(format!("session enumeration failed: {e}")),
        };

        let mut meters: Vec<(String, u32, AudioMeterInformation)> = Vec::new();
        let count = sessions.get_count().unwrap_or(0);
        for index in 0..count {
            let Ok(session) = sessions.get_session(index) else {
                continue;
            };
            let pid = session.get_process_id().unwrap_or(0);
            let name = if pid == 0 {
                "System Sounds".to_string()
            } else {
                let exe = crate::desktop::win32::process_name(pid);
                if exe.is_empty() {
                    format!("pid {pid}")
                } else {
                    exe
                }
            };
            if let Ok(meter) = session.get_audiometerinformation() {
                meters.push((name, pid, meter));
            }
        }

        let mut peaks: std::collections::HashMap<(String, u32), f32> =
            std::collections::HashMap::new();
        let deadline = Instant::now() + Duration::from_millis(sample_ms.max(40));
        loop {
            for (name, pid, meter) in &meters {
                let value = meter.get_peak_value().unwrap_or(0.0);
                let slot = peaks.entry((name.clone(), *pid)).or_insert(0.0);
                if value > *slot {
                    *slot = value;
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        let mut apps: Vec<AudibleApp> = peaks
            .into_iter()
            .filter(|(_, peak)| *peak >= AUDIBLE_PEAK)
            .map(|((name, pid), peak)| AudibleApp {
                name,
                pid,
                peak: (peak * 10_000.0).round() / 10_000.0,
            })
            .collect();
        apps.sort_by(|a, b| b.peak.total_cmp(&a.peak));

        AudioActivity {
            available: true,
            is_audible: !apps.is_empty(),
            loudest: apps.first().map(|a| a.name.clone()).unwrap_or_default(),
            apps,
            error: None,
        }
    })
}

/// True/False when sessions are readable, None when they are not.
pub fn is_process_audible(exe_name: &str, sample_ms: u64) -> Option<bool> {
    let activity = audio_activity(sample_ms);
    if !activity.available {
        return None;
    }
    let wanted = exe_name.to_lowercase();
    Some(activity.apps.iter().any(|a| a.name.to_lowercase() == wanted))
}

pub fn wait_for_audible(exe_name: &str, expect: bool, timeout_secs: f64) -> Option<bool> {
    let deadline = crate::util::unix_now() + timeout_secs;
    while crate::util::unix_now() < deadline {
        match is_process_audible(exe_name, 200) {
            None => return None,
            Some(value) if value == expect => return Some(true),
            Some(_) => std::thread::sleep(Duration::from_millis(150)),
        }
    }
    Some(false)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AudioLevels {
    pub rms: f32,
    pub peak: f32,
    pub dbfs: f32,
    pub silent: bool,
    pub seconds: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Records `seconds` of the default speakers (system loopback) as 16 kHz mono samples.
pub fn capture_desktop_audio(seconds: f64) -> (Vec<i16>, AudioLevels) {
    let seconds = seconds.clamp(1.0, 30.0);
    with_com(|| {
        let enumerator = match wasapi::DeviceEnumerator::new() {
            Ok(e) => e,
            Err(e) => return (Vec::new(), failed(e.to_string())),
        };
        let device = match enumerator.get_default_device(Direction::Render) {
            Ok(d) => d,
            Err(e) => return (Vec::new(), failed(e.to_string())),
        };
        let mut client = match device.get_iaudioclient() {
            Ok(c) => c,
            Err(e) => return (Vec::new(), failed(e.to_string())),
        };
        let format = match client.get_mixformat() {
            Ok(f) => f,
            Err(e) => return (Vec::new(), failed(e.to_string())),
        };
        let rate = format.get_samplespersec();
        let channels = format.get_nchannels() as usize;
        let block_align = format.get_blockalign() as usize;

        let mode = StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: 0,
        };
        if let Err(e) = client.initialize_client(&format, Direction::Capture, mode, None) {
            return (Vec::new(), failed(format!("loopback init failed: {e}")));
        }
        let Ok(handle) = client.set_get_eventhandle() else {
            return (Vec::new(), failed("no audio event handle"));
        };
        let Ok(capture) = client.get_audiocaptureclient() else {
            return (Vec::new(), failed("no capture client"));
        };
        if client.start_stream().is_err() {
            return (Vec::new(), failed("could not start loopback stream"));
        }

        let mut raw: Vec<u8> = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        let deadline = Instant::now() + Duration::from_secs_f64(seconds);
        while Instant::now() < deadline {
            let _ = handle.wait_for_event(100);
            while capture.get_next_packet_size().unwrap_or(Some(0)).unwrap_or(0) > 0 {
                if capture.read_from_device_to_deque(&mut queue).is_err() {
                    break;
                }
            }
            raw.extend(queue.drain(..));
        }
        let _ = client.stop_stream();

        let samples = bytes_to_i16(&raw, &format);
        let mono = to_16k_mono(&samples, channels, rate);
        let _ = block_align;
        let levels = levels_of(&mono, TARGET_RATE);
        (mono, levels)
    })
}

fn failed(error: impl Into<String>) -> AudioLevels {
    AudioLevels {
        silent: true,
        dbfs: -120.0,
        error: Some(error.into()),
        ..Default::default()
    }
}

/// Converts raw capture bytes into interleaved i16 samples, honouring the mix format.
fn bytes_to_i16(bytes: &[u8], format: &WaveFormat) -> Vec<i16> {
    let float = format
        .get_subformat()
        .map(|s| matches!(s, SampleType::Float))
        .unwrap_or(false);
    let bits = format.get_bitspersample();

    if float {
        bytes
            .chunks_exact(4)
            .map(|c| {
                let value = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                (value.clamp(-1.0, 1.0) * 32767.0) as i16
            })
            .collect()
    } else if bits == 16 {
        bytes
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    } else if bits == 32 {
        // 32-bit integer PCM: take the high 16 bits.
        bytes
            .chunks_exact(4)
            .map(|c| {
                let value = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                (value >> 16) as i16
            })
            .collect()
    } else {
        bytes
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }
}

fn to_16k_mono(samples: &[i16], channels: usize, rate: u32) -> Vec<i16> {
    if samples.is_empty() {
        return Vec::new();
    }
    let channels = channels.max(1);
    let usable = samples.len() / channels * channels;
    let mono: Vec<f32> = samples[..usable]
        .chunks_exact(channels)
        .map(|frame| {
            frame.iter().map(|s| *s as f32 / 32768.0).sum::<f32>() / channels as f32
        })
        .collect();

    if rate == TARGET_RATE || mono.len() < 2 {
        return mono.iter().map(|s| (s * 32767.0) as i16).collect();
    }
    if rate % TARGET_RATE == 0 {
        let factor = (rate / TARGET_RATE) as usize;
        let usable = mono.len() / factor * factor;
        return mono[..usable]
            .chunks_exact(factor)
            .map(|w| (w.iter().sum::<f32>() / factor as f32 * 32767.0) as i16)
            .collect();
    }
    let out_len = (mono.len() as f64 * TARGET_RATE as f64 / rate as f64) as usize;
    (0..out_len)
        .map(|i| {
            let src = i as f64 * rate as f64 / TARGET_RATE as f64;
            let idx = (src as usize).min(mono.len() - 1);
            let next = (idx + 1).min(mono.len() - 1);
            let frac = (src - idx as f64) as f32;
            ((mono[idx] + (mono[next] - mono[idx]) * frac) * 32767.0) as i16
        })
        .collect()
}

fn levels_of(samples: &[i16], rate: u32) -> AudioLevels {
    if samples.is_empty() {
        return AudioLevels {
            silent: true,
            dbfs: -120.0,
            ..Default::default()
        };
    }
    let floats: Vec<f32> = samples.iter().map(|s| *s as f32 / 32768.0).collect();
    let rms = (floats.iter().map(|s| s * s).sum::<f32>() / floats.len() as f32).sqrt();
    let peak = floats.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    let dbfs = if rms > 1e-6 { 20.0 * rms.log10() } else { -120.0 };
    AudioLevels {
        rms: (rms * 100_000.0).round() / 100_000.0,
        peak: (peak * 10_000.0).round() / 10_000.0,
        dbfs: (dbfs * 10.0).round() / 10.0,
        silent: rms < 0.0008,
        seconds: (samples.len() as f32 / rate as f32 * 10.0).round() / 10.0,
        error: None,
    }
}

/// 16-bit mono PCM -> WAV bytes.
pub fn wav_bytes(samples: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// Friendly device names for the settings dropdowns.
pub fn list_audio_devices(input: bool) -> Vec<String> {
    with_com(|| {
        let Ok(enumerator) = wasapi::DeviceEnumerator::new() else {
            return Vec::new();
        };
        let direction = if input {
            Direction::Capture
        } else {
            Direction::Render
        };
        let Ok(collection) = enumerator.get_device_collection(direction) else {
            return Vec::new();
        };
        let mut names = Vec::new();
        let count = collection.get_nbr_devices().unwrap_or(0);
        for index in 0..count {
            if let Ok(device) = collection.get_device_at_index(index) {
                if let Ok(name) = device.get_friendlyname() {
                    if !name.trim().is_empty() && !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
        names
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_wellformed() {
        let wav = wav_bytes(&[0, 100, -100], 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + 6);
    }

    #[test]
    fn mono_downmix_averages_channels() {
        let stereo = [100i16, 300, -100, -300];
        let mono = to_16k_mono(&stereo, 2, 16_000);
        assert_eq!(mono.len(), 2);
        assert!(mono[0] > 0);
    }

    #[test]
    fn resampling_preserves_length_ratio() {
        let input = vec![100i16; 48_000];
        let out = to_16k_mono(&input, 1, 48_000);
        assert!((out.len() as i32 - 16_000).abs() <= 2, "got {}", out.len());
    }

    #[test]
    fn activity_probe_never_panics() {
        let activity = audio_activity(40);
        assert!(activity.available || activity.error.is_some());
    }
}
