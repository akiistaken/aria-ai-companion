//! Timers: motion, proactive events, game dodge, music vibe, boredom, UI typing and the render policy.

use super::*;

/// Registers a repeating timer owned by the app, so dropping the app stops every timer.
pub(crate) fn own_timer(app: AppHandle, interval: Duration, f: impl Fn() + 'static) {
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, interval, f);
    app.borrow_mut().timers.push(timer);
}

pub(crate) fn start_timers(app: &AppHandle) {
    {
        let app = app.clone();
        own_timer(app.clone(), Duration::from_millis(33), move || {
            let _ = try_with_app_mut(|_| tick_motion(&app));
        });
    }
    {
        let app = app.clone();
        own_timer(app.clone(), Duration::from_millis(6000), move || {
            tick_proactive(&app)
        });
    }
    {
        let app = app.clone();
        own_timer(app.clone(), Duration::from_millis(1800), move || {
            tick_music(&app)
        });
    }
    {
        let app = app.clone();
        own_timer(app.clone(), Duration::from_secs(18), move || {
            tick_boredom(&app)
        });
    }
    {
        let app = app.clone();
        own_timer(app.clone(), Duration::from_millis(90), move || {
            let _ = try_with_app_mut(|_| tick_ui(&app));
        });
    }
    {
        let app = app.clone();
        // Re-assert the body layout so the three windows follow the Slint position.
        own_timer(app.clone(), Duration::from_millis(400), move || {
            layout_windows(&app)
        });
    }
    {
        let app = app.clone();
        own_timer(app.clone(), Duration::from_secs(15), move || {
            if crate::discord::is_enabled() && app.borrow().last_discord.elapsed().as_secs() >= 15 {
                app.borrow_mut().last_discord = Instant::now();
                crate::discord::update("", "", &config::current_persona());
            }
        });
    }
    {
        let app = app.clone();
        own_timer(app.clone(), Duration::from_secs(1), move || {
            let _ = try_with_app_mut(|_| apply_render_policy(&app));
        });
    }
    own_timer(app.clone(), Duration::from_secs(90), || {
        crate::win32::trim_working_set();
    });
}

pub(crate) fn tick_motion(app: &AppHandle) {
    let target = app.borrow().target;
    if let Some((tx, ty)) = target {
        if !app.borrow().dragging {
            let (cx, cy) = app.borrow().pos;
            let dx = tx - cx;
            let dy = ty - cy;
            let dist = ((dx * dx + dy * dy) as f64).sqrt();
            let speed = 4.5;
            if dist <= speed {
                set_window_pos(app, tx, ty);
                let (emotion, facing) = {
                    let mut a = app.borrow_mut();
                    a.target = None;
                    (a.post_walk_emotion.clone(), a.facing_right)
                };
                app.borrow().stage.set_state(&emotion, facing, false, false);
                app.borrow().ui.set_status_line(status_for(&emotion).into());
            } else {
                let step_x = cx + ((dx as f64 / dist) * speed).round() as i32;
                let step_y = cy + ((dy as f64 / dist) * speed).round() as i32;
                set_window_pos(app, step_x, step_y);
                let facing = dx > 0;
                app.borrow_mut().facing_right = facing;
                app.borrow().stage.set_state("walking", facing, true, false);
                app.borrow().ui.set_status_line("Walking".into());
            }
        }
    }
    let cursor = crate::input::cursor_position();
    let (win_pos, (w, h)) = (app.borrow().pos, window_size(app));
    let cx = win_pos.0 + w / 2;
    let cy = win_pos.1 + h / 2;
    let nx = ((cursor.0 - cx) as f32 / (w.max(100) as f32 * 1.5)).clamp(-1.0, 1.0);
    let ny = ((cy - cursor.1) as f32 / (h.max(100) as f32)).clamp(-1.0, 1.0);
    app.borrow().stage.set_mouse_look(nx, ny);
    {
        // Cursor relative to the avatar window, so the renderer can hit-test the model and run its
        // gesture detectors (the layered window gets no input events of its own). Positions far
        // outside are clamped: they only need to read as "not over the model".
        let (ax, ay, aw, ah) = avatar_rect(app);
        let x = (cursor.0 - ax).clamp(-64, aw + 64) as f32;
        let y = (cursor.1 - ay).clamp(-64, ah + 64) as f32;
        let (left, right, middle) = crate::input::mouse_buttons();
        app.borrow().stage.set_pointer(x, y, left, right, middle);
    }
    if crate::voice::is_speaking() {
        let a = app.borrow();
        a.stage
            .set_state(&a.last_emotion, a.facing_right, a.target.is_some(), true);
        let (aa, oh) = crate::voice::mouth_levels();
        a.stage.set_mouth(aa, oh);
    } else {
        app.borrow().stage.set_mouth(0.0, 0.0);
    }
}

