//! Speech and conversation: the bubble, persona accent, microphone and prompts.

use super::*;

/// Persona to accent colour, across every window.
pub(crate) fn apply_persona(app: &AppHandle) {
    let key = crate::persona::current_key();
    let profile = crate::persona::get(&key);
    let (wash, edge) = accent_tints(profile.accent);
    {
        let mut a = app.borrow_mut();
        if a.persona == key && a.accent.0 == profile.accent {
            return;
        }
        a.persona = key.clone();
        a.accent = (profile.accent.to_string(), wash.clone(), edge.clone());
        let ui = &a.ui;
        ui.set_accent_color(hex_to_color(profile.accent));
        ui.set_accent_wash(hex_to_color(&wash));
        ui.set_accent_edge(hex_to_color(&edge));
        ui.set_persona_badge(profile.short_badge.into());
        ui.set_persona_name(profile.name.into());
        // The persona also styles how she stands and moves.
        a.stage.set_mood(&key);
    }
    let color = hex_to_color(profile.accent);
    let wash_color = hex_to_color(&wash);
    let edge_color = hex_to_color(&edge);
    {
        let a = app.borrow();
        if let Some(studio) = a.studio.as_ref() {
            studio.set_accent_color(color);
            studio.set_accent_wash(wash_color);
            studio.set_accent_edge(edge_color);
            studio.set_mood_badge(profile.short_badge.into());
        }
        if let Some(menu) = a.menu.as_ref() {
            menu.set_accent_color(color);
            menu.set_accent_wash(wash_color);
            menu.set_accent_edge(edge_color);
            menu.set_mood_badge(profile.short_badge.into());
        }
        if let Some(dial) = a.dial.as_ref() {
            dial.set_accent_color(color);
            dial.set_accent_wash(wash_color);
            dial.set_accent_edge(edge_color);
        }
    }
    sync_studio(app);
}

pub(crate) fn greeting(app: &AppHandle) {
    react(app, "The app just opened. Greet the user for this visit, fitting the current time and mood.", "wave");
}

/// Gestures start immediately; their words arrive asynchronously from the same AI as chat.
/// Coalesce rapid touches and discard a delayed reaction if a conversation has overtaken it.
pub(crate) fn react(app: &AppHandle, situation: &str, emotion: &str) {
    let (generation, started) = {
        let mut a = app.borrow_mut();
        if a.busy || a.reaction_pending || crate::voice::is_recording() {
            return;
        }
        a.reaction_pending = true;
        a.last_emotion = emotion.into();
        a.stage.set_state(emotion, false);
        a.bubble_hide_at = Some(Instant::now() + Duration::from_secs(5));
        (a.speech_generation, Instant::now())
    };
    let (situation, emotion) = (situation.to_string(), emotion.to_string());
    std::thread::spawn(move || {
        let reply = crate::ai::tasks::proactive_line(&situation, &emotion);
        post(move |app| {
            let current = {
                let mut a = app.borrow_mut();
                a.reaction_pending = false;
                a.speech_generation == generation && !a.busy && !a.dragging
                    && !crate::voice::is_recording() && started.elapsed() < Duration::from_secs(25)
            };
            if current {
                if let Some((text, emotion)) = reply {
                    say(app, &text, &emotion, vec![], true);
                }
            }
        });
    });
}

/// The single speech path: bubble, stage expression, voice, auto-hide.
///
/// `speak` marks a real line (spoken when TTS is on). Real lines are also what a click on her
/// brings back ([`show_last_chat`]); status lines like "Thinking…" are not.
pub(crate) fn say(app: &AppHandle, text: &str, emotion: &str, logs: Vec<String>, speak: bool) {
    let settings = config::get();
    {
        let mut a = app.borrow_mut();
        a.speech_generation = a.speech_generation.wrapping_add(1);
        a.full_bubble_text = text.to_string();
        a.typed = 0;
        a.last_emotion = emotion.to_string();
        if speak && !text.trim().is_empty() {
            a.last_chat = Some((text.to_string(), emotion.to_string()));
        }
        a.ui.set_bubble_text("".into());
        a.ui.set_bubble_layout_text(text.into());
        a.ui.set_bubble_shown(settings.subtitles_enabled);
        a.ui.set_status_line(status_for(emotion).into());
        a.bubble_hide_at = Some(Instant::now() + bubble_duration(text) + Duration::from_millis(3200));
    }
    sync_chrome(app);
    // `talking` is only true while audio really plays; the motion tick keeps it current.
    app.borrow().stage.set_state(emotion, crate::voice::is_speaking());
    for entry in &logs {
        log::info!("{entry}");
    }
    if speak {
        if settings.tts_enabled {
            crate::voice::speak_async(text, emotion);
        }
        app.borrow_mut().speaking_until = Some(Instant::now() + bubble_duration(text));
    }
    if let Some(studio) = app.borrow().studio.as_ref() {
        studio.set_status_text(logs.first().cloned().unwrap_or_default().into());
    }
}

pub(crate) fn status_for(emotion: &str) -> &'static str {
    match emotion {
        "talking" => "Speaking",
        "happy" => "Feeling good",
        "laugh" => "Laughing",
        "annoyed" => "Pouting",
        "angry" => "Fuming",
        "sad" => "Feeling down",
        "cry" => "Crying",
        "thinking" => "Thinking",
        "surprised" => "Surprised",
        "shy" => "Flustered",
        "wave" => "Waving",
        "dizzy" => "Dizzy",
        "walking" => "Walking",
        "dance" => "Dancing",
        "groove" => "Vibing",
        "sit" => "Sitting",
        "sleepy" => "Resting",
        "stretch" => "Stretching",
        "look_around" => "Looking around",
        "hum" => "Humming",
        "twirl" => "Twirling",
        "fix_hair" => "Fixing her hair",
        "clap" => "Clapping",
        _ => "Standing by",
    }
}

