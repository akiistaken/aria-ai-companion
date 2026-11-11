//! Where Aria is: window geometry, walking, dragging, edge snap, peeking and the avatar slot.

use super::*;

pub(crate) fn setup_window(app: &AppHandle) {
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

/// Named desktop positions, plus perching on the active window.
pub(crate) fn walk_to(app: &AppHandle, location: &str, emotion: &str) {
    let loc = location.to_lowercase();
    let (sx, sy, sw, sh) = crate::win32::work_area();
    let (w, h) = window_size(app);
    let margin = 25;
    let (_, cy) = app.borrow().pos;
    let (tx, ty) = match loc.as_str() {
        "left" => (sx + margin, cy),
        "right" => (sx + sw - w - margin, cy),
        "center" => (sx + (sw - w) / 2, sy + (sh - h) / 2),
        "top_left" => (sx + margin, sy + margin),
        "top_right" => (sx + sw - w - margin, sy + margin),
        "bottom_left" => (sx + margin, sy + sh - h - margin),
        "bottom_right" => (sx + sw - w - margin, sy + sh - h - margin),
        "perch" => perch_point(app).unwrap_or((sx + sw - w - margin, sy + margin)),
        _ => (
            crate::util::rand_range(sx + margin, (sx + sw - w - margin).max(sx + margin)),
            crate::util::rand_range(
                (sy + sh - h - 180).max(sy + margin),
                (sy + sh - h - margin).max(sy + margin),
            ),
        ),
    };
    let facing = tx > app.borrow().pos.0;
    {
        let mut a = app.borrow_mut();
        a.target = Some((tx, ty));
        a.post_walk_emotion = if emotion == "walking" {
            "idle".into()
        } else {
            emotion.to_string()
        };
        a.facing_right = facing;
        a.user_placed = false;
    }
    app.borrow().stage.set_state("walking", facing, true, false);
}

pub(crate) fn perch_point(app: &AppHandle) -> Option<(i32, i32)> {
    let win = crate::win32::active_window()?;
    let lower = win.title.to_lowercase();
    if lower.is_empty() || lower.contains("aria") || lower.contains("program manager") {
        return None;
    }
    let (left, top, w, h) = crate::win32::window_rect(win.hwnd)?;
    if w < 260 || h < 160 {
        return None;
    }
    let (sx, sy, sw, sh) = crate::win32::work_area();
    let (awn, ahn) = window_size(app);
    let tx = (left + w - awn - 35).clamp(sx + 10, (sx + sw - awn - 10).max(sx + 10));
    let ty = (top - (ahn as f32 * 0.42) as i32).clamp(sy + 5, (sy + sh - ahn - 10).max(sy + 5));
    Some((tx, ty))
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
    // Scope the read borrow so the write below cannot collide with it.
    {
        let a = app.borrow();
        a.ui.window().set_position(PhysicalPosition::new(x, y));
    }
    app.borrow_mut().pos = (x, y);
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
        a.stationary_since = Instant::now();
    }
    app.borrow().stage.set_state("idle", false, false, false);
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
    stage.set_window_rect(x, y, w, h);
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
    let key = (size.width as i32, size.height as i32, hole);
    if a.region_key == Some(key) {
        return;
    }
    let Some(hwnd) = mascot_hwnd(&a.ui) else {
        return;
    };
    drop(a);
    crate::win32::set_region_with_hole(hwnd, size.width as i32, size.height as i32, hole);
    app.borrow_mut().region_key = Some(key);
}

/// The mascot window's Win32 handle.
pub(crate) fn mascot_hwnd(ui: &MascotWindow) -> Option<isize> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    ui.window()
        .with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
            _ => None,
        })
        .flatten()
}

pub(crate) fn start_drag(app: &AppHandle) {
    let (pos, cursor) = (app.borrow().pos, crate::input::cursor_position());
    let mut a = app.borrow_mut();
    a.dragging = true;
    a.drag_origin = Some((pos.0, pos.1, cursor.0, cursor.1));
    a.target = None;
    a.user_placed = true;
}

pub(crate) fn update_drag(app: &AppHandle, cursor: (i32, i32)) {
    if let Some((win_x, win_y, cur_x, cur_y)) = app.borrow().drag_origin {
        set_window_pos(app, win_x + (cursor.0 - cur_x), win_y + (cursor.1 - cur_y));
    }
}

pub(crate) fn end_drag(app: &AppHandle) {
    app.borrow_mut().dragging = false;
    app.borrow_mut().drag_origin = None;
    app.borrow_mut().stationary_since = Instant::now();
    app.borrow().stage.set_drag_inertia(0.0, 0.0);
    check_edge_snap(app);
}

/// Magnetic bezel snapping with the peeking lean.
pub(crate) fn check_edge_snap(app: &AppHandle) {
    if !config::get().edge_snapping_enabled {
        return;
    }
    let (sx, sy, sw, sh) = crate::win32::work_area();
    let (x, y) = app.borrow().pos;
    let (w, h) = window_size(app);

    const THRESHOLD: i32 = 55;
    let mut target_x = x;
    let mut target_y = y;
    let mut side = "none";
    if (x - sx).abs() <= THRESHOLD {
        target_x = sx - (w as f32 * 0.12) as i32;
        side = "left";
    } else if ((x + w) - (sx + sw)).abs() <= THRESHOLD {
        target_x = sx + sw - (w as f32 * 0.88) as i32;
        side = "right";
    }
    if ((y + h) - (sy + sh)).abs() <= THRESHOLD {
        target_y = sy + sh - h;
    }
    if target_x != x || target_y != y {
        app.borrow_mut().target = Some((target_x, target_y));
        app.borrow_mut().post_walk_emotion = "idle".into();
    }
    set_peeking(app, side);
}
