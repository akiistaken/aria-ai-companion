//! System telemetry: CPU, RAM, disk, battery and the windows on screen.

use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
pub struct SystemStats {
    pub cpu_percent: f32,
    pub ram_percent: f32,
    pub ram_used_gb: f32,
    pub ram_total_gb: f32,
    pub disk_free_gb: f32,
    pub battery_percent: Option<u8>,
    pub power_plugged: bool,
    pub active_window: String,
    pub open_windows_sample: Vec<String>,
}

fn system() -> &'static Mutex<sysinfo::System> {
    static SYS: OnceLock<Mutex<sysinfo::System>> = OnceLock::new();
    SYS.get_or_init(|| {
        Mutex::new(sysinfo::System::new().unwrap_or_else(|e| {
            log::warn!("system info unavailable: {e}");
            sysinfo::System::new_with_specifics(sysinfo::RefreshKind::nothing()).expect("sysinfo cannot initialise")
        }))
    })
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// Current stats, cached for 1.8 s (several callers ask within one tick).
pub fn system_stats() -> SystemStats {
    static CACHE: Mutex<Option<(Instant, SystemStats)>> = Mutex::new(None);
    if let Some((at, stats)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if at.elapsed() < Duration::from_millis(1800) {
            return stats.clone();
        }
    }

    let (cpu, ram_percent, ram_used_gb, ram_total_gb) = {
        let mut sys = system().lock().unwrap_or_else(|e| e.into_inner());
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let (total, used) = (sys.total_memory(), sys.used_memory());
        let gb = |bytes: u64| bytes as f32 / 1024.0 / 1024.0 / 1024.0;
        let percent = if total > 0 { (used as f64 / total as f64 * 100.0) as f32 } else { 0.0 };
        (sys.global_cpu_usage(), percent, gb(used), gb(total))
    };
    let (battery_percent, power_plugged) = match battery() {
        Some((pct, plugged)) => (Some(pct), plugged),
        None => (None, true),
    };
    let active_window = crate::desktop::win32::active_window()
        .map(|w| w.title)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Desktop".into());
    let mut open_windows_sample: Vec<String> = Vec::new();
    for w in crate::desktop::win32::enum_windows(false) {
        let t = w.title.trim().to_string();
        if !t.is_empty() && t != "Aria" && !open_windows_sample.contains(&t) {
            open_windows_sample.push(t);
        }
        if open_windows_sample.len() >= 8 {
            break;
        }
    }

    let stats = SystemStats {
        cpu_percent: round1(cpu),
        ram_percent: round1(ram_percent),
        ram_used_gb: round1(ram_used_gb),
        ram_total_gb: round1(ram_total_gb),
        disk_free_gb: round1(free_disk_gb("C:\\")),
        battery_percent,
        power_plugged,
        active_window,
        open_windows_sample,
    };
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), stats.clone()));
    stats
}

fn free_disk_gb(path: &str) -> f32 {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut available = 0u64;
    // SAFETY: valid out-pointers for the duration of the call.
    let ok = unsafe { GetDiskFreeSpaceExW(&HSTRING::from(path), Some(&mut available), None, None).is_ok() };
    if ok {
        available as f32 / 1024.0 / 1024.0 / 1024.0
    } else {
        0.0
    }
}

/// `(percent, plugged in)`, or `None` on machines without a battery.
fn battery() -> Option<(u8, bool)> {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: `status` is a valid out-pointer.
    unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
    // 255 means "unknown", which is how desktops report themselves.
    (status.BatteryLifePercent != 255).then_some((status.BatteryLifePercent, status.ACLineStatus == 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_are_in_plausible_ranges() {
        let s = system_stats();
        assert!((0.0..=100.0).contains(&s.cpu_percent));
        assert!((0.0..=100.0).contains(&s.ram_percent));
        assert!(s.ram_total_gb > 0.0);
    }
}
