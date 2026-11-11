//! Mouse input for the avatar, plus the gesture detectors.
//!
//! Ported from `docs/reference/stage_bridge.js` with its tuning values kept exact: a head-pat needs
//! four direction reversals of at least 8 px inside 1.25 s with a 5 s cool-down; a shake needs five
//! reversals of at least 12 px inside 0.85 s while dragging; a drag starts after 5 px of travel.
//!
//! The window is layered and receives no mouse input of its own. The app forwards the global
//! cursor position (relative to this window) and the button states every tick. Because those
//! buttons are global, a gesture only starts from a press that lands **on the model** (judged by
//! the last frame's alpha, see [`crate::present::HitMask`]); a press anywhere else on the desktop
//! is ignored until it is released.

use bevy::prelude::*;

use aria_protocol::Event;

use crate::behaviors::AvatarState;
use crate::link::AvatarLink;
use crate::present::HitMask;

const DRAG_DEAD_ZONE: f32 = 5.0;
const SHAKE_STEP: f32 = 12.0;
const SHAKE_REVERSALS: usize = 5;
const SHAKE_WINDOW: f32 = 0.85;
const HEADPAT_STEP: f32 = 8.0;
const HEADPAT_REVERSALS: usize = 4;
const HEADPAT_WINDOW: f32 = 1.25;
const HEADPAT_COOLDOWN: f32 = 5.0;
/// Windows' default double-click time and a small travel tolerance.
const DOUBLE_CLICK_SECS: f32 = 0.5;
const DOUBLE_CLICK_SLOP: f32 = 6.0;
/// Hover tolerance around the model's silhouette, in pixels.
const HIT_RADIUS: i32 = 3;

