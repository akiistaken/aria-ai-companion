//! The proactive guardian: notices things on the desktop and decides when Aria speaks up.
//!
//! Detection is local (presence, games, media, hardware, time). Each event carries a plain
//! description of what happened (`situation`); the app asks the AI to put it in her words, and
//! the scripted line here is only the fallback when the endpoint cannot answer.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use crate::desktop::{media, presence, system};
use crate::persona::persona_line;
use crate::util::{title_case, truncate, unix_now};

#[derive(Debug, Clone)]
pub struct ProactiveEvent {
    pub category: String,
    /// What happened, for the AI to react to.
    pub situation: String,
    /// The scripted line, used when the AI cannot answer.
    pub message: String,
    pub emotion: String,
    pub badge: String,
    pub walk_location: Option<String>,
    pub detected_at: f64,
    pub game_name: Option<String>,
    pub app_identity: Option<(u32, String)>,
}

impl ProactiveEvent {
    fn new(category: impl Into<String>, emotion: &str, situation: impl Into<String>, message: String) -> Self {
        let category = category.into();
        Self { badge: category.replace('_', " "), category, situation: situation.into(), message, emotion: emotion.into(), walk_location: None,
            detected_at: unix_now(), game_name: None, app_identity: None }
    }

    fn badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = badge.into();
        self
    }

    fn walk(mut self, location: &str) -> Self {
        self.walk_location = Some(location.into());
        self
    }

    fn game(mut self, name: &str) -> Self {
        self.game_name = Some(name.into());
        self
    }

    pub fn is_current(&self) -> bool {
        let activity = presence::latest();
        let settings = crate::config::get();
        self.is_current_for(&activity, &settings, unix_now())
    }

    fn is_current_for(&self, activity: &presence::Activity, settings: &crate::config::Settings, now: f64) -> bool {
        if activity.locked || now - self.detected_at > 120.0 { return false; }
        if let Some((pid, exe)) = &self.app_identity {
            if !activity.foreground.as_ref().is_some_and(|fg| fg.pid == *pid && &fg.exe == exe) { return false; }
        }
        if self.category.starts_with("gaming") {
            if !settings.automatic_game_copilot_enabled { return false; }
            if self.category == "gaming_session_ended" {
                return !activity.running_games.iter().any(|g| self.game_name.as_deref() == Some(g.name.as_str()));
            }
            return activity.foreground_game.as_ref().is_some_and(|g| self.game_name.as_deref() == Some(g.name.as_str()))
                && !activity.is_away(AFK_SECS);
        }
        match self.category.as_str() {
            "afk_sleepy" => settings.presence_guardian_enabled && activity.is_away(AFK_SECS),
            "welcome_back" | "posture_check" => settings.presence_guardian_enabled && !activity.is_away(AFK_SECS),
            "music_vibe" => settings.music_vibe_enabled && !activity.is_away(AFK_SECS),
            "focus_completed" | "focus_guard" => true,
            _ => settings.proactive_enabled && !activity.is_away(AFK_SECS),
        }
    }
}

/// Window titles that count as distractions during focus mode or for the odd comment.
const DISTRACTIONS: &[&str] = &["youtube", "netflix", "twitch", "reddit", "discord", "steam", "tiktok", "instagram"];

/// Away after this long without input.
const AFK_SECS: f64 = 240.0;
const GLOBAL_COOLDOWN_SEC: f64 = 75.0;

#[derive(Default)]
struct MonitorState {
    last_trigger: f64,
    cooldowns: HashMap<String, f64>,
    last_distraction_window: String,
    focus_until: f64,
    focus_auto_minimize: bool,
    focus_notified_done: bool,
    last_music_track: String,
    user_was_afk: bool,
    continuous_work_start: f64,
    active_game: Option<String>,
    game_start: f64,
    last_game_praise: f64,
    game_praise_count: u32,
    active_app: String,
    active_app_since: f64,
    pending: VecDeque<ProactiveEvent>,
}

