//! Proactive desktop guardian: focus guard, AFK/posture care, hardware alerts, gaming copilot,
//! music vibe-sync and distraction commentary.

use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone)]
pub struct ProactiveEvent {
    pub category: String,
    pub message: String,
    pub emotion: String,
    pub badge: String,
    pub walk_location: Option<String>,
}

macro_rules! persona_line {
    ($($key:literal => $text:expr),+ $(,)?) => {{
        let options: &[(&str, String)] = &[$(($key, $text.to_string())),+];
        let current = crate::config::current_persona();
        let base = options
            .iter()
            .find(|(k, _)| *k == current.as_str())
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| options[0].1.clone());
        crate::config::flavor(&base, Some(&current))
    }};
}

const DISTRACTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "youtube",
        &[
            ("tsundere", "Hey! Are you watching YouTube instead of being productive? Don't blame me if you fall behind!"),
            ("yandere", "Watching YouTube instead of looking at me? You know I'm the only entertainment you ever need, right?"),
            ("tomboy", "Caught you slacking on YouTube! Let's knock out your tasks first and then watch clips together!"),
        ],
    ),
    (
        "netflix",
        &[
            ("tsundere", "Netflix right now? Are you actually going to get any work done today?"),
            ("yandere", "Watching a show without me? That's okay, I'll sit right here and watch every second with you."),
            ("tomboy", "Netflix time already? Save me some popcorn or let's finish up your work sprint first!"),
        ],
    ),
    (
        "twitch",
        &[
            ("tsundere", "Watching Twitch streams while I'm standing right here on your desktop? Pay attention to your own work!"),
            ("yandere", "Why are you watching someone else's stream when I'm right here on your desktop just for you?"),
            ("tomboy", "Checking out Twitch? Nice, just don't forget your own grind today!"),
        ],
    ),
    (
        "reddit",
        &[
            ("tsundere", "Scrolling Reddit again? Close that tab before you lose track of time!"),
            ("yandere", "Scrolling through strangers' posts again? You should be talking to me instead."),
            ("tomboy", "Doomscrolling Reddit again? Snap out of it, bro, let's get moving!"),
        ],
    ),
    (
        "discord",
        &[
            ("tsundere", "Chatting on Discord again? It's not like I care who you're talking to or anything!"),
            ("yandere", "Who are you messaging on Discord? You aren't replacing me with anyone else, right?"),
            ("tomboy", "Hopping on Discord with the squad? Tell them I said what's up!"),
        ],
    ),
    (
        "steam",
        &[
            ("tsundere", "Launching Steam? You better finish your tasks before you start gaming!"),
            ("yandere", "Firing up Steam? As long as you don't ignore me while you play, I'll allow it."),
            ("tomboy", "Oh heck yeah, launching Steam! What are we queuing up for today?"),
        ],
    ),
    (
        "tiktok",
        &[
            ("tsundere", "TikTok? Seriously? Your attention span is going to drop to zero if I don't keep an eye on you!"),
            ("yandere", "Close TikTok and pay attention to me instead, okay?"),
            ("tomboy", "TikTok brainrot alert! Put it away and let's crush today's goals!"),
        ],
    ),
];

pub const GAME_KEYWORDS: &[&str] = &[
    "valorant", "genshin", "elden ring", "cyberpunk", "minecraft", "counter-strike", "cs2",
    "overwatch", "apex legends", "league of legends", "dota", "honkai", "wuthering", "fortnite",
    "call of duty", "warzone", "rainbow six", "rocket league", "roblox", "helldivers",
    "monster hunter", "destiny", "dark souls", "palworld", "grand theft auto", "gta", "osu!",
    "team fortress", "terraria", "warframe", "pubg", "rust", "baldur's gate", "lethal company",
    "street fighter", "tekken", "marvel rivals", "tarkov", "black myth", "wukong", "hades",
    "sekiro", "god of war", "witcher", "forza", "halo", "dead by daylight", "fifa", "fc 24",
    "fc 25", "nba 2k", "stellaris", "civilization", "diablo", "path of exile",
    "world of warcraft", "final fantasy", "persona", "starfield", "satisfactory", "factorio",
    "subnautica", "sea of thieves", "borderlands", "steam", "epic games",
];

