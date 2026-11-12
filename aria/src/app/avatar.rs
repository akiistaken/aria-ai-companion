//! The avatar renderer: starting it, its events, its settings and the touch reactions.

use super::*;

pub(crate) fn headpat(app: &AppHandle) {
    crate::persona::mood::note_event("headpat");
    let stat = crate::memory::adjust_emotion("dere", 5);
    crate::memory::adjust_emotion("devotion", 2);
    app.borrow().stage.trigger_headpat();
    let line = crate::persona::reactions::pick(&crate::persona::current_key(), "headpat", 0.35);
    say(
        app,
        &line,
        "happy",
        vec![format!(
            "headpat, dere level {} ({})",
            stat.level, stat.tier
        )],
        true,
    );
    sync_studio(app);
}

pub(crate) fn poke(app: &AppHandle) {
    crate::persona::mood::note_event("poke");
    let stat = crate::memory::adjust_emotion("tsun", 5);
    let line = crate::persona::reactions::pick(&crate::persona::current_key(), "poke", 0.32);
    say(
        app,
        &line,
        "annoyed",
        vec![format!("poked, tsun level {} ({})", stat.level, stat.tier)],
        true,
    );
    sync_studio(app);
}

pub(crate) fn shake(app: &AppHandle) {
    crate::persona::mood::note_event("shake");
    crate::memory::adjust_emotion("tsun", 4);
    let stat = crate::memory::adjust_emotion("hype", 3);
    let line = crate::persona::reactions::pick(&crate::persona::current_key(), "shake", 0.32);
    say(
        app,
        &line,
        "dizzy",
        vec![format!("shaken, hype level {}", stat.level)],
        true,
    );
    sync_studio(app);
}

/// Starts the avatar renderer process and forwards its events to the UI thread.
pub(crate) fn attach_stage(app: &AppHandle) {
    let settings = config::get();
    let model = resolve_avatar_model(&settings);
    match crate::avatar_link::AvatarLink::start(model, move |event| {
        post(move |app| on_avatar_event(app, event));
    }) {
        Ok(link) => {
            app.borrow_mut().stage = link;
        }
        Err(e) => log::error!("avatar renderer could not start: {e}"),
    }
}

/// The VRM file to load: the user's custom model when it exists, else the bundled one.
pub(crate) fn resolve_avatar_model(settings: &config::Settings) -> String {
    let custom = settings.custom_vrm_path.trim();
    if !custom.is_empty() && std::path::Path::new(custom).is_file() {
        return custom.to_string();
    }
    config::assets_dir()
        .join("models")
        .join("aria_default.vrm")
        .to_string_lossy()
        .to_string()
}

/// Handles one event from the avatar renderer.
pub(crate) fn on_avatar_event(app: &AppHandle, event: aria_protocol::Event) {
    use aria_protocol::Event;
    match event {
        Event::Hello { .. } => log::debug!("avatar hello"),
        Event::Ready { hwnd, adapter } => {
            log::info!("avatar renderer ready (hwnd={hwnd}, adapter={adapter})");
            // A (re)started renderer knows nothing: forget what the old one was sent and push
            // the whole state again.
            let stage = app.borrow().stage.clone();
            stage.reset_sent_state();
            app.borrow_mut().region_key = None;
            apply_avatar_settings(app);
            {
                let a = app.borrow();
                stage.set_state(&a.last_emotion, crate::voice::is_speaking());
                stage.set_body(a.body.mode);
                stage.set_mood(&a.persona);
                stage.set_peeking(&a.peeking);
                stage.set_music_vibe(a.music_vibing, 0.6, 0.0, 0.0);
            }
            apply_render_policy(app);
            layout_windows(app);
            // Tie the avatar window to the mascot window: one z-order band, hidden together.
            if let Some(owner) = mascot_hwnd(&app.borrow().ui) {
                stage.set_owner(owner);
            }
        }
        Event::ModelLoaded { ok, error, .. } => {
            if ok {
                log::info!("avatar model loaded");
            } else {
                log::error!("avatar model failed: {}", error.unwrap_or_default());
            }
        }
        Event::DragStart => start_drag(app),
        Event::DragMove => {
            if !app.borrow().dragging {
                start_drag(app);
            }
            let cursor = crate::desktop::input::cursor_position();
            update_drag(app, cursor);
        }
        Event::DragEnd => end_drag(app),
        // One click brings back what she last said.
        Event::Click => show_last_chat(app),
        Event::Poke => poke(app),
        Event::Shake => shake(app),
        Event::Headpat => headpat(app),
        Event::MiddleClick => open_quick_dial(app),
        // Right-click on the model shows the chat bar; its ⋯ button opens the menu.
        Event::ContextMenu => show_bar(app),
        Event::HoverChanged { over } => {
            // Over the model the window must accept clicks; over empty space it must not, so the
            // desktop underneath stays usable.
            app.borrow().stage.set_click_through(!over);
            app.borrow_mut().over_model = over;
        }
        // Where her feet are in the window and how big she is: physics stands her on the taskbar
        // with this, and scales speeds and gravity to her size.
        Event::Stage {
            ground_offset,
            pixels_per_metre,
        } => {
            log::info!("avatar stage: feet {ground_offset:.0} px above the slot bottom, {pixels_per_metre:.0} px/m");
            app.borrow_mut().stage_geometry = Some((ground_offset, pixels_per_metre.max(1.0)));
        }
        Event::Error { msg } => log::error!("avatar error: {msg}"),
    }
}

/// Pushes the avatar-related settings to the renderer.
///
/// Called after any setting that changes how the avatar looks or is framed.
pub(crate) fn apply_avatar_settings(app: &AppHandle) {
    let settings = config::get();
    let stage = app.borrow().stage.clone();
    stage.set_camera_framing(
        &settings.body_framing,
        settings.camera_zoom,
        settings.camera_y_offset,
    );
    stage.set_lighting(&settings.stage_lighting);
    let props = settings
        .avatar_props
        .iter()
        .filter_map(|(k, v)| v.as_bool().map(|on| (k.clone(), on)))
        .collect();
    stage.set_props(props);
}
