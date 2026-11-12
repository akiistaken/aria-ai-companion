//! What is playing on the PC.
//!
//! The source of truth is Windows' system media sessions (SMTC): every app that shows up on the
//! volume overlay publishes one, with its title, artist and play state. That covers the Spotify
//! and Wynk apps and, through the browser, YouTube, YouTube Music, SoundCloud, Wynk web and the
//! rest, without per-site rules. Window titles are only a fallback for players that publish no
//! session (and to tell which site a browser session belongs to).

use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};

/// A stale player must not hold the music worker forever in WinRT's blocking `join`.
fn media_result<T: windows::core::RuntimeType + Send + 'static>(
    operation: windows_future::IAsyncOperation<T>,
) -> windows::core::Result<T> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    operation.when(move |result| { let _ = tx.send(result); })?;
    match rx.recv_timeout(Duration::from_millis(700)) {
        Ok(result) => result,
        Err(_) => {
            let _ = operation.Cancel();
            Err(windows::core::Error::new(windows::core::HRESULT(0x800705B4u32 as i32), "media session timed out"))
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct NowPlaying {
    pub is_playing: bool,
    pub track: String,
    pub artist: String,
    /// Where it plays: `Spotify`, `YouTube`, `SoundCloud`, `Wynk Music`, …
    pub source: String,
    /// Lower-cased exe of the app playing it, when known (`spotify.exe`, `chrome.exe`).
    #[serde(skip)]
    pub exe: String,
}

impl NowPlaying {
    /// "Artist - Title", or just the title.
    pub fn label(&self) -> String {
        match (self.artist.trim(), self.track.trim()) {
            ("", track) => track.to_string(),
            (artist, track) if track.contains(artist) => track.to_string(),
            (artist, track) => format!("{artist} - {track}"),
        }
    }
}

/// Browsers, whose sessions get their site from the window title.
const BROWSERS: &[&str] = &["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe", "vivaldi.exe"];

/// Window-title suffixes that name a site or player.
const TITLE_SOURCES: &[(&str, &str)] = &[
    (" - youtube music", "YouTube Music"),
    (" - youtube", "YouTube"),
    (" - soundcloud", "SoundCloud"),
    (" | soundcloud", "SoundCloud"),
    (" - wynk music", "Wynk Music"),
    (" | wynk music", "Wynk Music"),
    (" - spotify", "Spotify"),
    (" - twitch", "Twitch"),
    (" - jiosaavn", "JioSaavn"),
    (" - apple music", "Apple Music"),
];

/// The exe a session's app id points at, and a friendly name for it.
fn app_identity(app_id: &str) -> (String, String) {
    let lower = app_id.to_lowercase();
    let exe = lower.rsplit(['\\', '!']).next().unwrap_or(&lower).to_string();
    let exe = if exe.ends_with(".exe") { exe } else { format!("{exe}.exe") };
    let name = if lower.contains("spotify") {
        "Spotify"
    } else if lower.contains("wynk") {
        "Wynk Music"
    } else if lower.contains("chrome") {
        "Chrome"
    } else if lower.contains("msedge") || lower.contains("microsoftedge") {
        "Edge"
    } else if lower.contains("firefox") {
        "Firefox"
    } else if lower.contains("brave") {
        "Brave"
    } else if lower.contains("zunemusic") || lower.contains("media player") {
        "Media Player"
    } else if lower.contains("vlc") {
        "VLC"
    } else {
        return (exe.clone(), crate::util::title_case(exe.trim_end_matches(".exe")));
    };
    let exe = match name {
        "Spotify" => "spotify.exe".to_string(),
        "Edge" => "msedge.exe".to_string(),
        "Chrome" => "chrome.exe".to_string(),
        "Firefox" => "firefox.exe".to_string(),
        "Brave" => "brave.exe".to_string(),
        "Media Player" => "music.ui.exe".to_string(),
        "VLC" => "vlc.exe".to_string(),
        _ => exe,
    };
    (exe, name.to_string())
}

/// The site a browser is playing, from its window titles.
fn site_in_titles(exe: &str) -> Option<&'static str> {
    crate::desktop::win32::enum_windows(false)
        .into_iter()
        .filter(|w| w.exe == exe)
        .find_map(|w| {
            let low = w.title.to_lowercase();
            TITLE_SOURCES.iter().find(|(suffix, _)| low.ends_with(suffix)).map(|(_, site)| *site)
        })
}

/// Every system media session, playing ones first.
#[cfg(test)]
pub fn sessions() -> Vec<NowPlaying> {
    read_sessions(false)
}

fn read_sessions(playing_only: bool) -> Vec<NowPlaying> {
    crate::desktop::com::ensure_mta();
    let read = || -> windows::core::Result<Vec<NowPlaying>> {
        let manager = media_result(SessionManager::RequestAsync()?)?;
        let list = manager.GetSessions()?;
        let mut out = Vec::new();
        for i in 0..list.Size()? {
            let session = list.GetAt(i)?;
            let playing = session.GetPlaybackInfo().and_then(|p| p.PlaybackStatus()).is_ok_and(|s| s == PlaybackStatus::Playing);
            if playing_only && !playing { continue; }
            let props = session.TryGetMediaPropertiesAsync().and_then(media_result).ok();
            let (exe, mut source) = app_identity(&session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default());
            if BROWSERS.contains(&exe.as_str()) {
                if let Some(site) = site_in_titles(&exe) {
                    source = site.to_string();
                }
            }
            out.push(NowPlaying {
                is_playing: playing,
                track: props.as_ref().and_then(|p| p.Title().ok()).map(|s| s.to_string()).unwrap_or_default(),
                artist: props.as_ref().and_then(|p| p.Artist().ok()).map(|s| s.to_string()).unwrap_or_default(),
                source,
                exe,
            });
            if playing_only { break; }
        }
        out.sort_by_key(|s| !s.is_playing);
        Ok(out)
    };
    read().unwrap_or_else(|e| {
        log::debug!("media sessions unavailable: {e}");
        Vec::new()
    })
}

/// Players that publish no media session: a recognisable window title plus audible output.
fn from_window_titles() -> Option<NowPlaying> {
    let activity = crate::desktop::audio::audio_activity(90);
    // Spotify can keep playing with its window closed or a stuck media-session service.
    // Its audio session is sufficient evidence; the track title can recover on a later poll.
    if activity.apps.iter().any(|a| a.name == "spotify.exe") {
        return Some(NowPlaying { is_playing: true, source: "Spotify".into(), exe: "spotify.exe".into(), ..Default::default() });
    }
    for w in crate::desktop::win32::enum_windows(false) {
        let low = w.title.to_lowercase();
        let found = if w.exe == "vlc.exe" && low.ends_with(" - vlc media player") {
            Some(("VLC", &w.title[..w.title.len() - " - VLC media player".len()]))
        } else if w.exe == "spotify.exe" {
            Some(("Spotify", w.title.as_str()))
        } else if BROWSERS.contains(&w.exe.as_str()) || w.exe.contains("wynk") {
            TITLE_SOURCES
                .iter()
                .find(|(suffix, _)| low.ends_with(suffix))
                .map(|(suffix, site)| (*site, &w.title[..w.title.len() - suffix.len()]))
        } else {
            None
        };
        let Some((source, track)) = found else { continue };
        let track = strip_notification_count(track).trim_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '•' | '|'));
        if track.is_empty() {
            continue;
        }
        let audible = !activity.available || activity.apps.iter().any(|a| a.name.eq_ignore_ascii_case(&w.exe));
        if audible {
            return Some(NowPlaying { is_playing: true, track: track.to_string(), source: source.into(), exe: w.exe, ..Default::default() });
        }
    }
    None
}