struct MonitorState {
    last_trigger: f64,
    category_cooldowns: std::collections::HashMap<String, f64>,
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
}

static MONITOR: OnceLock<Mutex<MonitorState>> = OnceLock::new();

pub fn monitor() -> &'static Mutex<MonitorState> {
    MONITOR.get_or_init(|| {
        Mutex::new(MonitorState {
            last_trigger: 0.0,
            category_cooldowns: std::collections::HashMap::new(),
            last_distraction_window: String::new(),
            focus_until: 0.0,
            focus_auto_minimize: true,
            focus_notified_done: true,
            last_music_track: String::new(),
            user_was_afk: false,
            continuous_work_start: crate::util::unix_now(),
            active_game: None,
            game_start: 0.0,
            last_game_praise: 0.0,
            game_praise_count: 0,
        })
    })
}

const GLOBAL_COOLDOWN_SEC: f64 = 75.0;

pub fn is_focus_active() -> bool {
    let st = monitor().lock().unwrap_or_else(|e| e.into_inner());
    crate::util::unix_now() < st.focus_until
}

// TODO(wire-up): focus mode has no UI entry point yet.
#[allow(dead_code)]
pub fn focus_remaining_minutes() -> i64 {
    let st = monitor().lock().unwrap_or_else(|e| e.into_inner());
    let remaining = st.focus_until - crate::util::unix_now();
    if remaining <= 0.0 {
        0
    } else {
        ((remaining / 60.0).round() as i64).max(1)
    }
}

#[allow(dead_code)] // TODO(wire-up): focus mode has no UI entry point yet.
pub fn start_focus(minutes: i64, auto_minimize: bool) -> i64 {
    let mins = minutes.clamp(1, 240);
    let mut st = monitor().lock().unwrap_or_else(|e| e.into_inner());
    st.focus_until = crate::util::unix_now() + mins as f64 * 60.0;
    st.focus_auto_minimize = auto_minimize;
    st.focus_notified_done = false;
    st.last_distraction_window.clear();
    mins
}

#[allow(dead_code)] // TODO(wire-up): focus mode has no UI entry point yet.
pub fn stop_focus() {
    let mut st = monitor().lock().unwrap_or_else(|e| e.into_inner());
    st.focus_until = 0.0;
    st.focus_notified_done = true;
}

fn can_fire(st: &MonitorState, category: &str, now: f64, cooldown: f64) -> bool {
    if now - st.last_trigger < GLOBAL_COOLDOWN_SEC {
        return false;
    }
    let last = st.category_cooldowns.get(category).copied().unwrap_or(0.0);
    now - last >= cooldown
}

fn mark_fired(st: &mut MonitorState, category: &str, now: f64) {
    st.last_trigger = now;
    st.category_cooldowns.insert(category.to_string(), now);
}

/// Detect whether a game or fullscreen app is in the foreground. Returns (is_game, name).
pub fn detect_active_game_or_fullscreen() -> (bool, String) {
    let Some(win) = crate::win32::active_window() else {
        return (false, String::new());
    };
    let lower = win.title.to_lowercase();
    if lower.is_empty() || lower == "aria" || lower.contains("program manager") || lower.contains("windows input experience") {
        return (false, String::new());
    }
    for game in GAME_KEYWORDS {
        if lower.contains(game) {
            return (true, game.to_string());
        }
    }
    if let Some((_, _, w, h)) = crate::win32::window_rect(win.hwnd) {
        let (_, _, sw, sh) = crate::win32::screen_size();
        if w >= sw - 8 && h >= sh - 8 {
            return (true, win.title.chars().take(24).collect());
        }
    }
    (false, String::new())
}

/// Feeds current activity into the mood engine.
pub fn observe_mood() {
    let stats = crate::sysinfo::system_stats();
    crate::mood::observe(
        &stats.active_window,
        crate::win32::idle_seconds(),
        is_focus_active(),
    );
}