/// Render policy, re-evaluated every second: pause while the session is locked or the mascot is
/// not on screen, 10 FPS while a game has the foreground, 30 otherwise.
pub(crate) fn apply_render_policy(app: &AppHandle) {
    // Keep the mascot window (bubble + chat bar) in the topmost band. The avatar window is owned
    // by it and sits in that band too; if the mascot ever dropped out, other apps would cover the
    // chat chrome while the avatar stayed on top, and the two would look unrelated.
    if let Some(mascot) = mascot_hwnd(&app.borrow().ui) {
        if crate::win32::ensure_topmost(mascot) {
            log::info!("mascot window was not topmost; restored");
        }
    }
    let hidden = !app.borrow().ui.window().is_visible() || crate::win32::session_locked();
    let fps = if hidden {
        0
    } else if app.borrow().game_compacted {
        10
    } else {
        30
    };
    app.borrow().stage.set_fps(fps);
}

pub(crate) fn tick_proactive(app: &AppHandle) {
    // The guardian samples windows, WASAPI sessions and sysinfo; none of that may run on the UI    // thread, so the whole check happens on a worker and only events come back.

    static PROACTIVE_IN_FLIGHT: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    if PROACTIVE_IN_FLIGHT.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    if app.borrow().busy || app.borrow().dragging || crate::voice::is_recording() {
        PROACTIVE_IN_FLIGHT.store(true, std::sync::atomic::Ordering::SeqCst);
        std::thread::spawn(|| {
            crate::proactive::observe_mood();
            PROACTIVE_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
        });
        return;
    }
    PROACTIVE_IN_FLIGHT.store(true, std::sync::atomic::Ordering::SeqCst);
    std::thread::spawn(move || {
        crate::proactive::observe_mood();
        let game = crate::proactive::detect_active_game_or_fullscreen();
        let event = crate::proactive::check_for_event();
        PROACTIVE_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
        post(move |app| apply_proactive(app, game, event));
    });
}

pub(crate) fn apply_proactive(
    app: &AppHandle,
    game: (bool, String),
    event: Option<crate::proactive::ProactiveEvent>,
) {
    if let Some(change) = crate::mood::consume_change() {
        log::info!(
            "mood shift {} -> {} ({})",
            change.old,
            change.new,
            change.reason
        );
        apply_persona(app);
        if config::get().voice_follows_mood {
            let target = config::persona(&change.new).voice;
            config::update(|s| {
                if s.tts_voice != target {
                    s.tts_voice = target.to_string();
                }
            });
        }
    }
    apply_game_state(app, game);
    if app.borrow().busy || app.borrow().dragging || crate::voice::is_recording() {
        return;
    }
    if let Some(event) = event {
        if let Some(location) = &event.walk_location {
            if location == "perch"
                || event.category.starts_with("app_")
                || event.category.contains("music")
            {
                if let Some(point) = perch_point(app) {
                    let mut a = app.borrow_mut();
                    a.target = Some(point);
                    a.post_walk_emotion = event.emotion.clone();
                }
            } else {
                walk_to(app, location, &event.emotion);
            }
        }
        say(app, &event.message, &event.emotion, vec![event.badge], true);
    }
}

