//! Small shared helpers: time formatting, string cleanup, retry-free HTTP JSON.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Local time broken into fields, without pulling in a date crate.
#[derive(Debug, Clone, Copy)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub weekday: u32,
}

/// Converts a UNIX timestamp to local civil time.
pub fn local_time(ts: f64) -> LocalTime {
    let t = ts as i64;
    let (offset, _) = local_offset(t);
    civil_from_unix(t + offset as i64)
}

fn civil_from_unix(secs: i64) -> LocalTime {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    LocalTime {
        year,
        month,
        day,
        hour: (rem / 3600) as u32,
        minute: ((rem % 3600) / 60) as u32,
        second: (rem % 60) as u32,
        weekday: ((days + 4).rem_euclid(7)) as u32,
    }
}

/// The OS timezone offset in seconds for the given instant.
fn local_offset(_unix_secs: i64) -> (i32, bool) {
    #[cfg(windows)]
    {
        use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
        // TIME_ZONE_ID_DAYLIGHT is 2; the Win32 metadata does not export it as a constant.
        const TIME_ZONE_ID_DAYLIGHT: u32 = 2;
        // Windows reports the bias in minutes west of UTC and already accounts for the
        // current daylight-saving state in Bias unless the daylight id is returned.
        unsafe {
            let mut info = TIME_ZONE_INFORMATION::default();
            let id = GetTimeZoneInformation(&mut info);
            let minutes_west = if id == TIME_ZONE_ID_DAYLIGHT {
                info.Bias + info.DaylightBias
            } else {
                info.Bias
            };
            (minutes_west.saturating_neg() * 60, id == TIME_ZONE_ID_DAYLIGHT)
        }
    }
    #[cfg(not(windows))]
    {
        (0, false)
    }
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

const WEEKDAYS: [&str; 7] = [
    "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
];
const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September",
    "October", "November", "December",
];

/// Formats a timestamp using a small subset of chrono's strftime tokens.
pub fn format_stamp(ts: f64, fmt: &str) -> String {
    let t = local_time(ts);
    let hour12 = match t.hour % 12 {
        0 => 12,
        h => h,
    };
    let ampm = if t.hour < 12 { "AM" } else { "PM" };
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&t.year.to_string()),
            Some('m') => out.push_str(&format!("{:02}", t.month)),
            Some('d') => out.push_str(&format!("{:02}", t.day)),
            Some('H') => out.push_str(&format!("{:02}", t.hour)),
            Some('M') => out.push_str(&format!("{:02}", t.minute)),
            Some('S') => out.push_str(&format!("{:02}", t.second)),
            Some('I') => out.push_str(&format!("{:02}", hour12)),
            Some('p') => out.push_str(ampm),
            Some('A') => out.push_str(WEEKDAYS[t.weekday as usize % 7]),
            Some('B') => out.push_str(MONTHS[(t.month as usize).clamp(1, 12) - 1]),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

pub fn now_stamp(fmt: &str) -> String {
    format_stamp(unix_now(), fmt)
}

/// Trims to `max` chars on a word boundary where possible.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let clipped: String = text.chars().take(max.saturating_sub(1)).collect();
    match clipped.rfind(' ') {
        Some(i) if i > max / 2 => format!("{}…", &clipped[..i]),
        _ => format!("{clipped}…"),
    }
}

pub fn now_hour() -> u32 {
    local_time(unix_now()).hour
}

// TODO(wire-up): greeting time-of-day helper, superseded by mood.rs.
#[allow(dead_code)]
pub fn today_hour_minute() -> (u32, u32) {
    let t = local_time(unix_now());
    (t.hour, t.minute)
}

/// A tiny seeded RNG-free random in [lo, hi].
pub fn rand_range(lo: i32, hi: i32) -> i32 {
    if hi <= lo {
        return lo;
    }
    use rand::Rng;
    rand::rng().random_range(lo..=hi)
}

pub fn rand_f32() -> f32 {
    use rand::Rng;
    rand::rng().random::<f32>()
}

pub fn pick_str<'a>(options: &'a [&'a str]) -> &'a str {
    use rand::seq::IndexedRandom;
    options
        .choose(&mut rand::rng())
        .copied()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_formatting_matches_tokens() {
        // 2026-11-11T12:00:00Z: midday UTC is still 2026-11-11 in every zone from -11 to +11.
        let ts = 1_794_398_400.0;
        let s = format_stamp(ts, "%Y-%m-%d %H:%M");
        assert_eq!(s.len(), 16, "got {s}");
        assert!(s.starts_with("2026-11-11"), "got {s}");
    }

    #[test]
    fn truncation_respects_limit() {
        let long = "a".repeat(200);
        assert!(truncate(&long, 20).chars().count() <= 20);
        assert_eq!(truncate("short", 20), "short");
    }
}