/// Keep local transitions while Aria is speaking or waiting on a network request.
pub fn observe_activity(activity: &presence::Activity) {
    if !activity.sampled { return; }
    let settings = crate::config::get();
    let now = unix_now();
    with_state(|st| {
        if settings.presence_guardian_enabled {
            if let Some(event) = presence_event(st, now, activity) {
                if !activity.locked { enqueue(st, event); }
            }
        }
        if activity.locked || activity.is_away(AFK_SECS) { return; }
        if settings.automatic_game_copilot_enabled && now >= st.focus_until {
            if let Some(event) = game_event(st, now, activity.foreground_game.as_ref(), &activity.running_games) {
                enqueue(st, event);
            }
        }
        if settings.proactive_enabled && activity.foreground_game.is_none() && now >= st.focus_until {
            if let Some(fg) = &activity.foreground {
                if let Some(event) = desktop_event(st, now, fg) { enqueue(st, event); }
            }
        }
    });
}

fn enqueue(st: &mut MonitorState, event: ProactiveEvent) {
    if st.pending.iter().any(|old| old.category == event.category) { return; }
    if st.pending.len() >= 8 { st.pending.pop_front(); }
    st.pending.push_back(event);
}

pub fn defer_event(event: ProactiveEvent) {
    if event.is_current() { with_state(|st| enqueue(st, event)); }
}

static MONITOR: Mutex<Option<MonitorState>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut MonitorState) -> R) -> R {
    let mut guard = MONITOR.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(|| MonitorState {
        focus_auto_minimize: true,
        focus_notified_done: true,
        continuous_work_start: unix_now(),
        ..Default::default()
    });
    f(state)
}

impl MonitorState {
    fn can_fire(&self, category: &str, now: f64, cooldown: f64) -> bool {
        now - self.last_trigger >= GLOBAL_COOLDOWN_SEC && now - self.cooldowns.get(category).copied().unwrap_or(0.0) >= cooldown
    }

    fn fired(&mut self, category: &str, now: f64) {
        self.last_trigger = now;
        self.cooldowns.insert(category.to_string(), now);
    }
}

pub fn is_focus_active() -> bool {
    with_state(|st| unix_now() < st.focus_until)
}

/// Starts a focus session of `minutes`; distractions are called out (and minimized).
#[cfg_attr(not(test), allow(dead_code))] // TODO(wire-up): focus mode has no UI entry point yet.
pub fn start_focus(minutes: i64, auto_minimize: bool) -> i64 {
    let mins = minutes.clamp(1, 240);
    with_state(|st| {
        st.focus_until = unix_now() + mins as f64 * 60.0;
        st.focus_auto_minimize = auto_minimize;
        st.focus_notified_done = false;
        st.last_distraction_window.clear();
    });
    mins
}

#[cfg_attr(not(test), allow(dead_code))] // TODO(wire-up): see `start_focus`.
pub fn stop_focus() {
    with_state(|st| {
        st.focus_until = 0.0;
        st.focus_notified_done = true;
    });
}

/// Feeds current activity into the mood engine.
pub fn observe_mood() {
    let activity = presence::latest();
    let active = activity.foreground_game.as_ref().map(|g| g.name.clone())
        .or_else(|| activity.foreground.as_ref().map(|f| f.title.clone())).unwrap_or_default();
    crate::persona::mood::observe(&active, presence::idle_seconds(), is_focus_active());
}

