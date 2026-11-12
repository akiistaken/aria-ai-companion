//! Where Aria is: window geometry, walking, dragging, edge snap, peeking and the avatar slot.
//! The motion itself (gravity, walking, jumping) is [`physics`]; this module feeds it the desktop
//! and moves the windows to match.

use super::*;
use aria_protocol::BodyMode;

pub(crate) fn setup_window(app: &AppHandle) {
    // Note: this runs before the window is shown, when the winit window does not exist yet, so
    // this closure is normally skipped. Topmost is enforced with Win32 instead
    // (`ticks::apply_render_policy`), and frame/transparency in `main::setup_native_window`.
    app.borrow().ui.window().with_winit_window(|w| {
        w.set_window_level(winit::window::WindowLevel::AlwaysOnTop);
        w.set_decorations(false);
        w.set_resizable(false);
        w.set_transparent(true);
    });
    let scale = config::get().avatar_scale;
    let (_, _, aw, ah) = crate::win32::work_area();
    let sf = app.borrow().ui.window().scale_factor();
    let width = STAGE_BASE_W * scale + 24.0;
    let height = STAGE_BASE_H * scale + 54.0;
    let x = (aw - (width * sf) as i32 - 40).max(20);
    let y = (ah - (height * sf) as i32 - 20).max(20);
    app.borrow()
        .ui
        .window()
        .set_size(slint::LogicalSize::new(width, height));
    app.borrow()
        .ui
        .window()
        .set_position(PhysicalPosition::new(x, y));
    app.borrow_mut().pos = (x, y);
}

/// Recomputes the window height from the current chrome (bubble + bar).
///
/// The bubble's height comes from its laid-out text, so the window grows with long replies. It
/// grows **upward**: the window moves up by however much the bubble grew, which keeps the avatar
/// (and the command bar under it) still on screen.
pub(crate) fn sync_chrome(app: &AppHandle) {
    let scale = config::get().avatar_scale;
    let (bubble_slot, chrome, sf, previous) = {
        let a = app.borrow();
        a.ui.set_stage_w(STAGE_BASE_W * scale);
        a.ui.set_stage_h(STAGE_BASE_H * scale);
        (
            a.ui.get_bubble_slot_height(),
            a.ui.get_chrome_height(),
            a.ui.window().scale_factor(),
            a.bubble_slot,
        )
    };
    let width = STAGE_BASE_W * scale + 24.0;
    let height = STAGE_BASE_H * scale + bubble_slot + chrome + 10.0;
    app.borrow()
        .ui
        .window()
        .set_size(slint::LogicalSize::new(width, height));
    app.borrow_mut().bubble_slot = bubble_slot;
    let grew = ((bubble_slot - previous) * sf).round() as i32;
    let (x, y) = app.borrow().pos;
    // Always ends in `layout_windows`, which also re-cuts the stage hole for the new size.
    set_window_pos(app, x, y - grew);
}

/// Walks to a named spot. With gravity she stays on what she stands on, so only the horizontal
/// part of a location matters ("top_left" walks to the left like "bottom_left"), except `perch`,
/// which jumps up onto the active window.
pub(crate) fn walk_to(app: &AppHandle, location: &str, emotion: &str) {
    let loc = location.to_lowercase();
    let (feet_x, feet_y) = feet(app);
    let (sx, _, sw, _) = crate::win32::work_area_at(feet_x, feet_y - 10);
    let (w, _) = window_size(app);
    let margin = 25;
    let (left, right) = (sx + margin, (sx + sw - w - margin).max(sx + margin));
    let mut perch = None;
    let tx = match loc.as_str() {
        "left" | "top_left" | "bottom_left" => left,
        "right" | "top_right" | "bottom_right" => right,
        "center" => sx + (sw - w) / 2,
        "perch" => match perch_spot(app) {
            Some((x, hwnd)) => {
                perch = Some(hwnd);
                x
            }
            None => right,
        },
        _ => crate::util::rand_range(left, right),
    };
    {
        let mut a = app.borrow_mut();
        a.target = Some((tx, a.pos.1));
        a.perch_target = perch;
        a.post_walk_emotion = if emotion == "walking" {
            "idle".into()
        } else {
            emotion.to_string()
        };
        a.user_placed = false;
    }
    set_peeking(app, "none");
}

