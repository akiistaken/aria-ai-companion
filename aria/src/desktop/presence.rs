//! What the user is doing: at the desk or away, and whether a game has the screen.
//!
//! Game detection weighs three signals, strongest first:
//! 1. the foreground app is installed in a game library (Steam, Epic, Riot, GOG, Xbox, …);
//! 2. its title or exe names a known game;
//! 3. it covers its whole monitor and is not a browser, player or the shell.
//!
//! Windows come from Cua Driver when it is running (front-most by real z-order, with owning
//! exe and bounds) and from plain Win32 otherwise.

/// Known games, matched against lower-cased window titles and exe names.
pub const GAME_KEYWORDS: &[&str] = &[
    "valorant", "genshin", "elden ring", "cyberpunk", "minecraft", "counter-strike", "cs2", "overwatch",
    "apex legends", "league of legends", "dota", "honkai", "wuthering", "fortnite", "call of duty", "warzone",
    "rainbow six", "rocket league", "roblox", "helldivers", "monster hunter", "destiny", "dark souls", "palworld",
    "grand theft auto", "gta", "osu!", "team fortress", "terraria", "warframe", "pubg", "baldur's gate",
    "lethal company", "street fighter", "tekken", "marvel rivals", "tarkov", "black myth", "wukong", "hades",
    "sekiro", "god of war", "witcher", "forza", "halo", "dead by daylight", "fifa", "ea sports fc", "nba 2k",
    "stellaris", "civilization", "diablo", "path of exile", "world of warcraft", "final fantasy", "persona 5",
    "starfield", "satisfactory", "factorio", "subnautica", "sea of thieves", "borderlands", "zenless",
];

/// Install folders that mean "this is a game".
const GAME_LIBRARIES: &[&str] = &[
    r"\steamapps\common\",
    r"\epic games\",
    r"\riot games\",
    r"\gog galaxy\games\",
    r"\xboxgames\",
    r"\windowsapps\microsoft.minecraft",
    r"\battle.net\",
    r"\ubisoft game launcher\games\",
    r"\ea games\",
    r"\hoyoplay\games\",
];

/// Apps that go fullscreen without being games.
const NOT_GAMES: &[&str] = &[
    "chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe", "vivaldi.exe", "vlc.exe", "mpv.exe",
    "potplayermini64.exe", "explorer.exe", "applicationframehost.exe", "powerpnt.exe", "obs64.exe",
    "textinputhost.exe", "nvidia overlay.exe", "searchhost.exe", "lockapp.exe", "shellexperiencehost.exe",
    "steam.exe", "steamwebhelper.exe", "epicgameslauncher.exe", "aria.exe", "aria-avatar.exe", "cua-driver.exe",
];

/// The keyword a title (or exe name) contains, if it names a known game.
pub fn game_in_title(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    GAME_KEYWORDS.iter().copied().find(|g| match *g {
        // Short names only count as whole words ("gta", not "agtable").
        "gta" | "cs2" | "dota" | "halo" | "fifa" => lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|w| w == *g || (*g == "dota" && w == "dota2")),
        _ => lower.contains(g),
    })
}

/// The foreground app, as far as game detection cares.
#[derive(Debug, Clone, Default)]
pub struct Foreground {
    pub title: String,
    /// Lower-cased exe name.
    pub exe: String,
    /// Lower-cased full path, when readable.
    pub path: String,
    /// Covers its whole monitor.
    pub fullscreen: bool,
}

/// What [`detect_game`] concluded.
#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    /// A short, presentable name.
    pub name: String,
    /// Why: `library`, `title` or `fullscreen`.
    pub signal: &'static str,
}

/// Decides whether `fg` is a game. Pure, so the rules are testable.
pub fn classify(fg: &Foreground) -> Option<Game> {
    if fg.exe.is_empty() && fg.title.is_empty() {
        return None;
    }
    if NOT_GAMES.contains(&fg.exe.as_str()) {
        return None;
    }
    let pretty = |fallback: &str| {
        let title = fg.title.trim();
        if title.is_empty() {
            crate::util::title_case(fallback.trim_end_matches(".exe"))
        } else {
            crate::util::truncate(title, 28)
        }
    };
    if GAME_LIBRARIES.iter().any(|lib| fg.path.contains(lib)) {
        return Some(Game { name: pretty(&fg.exe), signal: "library" });
    }
    if let Some(keyword) = game_in_title(&fg.title).or_else(|| game_in_title(&fg.exe)) {
        let name = if keyword == "cs2" { "Counter-Strike 2".to_string() } else { crate::util::title_case(keyword) };
        return Some(Game { name, signal: "title" });
    }
    if fg.fullscreen && !fg.title.trim().is_empty() {
        return Some(Game { name: pretty(&fg.exe), signal: "fullscreen" });
    }
    None
}