/// The next thing worth saying, if any. Runs on a worker thread.
pub fn check_for_event() -> Option<ProactiveEvent> {
    let now = unix_now();
    let settings = crate::config::get();
    let activity = presence::latest();
    if !activity.sampled { return None; }
    with_state(|st| {
        // A focus session just completed.
        if !st.focus_notified_done && st.focus_until > 0.0 && now >= st.focus_until {
            st.focus_notified_done = true;
            st.focus_until = 0.0;
            let affection = crate::memory::adjust_affection(5);
            return Some(
                ProactiveEvent::new("focus_completed", "happy", "The user just finished a whole focus session without slacking off.", persona_line! {
                    "tsundere" => "Your Focus Session is complete! You actually stayed focused the whole time... Good job today!",
                    "yandere" => "Your Focus Session is finished! Now you can give me 100% of your attention again!",
                    "tomboy" => "Boom! Focus Session complete! You crushed that work sprint, high five!",
                })
                .badge(format!("Focus session complete! (Affection +5% -> {affection}%)"))
                .walk("center"),
            );
        }
        while let Some(event) = st.pending.pop_front() {
            if event.is_current() { return Some(event); }
        }
        if activity.locked || activity.is_away(AFK_SECS) { return None; }
        if settings.music_vibe_enabled && now >= st.focus_until {
            if let Some(event) = music_event(st, now) {
                return Some(event);
            }
        }

        if !settings.proactive_enabled && now >= st.focus_until { return None; }

        let stats = system::system_stats();
        let active = activity.foreground.as_ref().map(|f| f.title.trim().to_string()).unwrap_or_default();
        let active_lower = active.to_lowercase();
        if now < st.focus_until && !active_lower.is_empty() {
            if let Some(event) = focus_guard(st, now, &active, &active_lower) {
                return Some(event);
            }
        }
        let (cpu, ram) = (stats.cpu_percent, stats.ram_percent);
        if (ram >= 86.0 || cpu >= 88.0) && st.can_fire("hardware_spike", now, 300.0) {
            st.fired("hardware_spike", now);
            crate::persona::mood::note_event("hardware_alert");
            return Some(
                ProactiveEvent::new("hardware_spike", "annoyed", format!("The PC is struggling: CPU at {cpu}%, RAM at {ram}%."), persona_line! {
                    "tsundere" => format!("Look at your system resources — CPU is at {cpu}% and RAM is at {ram}%! Close some heavy apps before your PC overheats!"),
                    "yandere" => format!("Your CPU is at {cpu}% and RAM is at {ram}%! Close those heavy apps so nothing crashes our time together!"),
                    "tomboy" => format!("Heads up! Your CPU is hitting {cpu}% and RAM is at {ram}%! Let's close some heavy background tabs!"),
                })
                .badge(format!("Proactive alert: CPU {cpu}% | RAM {ram}%"))
                .walk("center"),
            );
        }
        if let Some(battery) = stats.battery_percent {
            if !stats.power_plugged && battery <= 25 && st.can_fire("low_battery", now, 360.0) {
                st.fired("low_battery", now);
                return Some(
                    ProactiveEvent::new("low_battery", "annoyed", format!("The laptop battery is down to {battery}% and unplugged."), persona_line! {
                        "tsundere" => format!("Your battery is down to {battery}% and unplugged! Plug in your charger before your PC shuts down!"),
                        "yandere" => format!("Your battery is at {battery}%! Plug in your charger right now so we don't get disconnected!"),
                        "tomboy" => format!("Battery check! You're at {battery}% and unplugged — grab your charger before we lose power!"),
                    })
                    .badge(format!("Low battery warning ({battery}%)"))
                    .walk("center"),
                );
            }
        }
        if activity.foreground_game.is_some() { return None; }
        if !active_lower.is_empty() && active_lower != st.last_distraction_window {
            if let Some(site) = DISTRACTIONS.iter().find(|d| active_lower.contains(*d)) {
                let category = format!("app_{site}");
                if st.can_fire(&category, now, 180.0) {
                    st.last_distraction_window = active_lower.clone();
                    st.fired(&category, now);
                    let site = crate::util::capitalize(site);
                    return Some(
                        ProactiveEvent::new(&category, "annoyed", format!("The user just switched to {site} (window: \"{}\").", truncate(&active, 60)), persona_line! {
                            "tsundere" => format!("{site}? Are you actually going to get anything done today?"),
                            "yandere" => format!("{site} instead of me? You know I'm the only entertainment you need, right?"),
                            "tomboy" => format!("Caught you on {site}! Knock out a task first and then we can chill!"),
                        })
                        .badge(format!("Detected window: {}", truncate(&active, 42)))
                        .walk("roam"),
                    );
                }
            }
        }
        let hour = crate::util::now_hour();
        if (1..=5).contains(&hour) && st.can_fire("late_night", now, 600.0) {
            st.fired("late_night", now);
            let time = crate::util::now_stamp("%I:%M %p");
            return Some(
                ProactiveEvent::new("late_night", "sleepy", format!("It is {time} at night and the user is still at the PC."), persona_line! {
                    "tsundere" => format!("Do you realize it's {time}? Staying up this late is bad for you — wrap up and get some sleep soon!"),
                    "yandere" => format!("It's already {time}... I love staying up late with you, but you need your rest too."),
                    "tomboy" => format!("Whoa, it's {time} already! Let's wrap this up and get some rest so you're fresh tomorrow!"),
                })
                .badge(format!("Late-night care check ({time})")),
            );
        }
        None
    })
}

