//! Timers: motion, proactive events, game dodge, music vibe, boredom, UI typing and the render policy.

use super::*;

/// Registers a repeating timer owned by the app, so dropping the app stops every timer.
pub(crate) fn own_timer(app: AppHandle, interval: Duration, f: impl Fn() + 'static) {
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, interval, f);
    app.borrow_mut().timers.push(timer);
}

/// The body tick. 60 Hz: the window is moved once per tick, and at 30 Hz a walk across the
/// desktop visibly stepped, however smooth the animation inside the window was.
pub(crate) const MOTION_TICK: Duration = Duration::from_millis(16);

pub(crate) fn start_timers(app: &AppHandle) {
    {
        let app = app.clone();
        own_timer(app.clone(), MOTION_TICK, move || {
            let _ = try_with_app_mut(|_| tick_motion(&app));
        });
    }
    {
        let app = app.clone();
        own_timer(app.clone(), Duration::from_secs(5), move || tick_idle_events(&app));
    }
    {
        let app = app.clone();
        own_timer(app.clone(), Duration::from_millis(6000), move || {
            tick_proactive(&app)
        });
    }
    {
        let app = app.clone();
        own_timer(app.clone(), Duration::from_millis(1000), move || tick_music(&app));
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
                crate::discord::update("", "", &crate::persona::current_key());
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
        crate::desktop::win32::trim_working_set();
    });
}

/// The body tick: physics, where she looks, the cursor for the gestures, and voice.
pub(crate) fn tick_motion(app: &AppHandle) {
    let dt = {
        let mut a = app.borrow_mut();
        let now = Instant::now();
        let dt = now.duration_since(a.last_motion).as_secs_f32().min(0.1);
        a.last_motion = now;
        dt
    };
    step_body(app, dt);
    let cursor = crate::desktop::input::cursor_position();
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
        let (left, right, middle) = crate::desktop::input::mouse_buttons();
        app.borrow().stage.set_pointer(x, y, left, right, middle);
    }
    // `talking` follows the audio that is actually playing, so the mouth and the talking
    // gestures can never run on after the voice stops (or without a voice at all).
    let speaking = crate::voice::is_speaking();
    let a = app.borrow();
    let moving = a.target.is_some() || a.body.is_moving();
    a.stage.set_state(shown_emotion(&a.last_emotion, moving), speaking);
    let (aa, oh) = if speaking { crate::voice::mouth_levels() } else { (0.0, 0.0) };
    a.stage.set_mouth(aa, oh);
}

/// The emotion to act out: sitting, dozing and dancing do not survive being on the move.
fn shown_emotion(emotion: &str, moving: bool) -> &str {
    if moving && matches!(emotion, "sit" | "sleepy" | "dance" | "walking") {
        "idle"
    } else {
        emotion
    }
}

/// Render policy, re-evaluated every second: pause while the session is locked or the mascot is
/// not on screen, 10 FPS while a game has the foreground, 30 otherwise.
pub(crate) fn apply_render_policy(app: &AppHandle) {
    // Keep the mascot window (bubble + chat bar) in the topmost band. The avatar window is owned
    // by it and sits in that band too; if the mascot ever dropped out, other apps would cover the
    // chat chrome while the avatar stayed on top, and the two would look unrelated.
    if let Some(mascot) = mascot_hwnd(&app.borrow().ui) {
        if crate::desktop::win32::ensure_topmost(mascot) {
            log::info!("mascot window was not topmost; restored");
        }
    }
    let hidden = !app.borrow().ui.window().is_visible() || crate::desktop::win32::session_locked();
    let fps = if hidden {
        0
    } else if app.borrow().game_compacted {
        10
    } else {
        30
    };
    app.borrow().stage.set_fps(fps);
}

/// Runs `work` on a worker unless the previous run behind `flag` is still going.
fn once_at_a_time(flag: &'static std::sync::atomic::AtomicBool, work: impl FnOnce() + Send + 'static) {
    use std::sync::atomic::Ordering;
    if flag.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        work();
        flag.store(false, Ordering::SeqCst);
    });
}

/// The guardian samples windows, WASAPI sessions and sysinfo, and may ask the AI for her line;
/// none of that may run on the UI thread, so it all happens on a worker and only results come
/// back.
pub(crate) fn tick_proactive(app: &AppHandle) {
    static IN_FLIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let occupied = { let a = app.borrow(); a.busy || a.dragging } || crate::voice::is_recording();
    once_at_a_time(&IN_FLIGHT, move || {
        crate::proactive::observe_mood();
        if occupied {
            return;
        }
        let game = crate::desktop::presence::detect_game();
        let event = crate::proactive::check_for_event().map(|mut event| {
            // Her own words for it, when the endpoint answers; the script otherwise.
            if let Some((text, emotion)) = crate::ai::tasks::proactive_line(&event.situation, &event.emotion) {
                event.message = text;
                event.emotion = emotion;
            }
            event
        });
        post(move |app| apply_proactive(app, game, event));
    });
}

