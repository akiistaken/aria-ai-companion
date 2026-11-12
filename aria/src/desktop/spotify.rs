//! Media transport: Spotify with closed-loop verification, and system-wide media keys.
//!
//! Every Spotify action is checked afterwards (is it audible now, did the track change), and the
//! result says whether it was `verified`, so the model never claims a song started when it did
//! not.

use serde_json::{json, Value};
use std::time::Duration;

use crate::desktop::{audio, launcher, win32};

const SPOTIFY_EXE: &str = "spotify.exe";
const IDLE_TITLES: &[&str] = &["spotify", "spotify free", "spotify premium", ""];

/// `WM_APPCOMMAND` codes Spotify answers.
const APPCOMMAND: &[(&str, i32)] =
    &[("play_pause", 14), ("next", 11), ("previous", 12), ("stop", 13), ("play", 46), ("pause", 47)];

fn windows() -> Vec<win32::WindowInfo> {
    win32::find_windows(Some(&[SPOTIFY_EXE]), None, true, false)
        .into_iter()
        .filter(|w| w.class.starts_with("Chrome_WidgetWin"))
        .collect()
}

/// The track in Spotify's title bar ("Artist - Title"), or "" when paused.
pub fn track() -> String {
    windows()
        .into_iter()
        .map(|w| w.title.trim().to_string())
        .find(|t| !IDLE_TITLES.contains(&t.to_lowercase().as_str()))
        .unwrap_or_default()
}

pub fn running() -> bool {
    !windows().is_empty()
}

fn main_hwnd() -> Option<isize> {
    let wins = windows();
    wins.iter().find(|w| !w.title.trim().is_empty()).or(wins.first()).map(|w| w.hwnd)
}

/// Presses a media key: `play_pause`, `next` or `previous`.
pub fn press_media_key(action: &str) {
    let vk = match action {
        "next" => 0xB0u16,
        "previous" => 0xB1u16,
        _ => 0xB3u16,
    };
    crate::desktop::input::press_virtual_key(vk);
}

/// Sends a transport command straight to the Spotify window, falling back to the media key.
fn transport(action: &str) -> &'static str {
    if let Some(hwnd) = main_hwnd() {
        if let Some((_, cmd)) = APPCOMMAND.iter().find(|(k, _)| *k == action) {
            if win32::post_app_command(hwnd, *cmd) {
                return "spotify_app_command";
            }
        }
    }
    press_media_key(action);
    "media_key"
}

fn audible(sample_ms: u64) -> Option<bool> {
    audio::is_process_audible(SPOTIFY_EXE, sample_ms)
}

fn wait_audible(expect: bool, timeout_secs: f64) -> Option<bool> {
    audio::wait_for_audible(SPOTIFY_EXE, expect, timeout_secs)
}

