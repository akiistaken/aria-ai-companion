//! System telemetry (CPU/RAM/disk/battery) plus now-playing media detection.

use serde::Serialize;
use std::sync::{Mutex, OnceLock};

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

struct Cache<T> {
    value: Option<T>,
    at: f64,
}

static STATS_CACHE: OnceLock<Mutex<Cache<SystemStats>>> = OnceLock::new();

fn stats_cache() -> &'static Mutex<Cache<SystemStats>> {
    STATS_CACHE.get_or_init(|| {
        Mutex::new(Cache {
            value: None,
            at: 0.0,
        })
    })
}

static SYS: OnceLock<Mutex<sysinfo::System>> = OnceLock::new();

fn system() -> &'static Mutex<sysinfo::System> {
    SYS.get_or_init(|| {
        Mutex::new(
            sysinfo::System::new().unwrap_or_else(|e| {
                log::warn!("system info unavailable: {e}");
                sysinfo::System::new_with_specifics(sysinfo::RefreshKind::nothing())
                    .expect("sysinfo cannot initialise")
            }),
        )
    })
}

pub fn system_stats() -> SystemStats {
    let now = crate::util::unix_now();
    {
        let cache = stats_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(value) = &cache.value {
            if now - cache.at < 1.8 {
                return value.clone();
            }
        }
    }

    let (cpu, ram_percent, ram_used_gb, ram_total_gb) = {
        let mut sys = system().lock().unwrap_or_else(|e| e.into_inner());
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let cpu = sys.global_cpu_usage();
        let total = sys.total_memory();
        let used = sys.used_memory();
        let percent = if total > 0 {
            (used as f64 / total as f64 * 100.0) as f32
        } else {
            0.0
        };
        (
            cpu,
            percent,
            used as f32 / 1024.0 / 1024.0 / 1024.0,
            total as f32 / 1024.0 / 1024.0 / 1024.0,
        )
    };

    let disk_free_gb = free_disk_gb("C:\\");

    let (battery_percent, power_plugged) = match battery() {
        Some((pct, plugged)) => (Some(pct), plugged),
        None => (None, true),
    };

    let active = crate::win32::active_window()
        .map(|w| w.title)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Desktop".into());

    let mut sample: Vec<String> = Vec::new();
    for w in crate::win32::enum_windows(false) {
        let t = w.title.trim().to_string();
        if !t.is_empty() && !sample.contains(&t) && t != "Aria" {
            sample.push(t);
        }
        if sample.len() >= 8 {
            break;
        }
    }

    let stats = SystemStats {
        cpu_percent: (cpu * 10.0).round() / 10.0,
        ram_percent: (ram_percent * 10.0).round() / 10.0,
        ram_used_gb: (ram_used_gb * 10.0).round() / 10.0,
        ram_total_gb: (ram_total_gb * 10.0).round() / 10.0,
        disk_free_gb: (disk_free_gb * 10.0).round() / 10.0,
        battery_percent,
        power_plugged,
        active_window: active,
        open_windows_sample: sample,
    };

    let mut cache = stats_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.value = Some(stats.clone());
    cache.at = now;
    stats
}

fn free_disk_gb(path: &str) -> f32 {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut free = 0u64;
    let mut total = 0u64;
    let mut available = 0u64;
    let h = HSTRING::from(path);
    let ok = unsafe {
        GetDiskFreeSpaceExW(&h, Some(&mut available), Some(&mut total), Some(&mut free)).is_ok()
    };
    if ok {
        available as f32 / 1024.0 / 1024.0 / 1024.0
    } else {
        0.0
    }
}

fn battery() -> Option<(u8, bool)> {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return None;
    }
    // 255 means "unknown", which is how desktops report themselves.
    if status.BatteryLifePercent == 255 {
        return None;
    }
    Some((status.BatteryLifePercent, status.ACLineStatus == 1))
}

#[derive(Debug, Clone, Serialize)]
pub struct NowPlaying {
    pub is_playing: bool,
    pub track: String,
    pub source: String,
}

struct MediaCache {
    value: Option<NowPlaying>,
    at: f64,
}

static MEDIA_CACHE: OnceLock<Mutex<MediaCache>> = OnceLock::new();

fn media_cache() -> &'static Mutex<MediaCache> {
    MEDIA_CACHE.get_or_init(|| {
        Mutex::new(MediaCache {
            value: None,
            at: 0.0,
        })
    })
}