pub(crate) fn apply_proactive(
    app: &AppHandle,
    game: Option<crate::desktop::presence::Game>,
    event: Option<crate::proactive::ProactiveEvent>,
) {
    if let Some(change) = crate::persona::mood::consume_change() {
        log::info!("mood shift {} -> {} ({})", change.old, change.new, change.reason);
        apply_persona(app);
        if config::get().voice_follows_mood {
            let target = crate::persona::get(&change.new).voice;
            config::update(|s| s.tts_voice = target.to_string());
        }
    }
    apply_game_state(app, game);
    if app.borrow().busy || app.borrow().dragging || crate::voice::is_recording() {
        return;
    }
    if let Some(event) = event {
        if let Some(location) = &event.walk_location {
            // App and music events hop up onto the active window, when there is one to stand on.
            let perch = location == "perch" || event.category.starts_with("app_") || event.category.contains("music");
            if !perch {
                walk_to(app, location, &event.emotion);
            } else if perch_spot(app).is_some() {
                walk_to(app, "perch", &event.emotion);
            }
        }
        say(app, &event.message, &event.emotion, vec![event.badge], true);
    }
}

/// Game-dodge state machine, fed by the worker's detection.
pub(crate) fn apply_game_state(app: &AppHandle, game: Option<crate::desktop::presence::Game>) {
    let settings = config::get();
    if !settings.smart_gaming_dodge_enabled && !settings.automatic_game_copilot_enabled {
        return;
    }
    let compacted = app.borrow().game_compacted;
    match game {
        Some(game) if !compacted => {
            {
                let mut a = app.borrow_mut();
                a.game_compacted = true;
                a.pre_game_scale = settings.avatar_scale;
                a.pre_game_pos = Some(a.pos);
                a.last_copilot_vision = Instant::now();
            }
            apply_scale(app, 0.52);
            let (sx, sy, sw, sh) = crate::desktop::win32::work_area();
            let (w, h) = window_size(app);
            set_window_pos(app, sx + sw - (w as f32 * 0.86) as i32, sy + sh - h - 8);
            set_peeking(app, "right");
            log::info!("game detected: {} ({})", game.name, game.signal);
            say(
                app,
                &crate::persona::flav(&format!("Game detected ({}). I'll cheer you on and watch your back from the corner.", game.name)),
                "talking",
                vec![format!("game copilot active ({})", game.name)],
                true,
            );
        }
        Some(game) if settings.automatic_game_copilot_enabled => {
            let due = app.borrow().last_copilot_vision.elapsed().as_secs() >= 85 && !app.borrow().busy;
            if due {
                app.borrow_mut().last_copilot_vision = Instant::now();
                let prompt = format!(
                    "You are Aria acting as an Automatic Live Game Co-Pilot for {}. Look at this live screenshot right \
                     now. In your active character persona, give a punchy 1-sentence tactical observation, clutch \
                     callout, warning about health, ammo or enemies, or enthusiastic encouragement. Keep it under 25 words.",
                    game.name
                );
                submit_prompt(app, prompt, true);
            }
        }
        None if compacted => {
            app.borrow_mut().game_compacted = false;
            set_peeking(app, "none");
            let scale = app.borrow().pre_game_scale;
            apply_scale(app, scale);
            let pre_game_pos = app.borrow().pre_game_pos;
            if let Some(pos) = pre_game_pos {
                app.borrow_mut().target = Some(pos);
            }
            say(app, &crate::persona::flav("Back to your desktop. Restored to full size."), "idle", vec!["desktop restored".into()], false);
        }
        _ => {}
    }
}

/// Music: what is playing (system media sessions) and its beat (the beat tracker), forwarded to
/// the renderer so she grooves and dances in time.
pub(crate) fn tick_music(app: &AppHandle) {
    static IN_FLIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !config::get().music_vibe_enabled {
        crate::desktop::beat::listen(false, "");
        if app.borrow().music_vibing {
            app.borrow_mut().music_vibing = false;
            app.borrow().stage.set_music_vibe(false, 0.0, 0.0, 0.0);
        }
        return;
    }
    once_at_a_time(&IN_FLIGHT, || {
        let playing = crate::desktop::media::now_playing();
        // The tracker samples the music app's own meter, so Aria's voice is never the beat.
        crate::desktop::beat::listen(playing.is_playing, &playing.exe);
        let beat = crate::desktop::beat::latest();
        post(move |app| apply_music_state(app, playing, beat));
    });
}

pub(crate) fn apply_music_state(
    app: &AppHandle,
    playing: crate::desktop::media::NowPlaying,
    beat: Option<crate::desktop::beat::Music>,
) {
    let was = app.borrow().music_vibing;
    let on = playing.is_playing;
    if on {
        // The beat phase is measured "now"; the renderer keeps its own clock running from it.
        let (energy, bpm, phase) = match beat {
            Some(b) if b.confidence > 0.0 => (b.energy.max(0.25), b.bpm, b.phase_at(Instant::now())),
            Some(b) => (b.energy.max(0.25), 0.0, 0.0),
            None => (0.6, 0.0, 0.0),
        };
        app.borrow().stage.set_music_vibe(true, energy, bpm, phase);
    }
    if on != was {
        app.borrow_mut().music_vibing = on;
        if on {
            log::info!("music: {} on {}", playing.label(), playing.source);
        } else {
            app.borrow().stage.set_music_vibe(false, 0.0, 0.0, 0.0);
        }
        app.borrow().ui.set_status_line(if on { "Vibing to your music" } else { "Standing by" }.into());
    }
}

