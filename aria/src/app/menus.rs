//! Input surfaces: the mascot's buttons, global hotkeys, the context menu and the quick dial.

use super::*;

pub(crate) fn wire_callbacks(app: &AppHandle) {
    let weak = app.borrow().ui.as_weak();
    {
        let app = app.clone();
        if let Some(ui) = weak.upgrade() {
            ui.on_send_text(move |text| submit_prompt(&app, text.to_string(), false));
        }
    }
    {
        let app = app.clone();
        if let Some(ui) = weak.upgrade() {
            ui.on_toggle_vision(move || {
                let next = !app.borrow().ui.get_vision();
                app.borrow().ui.set_vision(next);
                say(
                    &app,
                    &config::flav(if next {
                        "Continuous screen vision is on — I can see your screen while we talk."
                    } else {
                        "I'll only look at your screen when you ask me to."
                    }),
                    if next { "annoyed" } else { "idle" },
                    vec![format!("screen vision {}", if next { "on" } else { "off" })],
                    false,
                );
            });
        }
    }
    {
        let app = app.clone();
        if let Some(ui) = weak.upgrade() {
            ui.on_toggle_mic(move || toggle_mic(&app));
        }
    }
    {
        let app = app.clone();
        if let Some(ui) = weak.upgrade() {
            ui.on_toggle_tts(move || {
                let next = !config::get().tts_enabled;
                config::update(|s| s.tts_enabled = next);
                app.borrow().ui.set_tts(next);
                if !next {
                    crate::voice::stop_speaking();
                }
                say(
                    &app,
                    &config::flav(if next {
                        "My voice is back on."
                    } else {
                        "You muted my voice — I'll use text for now."
                    }),
                    if next { "talking" } else { "annoyed" },
                    vec![],
                    false,
                );
                sync_studio(&app);
            });
        }
    }
    {
        let app = app.clone();
        if let Some(ui) = weak.upgrade() {
            ui.on_toggle_subtitles(move || {
                let next = !config::get().subtitles_enabled;
                config::update(|s| s.subtitles_enabled = next);
                app.borrow().ui.set_subtitles(next);
                if !next {
                    hide_bubble(&app);
                }
                sync_chrome(&app);
                sync_studio(&app);
            });
        }
    }
    {
        let app = app.clone();
        if let Some(ui) = weak.upgrade() {
            ui.on_open_studio(move || open_studio(&app));
        }
    }
    {
        let app = app.clone();
        if let Some(ui) = weak.upgrade() {
            ui.on_collapse_bar(move || hide_bar(&app));
        }
    }
    {
        let app = app.clone();
        if let Some(ui) = weak.upgrade() {
            ui.on_expand_bar(move || show_bar(&app));
        }
    }
    {
        let app = app.clone();
        if let Some(ui) = weak.upgrade() {
            // The ⋯ button: the menu right-click used to open.
            ui.on_open_menu(move || {
                // Open just above the bar (which sits above her head), so the menu never covers
                // the bar or hides behind it.
                let cursor = crate::input::cursor_position();
                let (_, y) = app.borrow().pos;
                let bar_top = {
                    let a = app.borrow();
                    let sf = a.ui.window().scale_factor();
                    y + ((a.ui.get_stage_y() - a.ui.get_chrome_height()) * sf).round() as i32
                };
                let sf = app.borrow().ui.window().scale_factor();
                open_context_menu(&app, (cursor.0, bar_top - (6.0 * sf).round() as i32));
            });
        }
    }
    {
        let app = app.clone();
        if let Some(ui) = weak.upgrade() {
            ui.on_open_quick_dial(move || open_quick_dial(&app));
        }
    }
}

/// Global hotkeys: Alt+Space summons, Ctrl+Shift+M toggles the mic, Ctrl+Shift+C opens the studio.
pub(crate) fn install_hotkeys() {
    std::thread::spawn(move || {
        use global_hotkey::hotkey::HotKey;

        use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

        use std::str::FromStr;
        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => m,
            Err(e) => {
                log::warn!("global hotkeys unavailable: {e}");
                return;
            }
        };
        let mut registered = [0u32; 3];
        for (index, combo) in ["Alt+Space", "Control+Shift+M", "Control+Shift+C"]
            .iter()
            .enumerate()
        {
            match HotKey::from_str(combo) {
                Ok(hotkey) => {
                    registered[index] = hotkey.id();
                    if let Err(e) = manager.register(hotkey) {
                        log::warn!("hotkey {combo} registration failed: {e}");
                        registered[index] = 0;
                    }
                }
                Err(e) => log::warn!("hotkey {combo} parse failed: {e}"),
            }
        }
        let receiver = GlobalHotKeyEvent::receiver();
        while let Ok(event) = receiver.recv() {
            if event.state != HotKeyState::Pressed {
                continue;
            }
            let action = if event.id == registered[0] && registered[0] != 0 {
                0
            } else if event.id == registered[1] && registered[1] != 0 {
                1
            } else if event.id == registered[2] && registered[2] != 0 {
                2
            } else {
                continue;
            };
            post(move |app| match action {
                0 => summon(app),
                1 => toggle_mic(app),
                _ => open_studio(app),
            });
        }
    });
}