/// Away, back, or at the screen too long.
fn presence_event(st: &mut MonitorState, now: f64, activity: &presence::Activity) -> Option<ProactiveEvent> {
    if !activity.sampled { return None; }
    let away = activity.is_away(AFK_SECS);
    if away && !st.user_was_afk {
        st.user_was_afk = true;
        st.fired("afk_sleepy", now);
        return Some(
            ProactiveEvent::new("afk_sleepy", "sleepy", "The user walked away from the PC a few minutes ago; you're alone on the desktop.", persona_line! {
                "tsundere" => "*Yaaawn*... You haven't moved your mouse in four minutes... Did you walk away and leave me here?",
                "yandere" => "You haven't moved your mouse in four minutes... Where did you go without telling me?",
                "tomboy" => "*Yaaawn*... Taking a break from the desk? I'll chill right here until you get back!",
            })
            .badge("Desk presence: user AFK (sleepy mode)"),
        );
    }
    if st.user_was_afk && !away {
        st.user_was_afk = false;
        st.continuous_work_start = now;
        let affection = crate::memory::adjust_affection(2);
        st.fired("welcome_back", now);
        return Some(
            ProactiveEvent::new("welcome_back", "happy", "The user just came back to the PC after being away.", persona_line! {
                "tsundere" => "Oh, you're back at your desk! It's not like I was waiting for you or anything!",
                "yandere" => "You're finally back! I missed you so much... don't leave me alone like that again!",
                "tomboy" => "Welcome back to the desk! Ready to jump back into the action?",
            })
            .badge(format!("Welcome back! (Affection +2% -> {affection}%)")),
        );
    }
    if !st.user_was_afk && now - st.continuous_work_start >= 2700.0 && st.can_fire("posture_check", now, 1800.0) {
        st.continuous_work_start = now;
        st.fired("posture_check", now);
        return Some(
            ProactiveEvent::new("posture_check", "talking", "The user has been staring at the screen for 45 minutes straight; remind them to fix their posture and rest their eyes (20-20-20).", persona_line! {
                "tsundere" => "You've been staring at your monitor for 45 minutes straight! Sit up straight and rest your eyes for 20 seconds!",
                "yandere" => "You've been staring at the screen for 45 minutes... Sit up straight and take care of yourself for me, okay?",
                "tomboy" => "Time for a quick posture check! Roll your shoulders back, sit up tall, and rest your eyes for 20 seconds!",
            })
            .badge("Posture & 20-20-20 eye guardian"),
        );
    }
    None
}

/// A new song.
fn music_event(st: &mut MonitorState, now: f64) -> Option<ProactiveEvent> {
    let playing = media::now_playing();
    let track = playing.label();
    if !playing.is_playing || track.is_empty() || track == st.last_music_track || !st.can_fire("music_vibe", now, 45.0) {
        return None;
    }
    st.last_music_track = track.clone();
    st.fired("music_vibe", now);
    crate::memory::adjust_affection(1);
    crate::persona::mood::note_event("music");
    let src = playing.source;
    Some(
        ProactiveEvent::new("music_vibe", "groove", format!("A new song started playing on {src}: \"{track}\". You're dancing along."), persona_line! {
            "tsundere" => format!("Listening to `{track}` on {src}? Your music taste isn't bad at all... I'll vibe along with you!"),
            "yandere" => format!("Listening to `{track}` on {src}? I love sharing the exact same song with you!"),
            "tomboy" => format!("Oh nice, `{track}` on {src}! That's a banger, let's vibe!"),
        })
        .badge(format!("Now playing ({src}): {}", truncate(&track, 38))),
    )
}