/// YouTube prefixes unread notifications to its titles: "(3) Song - YouTube".
fn strip_notification_count(title: &str) -> &str {
    let Some(rest) = title.strip_prefix('(') else { return title };
    match rest.split_once(") ") {
        Some((count, after)) if !count.is_empty() && count.chars().all(|c| c.is_ascii_digit() || c == '+') => after,
        _ => title,
    }
}

static CACHE: Mutex<Option<(Instant, NowPlaying)>> = Mutex::new(None);

/// Bounds the entire WinRT query, including synchronous COM calls made before an async
/// operation exists. At most one query can be outstanding, even if Windows never returns it.
#[derive(Default)]
struct MediaProbe {
    pending: Option<std::sync::mpsc::Receiver<Option<NowPlaying>>>,
}

impl MediaProbe {
    fn poll(&mut self, query: impl FnOnce() -> Option<NowPlaying> + Send + 'static, timeout: Duration) -> Option<NowPlaying> {
        use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
        if let Some(rx) = &self.pending {
            return match rx.try_recv() {
                Ok(value) => { self.pending = None; value }
                Err(TryRecvError::Disconnected) => { self.pending = None; None }
                Err(TryRecvError::Empty) => None,
            };
        }
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new().name("aria-media-query".into()).spawn(move || { let _ = tx.send(query()); }).ok()?;
        match rx.recv_timeout(timeout) {
            Ok(value) => value,
            Err(RecvTimeoutError::Timeout) => {
                self.pending = Some(rx);
                log::warn!("music metadata timed out; using active audio sessions");
                None
            }
            Err(RecvTimeoutError::Disconnected) => None,
        }
    }
}

