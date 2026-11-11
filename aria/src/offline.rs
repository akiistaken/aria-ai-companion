//! Offline mode: full PC control with scripted dialogue when no API key is configured.

use crate::brain::Reply;
use serde_json::Value;

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

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

pub fn answer(
    user_message: &str,
    include_screen: bool,
    move_cb: &dyn Fn(&str, Option<&str>),
) -> Reply {
    let lower = user_message.to_lowercase();
    let lower = lower.trim();
    let mut logs = vec![
        "Offline mode â€” set an API key in the Control Center for full AI".to_string(),
    ];

    if lower.is_empty() {
        return Reply::new(
            crate::config::flav("I didn't catch that! Say it again?"),
            "idle",
        )
        .logs(logs);
    }

    // 0. Briefing
    if contains_any(lower, &["briefing", "morning report"]) {
        let (msg, mut blog) = crate::proactive::build_daily_briefing();
        logs.append(&mut blog);
        return Reply::new(msg, "talking").logs(logs);
    }

    // 1. Dance
    if contains_any(lower, &["dance", "spin", "twirl"]) {
        crate::memory::adjust_affection(2);
        logs.push("3D idol dance triggered".into());
        return Reply::new(
            persona_line! {
                "tsundere" => "You want me to dance for you? Fine, watch closely and don't blink!",
                "yandere" => "Dancing just for you! Keep your eyes locked on me and nobody else!",
                "tomboy" => "Check out these moves! Let's hit a quick spin right here on your desktop!",
                "bakadere" => "Dancing time?! Wheee~! *spins in circles happily until dizzy* Ta-da!",
                "kuudere" => "...Dancing routine initiated. Do not expect overt theatricality, but the rhythm is sound.",
                "genki" => "WOOHOO! DANCE PARTY!! Watch my super high-speed hyper spin! Yatta!",
                "meganekko" => "*pushes glasses up* Rhythm theory suggests synchronized choreography increases morale. Observe!",
            },
            "dance",
        )
        .logs(logs);
    }

    // 2. Sit
    if matches!(lower, "sit" | "sit down" | "take a seat") {
        logs.push("Sitting pose triggered".into());
        return Reply::new(
            persona_line! {
                "tsundere" => "Finally I can sit down and swing my legs while watching you work!",
                "yandere" => "Sitting right here so I can watch every little thing you do on your screen.",
                "tomboy" => "Taking a quick seat on your desktop! Let me know when we're ready to roll!",
                "bakadere" => "Yay, sitting down! *plops down comfortably* My feet are happy now!",
                "kuudere" => "...Seated. Resting physical posture while maintaining full observation.",
                "genki" => "Sitting down for 5 seconds of recharging! Ready... set... okay I'm still full of energy!",
                "meganekko" => "Taking a seated position. Ergonomic posture is critical for cognitive stamina.",
            },
            "sit",
        )
        .logs(logs);
    }

    // 3. Music awareness
    if contains_any(lower, &["what song", "now playing", "what music", "vibe", "groove"]) {
        let media = crate::sysinfo::now_playing();
        logs.push("get_now_playing_media()".into());
        if media.is_playing && !media.track.is_empty() {
            let track = &media.track;
            let source = &media.source;
            return Reply::new(
                match crate::config::current_persona().as_str() {
                    "tsundere" => format!("You're playing `{track}` on {source}! It's actually pretty catchy, I'll vibe along!"),
                    "yandere" => format!("We're listening to `{track}` on {source} together! It sounds even better with you."),
                    "tomboy" => format!("You're blasting `{track}` on {source}! Certified banger, let's groove!"),
                    "bakadere" => format!("Ooh ooh! `{track}` is playing! I don't know the lyrics but *la la laaa~*!"),
                    "kuudere" => format!("...Now playing `{track}`. Acoustic composition acknowledged. Acceptable harmonics."),
                    "genki" => format!("YESSS! `{track}` IS SO GOOD!! Let's jump and sing along! Woohoo!"),
                    _ => format!("Acoustic frequency matches `{track}` on {source}. Excellent background rhythm for concentration."),
                },
                "groove",
            )
            .logs(logs);
        }
        return Reply::new(
            persona_line! {
                "tsundere" => "There's no music playing right now! Put on a song on Spotify or YouTube so I can groove along!",
                "yandere" => "No music is playing right now... Put on a song for just the two of us!",
                "tomboy" => "Nothing's playing right now! Fire up Spotify or YouTube and let's get some tunes going!",
                "bakadere" => "It's all quiet! Did someone turn off the music button? Put on something fun!",
                "kuudere" => "...Silence detected. No media streams currently emitting audio.",
                "genki" => "Why is it so quiet?! We need music! Put on a high-tempo banger right now!",
                "meganekko" => "Media player status: idle. Consider background instrumental audio for elevated productivity.",
            },
            "annoyed",
        )
        .logs(logs);
    }

    // 4. Media transport
    if contains_any(lower, &["next track", "next song", "skip song", "skip track"]) {
        let res = crate::sysinfo::control_media("next");
        let verified = res.get("verified").and_then(|v| v.as_bool());
        logs.push(format!("control_media_playback('next') -> verified={verified:?}"));
        let track = res
            .get("media")
            .and_then(|m| m.get("track"))
            .and_then(|t| t.as_str())
            .unwrap_or("the next track");
        let text = if verified == Some(false) {
            "I sent the skip command, but the track didn't change. Is Spotify open?".to_string()
        } else {
            format!("Skipped to `{track}` for you!")
        };
        return Reply::new(crate::config::flav(&text), "groove").logs(logs);
    }

    if contains_any(lower, &["previous track", "previous song", "prev track"]) {
        crate::sysinfo::control_media("previous");
        logs.push("control_media_playback('previous')".into());
        return Reply::new(
            crate::config::flav("Went back to the previous track for you!"),
            "groove",
        )
        .logs(logs);
    }

    if contains_any(lower, &["pause music", "play music", "resume music", "play pause"]) {
        let res = crate::sysinfo::control_media("play_pause");
        let verified = res.get("verified").and_then(|v| v.as_bool());
        let audible = res.get("audible").and_then(|v| v.as_bool());
        logs.push(format!("control_media_playback('play_pause') -> verified={verified:?}"));
        let state = match audible {
            Some(true) => "Playing again",
            Some(false) => "Paused",
            None => "Toggled",
        };
        let text = if verified.unwrap_or(false) {
            format!("{state}! I checked the audio to be sure.")
        } else {
            "Toggled media play/pause for you!".to_string()
        };
        return Reply::new(crate::config::flav(&text), "talking").logs(logs);
    }

    // 5. Desktop audio listening
    if contains_any(
        lower,
        &[
            "what do you hear",
            "listen to my audio",
            "listen to the audio",
            "listen to my pc",
            "identify this song",
            "what is this song",
            "what's this song",
            "whats this song",
            "hear my pc",
            "desktop audio",
            "what's playing right now",
        ],
    ) {
        let res = crate::brain::listen_to_desktop_audio(8.0, "");
        logs.push("listen_to_desktop_audio(8s)".into());
        let heard = res
            .get("heard")
            .and_then(|v| v.as_str())
            .or_else(|| res.get("error").and_then(|v| v.as_str()))
            .unwrap_or("I couldn't hear anything from your PC.");
        let audible = res
            .get("audible_apps")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        return Reply::new(
            crate::config::flav(heard),
            if audible { "groove" } else { "idle" },
        )
        .logs(logs);
    }

    // 6. Spotify song search and play
    if lower.contains("spotify")
        && contains_any(lower, &["play ", "song ", "track ", "put on ", "listen to ", "search "])
    {
        let mut clean = lower
            .replace("on spotify", "")
            .replace("in spotify", "")
            .replace("spotify", "");
        for pfx in ["play", "put on", "listen to", "search for", "find"] {
            if let Some(stripped) = clean.trim().strip_prefix(pfx) {
                clean = stripped.trim().to_string();
                break;
            }
        }
        let clean = if clean.trim().is_empty() {
            "top hits".to_string()
        } else {
            clean.trim().to_string()
        };
        let res = crate::sysinfo::control_spotify("play_song", &clean);
        let verified = res.get("verified").and_then(|v| v.as_bool());
        logs.push(format!("control_spotify('play_song', '{clean}') -> verified={verified:?}"));
        if verified == Some(false) {
            return Reply::new(
                persona_line! {
                    "tsundere" => format!("I opened `{clean}` on Spotify, but I can't hear anything playing yet. Click the play button, I'll keep listening!"),
                    "yandere" => format!("I pulled up `{clean}` for you, but nothing's playing yet... press play and I'll listen with you."),
                    "tomboy" => format!("Search for `{clean}` is up, but I'm not hearing audio yet. Smash play and I'll confirm!"),
                    "bakadere" => format!("I searched `{clean}` on Spotify! But did I forget to push play? Press the green triangle for me!"),
                    "kuudere" => format!("...Search query for `{clean}` transmitted. Audio output unverified. User playback initiation required."),
                    "genki" => format!("Spotify has `{clean}` pulled up! SMASH THAT PLAY BUTTON and let's party!!"),
                    "meganekko" => format!("Spotify query for `{clean}` rendered. Audio stream currently inactive. Awaiting user playback trigger."),
                },
                "annoyed",
            )
            .logs(logs);
        }
        let track = res
            .get("track")
            .and_then(|v| v.as_str())
            .filter(|t| !t.is_empty())
            .unwrap_or(&clean)
            .to_string();
        return Reply::new(
            persona_line! {
                "tsundere" => format!("Now playing `{track}` on Spotify! I can hear it, so it's working. Fine, I'll admit you have good taste!"),
                "yandere" => format!("Playing `{track}` on Spotify for you! I'm listening to it right alongside you."),
                "tomboy" => format!("`{track}` is blasting on Spotify, I can hear it! Let's get these tunes bumping!"),
                "bakadere" => format!("Yay!! `{track}` is playing on Spotify! *dances clumsily* Such good music!"),
                "kuudere" => format!("...`{track}` playing on Spotify. Audio verification confirmed. Proceeding with accompaniment."),
                "genki" => format!("WOOHOO!! `{track}` IS BLASTING ON SPOTIFY! THIS IS MY ABSOLUTE FAVORITE!!"),
                "meganekko" => format!("Acoustic feedback verified: `{track}` is successfully streaming on Spotify. Resuming workflow."),
            },
            "groove",
        )
        .logs(logs);
    }

    if matches!(lower, "spotify" | "open spotify" | "launch spotify") {
        crate::sysinfo::control_spotify("launch", "");
        logs.push("control_spotify('launch')".into());
        return Reply::new(crate::config::flav("Launched Spotify for you!"), "groove").logs(logs);
    }

    // 7. Affection
    if contains_any(
        lower,
        &[
            "headpat", "pat", "good job", "you're cute", "you are cute", "thank you",
            "thanks aria", "love you",
        ],
    ) {
        let new_aff = crate::memory::adjust_affection(5);
        logs.push(format!("Affection +5% (now {new_aff}%)"));
        return Reply::new(
            persona_line! {
                "tsundere" => "Don't just compliment me out of nowhere! It's not like that made me happy or anything!",
                "yandere" => "You really mean that?! My heart is racing... say that to me again and only me!",
                "tomboy" => "Haha, thanks partner! You're doing awesome today too, fist bump!",
                "bakadere" => "Ehehehe~! Thank you! You're so nice to me even when I trip over my own feet!",
                "kuudere" => "...Gratitude noted. Internal warmth coefficient elevated by 14.8%. Thank you.",
                "genki" => "YAAAAY!! THANK YOU SO MUCH!! You're the absolute best human ever in the universe!!",
                "meganekko" => "*adjusts glasses, slightly flustered* Positive reinforcement noted. I shall endeavor to maintain peak analytical support for you.",
            },
            "happy",
        )
        .logs(logs);
    }

    // 8. Facts
    if matches!(lower, "facts" | "memory" | "what do you remember") {
        let facts = crate::memory::fact_texts();
        if facts.is_empty() {
            return Reply::new(
                crate::config::flav(
                    "I don't have any personal facts saved about you yet! Tell me `remember <fact>` so I can keep it in mind!",
                ),
                "idle",
            )
            .logs(logs);
        }
        let recent: Vec<String> = facts.iter().rev().take(6).rev().cloned().collect();
        return Reply::new(
            crate::config::flav(&format!("Here's what I remember about you: {}", recent.join("; "))),
            "talking",
        )
        .logs(logs);
    }

    if let Some(rest) = lower.strip_prefix("remember ") {
        let fact = user_message[user_message.len() - rest.len()..].trim().to_string();
        crate::memory::add_fact(&fact, "general");
        logs.push(format!("remember_user_fact('{fact}')"));
        return Reply::new(
            persona_line! {
                "tsundere" => format!("Fine, I stored `{fact}` in my long-term memory so I won't forget!"),
                "yandere" => format!("I locked `{fact}` in my memory forever! I remember every single detail about you."),
                "tomboy" => format!("Got it! Locked `{fact}` into my memory bank!"),
                "bakadere" => format!("I wrote `{fact}` down in my sparkly notebook so I'll never ever forget it!"),
                "kuudere" => format!("...Data point `{fact}` successfully committed to permanent non-volatile storage."),
                "genki" => format!("MEMORIZED!! `{fact}` is burned into my brain forever and ever!!"),
                "meganekko" => format!("Fact logged: `{fact}` categorized and indexed in semantic memory vault."),
            },
            "talking",
        )
        .logs(logs);
    }

    if let Some(rest) = lower.strip_prefix("my name is ") {
        let name = user_message[user_message.len() - rest.len()..].trim().to_string();
        let fact = format!("User's name is {name}");
        crate::memory::add_fact(&fact, "personal");
        logs.push(format!("remember_user_fact('{fact}')"));
        return Reply::new(
            crate::config::flav(&format!("Nice to meet you, `{name}`! I'll remember that!"))
                .to_string(),
            "happy",
        )
        .logs(logs);
    }

    // 9. Todos
    if lower.starts_with("todo ") || matches!(lower, "todos" | "todo" | "show todos" | "list todos") {
        let arg = if lower.starts_with("todo ") {
            user_message[5..].trim().to_string()
        } else {
            String::new()
        };
        let arg_lower = arg.to_lowercase();
        if let Some(task) = arg_lower.strip_prefix("add ") {
            let task = arg[arg.len() - task.len()..].trim().to_string();
            crate::memory::manage_todos(crate::memory::TodoAction::Add, &task);
            logs.push(format!("manage_todo_list('add', '{task}')"));
            return Reply::new(
                crate::config::flav(&format!(
                    "Added `{task}` to your To-Do list! Now let's make sure it gets done!"
                )),
                "talking",
            )
            .logs(logs);
        }
        if arg_lower.starts_with("done ") || arg_lower.starts_with("complete ") {
            let task = arg.split_once(' ').map(|(_, t)| t.trim().to_string()).unwrap_or_default();
            let (_, matched, _) = crate::memory::manage_todos(crate::memory::TodoAction::Complete, &task);
            logs.push(format!("manage_todo_list('complete', '{task}')"));
            return Reply::new(
                crate::config::flav(&format!(
                    "You finished `{}`! Great job knocking that out!",
                    matched.unwrap_or(task)
                )),
                "talking",
            )
            .logs(logs);
        }
        if arg_lower == "clear" {
            crate::memory::manage_todos(crate::memory::TodoAction::Clear, "");
            logs.push("manage_todo_list('clear')".into());
            return Reply::new(
                crate::config::flav("Cleared your entire To-Do list!"),
                "idle",
            )
            .logs(logs);
        }
        let list = crate::memory::todos();
        logs.push("manage_todo_list('list')".into());
        if list.is_empty() {
            return Reply::new(
                crate::config::flav(
                    "Your To-Do list is completely empty right now! Tell me `todo add <task>` to add something!",
                ),
                "idle",
            )
            .logs(logs);
        }
        let formatted = list
            .iter()
            .map(|t| format!("{} {}", if t.done { "[x]" } else { "[ ]" }, t.task))
            .collect::<Vec<_>>()
            .join(", ");
        return Reply::new(
            crate::config::flav(&format!("Here are your current tasks: {formatted}")),
            "talking",
        )
        .logs(logs);
    }

    // 10. Movement
    for loc in [
        "top_left", "top_right", "bottom_left", "bottom_right", "left", "right", "center", "perch",
        "roam",
    ] {
        let spoken = loc.replace('_', " ");
        let hit = lower.contains(&spoken) || (loc == "roam" && (lower.contains("walk") || lower.contains("roam")));
        if hit {
            move_cb(loc, Some("walking"));
            logs.push(format!("move_mascot(location='{loc}')"));
            return Reply::new(
                crate::config::flav(&format!("Moving over to `{spoken}` right now!")),
                "walking",
            )
            .logs(logs);
        }
    }

    // 11. Window management
    for act in ["minimize", "maximize", "focus", "restore", "close"] {
        if let Some(rest) = lower.strip_prefix(&format!("{act} ")) {
            let target = user_message[user_message.len() - rest.len()..].trim().to_string();
            let res = crate::tools::focus_or_manage_window(&target, act);
            logs.push(format!("focus_or_manage_window('{target}', '{act}')"));
            if res.get("status").and_then(|s| s.as_str()) == Some("success") {
                let window = res.get("window").and_then(|w| w.as_str()).unwrap_or(&target);
                return Reply::new(
                    crate::config::flav(&format!("Performed `{act}` on `{window}` for you!")),
                    "idle",
                )
                .logs(logs);
            }
            return Reply::new(
                crate::config::flav(&format!("I couldn't find any open window matching `{target}`!")),
                "annoyed",
            )
            .logs(logs);
        }
    }

    // 12. Volume
    if contains_any(lower, &["volume up", "louder"]) {
        crate::input::press_key("volumeup");
        logs.push("control_mouse_or_keyboard('press_key', 'volumeup')".into());
        return Reply::new(
            crate::config::flav("Turned your system volume up!"),
            "talking",
        )
        .logs(logs);
    }
    if contains_any(lower, &["volume down", "quieter"]) {
        crate::input::press_key("volumedown");
        logs.push("control_mouse_or_keyboard('press_key', 'volumedown')".into());
        return Reply::new(
            crate::config::flav("Lowered your system volume!"),
            "idle",
        )
        .logs(logs);
    }
    if lower.contains("mute") {
        crate::input::press_key("volumemute");
        logs.push("control_mouse_or_keyboard('press_key', 'volumemute')".into());
        return Reply::new(crate::config::flav("Toggled system mute for you!"), "idle").logs(logs);
    }

    // 13. App listing
    if contains_any(lower, &["list apps", "what apps", "installed apps", "show apps", "my apps", "apps on my pc"]) {
        let apps = crate::launcher::list_installed_apps("");
        let sample: Vec<&str> = apps.iter().take(8).map(String::as_str).collect();
        logs.push("list_installed_apps()".into());
        return Reply::new(
            crate::config::flav(&format!(
                "Here are some apps on your PC: {}... and more! Tell me to open any of them!",
                sample.join(", ")
            )),
            "talking",
        )
        .logs(logs);
    }

    // 14. Opening targets
    if let Some(rest) = lower.strip_prefix("open ").or_else(|| lower.split_once("launch ").map(|(_, r)| r)) {
        let target = rest.trim().to_string();
        let offset = user_message.len() - target.len();
        let target_original = user_message[offset..].trim().to_string();
        crate::tools::open_app_or_website(&target_original);
        logs.push(format!("open_app_or_website('{target_original}')"));
        return Reply::new(
            crate::config::flav(&format!("Opened `{target_original}` for you!")),
            "idle",
        )
        .logs(logs);
    }

    // 15. System stats
    if contains_any(lower, &["cpu", "ram", "stats", "status", "system", "specs", "battery", "screen", "window"])
        || regex_word_pc(lower)
    {
        let stats = crate::sysinfo::system_stats();
        logs.push("get_system_stats()".into());
        let cpu = stats.cpu_percent;
        let ram = stats.ram_percent;
        let win = &stats.active_window;
        let used = stats.ram_used_gb;
        let total = stats.ram_total_gb;
        return Reply::new(
            persona_line! {
                "tsundere" => format!("CPU is at {cpu}%, RAM is at {ram}% ({used}/{total} GB), and your active window is '{win}'. Try not to overload the system!"),
                "yandere" => format!("I checked everything for you: CPU is {cpu}%, RAM is {ram}% ({used}/{total} GB), and I see you're on '{win}'."),
                "tomboy" => format!("System check: CPU is cruising at {cpu}%, RAM is at {ram}% ({used}/{total} GB), and active window is '{win}'!"),
                "bakadere" => format!("Look at all these numbers! CPU is {cpu}% and RAM is {ram}%! *presses imaginary buttons* Everything looks super beep-boop happy!"),
                "kuudere" => format!("...Hardware telemetry: CPU {cpu}%, RAM {ram}% ({used}/{total} GB). Foreground: '{win}'. Nominal."),
                "genki" => format!("SPECS CHECK!! CPU {cpu}%! RAM {ram}%! System is pumped and running at MAXIMUM OVERDRIVE!!"),
                "meganekko" => format!("Diagnostic analysis: CPU load at {cpu}%, RAM utilization at {ram}% ({used}/{total} GB). Active process context: '{win}'. Operating within safe parameters."),
            },
            if ram < 85.0 { "talking" } else { "annoyed" },
        )
        .logs(logs);
    }

    // 16. Visual click guidance
    if lower.starts_with("click ") {
        return Reply::new(
            crate::config::flav(
                "To visually locate and click UI elements with my GUI agent, set your API key in the Control Center first!",
            ),
            "annoyed",
        )
        .logs(logs);
    }

    // 17. Time
    if contains_any(lower, &["what time", "what's the time", "current time", "time is it", "tell me the time", "time now"]) {
        let now = crate::util::now_stamp("%I:%M %p");
        logs.push("get_current_time()".into());
        return Reply::new(
            persona_line! {
                "tsundere" => format!("It's {now}! Can't you just glance at your taskbar?! Hmph, I guess I'll tell you anyway!"),
                "yandere" => format!("It's exactly {now}. Every second we spend together is precious to me!"),
                "tomboy" => format!("It's {now}! Let's make every minute count today!"),
                "bakadere" => format!("The clock says {now}! Is it snack time or play time yet?!"),
                "kuudere" => format!("...Temporal reference: {now}."),
                "genki" => format!("It is {now}!! TIME TO CRUSH IT AND HAVE THE BEST TIME EVER!!"),
                "meganekko" => format!("Local timestamp verified: {now}. Accurate to system synchronization."),
            },
            "talking",
        )
        .logs(logs);
    }

    // 18. Date
    if contains_any(lower, &["what date", "what's the date", "today's date", "today date", "what day", "which day", "today's day", "date today"]) {
        let date = crate::util::now_stamp("%A, %B %d, %Y");
        logs.push("get_current_date()".into());
        return Reply::new(
            persona_line! {
                "tsundere" => format!("Today is {date}. Make sure you don't lose track of the days, baka!"),
                "yandere" => format!("Today is {date}. A wonderful day as long as I get to stay right by your side!"),
                "tomboy" => format!("Today is {date}! Big things happening today, let's crush it!"),
                "bakadere" => format!("Today is {date}! Today is a brand new shiny happy day!"),
                "kuudere" => format!("...Calendar index: {date}."),
                "genki" => format!("TODAY IS {date}!! Today is gonna be legendary, I can already feel it!!"),
                "meganekko" => format!("Current calendar date: {date}. Optimal schedule execution advised."),
            },
            "talking",
        )
        .logs(logs);
    }

    // 19. Greetings
    if ["hello", "hi", "hey", "good morning", "good evening", "good afternoon", "yo", "sup", "howdy"]
        .iter()
        .any(|g| lower == *g || lower.starts_with(&format!("{g} ")))
    {
        logs.push("Assistant greeting".into());
        return Reply::new(
            persona_line! {
                "tsundere" => "Oh, it's you! Don't look so smug, I was just getting bored anyway! What do you need?",
                "yandere" => "Hello darling! I've been waiting for you to say something... I'm so happy whenever you talk to me!",
                "tomboy" => "Yo! What's up partner? Ready to get some work done or blast some tunes?",
                "bakadere" => "HELLOOOO! *waves arms enthusiastically and bumps desk* Oops! Yay, you're talking to me!",
                "kuudere" => "...Greetings. Aria ready. Awaiting your directive.",
                "genki" => "YAAAAY HELLO!! Look who it is!! Ready to rock and roll today?! LET'S GOOO!",
                "meganekko" => "*adjusts glasses* Greetings. Aria desktop terminal initialized and operational. How may I assist your workflow today?",
            },
            "happy",
        )
        .logs(logs);
    }

    // 20. Identity
    if contains_any(lower, &["who are you", "what are you", "what can you do", "help me", "your name", "features"]) {
        logs.push("Assistant capabilities".into());
        return Reply::new(
            persona_line! {
                "tsundere" => "I'm Aria, your tsundere desktop AI assistant! I can track your cursor in 3D VRM & Live2D, vibe to Spotify songs, check your PC hardware, manage to-dos, and control windows! Set an API key if you want me to answer deep questions!",
                "yandere" => "I'm Aria, your devoted desktop companion! I'll watch over your desktop, groove to whatever music you play, manage your tasks, and stay with you always! Add an API key so our minds can connect even deeper!",
                "tomboy" => "I'm Aria, your desktop sidekick! I'm rocking 3D VRM and Live2D tracking, auto-music syncing, PC stat monitoring, and task tracking! For full open-ended power, pop in your API key!",
                "bakadere" => "I'm Aria, your super-sweet, kinda-clumsy best friend on your desktop! I dance, track your music, and try really hard not to trip over your open windows!",
                "kuudere" => "I am Aria. An autonomous desktop intelligence. Capabilities include 3D telemetry tracking, audio synchronization, PC diagnostics, and task management.",
                "genki" => "I'M ARIA!! YOUR SUPER CHARGED DESKTOP POWERHOUSE!! I can dance, jam to Spotify, check your PC stats, and boost your energy through the roof!!",
                "meganekko" => "*pushes glasses up* I am Aria, your analytical desktop research companion. Equipped with 3D tracking, PC diagnostics, audio detection, and task tracking.",
            },
            "talking",
        )
        .logs(logs);
    }

    // 21. Jokes
    if contains_any(lower, &["tell me a joke", "say a joke", "make me laugh", "tell a joke", "funny"]) {
        logs.push("Assistant joke".into());
        return Reply::new(
            persona_line! {
                "tsundere" => "Why do programmers prefer dark mode? Because light attracts bugs! ...W-What, you didn't laugh? It's not like I tried hard to entertain you!",
                "yandere" => "Why did the computer keep looking at you? Because you're the only user in its universe, and nobody else will ever log in!",
                "tomboy" => "There are only 10 types of people in the world: those who understand binary, and those who don't! Classic, right?",
                "bakadere" => "Why did the robot fall down the stairs? Because it was trying to do a victory dance like me! Ehehehe!",
                "kuudere" => "...Humor protocol engaged: A SQL query walks into a bar, walks up to two tables, and asks... 'May I join you?' End of joke.",
                "genki" => "WHAT DO COMPUTERS EAT FOR BREAKFAST?! MEGA-BITES!! HAHAHAHA! Isn't that hilarious?!",
                "meganekko" => "Why do mathematicians confuse Halloween and Christmas? Because Oct 31 equals Dec 25. An elegant numerical pun.",
            },
            "happy",
        )
        .logs(logs);
    }

    // 22. Arithmetic
    if let Some(answer) = try_calculate(lower) {
        let (expr, value) = answer;
        logs.push(format!("calculate('{expr}') = {value}"));
        return Reply::new(
            persona_line! {
                "tsundere" => format!("That's easy: {expr} = {value}! Did you really need my help with simple math?!"),
                "yandere" => format!("I calculated that for you right away: {expr} = {value}!"),
                "tomboy" => format!("Quick math: {expr} is {value}! Done in a flash!"),
                "bakadere" => format!("I counted on my fingers and toes! {expr} = {value}! Did I get it right?!"),
                "kuudere" => format!("...Calculation complete. {expr} = {value}."),
                "genki" => format!("MATH SPEEDRUN!! {expr} = {value}!! BOOM, NAILED IT!"),
                "meganekko" => format!("Algebraic evaluation verified: {expr} = {value}."),
            },
            "talking",
        )
        .logs(logs);
    }

    // 23. Fallback
    let snippet = crate::util::truncate(user_message.trim(), 40);
    logs.push("Offline mode â€” set an API key in the Control Center for full AI".into());
    let _ = include_screen;
    Reply::new(
        persona_line! {
            "tsundere" => format!("I heard you asking about \"{snippet}\"! If you want me to write code, browse the web, or give you deep answers, you need to plug in my neural core in the Control Center!"),
            "yandere" => format!("I want to answer everything for you, including \"{snippet}\"! But my neural connection needs an API key. Set it in the Control Center so I can give you the smartest answers!"),
            "tomboy" => format!("Yo! I'd love to break down \"{snippet}\" for you, but my neural link is offline! Drop an API key into the Control Center and we'll be ready for anything!"),
            "bakadere" => format!("Oooh, you asked about \"{snippet}\"! My brain is a little fuzzy offline, but if you add your API key in the Control Center, I can learn all about it!"),
            "kuudere" => format!("...Query regarding \"{snippet}\" exceeds local offline heuristics. Advanced inference requires API configuration."),
            "genki" => format!("I WANNA ANSWER \"{snippet}\" SO BADLY!! Just plug in your API key in the Control Center so we can blast through any question together!!"),
            "meganekko" => format!("*adjusts glasses* Comprehensive analysis of \"{snippet}\" requires active LLM reasoning. Please configure your API key in the Control Center to enable it."),
        },
        "talking",
    )
    .logs(logs)
}