/// A spot on top of the active window to perch on: the window x for the mascot window, and the
/// window to stand on. `None` when there is no suitable window or no headroom above it.
pub(crate) fn perch_spot(app: &AppHandle) -> Option<(i32, isize)> {
    let win = crate::win32::active_window()?;
    let lower = win.title.to_lowercase();
    if lower.is_empty() || lower.contains("aria") || lower.contains("program manager") {
        return None;
    }
    let (left, top, right) = crate::win32::window_top_edge(win.hwnd)?;
    if right - left < 260 {
        return None;
    }
    let (sx, sy, sw, _) = crate::win32::work_area_at((left + right) / 2, top);
    let (_, _, _, avatar_h) = avatar_rect(app);
    // A maximized window's top is the top of the screen: no room to stand there.
    if top - avatar_h < sy {
        return None;
    }
    let (w, _) = window_size(app);
    // Near the window's right end, like sitting on the corner of a monitor.
    let tx = (right - w - 35).clamp(sx + 10, (sx + sw - w - 10).max(sx + 10));
    Some((tx, win.hwnd))
}

/// From the mascot window's top-left corner to the soles of her feet, in physical pixels.
pub(crate) fn feet_offset(app: &AppHandle) -> (f32, f32) {
    let (x, y) = app.borrow().pos;
    let (ax, ay, aw, ah) = avatar_rect(app);
    let ground = app.borrow().stage_geometry.map(|g| g.0).unwrap_or(0.0);
    ((ax - x) as f32 + aw as f32 * 0.5, (ay - y + ah) as f32 - ground)
}

/// Where her feet are on the desktop, in physical pixels.
pub(crate) fn feet(app: &AppHandle) -> (i32, i32) {
    let (dx, dy) = feet_offset(app);
    let (x, y) = app.borrow().pos;
    (x + dx.round() as i32, y + dy.round() as i32)
}

/// One physics tick: gravity, walking, jumping and landing, then the windows follow her feet.
pub(crate) fn step_body(app: &AppHandle, dt: f32) {
    let Some((ground_offset, ppm)) = app.borrow().stage_geometry else {
        return;
    };
    let (fdx, fdy) = feet_offset(app);
    let pos = app.borrow().pos;
    let actual = (pos.0 as f32 + fdx, pos.1 as f32 + fdy);

    if app.borrow().dragging {
        app.borrow_mut().body.hold(actual.0, actual.1, dt);
        app.borrow().stage.set_body(BodyMode::Held);
        return;
    }
    {
        // Moved by something else (summon, the game dodge, a new framing): carry on from there.
        let mut a = app.borrow_mut();
        if (a.body.x - actual.0).abs() > 1.5 || (a.body.y - actual.1).abs() > 1.5 {
            a.body.x = actual.0;
            a.body.y = actual.1;
        }
    }

    let (_, _, _, avatar_h) = avatar_rect(app);
    let height = avatar_h as f32 - ground_offset;
    let (sx, sy, sw, sh) =
        crate::win32::work_area_at(actual.0 as i32, (actual.1 - height * 0.5) as i32);
    let (perches, goal) = {
        let a = app.borrow();
        let mut perches = Vec::new();
        let mut candidates = vec![a.body.perch, a.perch_target];
        candidates.dedup();
        for hwnd in candidates.into_iter().flatten() {
            let Some((left, top, right)) = crate::win32::window_top_edge(hwnd) else { continue };
            // Only while there is room above it for her whole body.
            if top as f32 - height * 0.9 >= sy as f32 {
                perches.push(physics::Surface {
                    left: left as f32,
                    right: right as f32,
                    top: top as f32,
                    perch: Some(hwnd),
                });
            }
        }
        let goal = a.target.map(|(tx, _)| physics::Goal {
            x: tx as f32 + fdx,
            perch: a.perch_target,
        });
        (perches, goal)
    };
    let env = physics::Env {
        floor: (sy + sh) as f32,
        perches: &perches,
        walls: (sx as f32, (sx + sw) as f32),
        ceiling: sy as f32 + height * 0.85,
        ppm,
    };
    let pace = physics::Pace::for_persona(&config::current_persona());
    let events = app.borrow_mut().body.step(&env, goal, pace, dt);

    let (bx, by, mode) = {
        let b = &app.borrow().body;
        (b.x, b.y, b.mode)
    };
    let next = ((bx - fdx).round() as i32, (by - fdy).round() as i32);
    if next != pos {
        set_window_pos(app, next.0, next.1);
    }
    app.borrow().stage.set_body(mode);
    if goal.is_some() && app.borrow().ui.get_status_line() != "Walking" {
        app.borrow().ui.set_status_line("Walking".into());
    }

    for event in events {
        match event {
            physics::Event::Arrived => {
                let emotion = {
                    let mut a = app.borrow_mut();
                    a.target = None;
                    a.perch_target = None;
                    a.stationary_since = Instant::now();
                    a.last_emotion = a.post_walk_emotion.clone();
                    a.last_emotion.clone()
                };
                app.borrow().ui.set_status_line(status_for(&emotion).into());
            }
            physics::Event::Landed { speed } => {
                app.borrow_mut().stationary_since = Instant::now();
                if speed > physics::HARD_LANDING {
                    log::info!("hard landing at {speed:.1} m/s");
                }
                // Dropped near a screen edge: slide into the bezel peek.
                if app.borrow().target.is_none() {
                    check_edge_snap(app);
                }
            }
            physics::Event::Fell => {}
        }
    }
}