/// How long a line stays up: long enough to read, never forever.
fn bubble_duration(text: &str) -> Duration {
    Duration::from_millis(((text.chars().count() as f32) * 58.0).clamp(2200.0, 9500.0) as u64)
}

/// Emotions that are a reaction to something said: they fade back to idle with the bubble.
/// Sitting, dozing and dancing are things she keeps doing until something else happens.
fn is_reaction(emotion: &str) -> bool {
    !matches!(emotion, "idle" | "sit" | "sleepy" | "dance" | "groove")
}

/// A click on her: brings back the last thing she said, without saying it again.
pub(crate) fn show_last_chat(app: &AppHandle) {
    let Some((text, emotion)) = app.borrow().last_chat.clone() else {
        say(app, &crate::persona::flav("We haven't talked yet. Right-click me to open the chat bar!"), "wave", vec![], false);
        return;
    };
    // Re-showing is not a new line: no voice, and the gesture stays calm rather than replaying
    // the original reaction.
    let calm = if is_reaction(&emotion) { "talking" } else { emotion.as_str() };
    say(app, &text, calm, vec!["showing the last chat".into()], false);
}

pub(crate) fn hide_bubble(app: &AppHandle) {
    app.borrow().ui.set_bubble_shown(false);
    {
        let mut a = app.borrow_mut();
        a.bubble_hide_at = None;
        // Still thinking: keep the pose until the reply arrives.
        if is_reaction(&a.last_emotion) && !a.busy {
            a.last_emotion = "idle".into();
            a.ui.set_status_line(status_for("idle").into());
        }
    }
    sync_chrome(app);
}

/// Click-to-talk: the mic button (or Ctrl+Shift+M) starts a recording, and again stops it.
pub(crate) fn toggle_mic(app: &AppHandle) {
    if crate::voice::is_recording() {
        stop_mic(app);
    } else {
        start_mic(app, "Listening — click the mic again when you're done.");
    }
}

/// Push-to-talk: recording while the key is held.
pub(crate) fn push_to_talk(app: &AppHandle, held: bool) {
    if config::get().mic_mode != "push_to_talk" {
        return;
    }
    match (held, crate::voice::is_recording()) {
        (true, false) => start_mic(app, "Listening — let go of the keys when you're done."),
        (false, true) => stop_mic(app),
        _ => {}
    }
}

fn start_mic(app: &AppHandle, prompt: &str) {
    match crate::voice::start_recording() {
        Ok(()) => {
            app.borrow().ui.set_recording(true);
            say(app, prompt, "idle", vec!["recording from the microphone".into()], false);
        }
        Err(e) => say(
            app,
            &crate::persona::flav("I couldn't reach your microphone. Check the input device in the Control Center."),
            "annoyed",
            vec![format!("microphone unavailable: {e}")],
            false,
        ),
    }
}

/// Starts, stops or restarts hands-free listening to match the mic settings. Called at startup
/// and whenever the mic mode, wake word or input device changes.
pub(crate) fn apply_mic_settings() {
    crate::voice::listener::apply(|phrase| {
        post(move |app| {
            let include_screen = app.borrow().ui.get_vision();
            show_bar(app);
            submit_prompt(app, phrase, include_screen);
        });
    });
}

pub(crate) fn stop_mic(app: &AppHandle) {
    app.borrow().ui.set_recording(false);
    let Some(wav) = crate::voice::stop_recording_wav() else {
        say(
            app,
            &crate::persona::flav("I didn't hear anything on the mic. Try speaking a bit louder."),
            "idle",
            vec!["no audio captured".into()],
            false,
        );
        return;
    };
    say(
        app,
        "Transcribing…",
        "talking",
        vec!["transcribing voice audio".into()],
        false,
    );
    let include_screen = app.borrow().ui.get_vision();
    std::thread::spawn(move || {
        let text = crate::voice::transcribe_wav(&wav);
        post(move |app| {
            let clean = text.trim().to_string();
            if clean.is_empty() {
                say(
                    app,
                    &crate::persona::flav("I couldn't catch any speech. Speak clearly and try again."),
                    "idle",
                    vec!["no speech recognised".into()],
                    false,
                );
                return;
            }
            submit_prompt(app, clean, include_screen);
        });
    });
}

pub(crate) fn submit_prompt(app: &AppHandle, prompt: String, include_screen: bool) {
    if app.borrow().busy || prompt.trim().is_empty() {
        return;
    }
    {
        let mut a = app.borrow_mut();
        a.busy = true;
        a.speech_generation = a.speech_generation.wrapping_add(1);
        a.ui.set_busy(true);
        a.ui.set_draft("".into());
    }
    if config::get().subtitles_enabled {
        say(app, "Thinking…", "thinking", vec!["thinking".into()], false);
    } else {
        // No bubble, but she still visibly thinks it over.
        app.borrow_mut().last_emotion = "thinking".into();
        app.borrow().stage.set_state("thinking", false);
    }
    std::thread::spawn(move || {
        let reply = crate::ai::chat(&prompt, include_screen, &|location: &str, emotion: Option<&str>| {
            let location = location.to_string();
            let emotion = emotion.unwrap_or("idle").to_string();
            post(move |app| walk_to(app, &location, &emotion));
        });
        post(move |app| {
            {
                let mut a = app.borrow_mut();
                a.busy = false;
                a.ui.set_busy(false);
            }
            // Walking (if the reply sent her somewhere) comes from the movement itself, so she
            // can walk off happily or sulkily.
            say(app, &reply.text, &reply.emotion, reply.logs.clone(), true);
            apply_persona(app);
        });
    });
}