/// Game-dodge state machine, fed by a worker-side detection result.
pub(crate) fn apply_game_state(app: &AppHandle, detection: (bool, String)) {
    if !config::get().smart_gaming_dodge_enabled && !config::get().automatic_game_copilot_enabled {
        return;
    }
    let (is_game, game) = detection;
    let compacted = app.borrow().game_compacted;
    if is_game && !compacted {
        {
            let mut a = app.borrow_mut();
            a.game_compacted = true;
            a.pre_game_scale = config::get().avatar_scale;
            a.pre_game_pos = Some(a.pos);
            a.last_copilot_vision = Instant::now();
        }
        apply_scale(app, 0.52);
        let (sx, sy, sw, sh) = crate::win32::work_area();
        let (w, h) = window_size(app);
        set_window_pos(app, sx + sw - (w as f32 * 0.86) as i32, sy + sh - h - 8);
        set_peeking(app, "right");
        let name = if game.is_empty() {
            "a game".to_string()
        } else {
            game
        };
        say(
            app,
            &config::flav(&format!(
                "Game detected ({name}). I'll cheer you on and watch your back from the corner."
            )),
            "talking",
            vec![format!("game copilot active ({name})")],
            true,
        );
    } else if !is_game && compacted {
        app.borrow_mut().game_compacted = false;
        set_peeking(app, "none");
        let scale = app.borrow().pre_game_scale;
        apply_scale(app, scale);
        let pre_game_pos = app.borrow().pre_game_pos;
        if let Some(pos) = pre_game_pos {
            app.borrow_mut().target = Some(pos);
        }
        say(
            app,
            &config::flav("Back to your desktop. Restored to full size."),
            "idle",
            vec!["desktop restored".into()],
            false,
        );
    } else if is_game && compacted && config::get().automatic_game_copilot_enabled {
        if app.borrow().last_copilot_vision.elapsed().as_secs() >= 85 && !app.borrow().busy {
            app.borrow_mut().last_copilot_vision = Instant::now();
            let name = if game.is_empty() {
                "a game".to_string()
            } else {
                game
            };
            let prompt = format!(                "You are Aria acting as an Automatic Live Game Co-Pilot for {name}. Look at this live screenshot right now. In your active character persona, give a punchy 1-sentence tactical observation, clutch callout, warning about health, ammo or enemies, or enthusiastic encouragement. Keep it under 25 words."            );
            submit_prompt(app, prompt, true);
        }
    }
}

pub(crate) fn tick_music(app: &AppHandle) {
    if !config::get().music_vibe_enabled {
        if app.borrow().music_vibing {
            app.borrow_mut().music_vibing = false;
            app.borrow().stage.set_music_vibe(false, 0.0);
        }
        return;
    }
    // Media and audio-session probes take tens to hundreds of milliseconds, so they run on a    // worker thread; only the resulting state change touches the UI.

    static PROBE_IN_FLIGHT: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    if PROBE_IN_FLIGHT.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    PROBE_IN_FLIGHT.store(true, std::sync::atomic::Ordering::SeqCst);
    let was = app.borrow().music_vibing;
    std::thread::spawn(move || {
        let media = crate::sysinfo::now_playing();
        let activity = crate::audio::audio_activity(50);
        let media_apps = [
            "spotify.exe",
            "chrome.exe",
            "msedge.exe",
            "firefox.exe",
            "brave.exe",
            "opera.exe",
            "vlc.exe",
        ];
        let media_peak = activity
            .apps
            .iter()
            .filter(|a| media_apps.contains(&a.name.to_lowercase().as_str()))
            .map(|a| a.peak)
            .fold(None, |acc: Option<f32>, p| {
                Some(acc.map_or(p, |m| m.max(p)))
            });
        let playing = media.is_playing || media_peak.is_some();
        // Session peaks for music sit around 0.2-0.6; map that onto the avatar's 0..1 groove.
        let energy = media_peak.map(|p| (p * 1.6).clamp(0.0, 1.0)).unwrap_or(0.6);
        PROBE_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
        post(move |app| apply_music_state(app, playing, was, energy));
    });
}

pub(crate) fn apply_music_state(app: &AppHandle, playing: bool, was: bool, energy: f32) {
    if playing {
        // Energy follows the music every probe, not only when playback starts.
        app.borrow().stage.set_music_vibe(true, energy);
    }
    if playing != was {
        app.borrow_mut().music_vibing = playing;
        if !playing {
            app.borrow().stage.set_music_vibe(false, 0.0);
        }
        app.borrow().ui.set_status_line(
            if playing {
                "Vibing to your music"
            } else {
                "Standing by"
            }
            .into(),
        );
    }
}