pub(crate) fn window_size(app: &AppHandle) -> (i32, i32) {
    let size = app.borrow().ui.window().size();
    (size.width as i32, size.height as i32)
}

/// Moves the whole body to `(x, y)`: the Slint chrome and the avatar window together.
///
/// This is the single funnel for every movement path (walking, dragging, edge snap, game dodge,
/// perch, boredom roam, initial placement), so the avatar can never drift away from its chrome.
pub(crate) fn set_window_pos(app: &AppHandle, x: i32, y: i32) {
    app.borrow_mut().pos = (x, y);
    // Both windows move in one `DeferWindowPos` batch, so the avatar and its chat chrome land in
    // the same frame. (Before, the chrome moved here and the avatar only when the renderer got
    // round to a socket message, up to ~15 ms later, so they visibly came apart while walking or
    // dragging.)
    let mascot = mascot_hwnd(&app.borrow().ui).unwrap_or(0);
    let avatar = app.borrow().stage.hwnd() as isize;
    let (ax, ay, _, _) = avatar_rect(app);
    let moved = mascot != 0
        && crate::win32::move_windows_together(&[(mascot, x, y), (avatar, ax, ay)]);
    if !moved {
        app.borrow()
            .ui
            .window()
            .set_position(PhysicalPosition::new(x, y));
    }
    layout_windows(app);
}

pub(crate) fn apply_scale(app: &AppHandle, scale: f32) {
    let scale = scale.clamp(0.5, 2.5);
    config::update(|s| s.avatar_scale = scale);
    sync_chrome(app);
    sync_studio(app);
}

pub(crate) fn set_peeking(app: &AppHandle, side: &str) {
    let side = match side {
        "left" | "right" => side,
        _ => "none",
    };
    app.borrow().ui.set_peeking(side != "none");
    app.borrow_mut().peeking = side.to_string();
    app.borrow().stage.set_peeking(side);
}

pub(crate) fn reset_pose(app: &AppHandle) {
    {
        let mut a = app.borrow_mut();
        a.target = None;
        a.perch_target = None;
        a.stationary_since = Instant::now();
    }
    set_peeking(app, "none");
    say(
        app,
        &config::flav("Back to standing by. What do you need?"),
        "idle",
        vec!["avatar reset to idle".into()],
        false,
    );
}

/// Positions the avatar window, speech bubble and command bar as one body.
///
/// Every movement path funnels through here, so the three windows can never drift apart.
pub(crate) fn layout_windows(app: &AppHandle) {
    let (x, y, w, h) = avatar_rect(app);
    let stage = app.borrow().stage.clone();
    // Size for the renderer (and the position, until its window is owned by ours).
    stage.set_window_rect(x, y, w, h);
    // Re-seat the avatar on its slot as well: the stage slot can shift inside the mascot window
    // (the bubble grows, the scale changes) without the mascot window itself moving.
    crate::win32::move_windows_together(&[(stage.hwnd() as isize, x, y)]);
    cut_stage_hole(app);
}

/// The avatar window's screen rectangle in physical pixels: exactly the mascot's stage slot.
pub(crate) fn avatar_rect(app: &AppHandle) -> (i32, i32, i32, i32) {
    let a = app.borrow();
    let sf = a.ui.window().scale_factor();
    let (x, y) = a.pos;
    (
        x + (a.ui.get_stage_x() * sf).round() as i32,
        y + (a.ui.get_stage_y() * sf).round() as i32,
        (a.ui.get_stage_w() * sf).round().max(1.0) as i32,
        (a.ui.get_stage_h() * sf).round().max(1.0) as i32,
    )
}