pub(crate) fn tick_boredom(app: &AppHandle) {
    let (idle, can) = {
        let a = app.borrow();
        (a.stationary_since.elapsed().as_secs_f64(), !a.dragging && a.target.is_none() && !a.busy && !a.game_compacted && a.roaming)
    };
    if !can || idle < 210.0 {
        return;
    }
    app.borrow_mut().stationary_since = Instant::now();
    let line = match crate::persona::current_key().as_str() {
        "yandere" => "I've been watching you from this spot for so long. Let me walk a little closer.",
        "tomboy" => "Whew, my legs were getting stiff. Let's shake it off with a quick stretch walk.",
        _ => "Standing in the same spot forever is boring. I'm stretching my legs a bit.",
    };
    say(app, &crate::persona::flav(line), "idle", vec!["boredom walk".into()], true);
    walk_to(app, "roam", "idle");
}

/// Idle moments: when nothing has happened for a while, she does something on her own (stretch,
/// twirl, hum, sit down, …). The AI picks it to suit her mood and what the user is doing; without
/// the endpoint a persona-typical gesture stands in. The pause between moments depends on how
/// lively the persona is.
pub(crate) fn tick_idle_events(app: &AppHandle) {
    static IN_FLIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !config::get().idle_events_enabled {
        return;
    }
    let ready = {
        let a = app.borrow();
        Instant::now() >= a.next_idle_event
            && !a.busy
            && !a.dragging
            && a.target.is_none()
            && !a.body.is_moving()
            && !a.game_compacted
            && !a.music_vibing
            && !a.ui.get_bubble_shown()
            && !matches!(a.last_emotion.as_str(), "sit" | "sleepy")
    } && !crate::voice::is_speaking()
        && !crate::voice::is_recording();
    if !ready {
        return;
    }
    let persona = crate::persona::current_key();
    let pause = idle_pause(&persona);
    app.borrow_mut().next_idle_event = Instant::now() + pause;
    let recent = app.borrow().recent_idle.clone();
    once_at_a_time(&IN_FLIGHT, move || {
        let activity = describe_activity();
        let event = crate::ai::tasks::idle_event(&activity, &recent).unwrap_or_else(|| crate::ai::tasks::idle_fallback(&persona));
        post(move |app| apply_idle_event(app, event));
    });
}

/// Seconds between idle moments: lively personas fidget more.
fn idle_pause(persona: &str) -> Duration {
    let (lo, hi) = match persona {
        "genki" | "bakadere" => (55, 120),
        "tomboy" | "yandere" => (75, 160),
        "kuudere" | "meganekko" => (120, 260),
        _ => (90, 200),
    };
    Duration::from_secs(crate::util::rand_range(lo, hi) as u64)
}

/// What the user is up to, in a few words, for the idle-event prompt.
fn describe_activity() -> String {
    if crate::desktop::presence::is_away(240.0) {
        return "away from the PC".into();
    }
    let window = crate::desktop::system::system_stats().active_window;
    match crate::persona::mood::classify_window(&window) {
        "work" => format!("working ({})", crate::util::truncate(&window, 40)),
        "study" => format!("reading ({})", crate::util::truncate(&window, 40)),
        "attention" => format!("browsing ({})", crate::util::truncate(&window, 40)),
        _ if window == "Desktop" => "looking at the desktop".into(),
        _ => format!("using {}", crate::util::truncate(&window, 40)),
    }
}

fn apply_idle_event(app: &AppHandle, event: crate::ai::tasks::IdleEvent) {
    // Something may have started while the AI was choosing.
    if app.borrow().busy || app.borrow().ui.get_bubble_shown() || app.borrow().dragging {
        return;
    }
    log::info!("idle moment: {} (walk: {}, line: {:?})", event.action, event.walk, event.line);
    {
        let mut a = app.borrow_mut();
        a.recent_idle.push(event.action.clone());
        if a.recent_idle.len() > 4 {
            a.recent_idle.remove(0);
        }
    }
    if event.walk != "none" {
        walk_to(app, &event.walk, &event.action);
    }
    if event.line.is_empty() {
        // A silent moment: the gesture alone, fading back to idle like any reaction.
        let mut a = app.borrow_mut();
        a.last_emotion = event.action.clone();
        a.bubble_hide_at = Some(Instant::now() + Duration::from_secs(6));
        a.stage.set_state(&event.action, false);
        a.ui.set_status_line(status_for(&event.action).into());
    } else {
        say(app, &event.line, &event.action, vec![format!("idle moment: {}", event.action)], true);
    }
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
    let cursor = crate::desktop::input::cursor_position();
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
        let reply = crate::ai::tasks::analyze_clipboard(&text);
        post(move |app| say(app, &reply.text, &reply.emotion, reply.logs.clone(), true));
    });
}