/// Shows the chat bar (right-click on the avatar, Alt+Space) and restarts its idle timer.
pub(crate) fn show_bar(app: &AppHandle) {
    app.borrow_mut().bar_activity = Instant::now();
    if !app.borrow().ui.get_bar_visible() {
        app.borrow().ui.set_bar_visible(true);
        sync_chrome(app);
    }
}

pub(crate) fn hide_bar(app: &AppHandle) {
    if app.borrow().ui.get_bar_visible() {
        app.borrow().ui.set_bar_visible(false);
        sync_chrome(app);
    }
}

pub(crate) fn summon(app: &AppHandle) {
    let _ = app.borrow().ui.window().show();
    let pos = app.borrow().pos;
    let _ = app
        .borrow()
        .ui
        .window()
        .set_position(PhysicalPosition::new(pos.0, pos.1));
    show_bar(app);
    say(
        app,
        &config::flav("You summoned me with Alt+Space. Tell me what you need."),
        "talking",
        vec!["summoned via global hotkey".into()],
        true,
    );
}

pub(crate) fn open_studio(app: &AppHandle) {
    if let Some(studio) = app.borrow().studio.as_ref() {
        let _ = studio.window().show();
        sync_studio(app);
        return;
    }
    let studio = match ControlCenter::new() {
        Ok(s) => s,
        Err(e) => {
            log::error!("control center failed: {e}");
            return;
        }
    };
    wire_studio(&studio);
    studio.show().ok();
    app.borrow_mut().studio = Some(studio);
    sync_studio(app);
}

pub(crate) fn open_quick_dial(app: &AppHandle) {
    if let Some(dial) = app.borrow().dial.as_ref() {
        let _ = dial.window().show();
        hide_from_taskbar(dial.window());
        return;
    }
    let dial = match QuickDial::new() {
        Ok(d) => d,
        Err(e) => {
            log::error!("quick dial failed: {e}");
            return;
        }
    };
    wire_dial(&dial);
    let (cx, cy) = crate::input::cursor_position();
    let _ = dial
        .window()
        .set_position(PhysicalPosition::new(cx - 130, cy - 130));
    dial.show().ok();
    hide_from_taskbar(dial.window());
    app.borrow_mut().dial = Some(dial);
}

/// Places the menu inside the work area. It opens from the chat bar's ⋯ button with `at` just
/// above the bar, so it opens upward (above `at`) and only drops below when there is no room
/// above; it grows to the right of `at` unless that would leave the screen.
fn place_menu(menu: &ContextMenu, at: (i32, i32)) {
    let (sx, sy, sw, sh) = crate::win32::work_area();
    let size = menu.window().size();
    let (w, h) = (size.width as i32, size.height as i32);
    let x = if at.0 + w <= sx + sw { at.0 } else { at.0 - w };
    let y = if at.1 - h >= sy { at.1 - h } else { at.1 };
    let x = x.clamp(sx, (sx + sw - w).max(sx));
    let y = y.clamp(sy, (sy + sh - h).max(sy));
    let _ = menu.window().set_position(PhysicalPosition::new(x, y));
}

pub(crate) fn open_context_menu(app: &AppHandle, at: (i32, i32)) {
    if let Some(menu) = app.borrow().menu.as_ref() {
        let _ = menu.window().show();
        hide_from_taskbar(menu.window());
        place_menu(menu, at);
        return;
    }
    let menu = match ContextMenu::new() {
        Ok(m) => m,
        Err(e) => {
            log::error!("context menu failed: {e}");
            return;
        }
    };
    wire_menu(&menu);
    let _ = menu
        .window()
        .set_position(PhysicalPosition::new(at.0.max(0), at.1.max(0)));
    menu.show().ok();
    hide_from_taskbar(menu.window());
    // Its size is only known once shown.
    place_menu(&menu, at);
    app.borrow_mut().menu = Some(menu);
    apply_persona(app);
}