/// Removes the stage slot from the mascot window's hit region.
///
/// The Slint window is transparent but still a normal window, so without this it would swallow
/// every click over the avatar's empty space and click-through could never reach the desktop.
/// With the slot cut out, the avatar's own layered window decides per pixel.
pub(crate) fn cut_stage_hole(app: &AppHandle) {
    let a = app.borrow();
    let sf = a.ui.window().scale_factor();
    let size = a.ui.window().size();
    let hole = (
        (a.ui.get_stage_x() * sf).round() as i32,
        (a.ui.get_stage_y() * sf).round() as i32,
        (a.ui.get_stage_w() * sf).round() as i32,
        (a.ui.get_stage_h() * sf).round() as i32,
    );
    let bar_visible = a.ui.get_bar_visible();
    let chrome = (a.ui.get_chrome_height() * sf).round() as i32;
    let key = (size.width as i32, size.height as i32, hole, bar_visible);
    if a.region_key == Some(key) {
        return;
    }
    let Some(hwnd) = mascot_hwnd(&a.ui) else {
        return;
    };
    drop(a);
    // Empty strips must not swallow clicks meant for the desktop: the padding under the stage
    // always, and the bar's slot above her head while the bar is hidden.
    let below_stage = hole.1 + hole.3;
    let under = (0, below_stage, size.width as i32, size.height as i32 - below_stage);
    let bar_slot = (0, hole.1 - chrome, size.width as i32, chrome);
    let holes: &[(i32, i32, i32, i32)] = if bar_visible {
        &[hole, under]
    } else {
        &[hole, under, bar_slot]
    };
    crate::win32::set_region_with_holes(hwnd, size.width as i32, size.height as i32, holes);
    app.borrow_mut().region_key = Some(key);
}

/// The mascot window's Win32 handle.
pub(crate) fn mascot_hwnd(ui: &MascotWindow) -> Option<isize> {
    window_hwnd(ui.window())
}

/// A Slint window's Win32 handle, once it has a native window.
pub(crate) fn window_hwnd(window: &slint::Window) -> Option<isize> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
            _ => None,
        })
        .flatten()
}

/// Keeps a Slint window off the taskbar: Aria lives in the tray.
pub(crate) fn hide_from_taskbar(window: &slint::Window) {
    if let Some(hwnd) = window_hwnd(window) {
        crate::win32::hide_from_taskbar(hwnd);
    }
}

/// Picked up: physics hands her to the cursor until she is let go.
pub(crate) fn start_drag(app: &AppHandle) {
    let (pos, cursor) = (app.borrow().pos, crate::input::cursor_position());
    {
        let mut a = app.borrow_mut();
        a.dragging = true;
        a.drag_origin = Some((pos.0, pos.1, cursor.0, cursor.1));
        a.target = None;
        a.perch_target = None;
        a.user_placed = true;
    }
    set_peeking(app, "none");
}

pub(crate) fn update_drag(app: &AppHandle, cursor: (i32, i32)) {
    // Copy out first: a borrow in the `if let` scrutinee lives through the whole block (edition
    // 2021), so `set_window_pos`'s `borrow_mut` would panic with "already borrowed".
    let origin = app.borrow().drag_origin;
    if let Some((win_x, win_y, cur_x, cur_y)) = origin {
        set_window_pos(app, win_x + (cursor.0 - cur_x), win_y + (cursor.1 - cur_y));
    }
}

/// Let go: she keeps the cursor's momentum and physics takes over (fall, bounce, land).
pub(crate) fn end_drag(app: &AppHandle) {
    let ppm = app.borrow().stage_geometry.map(|g| g.1).unwrap_or(200.0);
    let mut a = app.borrow_mut();
    a.dragging = false;
    a.drag_origin = None;
    a.stationary_since = Instant::now();
    a.body.release(ppm);
}

/// Magnetic bezel snapping with the peeking lean: landing near a screen edge, she shuffles
/// half out of view and leans back in.
pub(crate) fn check_edge_snap(app: &AppHandle) {
    if !config::get().edge_snapping_enabled {
        return;
    }
    let (feet_x, feet_y) = feet(app);
    let (sx, _, sw, _) = crate::win32::work_area_at(feet_x, feet_y - 10);
    let (x, y) = app.borrow().pos;
    let (w, _) = window_size(app);

    const THRESHOLD: i32 = 55;
    let (target_x, side) = if (x - sx).abs() <= THRESHOLD {
        (sx - (w as f32 * 0.12) as i32, "left")
    } else if ((x + w) - (sx + sw)).abs() <= THRESHOLD {
        (sx + sw - (w as f32 * 0.88) as i32, "right")
    } else {
        (x, "none")
    };
    if target_x != x {
        let mut a = app.borrow_mut();
        a.target = Some((target_x, y));
        a.post_walk_emotion = a.last_emotion.clone();
    }
    set_peeking(app, side);
}