pub fn check_for_event() -> Option<ProactiveEvent> {
    let now = crate::util::unix_now();
    let mut st = monitor().lock().unwrap_or_else(|e| e.into_inner());

    // A focus session just completed.
    if !st.focus_notified_done && st.focus_until > 0.0 && now >= st.focus_until {
        st.focus_notified_done = true;
        st.focus_until = 0.0;
        let new_aff = crate::memory::adjust_affection(5);
        return Some(ProactiveEvent {
            category: "focus_completed".into(),
            message: persona_line! {
                "tsundere" => "Your Focus Session is complete! You actually stayed focused the whole time... Good job today!",
                "yandere" => "Your Focus Session is finished! Now you can give me 100% of your attention again!",
                "tomboy" => "Boom! Focus Session complete! You crushed that work sprint, high five!",
            },
            emotion: "happy".into(),
            badge: format!("Focus session complete! (Affection +5% -> {new_aff}%)"),
            walk_location: Some("center".into()),
        });
    }

    let proactive = crate::config::get().proactive_enabled;
    if !proactive && now >= st.focus_until {
        return None;
    }

    let idle_sec = crate::win32::idle_seconds();
    if crate::config::get().presence_guardian_enabled {
        if idle_sec >= 240.0 && !st.user_was_afk {
            st.user_was_afk = true;
            mark_fired(&mut st, "afk_sleepy", now);
            return Some(ProactiveEvent {
                category: "afk_sleepy".into(),
                message: persona_line! {
                    "tsundere" => "*Yaaawn*... You haven't moved your mouse in four minutes... Did you walk away and leave me here?",
                    "yandere" => "You haven't moved your mouse in four minutes... Where did you go without telling me?",
                    "tomboy" => "*Yaaawn*... Taking a break from the desk? I'll chill right here until you get back!",
                },
                emotion: "sleepy".into(),
                badge: "Desk presence: user AFK (sleepy mode)".into(),
                walk_location: None,
            });
        } else if st.user_was_afk && idle_sec < 4.0 {
            st.user_was_afk = false;
            st.continuous_work_start = now;
            let new_aff = crate::memory::adjust_affection(2);
            mark_fired(&mut st, "welcome_back", now);
            return Some(ProactiveEvent {
                category: "welcome_back".into(),
                message: persona_line! {
                    "tsundere" => "Oh, you're back at your desk! It's not like I was waiting for you or anything!",
                    "yandere" => "You're finally back! I missed you so much... don't leave me alone like that again!",
                    "tomboy" => "Welcome back to the desk! Ready to jump back into the action?",
                },
                emotion: "happy".into(),
                badge: format!("Welcome back! (Affection +2% -> {new_aff}%)"),
                walk_location: None,
            });
        } else if !st.user_was_afk
            && (now - st.continuous_work_start) >= 2700.0
            && can_fire(&st, "posture_check", now, 1800.0)
        {
            st.continuous_work_start = now;
            mark_fired(&mut st, "posture_check", now);
            return Some(ProactiveEvent {
                category: "posture_check".into(),
                message: persona_line! {
                    "tsundere" => "You've been staring at your monitor for 45 minutes straight! Sit up straight and rest your eyes for 20 seconds!",
                    "yandere" => "You've been staring at the screen for 45 minutes... Sit up straight and take care of yourself for me, okay?",
                    "tomboy" => "Time for a quick posture check! Roll your shoulders back, sit up tall, and rest your eyes for 20 seconds!",
                },
                emotion: "talking".into(),
                badge: "Posture & 20-20-20 eye guardian".into(),
                walk_location: None,
            });
        }
    }

    // Music vibe sync.
    if crate::config::get().music_vibe_enabled && now >= st.focus_until {
        let media = crate::sysinfo::now_playing();
        if media.is_playing && !media.track.is_empty() {
            let track = media.track.clone();
            let src = media.source.clone();
            let cooldown = st.category_cooldowns.get("music_vibe").copied().unwrap_or(0.0);
            if track != st.last_music_track && now - cooldown >= 45.0 {
                st.last_music_track = track.clone();
                mark_fired(&mut st, "music_vibe", now);
                crate::memory::adjust_affection(1);
                return Some(ProactiveEvent {
                    category: "music_vibe".into(),
                    message: persona_line! {
                        "tsundere" => format!("Listening to `{track}` on {src}? Your music taste isn't bad at all... I'll vibe along with you!"),
                        "yandere" => format!("Listening to `{track}` on {src}? I love sharing the exact same song with you!"),
                        "tomboy" => format!("Oh nice, `{track}` on {src}! That's a banger, let's vibe!"),
                    },
                    emotion: "groove".into(),
                    badge: format!("Now playing ({src}): {}", crate::util::truncate(&track, 38)),
                    walk_location: None,
                });
            }
        }
    }

    let stats = crate::sysinfo::system_stats();
    let cpu = stats.cpu_percent;
    let ram = stats.ram_percent;
    let active_win = stats.active_window.trim().to_string();
    let active_lower = active_win.to_lowercase();

    // Focus guard: block distractions immediately.
    if now < st.focus_until && !active_lower.is_empty() {
        for (key, _) in DISTRACTIONS {
            let cooldown = st.category_cooldowns.get("focus_guard").copied().unwrap_or(0.0);
            if active_lower.contains(key) && now - cooldown >= 12.0 {
                mark_fired(&mut st, "focus_guard", now);
                let rem = (st.focus_until - now) / 60.0;
                let rem = (rem.round() as i64).max(1);
                let mut note = String::new();
                if st.focus_auto_minimize {
                    let res = crate::tools::focus_or_manage_window(&active_win, "minimize");
                    if res.get("status").and_then(|s| s.as_str()) == Some("success") {
                        note = " I just minimized that window for you!".into();
                    }
                }
                crate::memory::adjust_affection(-1);
                let title = capitalize(key);
                return Some(ProactiveEvent {
                    category: "focus_guard".into(),
                    message: persona_line! {
                        "tsundere" => format!("You still have {rem} min left in Focus Mode and you opened {title}?!{note} Get back to work right now!"),
                        "yandere" => format!("You have {rem} min left in Focus Mode and opened {title}?{note} Stay focused on your work with me!"),
                        "tomboy" => format!("Hold up! You still have {rem} min left in Focus Mode and opened {title}!{note} Lock back in!"),
                    },
                    emotion: "annoyed".into(),
                    badge: format!("Focus guard blocked: {} ({rem}m left)", crate::util::truncate(&active_win, 36)),
                    walk_location: Some("center".into()),
                });
            }
        }
    }

    // Hardware spike.
    if (ram >= 86.0 || cpu >= 88.0) && can_fire(&st, "hardware_spike", now, 300.0) {
        mark_fired(&mut st, "hardware_spike", now);
        return Some(ProactiveEvent {
            category: "hardware_spike".into(),
            message: persona_line! {
                "tsundere" => format!("Look at your system resources — CPU is at {cpu}% and RAM is at {ram}%! Close some heavy apps before your PC overheats!"),
                "yandere" => format!("Your CPU is at {cpu}% and RAM is at {ram}%! Close those heavy apps so nothing crashes our time together!"),
                "tomboy" => format!("Heads up! Your CPU is hitting {cpu}% and RAM is at {ram}%! Let's close some heavy background tabs!"),
            },
            emotion: "annoyed".into(),
            badge: format!("Proactive alert: CPU {cpu}% | RAM {ram}%"),
            walk_location: Some("center".into()),
        });
    }

    // Low battery.
    if let Some(battery) = stats.battery_percent {
        if !stats.power_plugged && battery <= 25 && can_fire(&st, "low_battery", now, 360.0) {
            mark_fired(&mut st, "low_battery", now);
            return Some(ProactiveEvent {
                category: "low_battery".into(),
                message: persona_line! {
                    "tsundere" => format!("Your battery is down to {battery}% and unplugged! Plug in your charger before your PC shuts down!"),
                    "yandere" => format!("Your battery is at {battery}%! Plug in your charger right now so we don't get disconnected!"),
                    "tomboy" => format!("Battery check! You're at {battery}% and unplugged — grab your charger before we lose power!"),
                },
                emotion: "annoyed".into(),
                badge: format!("Low battery warning ({battery}%)"),
                walk_location: Some("center".into()),
            });
        }
    }

    // Gaming copilot.
    if crate::config::get().automatic_game_copilot_enabled && !active_lower.is_empty() {
        let matched = GAME_KEYWORDS
            .iter()
            .find(|g| active_lower.contains(*g))
            .map(|g| g.to_string());

        match matched {
            Some(game) => {
                let mut clean = title_case(&game);
                if clean.to_lowercase() == "cs2" {
                    clean = "Counter-Strike 2".into();
                }
                if st.active_game.as_deref() != Some(game.as_str()) {
                    st.active_game = Some(game.clone());
                    st.game_start = now;
                    st.last_game_praise = now;
                    st.game_praise_count = 0;
                    let line = persona_line! {
                        "tsundere" => format!("Firing up {clean}? Fine! Don't blame me if you get distracted by how cute I am! Win this match, baka!"),
                        "yandere" => format!("Playing {clean}... I'll be watching every move you make. Destroy everyone who stands in your way, darling~"),
                        "tomboy" => format!("Yooo, {clean}! Lock in, partner! Let's hit some nasty plays and take the dub!"),
                    };
                    let mood = crate::config::current_persona();
                    return Some(ProactiveEvent {
                        category: format!("gaming_{game}_start"),
                        message: line,
                        emotion: if mood == "tsundere" { "annoyed" } else { "happy" }.into(),
                        badge: format!("Gaming started: {clean}"),
                        walk_location: Some("perch".into()),
                    });
                }

                if now - st.last_game_praise >= 150.0 && can_fire(&st, "gaming_praise", now, 150.0) {
                    st.last_game_praise = now;
                    mark_fired(&mut st, "gaming_praise", now);
                    st.game_praise_count += 1;
                    let session_mins = (((now - st.game_start) / 60.0) as i64).max(1);
                    let mode = st.game_praise_count % 3;
                    let (line, emotion) = match mode {
                        1 => (
                            persona_line! {
                                "tsundere" => format!("That play in {clean} was... actually pretty decent! D-Don't get cocky though! Keep your crosshair up!"),
                                "yandere" => format!("Your aim is so deadly in {clean}... I get shivers down my spine watching you dominate the lobby!"),
                                "tomboy" => format!("Filthy crosshair placement! Keep clicking heads like that in {clean} and it's an easy win!"),
                            },
                            "happy",
                        ),
                        2 => (
                            persona_line! {
                                "tsundere" => "Deep breath! Reset your mental, you can easily clutch this round! I believe in you... a little bit!",
                                "yandere" => "Stay focused, my love! Nobody in that lobby is worthy of defeating you. Crush them all!",
                                "tomboy" => "Shake off any bad rounds! You're in the zone, lock in and clutch it up, partner!",
                            },
                            "talking",
                        ),
                        _ => (
                            persona_line! {
                                "tsundere" => format!("You've been grinding {clean} for {session_mins} minutes! Drink some water, baka, you need sharp reflexes!"),
                                "yandere" => format!("{session_mins} minutes of watching you play... make sure you stretch your hands for me, okay?"),
                                "tomboy" => format!("{session_mins} mins deep in {clean}! Take a quick sip of water, shake out your wrists, and keep slaying!"),
                            },
                            "talking",
                        ),
                    };
                    let walk = if crate::util::rand_f32() < 0.35 {
                        Some("center".to_string())
                    } else {
                        None
                    };
                    return Some(ProactiveEvent {
                        category: format!("gaming_praise_{}", st.game_praise_count),
                        message: line,
                        emotion: emotion.into(),
                        badge: format!("Gaming motivation ({clean} • {session_mins}m)"),
                        walk_location: walk,
                    });
                }
            }
            None => {
                if let Some(prev) = st.active_game.take() {
                    let duration = now - st.game_start;
                    if duration > 180.0 {
                        let prev_title = title_case(&prev);
                        return Some(ProactiveEvent {
                            category: "gaming_session_ended".into(),
                            message: persona_line! {
                                "tsundere" => format!("Done with {prev_title}? Good job out there! ...Not that I was worried about your rank or anything!"),
                                "yandere" => format!("You finished {prev_title}! Now your attention is all mine again, darling~"),
                                "tomboy" => format!("GGs on that {prev_title} session! High five, partner, you played great!"),
                            },
                            emotion: "happy".into(),
                            badge: format!("Match ended ({prev_title})"),
                            walk_location: Some("bottom_right".into()),
                        });
                    }
                }
            }
        }
    }

    // Distraction detection.
    if !active_lower.is_empty() && active_lower != st.last_distraction_window {
        for (key, lines) in DISTRACTIONS {
            if active_lower.contains(key) && can_fire(&st, &format!("app_{key}"), now, 180.0) {
                st.last_distraction_window = active_lower.clone();
                mark_fired(&mut st, &format!("app_{key}"), now);
                let mood = crate::config::current_persona();
                let base = lines
                    .iter()
                    .find(|(k, _)| *k == mood)
                    .map(|(_, v)| *v)
                    .unwrap_or(lines[0].1);
                return Some(ProactiveEvent {
                    category: format!("app_{key}"),
                    message: crate::config::flavor(base, Some(&mood)),
                    emotion: "annoyed".into(),
                    badge: format!("Detected window: {}", crate::util::truncate(&active_win, 42)),
                    walk_location: Some("roam".into()),
                });
            }
        }
    }

    // Late-night care.
    let hour = crate::util::now_hour();
    if (1..=5).contains(&hour) && can_fire(&st, "late_night", now, 600.0) {
        mark_fired(&mut st, "late_night", now);
        let now_str = crate::util::now_stamp("%I:%M %p");
        return Some(ProactiveEvent {
            category: "late_night".into(),
            message: persona_line! {
                "tsundere" => format!("Do you realize it's {now_str}? Staying up this late is bad for you — wrap up and get some sleep soon!"),
                "yandere" => format!("It's already {now_str}... I love staying up late with you, but you need your rest too."),
                "tomboy" => format!("Whoa, it's {now_str} already! Let's wrap this up and get some rest so you're fresh tomorrow!"),
            },
            emotion: "talking".into(),
            badge: format!("Late-night care check ({now_str})"),
            walk_location: None,
        });
    }

    None
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// JARVIS daily briefing: time, hardware, affection, to-dos.
pub fn build_daily_briefing() -> (String, Vec<String>) {
    let hour = crate::util::now_hour();
    let tod = if hour < 12 {
        "Good morning"
    } else if hour < 18 {
        "Good afternoon"
    } else {
        "Good evening"
    };
    let time_str = crate::util::now_stamp("%I:%M %p");
    let stats = crate::sysinfo::system_stats();
    let cpu = stats.cpu_percent;
    let ram = stats.ram_percent;

    let mem = crate::memory::load();
    let aff = crate::memory::affection();
    let (tier_name, _) = crate::memory::affection_tier_label();
    let pending: Vec<&crate::memory::Todo> = mem.todos.iter().filter(|t| !t.done).collect();
    let facts = mem.facts.len();

    let bat_phrase = match stats.battery_percent {
        Some(b) => format!(", battery is at {b}%"),
        None => String::new(),
    };
    let todo_phrase = if pending.is_empty() {
        "Your To-Do list is currently clear.".to_string()
    } else {
        let top: Vec<&str> = pending.iter().take(3).map(|t| t.task.as_str()).collect();
        format!(
            "You have {} pending task{} ({}).",
            pending.len(),
            if pending.len() != 1 { "s" } else { "" },
            top.join(", ")
        )
    };

    let msg = persona_line! {
        "tsundere" => format!("{tod}! It's {time_str} — CPU is at {cpu}%, RAM is at {ram}%{bat_phrase}. {todo_phrase}"),
        "yandere" => format!("{tod}! It's {time_str} — I checked everything for you: CPU is {cpu}%, RAM is {ram}%{bat_phrase}. {todo_phrase}"),
        "tomboy" => format!("{tod}! It's {time_str} — Systems are running at {cpu}% CPU and {ram}% RAM{bat_phrase}. {todo_phrase} Let's get after it!"),
    };

    let logs = vec![format!(
        "JARVIS briefing | {tier_name} ({aff}%) | {} pending tasks | {facts} memories",
        pending.len()
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
        assert!(focus_remaining_minutes() >= 24);
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
    fn title_case_handles_phrases() {
        assert_eq!(title_case("counter strike"), "Counter Strike");
    }
}