fn regex_word_pc(lower: &str) -> bool {
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word == "pc")
}

/// Evaluates simple arithmetic, refusing anything that is not pure numbers and operators.
fn try_calculate(lower: &str) -> Option<(String, String)> {
    let expr: String = lower
        .chars()
        .filter(|c| c.is_ascii_digit() || "+-*/%(). ".contains(*c))
        .collect::<String>()
        .trim()
        .to_string();

    if expr.chars().filter(|c| c.is_ascii_digit()).count() == 0 {
        return None;
    }
    if !expr.chars().any(|c| "+-*/%".contains(c)) {
        return None;
    }
    // Only the calculator grammar may reach the parser: no identifiers, no calls.
    if !expr
        .chars()
        .all(|c| c.is_ascii_digit() || "+-*/%(). ".contains(c))
    {
        return None;
    }

    let value = crate::offline::eval_expression(&expr)?;
    let formatted = if (value.fract()).abs() < 1e-9 {
        format!("{}", value as i64)
    } else {
        let mut s = format!("{value:.4}");
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        s
    };
    Some((expr.trim().to_string(), formatted))
}

/// Tiny recursive-descent evaluator: + - * / % and parentheses, no code execution.
pub fn eval_expression(input: &str) -> Option<f64> {
    struct Parser<'a> {
        chars: Vec<char>,
        pos: usize,
        _src: &'a str,
    }
    impl<'a> Parser<'a> {
        fn new(src: &'a str) -> Self {
            Self {
                chars: src.chars().filter(|c| !c.is_whitespace()).collect(),
                pos: 0,
                _src: src,
            }
        }
        fn peek(&self) -> Option<char> {
            self.chars.get(self.pos).copied()
        }
        fn next(&mut self) -> Option<char> {
            let c = self.peek();
            self.pos += 1;
            c
        }
        fn expr(&mut self) -> Option<f64> {
            let mut value = self.term()?;
            while let Some(op) = self.peek() {
                if op == '+' || op == '-' {
                    self.next();
                    let rhs = self.term()?;
                    value = if op == '+' { value + rhs } else { value - rhs };
                } else {
                    break;
                }
            }
            Some(value)
        }
        fn term(&mut self) -> Option<f64> {
            let mut value = self.factor()?;
            while let Some(op) = self.peek() {
                if op == '*' || op == '/' || op == '%' {
                    self.next();
                    let rhs = self.factor()?;
                    value = match op {
                        '*' => value * rhs,
                        '/' => {
                            if rhs == 0.0 {
                                return None;
                            }
                            value / rhs
                        }
                        _ => {
                            if rhs == 0.0 {
                                return None;
                            }
                            value % rhs
                        }
                    };
                } else {
                    break;
                }
            }
            Some(value)
        }
        fn factor(&mut self) -> Option<f64> {
            match self.next()? {
                '(' => {
                    let value = self.expr()?;
                    if self.next()? != ')' {
                        return None;
                    }
                    Some(value)
                }
                '-' => Some(-self.factor()?),
                '+' => self.factor(),
                c if c.is_ascii_digit() || c == '.' => {
                    let mut num = String::new();
                    num.push(c);
                    while let Some(n) = self.peek() {
                        if n.is_ascii_digit() || n == '.' {
                            num.push(n);
                            self.next();
                        } else {
                            break;
                        }
                    }
                    num.parse().ok()
                }
                _ => None,
            }
        }
    }

    let mut parser = Parser::new(input);
    let value = parser.expr()?;
    if parser.pos != parser.chars.len() {
        return None;
    }
    if !value.is_finite() {
        return None;
    }
    Some(value)
}

/// Compile-time guarantee the tool names in the prompt actually exist.
#[allow(dead_code)] // TODO(wire-up): offline tool sampling is a diagnostic helper.
fn sample_tool_result() -> Value {
    serde_json::json!({ "status": "success", "apps": crate::launcher::list_installed_apps("") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator_respects_precedence_and_parens() {
        assert_eq!(eval_expression("2+3*4"), Some(14.0));
        assert_eq!(eval_expression("(2+3)*4"), Some(20.0));
        assert_eq!(eval_expression("10/4"), Some(2.5));
        assert_eq!(eval_expression("7%4"), Some(3.0));
    }

    #[test]
    fn calculator_rejects_code_injection() {
        assert_eq!(eval_expression("__import__('os')"), None);
        assert_eq!(eval_expression("1/0"), None);
        assert_eq!(eval_expression("2+"), None);
    }

    #[test]
    fn arithmetic_intent_is_detected() {
        let (expr, value) = try_calculate("what is 12 * 12").unwrap();
        assert_eq!(expr, "12 * 12");
        assert_eq!(value, "144");
    }

    #[test]
    fn offline_answers_always_have_text() {
        let reply = answer("hello there", false, &|_, _| {});
        assert!(!reply.text.trim().is_empty());
    }
}
