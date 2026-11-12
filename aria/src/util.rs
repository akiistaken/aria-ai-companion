//! Small shared helpers: time, text and encoding.

use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------------- time

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
    civil_from_unix(ts as i64 + local_offset_secs())
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

/// The OS timezone offset from UTC in seconds, daylight saving included.
fn local_offset_secs() -> i64 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    // TIME_ZONE_ID_DAYLIGHT is 2; the Win32 metadata does not export it as a constant.
    const TIME_ZONE_ID_DAYLIGHT: u32 = 2;
    let mut info = TIME_ZONE_INFORMATION::default();
    // SAFETY: `info` is a valid, writable TIME_ZONE_INFORMATION.
    let id = unsafe { GetTimeZoneInformation(&mut info) };
    // Windows reports minutes *west* of UTC.
    let minutes_west = if id == TIME_ZONE_ID_DAYLIGHT { info.Bias + info.DaylightBias } else { info.Bias };
    -(minutes_west as i64) * 60
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

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
    "November", "December",
];

/// Formats a timestamp using a small subset of strftime: `%Y %m %d %H %M %S %I %p %A %B %%`.
pub fn format_stamp(ts: f64, fmt: &str) -> String {
    let t = local_time(ts);
    let hour12 = match t.hour % 12 {
        0 => 12,
        h => h,
    };
    let mut out = String::new();
    let mut chars = fmt.chars();
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
            Some('I') => out.push_str(&format!("{hour12:02}")),
            Some('p') => out.push_str(if t.hour < 12 { "AM" } else { "PM" }),
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

pub fn now_hour() -> u32 {
    local_time(unix_now()).hour
}

// ---------------------------------------------------------------------------------- text

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

/// "hello world" → "Hello world".
pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// "Hello world" → "hello world".
pub fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// "counter strike" → "Counter Strike".
pub fn title_case(s: &str) -> String {
    s.split_whitespace().map(capitalize).collect::<Vec<_>>().join(" ")
}

/// Percent-encodes everything outside the URL-safe set; spaces become `%20`.
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn base64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

// ---------------------------------------------------------------------------------- random

pub fn rand_range(lo: i32, hi: i32) -> i32 {
    if hi <= lo {
        return lo;
    }
    use rand::Rng;
    rand::rng().random_range(lo..=hi)
}

pub fn rand_f32() -> f32 {
    rand::random::<f32>()
}

pub fn pick_str<'a>(options: &'a [&'a str]) -> &'a str {
    use rand::seq::IndexedRandom;
    options.choose(&mut rand::rng()).copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_formatting_matches_tokens() {
        // 2026-11-11T12:00:00Z: midday UTC is still 2026-11-11 in every zone from -11 to +11.
        let s = format_stamp(1_794_398_400.0, "%Y-%m-%d %H:%M");
        assert_eq!(s.len(), 16, "got {s}");
        assert!(s.starts_with("2026-11-11"), "got {s}");
    }

    #[test]
    fn truncation_respects_limit() {
        assert!(truncate(&"a".repeat(200), 20).chars().count() <= 20);
        assert_eq!(truncate("short", 20), "short");
    }

    #[test]
    fn text_helpers() {
        assert_eq!(title_case("counter strike"), "Counter Strike");
        assert_eq!(capitalize("élan"), "Élan");
        assert_eq!(lowercase_first("Hello"), "hello");
        assert_eq!(urlencode("daft punk"), "daft%20punk");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"M"), "TQ==");
    }
}