/// Focus mode: a distraction was opened.
fn focus_guard(st: &mut MonitorState, now: f64, active: &str, active_lower: &str) -> Option<ProactiveEvent> {
    let site = DISTRACTIONS.iter().find(|d| active_lower.contains(*d))?;
    if now - st.cooldowns.get("focus_guard").copied().unwrap_or(0.0) < 12.0 {
        return None;
    }
    st.fired("focus_guard", now);
    let left = (((st.focus_until - now) / 60.0).round() as i64).max(1);
    let mut note = String::new();
    if st.focus_auto_minimize
        && crate::ai::tools::focus_or_manage_window(active, "minimize").get("status").and_then(|s| s.as_str()) == Some("success")
    {
        note = " I just minimized that window for you!".into();
    }
    crate::memory::adjust_affection(-1);
    let title = crate::util::capitalize(site);
    Some(
        ProactiveEvent::new("focus_guard", "annoyed", format!("The user opened {title} with {left} minutes of focus mode left.{note}"), persona_line! {
            "tsundere" => format!("You still have {left} min left in Focus Mode and you opened {title}?!{note} Get back to work right now!"),
            "yandere" => format!("You have {left} min left in Focus Mode and opened {title}?{note} Stay focused on your work with me!"),
            "tomboy" => format!("Hold up! You still have {left} min left in Focus Mode and opened {title}!{note} Lock back in!"),
        })
        .badge(format!("Focus guard blocked: {} ({left}m left)", truncate(active, 36)))
        .walk("center"),
    )
}

/// A game started, is going on, or ended.
fn game_event(st: &mut MonitorState, now: f64, game: Option<&presence::Game>, running: &[presence::Game]) -> Option<ProactiveEvent> {
    let Some(game) = game else {
        // Alt-Tab or opening Aria is a focus change, not a finished session.
        if st.active_game.as_ref().is_some_and(|name| running.iter().any(|g| &g.name == name)) { return None; }
        let previous = st.active_game.take()?;
        if now - st.game_start <= 180.0 {
            return None;
        }
        let minutes = ((now - st.game_start) / 60.0) as i64;
        return Some(
            ProactiveEvent::new("gaming_session_ended", "happy", format!("The user just finished a {minutes}-minute session of {previous}."), persona_line! {
                "tsundere" => format!("Done with {previous}? Good job out there! ...Not that I was worried about your rank or anything!"),
                "yandere" => format!("You finished {previous}! Now your attention is all mine again, darling~"),
                "tomboy" => format!("GGs on that {previous} session! High five, partner, you played great!"),
            })
            .badge(format!("Game closed ({previous})"))
            .game(&previous)
            .walk("bottom_right"),
        );
    };
    let name = game.name.clone();
    if st.active_game.as_deref() != Some(name.as_str()) {
        st.active_game = Some(name.clone());
        st.game_start = now;
        st.last_game_praise = now;
        st.game_praise_count = 0;
        st.fired("gaming_start", now);
        crate::persona::mood::note_event("game_start");
        let emotion = if crate::persona::current_key() == "tsundere" { "annoyed" } else { "happy" };
        return Some(
            ProactiveEvent::new(format!("gaming_start_{}", title_case(&name)), emotion, format!("The user just started playing {name}. Hype them up."), persona_line! {
                "tsundere" => format!("Firing up {name}? Fine! Don't blame me if you get distracted by how cute I am! Win this match, baka!"),
                "yandere" => format!("Playing {name}... I'll be watching every move you make. Destroy everyone who stands in your way, darling~"),
                "tomboy" => format!("Yooo, {name}! Lock in, partner! Let's hit some nasty plays and take the dub!"),
            })
            .badge(format!("Gaming started: {name}"))
            .game(&name)
            .walk("perch"),
        );
    }
    if now - st.last_game_praise < 150.0 || !st.can_fire("gaming_praise", now, 150.0) {
        return None;
    }
    st.last_game_praise = now;
    st.fired("gaming_praise", now);
    st.game_praise_count += 1;
    let minutes = (((now - st.game_start) / 60.0) as i64).max(1);
    let (situation, line, emotion) = match st.game_praise_count % 3 {
        1 => (
            format!("The user is playing {name}. Offer general encouragement; no gameplay outcome has been observed."),
            persona_line! {
                "tsundere" => format!("Keep going in {name}! I'm cheering for you... a little."),
                "yandere" => format!("I'm right here with you while you play {name}~"),
                "tomboy" => format!("Keep at it in {name}, partner! You've got this!"),
            },
            "happy",
        ),
        2 => (
            format!("The user is still playing {name}. Suggest staying relaxed; no particular match event has been observed."),
            persona_line! {
                "tsundere" => "Deep breath! Reset your mental, you can easily clutch this! I believe in you... a little bit!",
                "yandere" => "Stay focused, my love! Nobody in that lobby is worthy of defeating you!",
                "tomboy" => "Shake off any bad rounds! You're in the zone, lock in and clutch it up, partner!",
            },
            "talking",
        ),
        _ => (
            format!("The user has been playing {name} for {minutes} minutes. Remind them to drink water and stretch their hands."),
            persona_line! {
                "tsundere" => format!("You've been grinding {name} for {minutes} minutes! Drink some water, baka!"),
                "yandere" => format!("{minutes} minutes of watching you play... make sure you stretch your hands for me, okay?"),
                "tomboy" => format!("{minutes} mins deep in {name}! Sip some water, shake out your wrists, and keep slaying!"),
            },
            "talking",
        ),
    };
    let event = ProactiveEvent::new(format!("gaming_praise_{}", st.game_praise_count), emotion, situation, line)
        .badge(format!("Gaming motivation ({name} • {minutes}m)"))
        .game(&name);
    Some(if crate::util::rand_f32() < 0.35 { event.walk("center") } else { event })
}