fn playing_session() -> Option<NowPlaying> {
    static PROBE: Mutex<MediaProbe> = Mutex::new(MediaProbe { pending: None });
    PROBE.lock().unwrap_or_else(|e| e.into_inner())
        .poll(|| read_sessions(true).into_iter().find(|s| s.is_playing), Duration::from_millis(1500))
}

/// The media playing right now (cached for 2 s).
pub fn now_playing() -> NowPlaying {
    if let Some((at, value)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if at.elapsed() < Duration::from_secs(2) {
            return value.clone();
        }
    }
    let value = playing_session()
        .or_else(from_window_titles)
        .unwrap_or_default();
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), value.clone()));
    value
}

/// Forgets the cached answer, after a play/pause/skip.
pub fn invalidate() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_metadata_cannot_block_playback_polling_or_spawn_more_queries() {
        let (release, wait) = std::sync::mpsc::channel();
        let mut probe = MediaProbe::default();
        let started = Instant::now();
        assert!(probe.poll(move || { wait.recv().unwrap(); Some(NowPlaying { is_playing: true, ..Default::default() }) }, Duration::from_millis(20)).is_none());
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(probe.poll(|| panic!("a second query must not start"), Duration::from_secs(1)).is_none());
        release.send(()).unwrap();
        let recovered = probe.pending.as_ref().unwrap().recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
        assert!(recovered.is_playing, "a late query can still complete");
    }

    #[test]
    fn app_ids_map_to_friendly_sources() {
        assert_eq!(app_identity("Spotify.exe"), ("spotify.exe".into(), "Spotify".into()));
        assert_eq!(app_identity("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), ("spotify.exe".into(), "Spotify".into()));
        assert_eq!(app_identity("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!App"), ("spotify.exe".into(), "Spotify".into()));
        assert_eq!(app_identity("Chrome").1, "Chrome");
        assert_eq!(app_identity("MSEdge"), ("msedge.exe".into(), "Edge".into()));
        assert_eq!(app_identity("C:\\Program Files\\Wynk\\Wynk Music.exe").1, "Wynk Music");
        assert_eq!(app_identity("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic").1, "Media Player");
    }

    #[test]
    fn labels_read_naturally() {
        let np = NowPlaying { track: "Beat It".into(), artist: "Michael Jackson".into(), ..Default::default() };
        assert_eq!(np.label(), "Michael Jackson - Beat It");
        let np = NowPlaying { track: "Coolio - Gangsta's Paradise".into(), artist: "Coolio".into(), ..Default::default() };
        assert_eq!(np.label(), "Coolio - Gangsta's Paradise");
    }

    #[test]
    fn notification_counts_are_stripped_but_numbers_in_titles_kept() {
        assert_eq!(strip_notification_count("(3) Lofi beats"), "Lofi beats");
        assert_eq!(strip_notification_count("(99+) Song"), "Song");
        assert_eq!(strip_notification_count("22 - Taylor Swift"), "22 - Taylor Swift");
        assert_eq!(strip_notification_count("(Live) Song"), "(Live) Song");
    }

    #[test]
    fn session_probe_never_panics() {
        let _ = sessions();
    }
}
