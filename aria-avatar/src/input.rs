//! Mouse input inside the avatar window, plus the gesture detectors.
//!
//! Ported from `docs/reference/stage_bridge.js` with its tuning values kept exact: a head-pat needs
//! four direction reversals inside 1.25 s with a 5 s cool-down, and a shake needs five reversals
//! inside 0.85 s. Both require a minimum travel per step so a slow drift never counts.
//!
//! The window is layered, so it receives no mouse input of its own. The main app sends this process
//! the global cursor position and the button state; hit-testing against the model happens here,
//! where the model's projected bounds are known.

use bevy::prelude::*;

use aria_protocol::Event;

use crate::behaviors::AvatarState;
use crate::link::AvatarLink;

/// Minimum horizontal travel, in pixels, for a step to count as part of a gesture.
const STEP_PX: f32 = 8.0;
/// Minimum travel for a drag step to register.
const DRAG_STEP_PX: f32 = 5.0;
/// Shake: reversals needed and the window they must fall inside.
const SHAKE_REVERSALS: usize = 5;
const SHAKE_WINDOW: f32 = 0.85;
/// Head pat: reversals needed, the window, and the cool-down.
const HEADPAT_REVERSALS: usize = 4;
const HEADPAT_WINDOW: f32 = 1.25;
const HEADPAT_COOLDOWN: f32 = 5.0;

/// Gesture state, owned by the avatar process.
#[derive(Resource, Default)]
pub struct GestureState {
    cursor: Option<Vec2>,
    last_x: f32,
    last_y: f32,
    dragging: bool,
    drag_origin: Option<Vec2>,
    /// Reversal timestamps for the shake detector.
    shake_reversals: Vec<f32>,
    shake_dir: i32,
    /// Reversal timestamps for the head-pat detector.
    headpat_reversals: Vec<f32>,
    headpat_dir: i32,
    last_headpat: f32,
    clock: f32,
}

impl GestureState {
    /// Feeds one cursor sample. `pressed` is the primary button state.
    fn update(&mut self, cursor: Vec2, pressed: bool, dt: f32, link: &AvatarLink) {
        self.clock += dt;
        let previous = self.cursor.replace(cursor);

        if previous.is_none() {
            self.last_x = cursor.x;
            self.last_y = cursor.y;
            return;
        }

        let step_x = cursor.x - self.last_x;

        if !pressed {
            self.detect_headpat(step_x, self.clock, cursor, link);
        }

        // Drag: press, then movement past a small dead zone.
        if pressed {
            if !self.dragging {
                self.dragging = true;
                self.drag_origin = Some(cursor);
                self.last_x = cursor.x;
                self.last_y = cursor.y;
                link.send(Event::DragStart);
            } else if let Some(origin) = self.drag_origin {
                let travelled = (cursor - origin).abs();
                if travelled.x + travelled.y > DRAG_STEP_PX {
                    link.send(Event::DragMove);
                    let step_y = cursor.y - self.last_y;
                    link.send(Event::DragInertia {
                        x: (step_x / 16.0).clamp(-1.0, 1.0),
                        y: (step_y / 16.0).clamp(-1.0, 1.0),
                    });
                    self.detect_shake(step_x, self.clock, link);
                }
            }
        } else if self.dragging {
            self.dragging = false;
            self.drag_origin = None;
            self.shake_reversals.clear();
            self.shake_dir = 0;
            link.send(Event::DragEnd);
        }

        self.last_x = cursor.x;
        self.last_y = cursor.y;
    }

    fn detect_shake(&mut self, step_x: f32, now: f32, link: &AvatarLink) {
        if step_x.abs() < 12.0 {
            return;
        }
        let dir = if step_x > 0.0 { 1 } else { -1 };
        if self.shake_dir != 0 && dir != self.shake_dir {
            self.shake_reversals
                .retain(|t| now - t <= SHAKE_WINDOW);
            self.shake_reversals.push(now);
            if self.shake_reversals.len() >= SHAKE_REVERSALS {
                self.shake_reversals.clear();
                link.send(Event::Shake);
            }
        }
        self.shake_dir = dir;
    }

    fn detect_headpat(&mut self, step_x: f32, now: f32, cursor: Vec2, link: &AvatarLink) {
        // The head band is the upper 40% of the window, horizontally centred.
        let in_band = cursor.y < crate::WINDOW_H * 0.4
            && (cursor.x - crate::WINDOW_W * 0.5).abs() < crate::WINDOW_W * 0.36;
        if !in_band {
            self.headpat_reversals.clear();
            self.headpat_dir = 0;
            return;
        }
        if step_x.abs() < STEP_PX {
            return;
        }
        let dir = if step_x > 0.0 { 1 } else { -1 };
        if self.headpat_dir != 0 && dir != self.headpat_dir {
            self.headpat_reversals
                .retain(|t| now - t <= HEADPAT_WINDOW);
            self.headpat_reversals.push(now);
            if self.headpat_reversals.len() >= HEADPAT_REVERSALS
                && now - self.last_headpat > HEADPAT_COOLDOWN
            {
                self.headpat_reversals.clear();
                self.last_headpat = now;
                link.send(Event::Headpat);
            }
        }
        self.headpat_dir = dir;
    }
}

/// True when the cursor is over the model, judged by its projected screen bounds.
///
/// Click-through depends on this: the main app toggles hit-testing so clicks in the empty area of
/// the window reach the desktop.
pub fn over_model(state: &AvatarState, cursor: Vec2) -> bool {
    let Some((min, max)) = state.screen_bounds else {
        return false;
    };
    cursor.x >= min.x && cursor.x <= max.x && cursor.y >= min.y && cursor.y <= max.y
}

/// Picks up cursor samples the app forwarded and feeds the detectors.
pub fn track_pointer(
    time: Res<Time>,
    mut gestures: ResMut<GestureState>,
    state: Res<AvatarState>,
    link: Res<AvatarLink>,
) {
    let Some(sample) = state.pointer else {
        return;
    };
    // An opaque report of the window state, so the app can toggle click-through.
    let over = over_model(&state, sample.position);
    if over != state.last_over_model {
        link.send(Event::HoverChanged { over });
    }
    gestures.update(sample.position, sample.pressed, time.delta_secs().min(0.1), &link);
}