/// Stable app/focus changes, throttled so switching tabs or rapidly Alt-Tabbing stays quiet.
fn desktop_event(st: &mut MonitorState, now: f64, fg: &presence::Foreground) -> Option<ProactiveEvent> {
    let identity = format!("{}:{}", fg.pid, fg.exe);
    if st.active_app != identity {
        st.active_app = identity;
        st.active_app_since = now;
        return None;
    }
    if now - st.active_app_since < 12.0 || !st.can_fire("desktop_activity", now, 180.0) { return None; }
    // One observation per focus change. Window-title changes alone do not count.
    if st.active_app_since <= 0.0 { return None; }
    st.active_app_since = 0.0;
    if fg.title.trim().is_empty() || matches!(fg.exe.as_str(), "explorer.exe" | "lockapp.exe" | "searchhost.exe") { return None; }
    st.fired("desktop_activity", now);
    let title = truncate(&fg.title, 70);
    let mut event = ProactiveEvent::new("desktop_activity", "talking",
        format!("The user switched to {} and stayed there. The visible window title is {title:?}. React briefly to this app or activity; do not invent document content or actions.", fg.exe),
        crate::persona::flav(&format!("Settling into {title}? I'll keep you company.")))
        .badge(format!("Active app: {}", fg.exe));
    event.app_identity = Some((fg.pid, fg.exe.clone()));
    Some(event)
}