pub(crate) fn tick_boredom(app: &AppHandle) {
    let (idle, can) = {
        let a = app.borrow();
        (
            a.stationary_since.elapsed().as_secs_f64(),
            !a.dragging && a.target.is_none() && !a.busy && !a.game_compacted && a.roaming,
        )
    };
    if !can || idle < 210.0 {
        return;
    }
    app.borrow_mut().stationary_since = Instant::now();
    let key = config::current_persona();
    let line = match key.as_str() {
        "yandere" => {
            "I've been watching you from this spot for so long. Let me walk a little closer."
        }
        "tomboy" => {
            "Whew, my legs were getting stiff. Let's shake it off with a quick stretch walk."
        }
        _ => "Standing in the same spot forever is boring. I'm stretching my legs a bit.",
    };
    say(
        app,
        &config::flav(line),
        "walking",
        vec!["boredom walk".into()],
        true,
    );
    walk_to(app, "roam", "idle");
}

pub(crate) fn tick_ui(app: &AppHandle) {
    let (full, typed, shown) = {
        let a = app.borrow();
        (a.full_bubble_text.clone(), a.typed, a.ui.get_bubble_shown())
    };
    if shown && typed < full.chars().count() {
        let next = (typed + 3).min(full.chars().count());
        let visible: String = full.chars().take(next).collect();
        app.borrow().ui.set_bubble_text(visible.into());
        app.borrow_mut().typed = next;
        // The bubble wraps onto new lines as it types; grow the window with it.
        let slot = app.borrow().ui.get_bubble_slot_height();
        if (slot - app.borrow().bubble_slot).abs() > 0.5 {
            sync_chrome(app);
        }
    }
    let expired = app
        .borrow()
        .bubble_hide_at
        .map(|t| Instant::now() >= t)
        .unwrap_or(false);
    if expired {
        hide_bubble(app);
    }
    if app
        .borrow()
        .speaking_until
        .map(|t| Instant::now() >= t)
        .unwrap_or(false)
    {
        app.borrow_mut().speaking_until = None;
        app.borrow_mut().duplex_pending = false;
    }
    tick_hover(app);
    tick_clipboard(app);
    apply_persona(app);
}

/// How long the chat bar stays up with nothing going on.
pub(crate) const BAR_IDLE_HIDE: Duration = Duration::from_secs(8);

/// Hides the chat bar once it has sat idle for `BAR_IDLE_HIDE`.
///
/// The bar only appears on right-click (or Alt+Space). It counts as in use while the cursor is
/// over Aria, there is unsent text, she is thinking or listening, or its menu is open.
pub(crate) fn tick_hover(app: &AppHandle) {
    if !app.borrow().ui.get_bar_visible() || !config::get().capsule_autohide {
        return;
    }
    let (x, y) = app.borrow().pos;
    let (w, h) = window_size(app);
    let cursor = crate::input::cursor_position();
    let hovered = cursor.0 >= x && cursor.0 <= x + w && cursor.1 >= y && cursor.1 <= y + h;
    let in_use = {
        let a = app.borrow();
        hovered
            || a.dragging
            || a.busy
            || !a.ui.get_draft().is_empty()
            || crate::voice::is_recording()
            || a.menu.as_ref().is_some_and(|m| m.window().is_visible())
    };
    if in_use {
        app.borrow_mut().bar_activity = Instant::now();
    } else if app.borrow().bar_activity.elapsed() >= BAR_IDLE_HIDE {
        hide_bar(app);
    }
}

pub(crate) fn tick_clipboard(app: &AppHandle) {
    if !app.borrow().clipboard_watch || app.borrow().busy {
        return;
    }
    let Ok(mut clipboard) = arboard::Clipboard::new() else {
        return;
    };
    let Ok(text) = clipboard.get_text() else {
        return;
    };
    let text = text.trim().to_string();
    if text.len() < 25 || text.len() > 12000 {
        return;
    }
    let signature: String = text.chars().take(160).collect();
    if app.borrow().last_clipboard == signature {
        return;
    }
    let markers = [
        "Traceback (most recent call last)",
        "SyntaxError:",
        "TypeError:",
        "AttributeError:",
        "ModuleNotFoundError:",
        "ImportError:",
        "KeyError:",
        "ValueError:",
        "ZeroDivisionError:",
    ];
    if !markers.iter().any(|m| text.contains(m)) {
        return;
    }
    app.borrow_mut().last_clipboard = signature;
    std::thread::spawn(move || {
        let reply = crate::brain::analyze_clipboard(&text);
        post(move |app| say(app, &reply.text, &reply.emotion, reply.logs.clone(), true));
    });
}