/// One cursor sample forwarded from the main app.
#[derive(Debug, Clone, Copy, Default)]
pub struct PointerSample {
    pub position: Vec2,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

/// Gesture state, owned by the avatar process.
#[derive(Resource, Default)]
pub struct GestureState {
    clock: f32,
    previous: PointerSample,
    have_previous: bool,
    /// A left press that started on the model: (start position, start time).
    press: Option<(Vec2, f32)>,
    dragging: bool,
    last_x: f32,
    last_y: f32,
    shake_dir: i32,
    shake_reversals: Vec<f32>,
    /// Last click released without dragging, for double-click detection.
    last_click: Option<(Vec2, f32)>,
    headpat_x: Option<f32>,
    headpat_dir: i32,
    headpat_reversals: Vec<f32>,
    last_headpat: f32,
    /// Last hover state reported; None until the first sample, so the app always learns the
    /// initial state (click-through stays off until it does).
    over_model: Option<bool>,
}

impl GestureState {
    fn update(&mut self, sample: PointerSample, over: bool, size: Vec2, dt: f32, link: &AvatarLink) {
        self.clock += dt;
        let now = self.clock;
        let prev = if self.have_previous { self.previous } else { sample };
        self.have_previous = true;
        self.previous = sample;
        let pos = sample.position;

        if self.over_model != Some(over) {
            self.over_model = Some(over);
            link.send(Event::HoverChanged { over });
        }

        // ---- Right and middle buttons: on press, over the model only ----
        if sample.right && !prev.right && over {
            link.send(Event::ContextMenu);
        }
        if sample.middle && !prev.middle && over {
            link.send(Event::MiddleClick);
        }

        // ---- Head-pat: scrubbing back and forth across the head band, no button held ----
        let in_band = pos.y >= 0.0
            && pos.y < size.y * 0.4
            && (pos.x - size.x * 0.5).abs() < size.x * 0.36
            && over;
        if in_band && !sample.left {
            if let Some(last_x) = self.headpat_x {
                let step = pos.x - last_x;
                if step.abs() >= HEADPAT_STEP {
                    let dir = if step > 0.0 { 1 } else { -1 };
                    if self.headpat_dir != 0 && dir != self.headpat_dir {
                        self.headpat_reversals.retain(|t| now - t <= HEADPAT_WINDOW);
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
                    self.headpat_x = Some(pos.x);
                }
            } else {
                self.headpat_x = Some(pos.x);
            }
        } else {
            self.headpat_x = None;
            self.headpat_dir = 0;
            self.headpat_reversals.clear();
        }

        // ---- Left button: press on the model, then drag / click / double-click ----
        if sample.left && !prev.left {
            if over {
                self.press = Some((pos, now));
                self.last_x = pos.x;
                self.last_y = pos.y;
                self.shake_dir = 0;
                self.shake_reversals.clear();
            }
        } else if sample.left {
            if let Some((origin, _)) = self.press {
                let travel = (pos - origin).abs();
                if !self.dragging && travel.x + travel.y > DRAG_DEAD_ZONE {
                    self.dragging = true;
                    link.send(Event::DragStart);
                }
                if self.dragging && (pos.x != self.last_x || pos.y != self.last_y) {
                    let step_x = pos.x - self.last_x;
                    let step_y = pos.y - self.last_y;
                    link.send(Event::DragMove);
                    link.send(Event::DragInertia {
                        x: (step_x / 16.0).clamp(-1.0, 1.0),
                        y: (step_y / 16.0).clamp(-1.0, 1.0),
                    });
                    if step_x.abs() >= SHAKE_STEP {
                        let dir = if step_x > 0.0 { 1 } else { -1 };
                        if self.shake_dir != 0 && dir != self.shake_dir {
                            self.shake_reversals.retain(|t| now - t <= SHAKE_WINDOW);
                            self.shake_reversals.push(now);
                            if self.shake_reversals.len() >= SHAKE_REVERSALS {
                                self.shake_reversals.clear();
                                link.send(Event::Shake);
                            }
                        }
                        self.shake_dir = dir;
                    }
                    self.last_x = pos.x;
                    self.last_y = pos.y;
                }
            }
        } else if prev.left {
            // Release.
            if self.dragging {
                link.send(Event::DragEnd);
            } else if let Some((origin, _)) = self.press {
                match self.last_click {
                    Some((at, when))
                        if now - when <= DOUBLE_CLICK_SECS
                            && at.distance(origin) <= DOUBLE_CLICK_SLOP =>
                    {
                        self.last_click = None;
                        link.send(Event::Poke);
                    }
                    _ => self.last_click = Some((origin, now)),
                }
            }
            self.dragging = false;
            self.press = None;
        }
    }
}

/// Feeds the latest cursor sample to the detectors.
pub fn track_pointer(
    time: Res<Time>,
    mut gestures: ResMut<GestureState>,
    mut state: ResMut<AvatarState>,
    mask: Res<HitMask>,
    link: Res<AvatarLink>,
) {
    let Some(sample) = state.pointer.take() else {
        return;
    };
    let size = Vec2::new(state.viewport.0 as f32, state.viewport.1 as f32);
    let over = mask.hit(sample.position.x, sample.position.y, HIT_RADIUS);
    gestures.update(sample, over, size, time.delta_secs().min(0.1), &link);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(x: f32, y: f32, left: bool) -> PointerSample {
        PointerSample {
            position: Vec2::new(x, y),
            left,
            ..default()
        }
    }

    #[test]
    fn presses_off_the_model_never_drag() {
        let link = AvatarLink::detached();
        let mut g = GestureState::default();
        let size = Vec2::new(260.0, 370.0);
        g.update(sample(10.0, 10.0, false), false, size, 0.03, &link);
        g.update(sample(10.0, 10.0, true), false, size, 0.03, &link);
        g.update(sample(80.0, 80.0, true), false, size, 0.03, &link);
        assert!(!g.dragging);
        let sent = link.drain_sent();
        assert!(!sent.iter().any(|e| matches!(e, Event::DragStart)));
    }

    #[test]
    fn press_on_model_then_move_drags_and_double_click_pokes() {
        let link = AvatarLink::detached();
        let mut g = GestureState::default();
        let size = Vec2::new(260.0, 370.0);
        g.update(sample(100.0, 200.0, false), true, size, 0.03, &link);
        g.update(sample(100.0, 200.0, true), true, size, 0.03, &link);
        g.update(sample(130.0, 200.0, true), true, size, 0.03, &link);
        g.update(sample(130.0, 200.0, false), true, size, 0.03, &link);
        let sent = link.drain_sent();
        assert!(sent.iter().any(|e| matches!(e, Event::DragStart)));
        assert!(sent.iter().any(|e| matches!(e, Event::DragEnd)));

        for _ in 0..2 {
            g.update(sample(100.0, 200.0, true), true, size, 0.05, &link);
            g.update(sample(100.0, 200.0, false), true, size, 0.05, &link);
        }
        assert!(link.drain_sent().iter().any(|e| matches!(e, Event::Poke)));
    }
}