fn wait_track_change(before: &str, timeout_secs: f64) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs_f64(timeout_secs);
    while std::time::Instant::now() < deadline {
        let current = track();
        if !current.is_empty() && current != before {
            return current;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    track()
}

fn snapshot(extra: Value) -> Value {
    let mut obj = match extra {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    obj.insert("audible".into(), json!(audible(150)));
    obj.insert("track".into(), json!(track()));
    crate::desktop::media::invalidate();
    Value::Object(obj)
}

/// Opens, plays, pauses, skips or searches Spotify, and reports whether it worked.
pub fn control(action: &str, query: &str) -> Value {
    let act = action.trim().to_lowercase();
    match act.as_str() {
        "open" | "launch" | "start" => {
            let result = match launcher::force_launch_app("spotify") {
                Some(v) => v,
                None => {
                    let mut v = launcher::force_open_url("https://open.spotify.com");
                    if let Some(m) = v.as_object_mut() {
                        m.insert("note".into(), "Spotify desktop isn't installed — opened the web player instead.".into());
                    }
                    v
                }
            };
            json!({ "status": "success", "action": "open", "result": result })
        }
        "play_song" | "search" | "play" if !query.trim().is_empty() => search_and_play(query),
        "play" | "resume" => {
            if audible(200) == Some(true) {
                return snapshot(json!({ "status": "success", "action": "play", "note": "Spotify is already playing." }));
            }
            if !running() {
                launcher::force_launch_app("spotify");
                std::thread::sleep(Duration::from_secs(2));
            }
            toggle_until("play", true)
        }
        "pause" | "stop" => {
            if audible(200) == Some(false) {
                return snapshot(json!({ "status": "success", "action": "pause", "note": "Nothing was playing in Spotify." }));
            }
            toggle_until("pause", false)
        }
        "toggle" | "play_pause" => {
            let was = audible(200);
            let method = transport("play_pause");
            let verified = was.and_then(|v| wait_audible(!v, 2.5));
            snapshot(json!({ "status": "success", "action": "play_pause", "method": method, "verified": verified }))
        }
        "next" | "skip" | "previous" | "prev" | "back" => {
            let key = if matches!(act.as_str(), "next" | "skip") { "next" } else { "previous" };
            let before = track();
            let mut method = transport(key);
            let mut changed = { let t = wait_track_change(&before, 3.5); !t.is_empty() && t != before };
            if !changed {
                method = "media_key+retry";
                press_media_key(key);
                let t = wait_track_change(&before, 3.0);
                changed = !t.is_empty() && t != before;
            }
            snapshot(json!({ "status": "success", "action": key, "method": method, "verified": changed }))
        }
        "now_playing" | "current" => serde_json::to_value(crate::desktop::media::now_playing()).unwrap_or_default(),
        _ => json!({ "status": "error", "error": format!("Unknown Spotify action: {action}") }),
    }
}

/// Play or pause, retried once with the toggle when the first command did not take.
fn toggle_until(action: &str, expect_audible: bool) -> Value {
    let mut method = transport(action);
    let mut verified = wait_audible(expect_audible, 2.5);
    if verified == Some(false) {
        transport("play_pause");
        method = "spotify_app_command+retry";
        verified = wait_audible(expect_audible, 2.5);
    }
    snapshot(json!({ "status": "success", "action": action, "method": method, "verified": verified }))
}

fn search_and_play(query: &str) -> Value {
    let mut clean = query.trim().to_string();
    if clean.to_lowercase().starts_with("play ") {
        clean = clean[5..].trim().to_string();
    }
    let before = track();
    let was_audible = audible(200);

    if launcher::protocol_command("spotify").is_none() && !running() {
        let url = format!("https://open.spotify.com/search/{}", crate::util::urlencode(&clean));
        return json!({
            "status": "success", "action": "web_search", "query": clean, "result": launcher::force_open_url(&url),
            "note": "Spotify desktop isn't installed; opened the web player search."
        });
    }

    let launch = launcher::launch_uri(&format!("spotify:search:{}", crate::util::urlencode(&clean)), Some(&[SPOTIFY_EXE]), "spotify");
    std::thread::sleep(Duration::from_millis(2400));

    // Press play on the top result: through the accessibility tree when Cua Driver runs,
    // otherwise by asking the model where the button is on a screenshot.
    let mut method = crate::ai::tasks::click_described(
        "the green circular Play button on the 'Top result' card in Spotify's search results \
         (or the first song row's play button)",
    );

    let mut verified = {
        let t = wait_track_change(&before, 5.0);
        (!t.is_empty() && t != before) || (was_audible != Some(true) && wait_audible(true, 1.0) == Some(true))
    };
    if !verified {
        transport("play");
        method.push_str("+play_command");
        let t = wait_track_change(&before, 3.0);
        verified = (!t.is_empty() && t != before) || wait_audible(true, 1.5) == Some(true);
    }
    snapshot(json!({
        "status": "success", "action": "search_and_play", "query": clean, "method": method, "verified": verified,
        "launch": { "method": launch.get("method"), "foreground": launch.get("foreground") },
        "note": if verified { "" } else { "Opened the search results but couldn't confirm a track started — it may need a click." }
    }))
}

/// System-wide play/pause/next/previous: through Spotify when it is the player, else media keys.
pub fn control_media(action: &str) -> Value {
    let act = match action.trim().to_lowercase().as_str() {
        a @ ("next" | "previous") => a.to_string(),
        _ => "play_pause".to_string(),
    };
    let current = crate::desktop::media::now_playing();
    let mut result = if current.exe == SPOTIFY_EXE || (running() && !track().is_empty()) {
        control(&act, "")
    } else {
        press_media_key(&act);
        json!({ "status": "success", "method": "media_key" })
    };
    crate::desktop::media::invalidate();
    std::thread::sleep(Duration::from_millis(250));
    if let Some(obj) = result.as_object_mut() {
        obj.insert("action".into(), json!(act));
        obj.insert("media".into(), serde_json::to_value(crate::desktop::media::now_playing()).unwrap_or_default());
    }
    result
}
