//! Speech and conversation: the bubble, persona accent, microphone and prompts.

use super::*;

/// Persona to accent colour, across every window.
pub(crate) fn apply_persona(app: &AppHandle) {
    let key = config::current_persona();
    let profile = config::persona(&key);
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
    let key = config::current_persona();
    let line = match key.as_str() {
    "yandere" => "I'm right here on your desktop watching over you. We're going to get so much done today.",
        "tomboy" => "Aria reporting in! Tell me what we're tackling first today.",
        "bakadere" => "Yaaay, you're here! I dropped my pencil five times waiting, but now my day is 100% amazing.",
        "kuudere" => "Aria operational. Diagnostics optimal. Standing by.",
        "genki" => "GOOD MORNING! Aria is pumped and 1000% ready to make today the greatest day ever.",
        "meganekko" => "*adjusts glasses* Aria reporting for duty. Cognitive engine primed. What problem shall we solve today?",
        _ => "Good job today! I just happened to be nearby on your desktop if you need anything.",    };
    say(app, &config::flav(line), "talking", vec![], true);
    let mut warm: Vec<(String, String)> = vec![(line.to_string(), "talking".into())];
    for kind in ["poke", "shake", "headpat"] {
        if let Some(text) = crate::reactions::first_line(&key, kind) {
            let emotion = match kind {
                "poke" => "annoyed",
                "shake" => "dizzy",
                _ => "happy",
            };
            warm.push((text, emotion.into()));
        }
    }
    crate::voice::precache(&warm);
}

/// The single speech path: bubble, stage expression, voice, auto-hide.
pub(crate) fn say(app: &AppHandle, text: &str, emotion: &str, logs: Vec<String>, speak: bool) {
    let tts_on = {
        let mut a = app.borrow_mut();
        a.full_bubble_text = text.to_string();
        a.typed = 0;
        a.last_emotion = emotion.to_string();
        let subtitles = config::get().subtitles_enabled;
        let tts_on = config::get().tts_enabled;
        a.ui.set_bubble_text("".into());
        a.ui.set_bubble_shown(subtitles);
        a.ui.set_status_line(status_for(emotion).into());
        tts_on
    };
    let duration_ms = ((text.chars().count() as f32) * 58.0).clamp(2200.0, 9500.0) as u64;
    app.borrow_mut().bubble_hide_at =
        Some(Instant::now() + Duration::from_millis(duration_ms + 3200));
    sync_chrome(app);
    {
        let a = app.borrow();
        let stage = &a.stage;
        if matches!(emotion, "dance" | "groove") {
            stage.set_music_vibe(true, 1.0);
        } else if ["idle", "sleepy", "walking", "sit", "annoyed"].contains(&emotion) {
            stage.set_music_vibe(false, 0.0);
        }
        stage.set_state(emotion, a.facing_right, a.target.is_some(), speak && tts_on);
    }
    for entry in &logs {
        log::info!("{entry}");
    }
    if speak {
        if tts_on {
            crate::voice::speak_async(text, emotion);
        }
        app.borrow_mut().speaking_until = Some(Instant::now() + Duration::from_millis(duration_ms));
    }
    if let Some(studio) = app.borrow().studio.as_ref() {
        studio.set_status_text(logs.first().cloned().unwrap_or_default().into());
    }
}

pub(crate) fn status_for(emotion: &str) -> &'static str {
    match emotion {
        "talking" => "Speaking",
        "happy" => "Feeling good",
        "annoyed" => "Pouting",
        "walking" => "Walking",
        "dance" => "Dancing",
        "groove" => "Vibing",
        "sit" => "Sitting",
        "sleepy" => "Resting",
        _ => "Standing by",
    }
}

pub(crate) fn hide_bubble(app: &AppHandle) {
    app.borrow().ui.set_bubble_shown(false);
    app.borrow_mut().bubble_hide_at = None;
    sync_chrome(app);
}

pub(crate) fn toggle_mic(app: &AppHandle) {
    if crate::voice::is_recording() {
        stop_mic(app);
        return;
    }
    if crate::voice::start_recording() {
        app.borrow().ui.set_recording(true);
        say(
            app,
            "Listening — click the mic again when you're done.",
            "idle",
            vec!["recording from the microphone".into()],
            false,
        );
    } else {
        say(
            app,
            &config::flav(
                "I couldn't reach your microphone. Check that an input device is connected.",
            ),
            "annoyed",
            vec!["microphone unavailable".into()],
            false,
        );
    }
}

pub(crate) fn stop_mic(app: &AppHandle) {
    app.borrow().ui.set_recording(false);
    let Some(wav) = crate::voice::stop_recording_wav() else {
        say(
            app,
            &config::flav("I didn't hear anything on the mic. Try speaking a bit louder."),
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
                    &config::flav("I couldn't catch any speech. Speak clearly and try again."),
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
    if app.borrow().busy {
        return;
    }
    {
        let mut a = app.borrow_mut();
        a.busy = true;
        a.ui.set_busy(true);
        a.ui.set_draft("".into());
    }
    if config::get().subtitles_enabled {
        say(app, "Thinking…", "talking", vec!["thinking".into()], false);
    }
    std::thread::spawn(move || {
        let reply = brain().chat(
            &prompt,
            include_screen,
            &|location: &str, emotion: Option<&str>| {
                let location = location.to_string();
                let emotion = emotion.unwrap_or("walking").to_string();
                post(move |app| walk_to(app, &location, &emotion));
            },
        );
        post(move |app| {
            {
                let mut a = app.borrow_mut();
                a.busy = false;
                a.ui.set_busy(false);
            }
            let emotion = if app.borrow().target.is_some() {
                "walking".to_string()
            } else {
                reply.emotion.clone()
            };
            say(app, &reply.text, &emotion, reply.logs.clone(), true);
            apply_persona(app);
        });
    });
}
