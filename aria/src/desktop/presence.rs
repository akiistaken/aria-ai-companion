//! What the user is doing: at the desk or away, and whether a game has the screen.
//!
//! Game detection combines executable identities, installed manifests/library paths, titles,
//! engine window classes/modules, and fullscreen graphics. Launchers and ordinary apps are excluded.
//!
//! A local worker samples Win32 independently of speech/AI. Foreground and running sessions are
//! separate: Alt-Tab restores the desktop without ending the game. No Cua Driver is required.

use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use super::{game_catalog, win32};

const GAME_EXECUTABLES: &[(&str, &str)] = &[
    ("valorant-win64-shipping.exe", "Valorant"), ("valorant.exe", "Valorant"),
    ("r5apex.exe", "Apex Legends"), ("r5apex_dx12.exe", "Apex Legends"),
    ("r5apex_dx12_retail.exe", "Apex Legends"), ("cs2.exe", "Counter-Strike 2"),
    ("league of legends.exe", "League of Legends"), ("dota2.exe", "Dota 2"),
    ("fortniteclient-win64-shipping.exe", "Fortnite"), ("gta5.exe", "Grand Theft Auto V"),
    ("gta5_enhanced.exe", "Grand Theft Auto V"), ("rdr2.exe", "Red Dead Redemption 2"),
    ("overwatch.exe", "Overwatch"), ("cod.exe", "Call of Duty"),
    ("rocketleague.exe", "Rocket League"), ("robloxplayerbeta.exe", "Roblox"),
    ("rainbowsix.exe", "Rainbow Six Siege"), ("rainbowsix_vulkan.exe", "Rainbow Six Siege"),
    ("eldenring.exe", "Elden Ring"), ("genshinimpact.exe", "Genshin Impact"),
    ("starrail.exe", "Honkai: Star Rail"), ("zenlesszonezero.exe", "Zenless Zone Zero"),
    ("destiny2.exe", "Destiny 2"), ("warframe.x64.exe", "Warframe"),
];

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
    "riot client.exe", "riotclientservices.exe", "riotclientux.exe", "riotclientuxrender.exe", "riotclientcrashhandler.exe", "eadesktop.exe", "ealauncher.exe",
    "eabackgroundservice.exe", "origin.exe", "battle.net.exe", "agent.exe", "ubisoftconnect.exe", "upc.exe",
    "galaxyclient.exe", "hoyoplay.exe", "launcher.exe", "easyanticheat.exe", "easyanticheat_eos.exe",
    "beservice.exe", "crashreportclient.exe", "crashpad_handler.exe", "werfault.exe", "uninstall.exe",
    "discord.exe", "spotify.exe", "code.exe", "notepad.exe", "notepad++.exe", "windowsterminal.exe",
    "powershell.exe", "pwsh.exe", "cmd.exe", "mstsc.exe", "winword.exe", "excel.exe", "acrord32.exe",
    "unity.exe", "unrealeditor.exe", "obs.exe", "devenv.exe", "mmc.exe", "taskmgr.exe", "systemsettings.exe",
    "snippingtool.exe", "screensketch.exe", "screenclippinghost.exe", "photos.exe",
    "antigravity.exe", "cursor.exe", "windsurf.exe", "zed.exe", "sublime_text.exe",
    "idea64.exe", "pycharm64.exe", "clion64.exe", "rider64.exe", "webstorm64.exe",
    "arc.exe", "zen.exe", "chromium.exe", "slack.exe", "teams.exe", "ms-teams.exe", "zoom.exe",
    "obsidian.exe", "notion.exe", "figma.exe", "blender.exe", "wt.exe", "alacritty.exe", "wezterm-gui.exe",
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
#[derive(Debug, Clone, Default, Serialize)]
pub struct Foreground {
    pub pid: u32,
    pub hwnd: isize,
    pub title: String,
    /// Lower-cased exe name.
    pub exe: String,
    /// Lower-cased full path, when readable.
    pub path: String,
    /// Covers its whole monitor.
    pub fullscreen: bool,
    pub class: String,
    pub game_engine: bool,
    pub graphics: bool,
}

/// What [`detect_game`] concluded.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Game {
    /// A short, presentable name.
    pub name: String,
    /// Evidence: executable, installed manifest, library, title, engine or fullscreen graphics.
    pub signal: &'static str,
}

/// Decides whether `fg` is a game. Pure, so the rules are testable.
#[cfg(test)]
pub fn classify(fg: &Foreground) -> Option<Game> {
    classify_with_catalog(fg, &[])
}