pub(crate) fn hide_window_of<T: ComponentHandle>(weak: &slint::Weak<T>) {
    if let Some(component) = weak.upgrade() {
        let _ = component.window().hide();
    }
}

pub(crate) fn wire_menu(menu: &ContextMenu) {
    let weak = menu.as_weak();
    {
        let weak = weak.clone();
        menu.on_summon(move || {
            hide_window_of(&weak);
            post(summon);
        });
    }
    {
        let weak = weak.clone();
        menu.on_toggle_chatbox(move || {
            hide_window_of(&weak);
            post(|app| {
                if app.borrow().ui.get_bar_visible() {
                    hide_bar(app);
                } else {
                    show_bar(app);
                }
            });
        });
    }
    {
        let weak = weak.clone();
        menu.on_open_studio(move || {
            hide_window_of(&weak);
            post(open_studio);
        });
    }
    {
        let weak = weak.clone();
        menu.on_quick_dial(move || {
            hide_window_of(&weak);
            post(open_quick_dial);
        });
    }
    {
        let weak = weak.clone();
        menu.on_open_vault(move || {
            hide_window_of(&weak);
            let facts = crate::memory::facts();
            let body = if facts.is_empty() {
                "Aria's Memory Vault is empty. She learns facts from conversation.\n".to_string()
            } else {
                let mut lines = vec![
                    "ARIA — MEMORY VAULT".to_string(),
                    format!("{} remembered facts\n", facts.len()),
                ];
                for (i, fact) in facts.iter().enumerate() {
                    lines.push(format!(
                        "[{:02}] [{}] {}\n    ({})",
                        i + 1,
                        fact.category.to_uppercase(),
                        fact.fact,
                        fact.added_at
                    ));
                }
                lines.join("\n")
            };
            let path = config::data_dir().join("memory_vault.txt");
            let _ = std::fs::write(&path, body);
            let _ = std::process::Command::new("notepad.exe").arg(&path).spawn();
        });
    }
    {
        let weak = weak.clone();
        menu.on_set_key(move || {
            hide_window_of(&weak);
            post(open_studio);
        });
    }
    {
        let weak = weak.clone();
        menu.on_toggle_mic_mode(move || {
            hide_window_of(&weak);
            post(|app| {
                let next = if config::get().mic_mode == "always_open" {
                    "click"
                } else {
                    "always_open"
                };
                config::update(|s| s.mic_mode = next.to_string());
                sync_studio(app);
            });
        });
    }
    {
        let weak = weak.clone();
        menu.on_reset_pose(move || {
            hide_window_of(&weak);
            post(reset_pose);
        });
    }
    {
        let weak = weak.clone();
        menu.on_open_log(move || {
            hide_window_of(&weak);
            crate::logger::open_log_file();
        });
    }
    menu.on_quit(move || {
        crate::tray::remove();
        let _ = slint::quit_event_loop();
        std::process::exit(0);
    });
}

pub(crate) fn wire_dial(dial: &QuickDial) {
    let weak = dial.as_weak();
    {
        let weak = weak.clone();
        dial.on_headpat(move || {
            hide_window_of(&weak);
            post(headpat);
        });
    }
    {
        let weak = weak.clone();
        dial.on_coffee(move || {
            hide_window_of(&weak);
            post(|app| {
                submit_prompt(
                    app,
                    "Let's take a quick coffee or tea break together!".into(),
                    false,
                )
            });
        });
    }
    {
        let weak = weak.clone();
        dial.on_dance(move || {
            hide_window_of(&weak);
            post(|app| {
                let stat = crate::memory::adjust_emotion("hype", 5);
                crate::memory::adjust_emotion("groove", 3);
                say(
                    app,
                    &config::flav("Watch closely — spinning right here on your desktop."),
                    "dance",
                    vec![format!("dance, hype level {}", stat.level)],
                    true,
                );
                sync_studio(app);
            });
        });
    }
    {
        let weak = weak.clone();
        dial.on_media(move || {
            hide_window_of(&weak);
            post(|app| submit_prompt(app, "play pause music".into(), false));
        });
    }
    {
        let weak = weak.clone();
        dial.on_mic(move || {
            hide_window_of(&weak);
            post(toggle_mic);
        });
    }
    {
        let weak = weak.clone();
        dial.on_studio(move || {
            hide_window_of(&weak);
            post(open_studio);
        });
    }
    dial.on_dismissed(move || {
        hide_window_of(&weak);
    });
}