fn covers_monitor(x: i32, y: i32, w: i32, h: i32) -> bool {
    let (mx, my, mw, mh) = crate::desktop::win32::monitor_rect_at(x + w / 2, y + h / 2);
    x <= mx + 2 && y <= my + 2 && x + w >= mx + mw - 2 && y + h >= my + mh - 2
}

/// The foreground app: front-most on-screen window from Cua Driver, or Win32's foreground.
pub fn foreground() -> Option<Foreground> {
    let ours = std::process::id();
    if let Some(windows) = crate::desktop::cua::windows() {
        // The driver lists overlays too (IME, GPU overlays); skip ours and the ones that are
        // not real apps, and take the front-most remaining window.
        let front = windows.into_iter().find(|w| {
            let exe = w.app_name.to_lowercase();
            w.pid != ours
                && !w.title.trim().is_empty()
                && !matches!(exe.as_str(), "textinputhost.exe" | "nvidia overlay.exe" | "cua-driver.exe" | "aria-avatar.exe")
        })?;
        let b = &front.bounds;
        return Some(Foreground {
            fullscreen: covers_monitor(b.x, b.y, b.width, b.height),
            exe: front.app_name.to_lowercase(),
            path: crate::desktop::win32::process_path(front.pid),
            title: front.title,
        });
    }
    let win = crate::desktop::win32::active_window()?;
    if win.pid == ours {
        return None;
    }
    let fullscreen = crate::desktop::win32::window_rect(win.hwnd).is_some_and(|(x, y, w, h)| covers_monitor(x, y, w, h));
    Some(Foreground { path: crate::desktop::win32::process_path(win.pid), exe: win.exe, title: win.title, fullscreen })
}

/// The game in the foreground, if any.
pub fn detect_game() -> Option<Game> {
    classify(&foreground()?)
}

/// Seconds since the last keyboard or mouse input.
pub fn idle_seconds() -> f64 {
    crate::desktop::win32::idle_seconds()
}

/// Away from the desk: no input for `threshold` seconds, unless a video or game is plainly being
/// watched (fullscreen with media playing), which is not being away.
pub fn is_away(threshold: f64) -> bool {
    if crate::desktop::win32::session_locked() {
        return true;
    }
    if idle_seconds() < threshold {
        return false;
    }
    let watching = foreground().is_some_and(|fg| fg.fullscreen) && crate::desktop::media::now_playing().is_playing;
    !watching
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fg(title: &str, exe: &str, path: &str, fullscreen: bool) -> Foreground {
        Foreground { title: title.into(), exe: exe.into(), path: path.into(), fullscreen }
    }

    #[test]
    fn games_are_found_by_library_title_and_fullscreen() {
        let steam = fg("Hollow Knight", "hollow_knight.exe", r"d:\steam\steamapps\common\hollow knight\hollow_knight.exe", false);
        assert_eq!(classify(&steam).map(|g| g.signal), Some("library"));
        let valorant = fg("VALORANT  ", "valorant-win64-shipping.exe", "", false);
        assert_eq!(classify(&valorant), Some(Game { name: "Valorant".into(), signal: "title" }));
        let unknown = fg("Indie Thing", "indie.exe", r"c:\games\indie.exe", true);
        assert_eq!(classify(&unknown).map(|g| g.signal), Some("fullscreen"));
    }

    #[test]
    fn browsers_and_players_are_not_games_even_fullscreen() {
        assert_eq!(classify(&fg("Elden Ring boss fight - YouTube", "chrome.exe", "", true)), None);
        assert_eq!(classify(&fg("movie.mkv", "vlc.exe", "", true)), None);
        assert_eq!(classify(&fg("Notes", "notepad.exe", "", false)), None);
    }

    #[test]
    fn short_keywords_need_whole_words() {
        assert_eq!(game_in_title("Grand Theft Auto V"), Some("grand theft auto"));
        assert_eq!(game_in_title("GTA V"), Some("gta"));
        assert_eq!(game_in_title("vegtable soup"), None);
        assert_eq!(game_in_title("Dota 2"), Some("dota"));
    }
}