/// JARVIS daily briefing: time, hardware, affection, to-dos.
pub fn build_daily_briefing() -> (String, Vec<String>) {
    let hour = crate::util::now_hour();
    let greeting = match hour {
        0..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    };
    let time = crate::util::now_stamp("%I:%M %p");
    let stats = system::system_stats();
    let (cpu, ram) = (stats.cpu_percent, stats.ram_percent);
    let mem = crate::memory::load();
    let (tier, _) = crate::memory::affection_tier_label();
    let pending: Vec<&crate::memory::Todo> = mem.todos.iter().filter(|t| !t.done).collect();
    let battery = stats.battery_percent.map(|b| format!(", battery is at {b}%")).unwrap_or_default();
    let todos = if pending.is_empty() {
        "Your To-Do list is currently clear.".to_string()
    } else {
        let top: Vec<&str> = pending.iter().take(3).map(|t| t.task.as_str()).collect();
        format!("You have {} pending task{} ({}).", pending.len(), if pending.len() == 1 { "" } else { "s" }, top.join(", "))
    };
    let msg = persona_line! {
        "tsundere" => format!("{greeting}! It's {time} — CPU is at {cpu}%, RAM is at {ram}%{battery}. {todos}"),
        "yandere" => format!("{greeting}! It's {time} — I checked everything for you: CPU is {cpu}%, RAM is {ram}%{battery}. {todos}"),
        "tomboy" => format!("{greeting}! It's {time} — Systems are running at {cpu}% CPU and {ram}% RAM{battery}. {todos} Let's get after it!"),
    };
    let logs = vec![format!(
        "JARVIS briefing | {tier} ({}%) | {} pending tasks | {} memories",
        crate::memory::affection(),
        pending.len(),
        mem.facts.len()
    )];
    (msg, logs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_mode_round_trips() {
        stop_focus();
        assert!(!is_focus_active());
        assert_eq!(start_focus(25, true), 25);
        assert!(is_focus_active());
        stop_focus();
        assert!(!is_focus_active());
    }

    #[test]
    fn briefing_mentions_time_and_system() {
        let (msg, logs) = build_daily_briefing();
        assert!(msg.contains("CPU"));
        assert!(!logs.is_empty());
    }

    #[test]
    fn cooldowns_gate_events() {
        let mut st = MonitorState::default();
        assert!(st.can_fire("x", 1000.0, 60.0));
        st.fired("x", 1000.0);
        assert!(!st.can_fire("x", 1030.0, 60.0), "inside its own cooldown");
        assert!(!st.can_fire("y", 1030.0, 10.0), "inside the global cooldown");
        assert!(st.can_fire("y", 1100.0, 10.0));
    }

    #[test]
    fn alt_tab_keeps_session_and_only_closing_game_ends_it() {
        let game = presence::Game { name: "Apex Legends".into(), signal: "executable" };
        let mut st = MonitorState { active_game: Some(game.name.clone()), game_start: 1000.0,
            last_game_praise: 1390.0, ..Default::default() };
        assert!(game_event(&mut st, 1400.0, None, std::slice::from_ref(&game)).is_none());
        assert_eq!(st.active_game.as_deref(), Some("Apex Legends"));
        assert!(game_event(&mut st, 1410.0, Some(&game), std::slice::from_ref(&game)).is_none());
        assert_eq!(st.game_start, 1000.0);
        assert_eq!(game_event(&mut st, 1500.0, None, &[]).unwrap().category, "gaming_session_ended");
        assert!(st.active_game.is_none());
        assert!(game_event(&mut st, 1510.0, None, &[]).is_none());
    }

    #[test]
    fn workflow_changes_wait_for_stable_focus_and_comment_once() {
        let fg = presence::Foreground { pid: 42, title: "Project - Visual Studio Code".into(), exe: "code.exe".into(), ..Default::default() };
        let mut st = MonitorState::default();
        assert!(desktop_event(&mut st, 1000.0, &fg).is_none());
        assert!(desktop_event(&mut st, 1006.0, &fg).is_none());
        assert_eq!(desktop_event(&mut st, 1012.0, &fg).unwrap().category, "desktop_activity");
        assert!(desktop_event(&mut st, 1500.0, &fg).is_none());
    }

    #[test]
    fn queued_reactions_are_bounded_and_revalidated_before_speech() {
        let game = presence::Game { name: "Valorant".into(), signal: "executable" };
        let mut event = ProactiveEvent::new("gaming_start_Valorant", "happy", "A game started", "Ready?".into()).game(&game.name);
        event.detected_at = 1000.0;
        let settings = crate::config::Settings::default();
        let mut activity = presence::Activity { sampled: true, foreground_game: Some(game.clone()), running_games: vec![game], ..Default::default() };
        assert!(event.is_current_for(&activity, &settings, 1010.0));
        activity.foreground_game = None;
        assert!(!event.is_current_for(&activity, &settings, 1010.0));
        let mut st = MonitorState::default();
        enqueue(&mut st, event.clone());
        enqueue(&mut st, event);
        assert_eq!(st.pending.len(), 1);
        for index in 0..12 {
            enqueue(&mut st, ProactiveEvent::new(format!("event_{index}"), "idle", "", "".into()));
        }
        assert_eq!(st.pending.len(), 8);
        assert_eq!(st.pending.back().unwrap().category, "event_11");
    }
}