fn classify_with_catalog(fg: &Foreground, catalog: &[game_catalog::InstalledGame]) -> Option<Game> {
    if fg.exe.is_empty() && fg.title.is_empty() {
        return None;
    }
    let exe = fg.exe.to_lowercase();
    let path = game_catalog::normal_path(&fg.path);
    if excluded(&exe, &path) {
        return None;
    }
    if let Some((_, name)) = GAME_EXECUTABLES.iter().find(|(known, _)| *known == exe) {
        return Some(Game { name: (*name).into(), signal: "executable" });
    }
    if let Some(name) = game_catalog::find(catalog, &path) {
        return Some(Game { name: name.into(), signal: "installed manifest" });
    }
    let pretty = |fallback: &str| {
        let title = fg.title.trim();
        if title.is_empty() {
            crate::util::title_case(fallback.trim_end_matches(".exe"))
        } else {
            crate::util::truncate(title, 28)
        }
    };
    if GAME_LIBRARIES.iter().any(|lib| path.contains(lib)) {
        let name = GAME_LIBRARIES.iter().find_map(|lib| path.split_once(lib).map(|(_, rest)| {
            crate::util::title_case(rest.split('\\').next().unwrap_or(&exe))
        })).unwrap_or_else(|| pretty(&exe));
        return Some(Game { name, signal: "library" });
    }
    if let Some(keyword) = game_in_title(&fg.title).or_else(|| game_in_title(&fg.exe)) {
        let name = if keyword == "cs2" { "Counter-Strike 2".to_string() } else { crate::util::title_case(keyword) };
        return Some(Game { name, signal: "title" });
    }
    if fg.game_engine || fg.class.eq_ignore_ascii_case("UnityWndClass") || fg.class.eq_ignore_ascii_case("UnrealWindow")
        || (exe.ends_with("-win64-shipping.exe") && path.contains(r"\binaries\win64\")) {
        return Some(Game { name: pretty(&exe), signal: "game engine" });
    }
    None
}

fn excluded(exe: &str, path: &str) -> bool {
    NOT_GAMES.contains(&exe) || exe.contains("launcher") || exe.contains("anticheat")
        || exe.contains("crashreport") || exe.starts_with("ue4editor") || exe.starts_with("ue5editor")
        || [r"\_commonredist\", r"\redist\", r"\installer\", r"\easyanticheat\"].iter().any(|part| path.contains(part))
}

fn covers_monitor(x: i32, y: i32, w: i32, h: i32) -> bool {
    let (mx, my, mw, mh) = crate::desktop::win32::monitor_rect_at(x + w / 2, y + h / 2);
    x <= mx + 2 && y <= my + 2 && x + w >= mx + mw - 2 && y + h >= my + mh - 2
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Activity {
    pub foreground: Option<Foreground>,
    pub foreground_game: Option<Game>,
    /// Games with live windows, including minimized/background games.
    pub running_games: Vec<Game>,
    pub idle_seconds: f64,
    pub locked: bool,
    pub watching_media: bool,
    pub media: String,
    pub sampled: bool,
}

impl Activity {
    pub fn is_away(&self, threshold: f64) -> bool {
        self.locked || (self.idle_seconds >= threshold && !self.watching_media)
    }

    pub fn description(&self) -> String {
        if !self.sampled { return "Checking PC activity…".into(); }
        if self.locked { return "PC locked or secure desktop active".into(); }
        let app = self.foreground.as_ref().map(|f| {
            format!("{} ({})", crate::util::truncate(&f.title, 60), f.exe)
        }).unwrap_or_else(|| "Desktop".into());
        let presence = if self.is_away(240.0) { "Away" } else if self.watching_media { "Watching media" } else { "At the PC" };
        if let Some(game) = &self.foreground_game {
            format!("{presence} · Playing {} · detected by {}", game.name, game.signal)
        } else if !self.running_games.is_empty() {
            format!("{presence} · {app} · Game open in background: {}", self.running_games.iter().map(|g| g.name.as_str()).collect::<Vec<_>>().join(", "))
        } else {
            format!("{presence} · {app}")
        }
    }
}

#[derive(Default, Clone)]
struct Tracker {
    latest: Activity,
    last_foreground: Option<(isize, u32)>,
    controllers: [Option<[i32; 7]>; 4],
    controller_input: Option<Instant>,
    modules: std::collections::HashMap<u32, (Instant, String, bool, bool)>,
}

static TRACKER: Mutex<Option<Tracker>> = Mutex::new(None);

pub fn latest() -> Activity {
    TRACKER.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|t| t.latest.clone()).unwrap_or_default()
}

fn overlay(win: &win32::WindowInfo) -> bool {
    win.pid == std::process::id() || matches!(win.exe.as_str(),
        "aria-avatar.exe" | "cua-driver.exe" | "textinputhost.exe" | "nvidia overlay.exe" | "gamebar.exe" | "gamebarftserver.exe")
}

fn as_foreground(win: &win32::WindowInfo) -> Foreground {
    Foreground { pid: win.pid, hwnd: win.hwnd, title: win.title.clone(), exe: win.exe.clone(),
        path: win32::process_path(win.pid), class: win.class.clone(),
        fullscreen: !win32::window_minimized(win.hwnd) && win32::window_rect(win.hwnd).is_some_and(|(x, y, w, h)| covers_monitor(x, y, w, h)),
        ..Default::default() }
}

/// Keep the app beneath Aria when her chat/overlay owns focus. An unrelated active app always wins.
fn select_foreground(active: Option<&win32::WindowInfo>, windows: &[win32::WindowInfo], last: Option<(isize, u32)>) -> Option<win32::WindowInfo> {
    if let Some(win) = active.filter(|w| !overlay(w)) { return Some(win.clone()); }
    if let Some((hwnd, pid)) = last {
        if let Some(win) = windows.iter().find(|w| w.hwnd == hwnd && w.pid == pid && w.visible && !win32::window_minimized(w.hwnd)) {
            return Some(win.clone());
        }
    }
    None
}

/// Local sample only. Called on its own worker every two seconds; never calls Cua or AI.
pub fn refresh() -> Activity {
    log::debug!("presence: checking desktop");
    let locked = win32::session_locked();
    log::debug!("presence: reading foreground");
    let active = win32::active_window();
    log::debug!("presence: enumerating windows");
    let windows = win32::enum_windows(false);
    log::debug!("presence: classifying {} windows", windows.len());
    let catalog = game_catalog::installed();
    let playing = super::media::latest();
    // Native queries must not hold the snapshot lock: the UI reads it while the worker probes.
    let mut tracker = TRACKER.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
    poll_controllers(&mut tracker);
    let idle = win32::idle_seconds().min(tracker.controller_input.map(|at| at.elapsed().as_secs_f64()).unwrap_or(f64::MAX));
    let window = if locked { None } else { select_foreground(active.as_ref(), &windows, tracker.last_foreground) };
    if let Some(win) = &window { tracker.last_foreground = Some((win.hwnd, win.pid)); }
    let mut foreground = window.as_ref().map(as_foreground);
    if let Some(fg) = foreground.as_mut().filter(|fg| !excluded(&fg.exe, &fg.path)) {
        if classify_with_catalog(fg, &catalog).is_none() {
            // Expire PID/module entries: games can initialize graphics after showing their window.
            let stale = tracker.modules.get(&fg.pid).is_none_or(|(at, path, _, _)| path != &fg.path || at.elapsed() >= Duration::from_secs(15));
            if stale {
                let modules = win32::process_modules(fg.pid);
                let engine = modules.iter().any(|m| matches!(m.as_str(), "unityplayer.dll" | "gameassembly.dll" | "cryengine.dll"));
                let graphics = modules.iter().any(|m| matches!(m.as_str(), "d3d9.dll" | "d3d11.dll" | "d3d12.dll" | "vulkan-1.dll" | "opengl32.dll"));
                tracker.modules.insert(fg.pid, (Instant::now(), fg.path.clone(), engine, graphics));
            }
            if let Some((_, _, engine, graphics)) = tracker.modules.get(&fg.pid) { fg.game_engine = *engine; fg.graphics = *graphics; }
        }
    }
    tracker.modules.retain(|_, (at, _, _, _)| at.elapsed() < Duration::from_secs(60));
    let foreground_game = foreground.as_ref().and_then(|fg| classify_with_catalog(fg, &catalog));
    let mut running_games = Vec::new();
    // Only real app windows: no background launchers/services inferred as gameplay.
    for win in windows.iter().filter(|w| !overlay(w) && (!w.title.trim().is_empty() || GAME_EXECUTABLES.iter().any(|(exe, _)| *exe == w.exe))) {
        let mut fg = as_foreground(win);
        if let Some((_, _, engine, graphics)) = tracker.modules.get(&fg.pid).filter(|(_, path, _, _)| path == &fg.path) {
            fg.game_engine = *engine;
            fg.graphics = *graphics;
        }
        if let Some(game) = classify_with_catalog(&fg, &catalog) {
            if !running_games.iter().any(|g: &Game| g.name == game.name) { running_games.push(game); }
        }
    }
    // Protected games may have no enumerable visible window while loading or minimized.
    // Explicit game executable identities still establish a running session, never focus.
    for exe in win32::running_process_names().values() {
        if let Some((_, name)) = GAME_EXECUTABLES.iter().find(|(known, _)| known == exe) {
            if !running_games.iter().any(|game: &Game| game.name == *name) {
                running_games.push(Game { name: (*name).into(), signal: "running executable" });
            }
        }
    }
    if let Some(game) = &foreground_game {
        if !running_games.iter().any(|g| g.name == game.name) { running_games.push(game.clone()); }
    }
    running_games.sort_by(|a, b| a.name.cmp(&b.name));
    let watching_media = foreground.as_ref().is_some_and(|fg| fg.fullscreen && playing.is_playing && fg.exe.eq_ignore_ascii_case(&playing.exe));
    let activity = Activity { foreground, foreground_game, running_games, idle_seconds: idle, locked,
        watching_media, media: if playing.is_playing { playing.label() } else { String::new() }, sampled: true };
    if !tracker.latest.sampled || activity.foreground_game != tracker.latest.foreground_game
        || activity.running_games != tracker.latest.running_games || activity.locked != tracker.latest.locked {
        log::info!("PC activity: {}", activity.description());
    }
    tracker.latest = activity.clone();
    *TRACKER.lock().unwrap_or_else(|e| e.into_inner()) = Some(tracker);
    activity
}

fn poll_controllers(tracker: &mut Tracker) {
    use windows::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};
    for index in 0..4 {
        let mut state = XINPUT_STATE::default();
        if unsafe { XInputGetState(index as u32, &mut state) } != 0 {
            tracker.controllers[index] = None;
            continue;
        }
        let g = state.Gamepad;
        // Ignore resting stick/trigger noise and connection itself as user input.
        let axis = |v: i16| if i32::from(v).abs() <= 8500 { 0 } else { i32::from(v) / 2048 };
        let trigger = |v: u8| if v <= 30 { 0 } else { i32::from(v) / 16 };
        let input = [i32::from(g.wButtons.0), trigger(g.bLeftTrigger), trigger(g.bRightTrigger),
            axis(g.sThumbLX), axis(g.sThumbLY), axis(g.sThumbRX), axis(g.sThumbRY)];
        if tracker.controllers[index].is_some_and(|old| old != input) || input != [0; 7] {
            tracker.controller_input = Some(Instant::now());
        }
        tracker.controllers[index] = Some(input);
    }
}

pub fn foreground() -> Option<Foreground> { latest().foreground }

/// The game in the foreground, if any.
pub fn detect_game() -> Option<Game> {
    latest().foreground_game
}

/// Seconds since keyboard, mouse or meaningful controller input.
pub fn idle_seconds() -> f64 {
    let activity = latest();
    if activity.sampled { activity.idle_seconds } else { win32::idle_seconds() }
}

/// Away after `threshold` seconds without input; fullscreen media viewing keeps presence active.
pub fn is_away(threshold: f64) -> bool {
    let activity = latest();
    if activity.sampled { activity.is_away(threshold) } else { win32::session_locked() || win32::idle_seconds() >= threshold }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fg(title: &str, exe: &str, path: &str, fullscreen: bool) -> Foreground {
        Foreground { title: title.into(), exe: exe.into(), path: path.into(), fullscreen, graphics: fullscreen, ..Default::default() }
    }

    #[test]
    fn games_are_found_by_library_executable_and_title() {
        let steam = fg("Hollow Knight", "hollow_knight.exe", r"d:\steam\steamapps\common\hollow knight\hollow_knight.exe", false);
        assert_eq!(classify(&steam).map(|g| g.signal), Some("library"));
        let valorant = fg("VALORANT  ", "valorant-win64-shipping.exe", "", false);
        assert_eq!(classify(&valorant), Some(Game { name: "Valorant".into(), signal: "executable" }));
        let by_title = fg("Elden Ring", "custom_client.exe", r"c:\custom\client.exe", false);
        assert_eq!(classify(&by_title), Some(Game { name: "Elden Ring".into(), signal: "title" }));
    }

    #[test]
    fn gpu_accelerated_non_game_fullscreen_apps_are_not_games() {
        let mut viewer = fg("3D Viewport", "viewer3d.exe", r"c:\tools\viewer3d.exe", true);
        viewer.graphics = true;
        assert_eq!(classify(&viewer), None, "GPU usage and fullscreen alone must not trigger game detection");
        let ide = fg("Valorant - Workspace", "antigravity.exe", r"c:\programs\antigravity.exe", true);
        assert_eq!(classify(&ide), None, "excluded GPU apps must never trigger even when a title matches");
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

    #[test]
    fn valorant_and_apex_are_detected_without_titles_paths_or_fullscreen() {
        for (exe, expected) in [
            ("VALORANT-Win64-Shipping.exe", "Valorant"),
            ("r5apex.exe", "Apex Legends"),
            ("r5apex_dx12.exe", "Apex Legends"),
            ("r5apex_dx12_retail.exe", "Apex Legends"),
        ] {
            let game = classify(&fg("", exe, "", false)).expect(exe);
            assert_eq!(game.name, expected);
            assert_eq!(game.signal, "executable");
        }
    }

    #[test]
    fn launchers_editors_and_helpers_are_excluded_even_in_game_libraries() {
        for exe in ["Riot Client.exe", "RiotClientServices.exe", "RiotClientCrashHandler.exe", "EpicGamesLauncher.exe", "EADesktop.exe",
            "EasyAntiCheat_EOS.exe", "CrashReportClient.exe", "Unity.exe", "UnrealEditor.exe"] {
            assert!(classify(&fg("VALORANT / Apex Legends", exe, r"D:\Steam\steamapps\common\Game\helper.exe", true)).is_none(), "{exe}");
        }
        let mut app = fg("A presentation", "presentation.exe", "", true);
        app.graphics = false;
        assert_eq!(classify(&app), None, "covering a monitor alone does not prove gameplay");
    }

    #[test]
    fn riot_electron_launcher_is_excluded_from_riot_install_library() {
        assert!(classify(&fg("Riot Client", "Riot Client.exe",
            r"D:\Riot Games\Riot Client\RiotClientElectron\Riot Client.exe", false)).is_none());
        assert_eq!(classify(&fg("", "VALORANT-Win64-Shipping.exe", "", false)).unwrap().name, "Valorant");
    }

    #[test]
    fn installed_names_and_engine_windows_find_windowed_games() {
        let catalog = vec![game_catalog::InstalledGame { root: r"e:\custom\indie".into(), name: "A New Indie Game".into() }];
        let window = fg("Loading", "game.exe", r"E:\Custom\Indie\bin\game.exe", false);
        assert_eq!(classify_with_catalog(&window, &catalog).unwrap().name, "A New Indie Game");
        let mut unity = fg("An Unknown Game", "game.exe", "", false);
        unity.class = "UnityWndClass".into();
        assert_eq!(classify(&unity).unwrap().signal, "game engine");
    }

    fn window(pid: u32, exe: &str) -> win32::WindowInfo {
        win32::WindowInfo { hwnd: 0, pid, title: "Window".into(), class: String::new(), exe: exe.into(), visible: true }
    }

    #[test]
    fn actual_focus_wins_over_overlay_z_order_and_aria_preserves_game_focus() {
        let game = window(42, "r5apex.exe");
        let browser = window(43, "chrome.exe");
        let aria = window(std::process::id(), "aria.exe");
        let windows = vec![aria.clone(), browser.clone(), game.clone()];
        assert_eq!(select_foreground(Some(&game), &windows, None).unwrap().pid, game.pid);
        assert_eq!(select_foreground(Some(&aria), &windows, Some((0, game.pid))).unwrap().pid, game.pid);
        assert_eq!(select_foreground(Some(&browser), &windows, Some((0, game.pid))).unwrap().pid, browser.pid);
        assert!(select_foreground(Some(&aria), &windows, Some((0, 999))).is_none());
    }

    #[test]
    fn presence_distinguishes_idle_viewing_controller_input_and_secure_desktop() {
        let mut activity = Activity { sampled: true, idle_seconds: 300.0, ..Default::default() };
        assert!(activity.is_away(240.0));
        activity.watching_media = true;
        assert!(!activity.is_away(240.0));
        activity.locked = true;
        assert!(activity.is_away(240.0));
        activity.locked = false;
        activity.watching_media = false;
        activity.idle_seconds = 2.0;
        assert!(!activity.is_away(240.0));
    }

    #[test]
    fn native_activity_probe_returns_a_snapshot() {
        let activity = refresh();
        assert!(activity.sampled);
        assert!(activity.idle_seconds >= 0.0);
        assert!(!activity.description().is_empty());
    }
}