const SPOTIFY_EXE: &str = "spotify.exe";
const SPOTIFY_IDLE_TITLES: &[&str] = &["spotify", "spotify free", "spotify premium", ""];

pub fn spotify_track() -> String {
    for w in spotify_windows() {
        let title = w.title.trim().to_string();
        if !SPOTIFY_IDLE_TITLES.contains(&title.to_lowercase().as_str()) {
            return title;
        }
    }
    String::new()
}

fn spotify_windows() -> Vec<crate::win32::WindowInfo> {
    crate::win32::find_windows(Some(&[SPOTIFY_EXE]), None, true, false)
        .into_iter()
        .filter(|w| w.class.starts_with("Chrome_WidgetWin"))
        .collect()
}

pub fn spotify_running() -> bool {
    !spotify_windows().is_empty()
}

fn spotify_main_hwnd() -> Option<isize> {
    let wins = spotify_windows();
    wins.iter()
        .find(|w| !w.title.trim().is_empty())
        .or(wins.first())
        .map(|w| w.hwnd)
}

const APPCOMMAND: &[(&str, i32)] = &[
    ("play_pause", 14),
    ("next", 11),
    ("previous", 12),
    ("stop", 13),
    ("play", 46),
    ("pause", 47),
];

const MEDIA_KEYS: &[(&str, &str)] = &[
    ("play_pause", "playpause"),
    ("next", "nexttrack"),
    ("previous", "prevtrack"),
    ("play", "playpause"),
    ("pause", "playpause"),
];

fn media_key(action: &str) -> &'static str {
    MEDIA_KEYS
        .iter()
        .find(|(k, _)| *k == action)
        .map(|(_, v)| *v)
        .unwrap_or("playpause")
}

fn press_media_key(action: &str) {
    let vk = match media_key(action) {
        "nexttrack" => 0xB0u16,
        "prevtrack" => 0xB1u16,
        _ => 0xB3u16,
    };
    crate::input::press_virtual_key(vk);
}

/// Sends a transport command straight to the Spotify window, falling back to the media key.
fn spotify_transport(action: &str) -> &'static str {
    if let Some(hwnd) = spotify_main_hwnd() {
        if let Some((_, cmd)) = APPCOMMAND.iter().find(|(k, _)| *k == action) {
            if crate::win32::post_app_command(hwnd, *cmd) {
                return "spotify_app_command";
            }
        }
    }
    press_media_key(action);
    "media_key"
}

fn spotify_audible(sample_ms: u64) -> Option<bool> {
    crate::audio::is_process_audible(SPOTIFY_EXE, sample_ms)
}

fn wait_spotify_state(expect_audible: bool, timeout_secs: f64) -> Option<bool> {
    crate::audio::wait_for_audible(SPOTIFY_EXE, expect_audible, timeout_secs)
}

fn wait_track_change(before: &str, timeout_secs: f64) -> String {
    let deadline = crate::util::unix_now() + timeout_secs;
    while crate::util::unix_now() < deadline {
        let cur = spotify_track();
        if !cur.is_empty() && cur != before {
            return cur;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    spotify_track()
}

fn spotify_snapshot(extra: serde_json::Value) -> serde_json::Value {
    let audible = spotify_audible(150);
    let mut obj = serde_json::Map::new();
    if let serde_json::Value::Object(map) = extra {
        obj.extend(map);
    }
    obj.insert("audible".into(), serde_json::json!(audible));
    obj.insert("track".into(), serde_json::json!(spotify_track()));
    serde_json::Value::Object(obj)
}

/// Spotify control with closed-loop verification: act, then listen to confirm it worked.
pub fn control_spotify(action: &str, query: &str) -> serde_json::Value {
    let act = action.trim().to_lowercase();
    match act.as_str() {
        "open" | "launch" | "start" => {
            let res = match crate::launcher::force_launch_app("spotify") {
                Some(mut v) => {
                    if let Some(m) = v.as_object_mut() {
                        m.insert("status".into(), "success".into());
                        m.insert("action".into(), "open".into());
                    }
                    v
                }
                None => {
                    let mut v = crate::launcher::force_open_url("https://open.spotify.com");
                    if let Some(m) = v.as_object_mut() {
                        m.insert("note".into(),
                            "Spotify desktop isn't installed — opened the web player instead.".into());
                    }
                    v
                }
            };
            return serde_json::json!({ "status": "success", "action": "open", "result": res });
        }
        _ => {}
    }

    if matches!(act.as_str(), "play_song" | "search" | "play") && !query.trim().is_empty() {
        return spotify_search_and_play(query);
    }

    if matches!(act.as_str(), "play" | "resume") {
        if spotify_audible(200) == Some(true) {
            return spotify_snapshot(serde_json::json!({
                "status": "success", "action": "play", "note": "Spotify is already playing."
            }));
        }
        if !spotify_running() {
            crate::launcher::force_launch_app("spotify");
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        let mut method = spotify_transport("play");
        let mut ok = wait_spotify_state(true, 2.5);
        if ok == Some(false) {
            spotify_transport("play_pause");
            method = "spotify_app_command+retry";
            ok = wait_spotify_state(true, 2.5);
        }
        return spotify_snapshot(serde_json::json!({
            "status": "success", "action": "play", "method": method, "verified": ok
        }));
    }

    if matches!(act.as_str(), "pause" | "stop") {
        if spotify_audible(200) == Some(false) {
            return spotify_snapshot(serde_json::json!({
                "status": "success", "action": "pause", "note": "Nothing was playing in Spotify."
            }));
        }
        let mut method = spotify_transport("pause");
        let mut ok = wait_spotify_state(false, 2.5);
        if ok == Some(false) {
            spotify_transport("play_pause");
            method = "spotify_app_command+retry";
            ok = wait_spotify_state(false, 2.5);
        }
        return spotify_snapshot(serde_json::json!({
            "status": "success", "action": "pause", "method": method, "verified": ok
        }));
    }

    if matches!(act.as_str(), "toggle" | "play_pause") {
        let was = spotify_audible(200);
        let method = spotify_transport("play_pause");
        let ok = match was {
            Some(v) => wait_spotify_state(!v, 2.5),
            None => None,
        };
        return spotify_snapshot(serde_json::json!({
            "status": "success", "action": "play_pause", "method": method, "verified": ok
        }));
    }

    if matches!(act.as_str(), "next" | "skip" | "previous" | "prev" | "back") {
        let key = if matches!(act.as_str(), "next" | "skip") {
            "next"
        } else {
            "previous"
        };
        let before = spotify_track();
        let mut method = spotify_transport(key);
        let mut new_track = wait_track_change(&before, 3.5);
        let mut changed = !new_track.is_empty() && new_track != before;
        if !changed {
            method = "media_key+retry";
            press_media_key(key);
            new_track = wait_track_change(&before, 3.0);
            changed = !new_track.is_empty() && new_track != before;
        }
        return spotify_snapshot(serde_json::json!({
            "status": "success", "action": key, "method": method, "verified": changed
        }));
    }

    if matches!(act.as_str(), "now_playing" | "current") {
        return serde_json::to_value(now_playing()).unwrap_or_default();
    }

    serde_json::json!({ "status": "error", "error": format!("Unknown Spotify action: {action}") })
}

fn spotify_search_and_play(query: &str) -> serde_json::Value {
    let mut clean = query.trim().to_string();
    if clean.to_lowercase().starts_with("play ") {
        clean = clean[5..].trim().to_string();
    }
    let before = spotify_track();
    let was_audible = spotify_audible(200);

    if crate::launcher::protocol_command("spotify").is_none() && !spotify_running() {
        let encoded = urlencode(&clean);
        let res = crate::launcher::force_open_url(&format!(
            "https://open.spotify.com/search/{encoded}"
        ));
        return serde_json::json!({
            "status": "success", "action": "web_search", "query": clean, "result": res,
            "note": "Spotify desktop isn't installed; opened the web player search."
        });
    }

    let uri = format!("spotify:search:{}", urlencode(&clean));
    let launch = crate::launcher::launch_uri(&uri, Some(&[SPOTIFY_EXE]), "spotify");
    std::thread::sleep(std::time::Duration::from_millis(2400));

    let mut method = "none".to_string();
    if !crate::config::api_key().is_empty() {
        let click = crate::tools::find_and_click_on_screen(
            "the green circular Play button on the 'Top result' card in Spotify's search results \
             (or the first song row's play button)",
            "single",
            None,
            false,
        );
        method = if click.get("status").and_then(|v| v.as_str()) == Some("success") {
            "vision_click".into()
        } else {
            "vision_click_failed".into()
        };
    }

    let mut new_track = wait_track_change(&before, 5.0);
    let mut verified = !new_track.is_empty() && new_track != before;
    if !verified && was_audible != Some(true) {
        verified = wait_spotify_state(true, 1.0) == Some(true);
    }
    if !verified {
        spotify_transport("play");
        method.push_str("+play_command");
        new_track = wait_track_change(&before, 3.0);
        verified = (!new_track.is_empty() && new_track != before)
            || wait_spotify_state(true, 1.5) == Some(true);
    }

    spotify_snapshot(serde_json::json!({
        "status": "success", "action": "search_and_play", "query": clean, "method": method,
        "verified": verified,
        "launch": { "method": launch.get("method"), "foreground": launch.get("foreground") },
        "note": if verified { "" } else { "Opened the search results but couldn't confirm a track started — it may need a click." }
    }))
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn now_playing() -> NowPlaying {
    let now = crate::util::unix_now();
    {
        let cache = media_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(value) = &cache.value {
            if now - cache.at < 3.5 {
                return value.clone();
            }
        }
    }

    let mut res = NowPlaying {
        is_playing: false,
        track: String::new(),
        source: String::new(),
    };

    let spotify = spotify_track();
    if !spotify.is_empty() {
        res = NowPlaying {
            is_playing: true,
            track: spotify,
            source: "Spotify".into(),
        };
    } else {
        'outer: for w in crate::win32::enum_windows(false) {
            let title = w.title.trim();
            let low = title.to_lowercase();
            let exe = w.exe.as_str();

            let (source, track) = if exe == "vlc.exe" && low.ends_with(" - vlc media player") {
                (
                    "Vlc".to_string(),
                    title[..title.len() - " - VLC media player".len()].to_string(),
                )
            } else {
                let suffixes = [
                    (" - youtube music", "Youtube Music"),
                    (" - youtube", "Youtube"),
                    (" - soundcloud", "Soundcloud"),
                    (" - twitch", "Twitch"),
                    (" - spotify", "Spotify"),
                ];
                let browsers = ["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe"];
                let mut found = (String::new(), String::new());
                if browsers.contains(&exe) {
                    for (suffix, label) in suffixes {
                        if low.ends_with(suffix) {
                            found = (
                                label.to_string(),
                                title[..title.len() - suffix.len()].to_string(),
                            );
                            break;
                        }
                    }
                }
                found
            };

            if source.is_empty() {
                continue;
            }
            let track = track.trim_matches(|c| c == ' ' || c == '-' || c == '•' || c == '|');
            if track.is_empty() || ["youtube", "spotify", "soundcloud", "twitch"].contains(&track.to_lowercase().as_str()) {
                continue;
            }
            let activity = crate::audio::audio_activity(90);
            let browser_audible = activity
                .apps
                .iter()
                .any(|a| a.name.to_lowercase() == exe);
            if !activity.available || browser_audible {
                res = NowPlaying {
                    is_playing: true,
                    track: track.to_string(),
                    source,
                };
                break 'outer;
            }
        }
    }

    let mut cache = media_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.value = Some(res.clone());
    cache.at = now;
    res
}

pub fn control_media(action: &str) -> serde_json::Value {
    let act = match action.trim().to_lowercase().as_str() {
        "next" | "previous" => action.trim().to_lowercase(),
        _ => "play_pause".to_string(),
    };

    let res = if spotify_running()
        && (spotify_audible(120) == Some(true) || !spotify_track().is_empty())
    {
        control_spotify(&act, "")
    } else {
        press_media_key(&act);
        serde_json::json!({ "status": "success", "method": "media_key" })
    };

    // Invalidate the media cache so the caller sees fresh state.
    media_cache().lock().unwrap_or_else(|e| e.into_inner()).at = 0.0;
    std::thread::sleep(std::time::Duration::from_millis(200));

    let playing = serde_json::to_value(now_playing()).unwrap_or_default();
    let mut obj = res.as_object().cloned().unwrap_or_default();
    obj.insert("action".into(), serde_json::json!(act));
    obj.insert("media".into(), playing);
    serde_json::Value::Object(obj)
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

    #[test]
    fn url_encoding_escapes_spaces() {
        assert_eq!(urlencode("daft punk"), "daft%20punk");
    }
}
