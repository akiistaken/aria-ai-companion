//! Desktop physics for Aria's body: gravity with the taskbar as the floor, walking with real
//! acceleration and braking, jumping up onto windows, riding a window she stands on, and being
//! picked up and thrown.
//!
//! Everything is in screen pixels (y down) at the **soles of her feet**, centred on her. Speeds are
//! authored in metres and converted with the renderer's pixels-per-metre, so a small Aria walks and
//! falls at the same pace, relative to her body, as a large one. The renderer measures how the
//! window really moved and animates from that, so whatever happens here is what she acts out.
//!
//! Pure logic, no Win32 or Slint: the app builds an [`Env`] each tick.

use aria_protocol::BodyMode;

/// Gravity in m/s². A touch under real gravity, so falls read a little floaty, like an anime.
pub const GRAVITY: f32 = 9.81 * 0.85;
/// Faster than this (m/s) on touchdown is a hard landing.
pub const HARD_LANDING: f32 = 6.5;
/// The fastest a throw can send her, in m/s.
const MAX_THROW: f32 = 9.0;
/// Accelerating and braking on foot, m/s².
const ACCEL: f32 = 2.6;
const BRAKE: f32 = 3.4;
/// Height a jump clears its landing spot by, in metres.
const JUMP_CLEARANCE: f32 = 0.45;
/// Longest horizontal distance she jumps, in metres.
const JUMP_REACH: f32 = 1.1;
/// Pause before take-off, in seconds: a beat of anticipation.
const WINDUP_SECS: f32 = 0.2;
/// Beyond this distance (metres) she jogs instead of walking.
const JOG_DISTANCE: f32 = 5.0;

/// Somewhere to stand: the taskbar's top edge, or a window's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    /// The window it belongs to; `None` for the floor.
    pub perch: Option<isize>,
}

impl Surface {
    fn under(&self, x: f32) -> bool {
        x >= self.left && x <= self.right
    }
}

/// The world around her this tick.
pub struct Env<'a> {
    /// The floor: the top of the taskbar (the bottom of the work area). Solid everywhere: she can
    /// never end up below it.
    pub floor: f32,
    /// Windows she may stand on.
    pub perches: &'a [Surface],
    /// How far left and right her feet may go.
    pub walls: (f32, f32),
    /// The highest her feet may go, so her head stays on screen.
    pub ceiling: f32,
    /// Window pixels per metre of model.
    pub ppm: f32,
}

impl Env<'_> {
    fn perch(&self, id: isize) -> Option<&Surface> {
        self.perches.iter().find(|s| s.perch == Some(id))
    }

    fn floor_surface(&self) -> Surface {
        Surface { left: f32::MIN, right: f32::MAX, top: self.floor, perch: None }
    }
}

/// Where she is headed: a spot on the floor, or on a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Goal {
    pub x: f32,
    pub perch: Option<isize>,
}

/// How fast a persona moves on foot, in m/s.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    pub walk: f32,
    pub jog: f32,
}

impl Pace {
    pub fn for_persona(persona: &str) -> Pace {
        let walk = match persona {
            "genki" => 1.25,
            "tomboy" => 1.15,
            "bakadere" => 1.05,
            "kuudere" => 0.8,
            "yandere" => 0.85,
            "meganekko" => 0.9,
            _ => 0.95,
        };
        Pace { walk, jog: walk * 1.75 }
    }
}

/// Something worth reacting to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Reached the goal.
    Arrived,
    /// Touched down at this speed (m/s).
    Landed { speed: f32 },
    /// Left the ground without jumping: walked off an edge, or her window went away.
    Fell,
}

/// Her body.
#[derive(Clone, Debug)]
pub struct Body {
    pub mode: BodyMode,
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// The window she stands on; `None` on the floor.
    pub perch: Option<isize>,
    /// The surface a jump in progress is aimed at: `Some(None)` for the floor.
    jump: Option<Option<isize>>,
    /// Crouching before a jump: seconds left, and the goal it is for.
    windup: Option<(f32, Goal)>,
}

impl Default for Body {
    fn default() -> Self {
        Body { mode: BodyMode::Ground, x: 0.0, y: 0.0, vx: 0.0, vy: 0.0, perch: None, jump: None, windup: None }
    }
}

/// Moves `value` toward `target` by at most `step`.
fn toward(value: f32, target: f32, step: f32) -> f32 {
    value + (target - value).clamp(-step, step)
}

impl Body {
    /// Puts her somewhere new without any motion (initial placement, a teleport).
    pub fn place(&mut self, x: f32, y: f32) {
        self.x = x;
        self.y = y;
        self.vx = 0.0;
        self.vy = 0.0;
    }

    pub fn is_moving(&self) -> bool {
        self.mode != BodyMode::Ground || self.vx.abs() > 1.0 || self.windup.is_some()
    }

    /// Follows the cursor while she is held, tracking the hand's velocity for the throw.
    pub fn hold(&mut self, x: f32, y: f32, dt: f32) {
        if self.mode != BodyMode::Held {
            self.mode = BodyMode::Held;
            self.place(x, y);
            self.perch = None;
            self.jump = None;
            self.windup = None;
            return;
        }
        let dt = dt.max(1e-3);
        // Smoothed over a few ticks: the last tick alone is too jittery to throw with.
        let blend = (dt * 14.0).min(1.0);
        self.vx += ((x - self.x) / dt - self.vx) * blend;
        self.vy += ((y - self.y) / dt - self.vy) * blend;
        self.x = x;
        self.y = y;
    }

    /// Lets go: she keeps the hand's velocity and falls.
    pub fn release(&mut self, ppm: f32) {
        let max = MAX_THROW * ppm;
        let speed = (self.vx * self.vx + self.vy * self.vy).sqrt();
        if speed > max {
            self.vx *= max / speed;
            self.vy *= max / speed;
        }
        self.mode = BodyMode::Air;
    }

    /// Advances the simulation by `dt` seconds.
    pub fn step(&mut self, env: &Env, goal: Option<Goal>, pace: Pace, dt: f32) -> Vec<Event> {
        let dt = dt.clamp(0.0, 0.1);
        match self.mode {
            BodyMode::Held => Vec::new(),
            BodyMode::Air => self.step_air(env, dt).into_iter().collect(),
            BodyMode::Ground => self.step_ground(env, goal, pace, dt).into_iter().collect(),
        }
    }

    fn step_air(&mut self, env: &Env, dt: f32) -> Option<Event> {
        let g = GRAVITY * env.ppm;
        let previous_y = self.y;
        self.vy += g * dt;
        self.vx *= (-0.35 * dt).exp();
        self.x += self.vx * dt;
        self.y += self.vy * dt;

        // Walls and the top of the screen bounce her back, losing most of the speed.
        if self.x < env.walls.0 {
            self.x = env.walls.0;
            self.vx = self.vx.abs() * 0.35;
        } else if self.x > env.walls.1 {
            self.x = env.walls.1;
            self.vx = -self.vx.abs() * 0.35;
        }
        if self.y < env.ceiling {
            self.y = env.ceiling;
            self.vy = self.vy.abs() * 0.2;
        }

        // A window only catches her when she jumps for it, and only from above.
        let caught = self.jump.flatten().and_then(|id| env.perch(id)).filter(|s| {
            s.under(self.x) && self.vy > 0.0 && previous_y <= s.top && self.y >= s.top
        });
        let surface = match caught {
            Some(surface) => *surface,
            None if self.y >= env.floor => env.floor_surface(),
            None => return None,
        };
        let speed = self.vy.max(0.0) / env.ppm;
        self.y = surface.top;
        self.vy = 0.0;
        // Landing scrubs off most of the sideways speed; the rest slides out on the ground.
        self.vx *= 0.3;
        self.mode = BodyMode::Ground;
        self.perch = surface.perch;
        self.jump = None;
        Some(Event::Landed { speed })
    }

    fn fall(&mut self) -> Option<Event> {
        self.mode = BodyMode::Air;
        self.perch = None;
        self.jump = None;
        self.windup = None;
        self.vy = self.vy.max(0.0);
        Some(Event::Fell)
    }

    fn step_ground(&mut self, env: &Env, goal: Option<Goal>, pace: Pace, dt: f32) -> Option<Event> {
        let ppm = env.ppm;

        // ---- What is she standing on? ----------------------------------------------------
        let support = match self.perch {
            Some(id) => match env.perch(id).filter(|s| s.under(self.x)) {
                Some(surface) => *surface,
                // Her window closed, minimized or moved out from under her, or she walked off.
                None => return self.fall(),
            },
            None => env.floor_surface(),
        };
        if support.perch.is_some() {
            // Ride the window wherever it goes.
            self.y = support.top;
        } else if self.y < support.top - 3.0 {
            // Put somewhere above the floor (a teleport, the game dodge): drop.
            return self.fall();
        } else {
            self.y = support.top;
        }

        // ---- A jump being wound up -------------------------------------------------------
        if let Some((left, jump_goal)) = self.windup.as_mut() {
            *left -= dt;
            self.vx = 0.0;
            if *left > 0.0 {
                return None;
            }
            let jump_goal = *jump_goal;
            self.windup = None;
            self.jump_to(env, jump_goal);
            return None;
        }

        // ---- Where to go -----------------------------------------------------------------
        let Some(goal) = goal else {
            self.coast(env, dt);
            return None;
        };
        let target = goal.perch.and_then(|id| env.perch(id)).copied();
        // A window that is gone is just a spot on the floor; on a window, stay a little inside its
        // edges, so a jump always comes down on it.
        let goal = match target {
            Some(s) => {
                let inset = (0.25 * ppm).min((s.right - s.left) * 0.5);
                Goal { x: goal.x.clamp(s.left + inset, s.right - inset), perch: s.perch }
            }
            None => Goal { x: goal.x, perch: None },
        };
        let same_level = goal.perch == self.perch;

        let destination = if same_level {
            goal.x
        } else if goal.perch.is_none() {
            // Getting down from a window: walk off its nearer edge on the way to the goal.
            if support.under(goal.x) {
                if goal.x - support.left < support.right - goal.x {
                    support.left - 0.3 * ppm
                } else {
                    support.right + 0.3 * ppm
                }
            } else {
                goal.x
            }
        } else {
            // Up (or across) to a window: walk to within a jump of the spot, then jump.
            let dx = goal.x - self.x;
            let reach = JUMP_REACH * ppm;
            if dx.abs() <= reach {
                self.vx = toward(self.vx, 0.0, BRAKE * ppm * dt * 2.0);
                if self.vx.abs() < 0.15 * ppm {
                    self.vx = 0.0;
                    self.windup = Some((WINDUP_SECS, goal));
                }
                self.x += self.vx * dt;
                return None;
            }
            goal.x - dx.signum() * reach * 0.85
        };

        let destination = destination.clamp(env.walls.0, env.walls.1);
        let dx = destination - self.x;
        if same_level && dx.abs() < 1.5 && self.vx.abs() < 0.1 * ppm {
            self.x = destination;
            self.vx = 0.0;
            return Some(Event::Arrived);
        }

        // ---- Walk: accelerate, cruise, and brake to stop exactly on the spot ----------------
        let top_speed = if dx.abs() > JOG_DISTANCE * ppm { pace.jog } else { pace.walk } * ppm;
        // The speed she can still stop from, one tick early so she does not arrive moving.
        let ahead = (dx.abs() - self.vx.abs() * dt).max(0.0);
        let braking = (2.0 * BRAKE * ppm * ahead).sqrt() * 0.95;
        let wanted = dx.signum() * top_speed.min(braking);
        let speeding_up = wanted.abs() > self.vx.abs() && wanted * self.vx >= 0.0;
        let rate = if speeding_up { ACCEL } else { BRAKE } * ppm;
        self.vx = toward(self.vx, wanted, rate * dt);
        let next = self.x + self.vx * dt;
        // Never step past the spot: reaching it ends the walk (the braking curve has her nearly
        // stopped by then, so the last few pixels per second just vanish).
        if dx == 0.0 || (destination - next) * dx <= 0.0 {
            self.x = destination;
            self.vx = 0.0;
        } else {
            self.x = next;
        }
        if self.x <= env.walls.0 || self.x >= env.walls.1 {
            self.x = self.x.clamp(env.walls.0, env.walls.1);
            self.vx = 0.0;
            if same_level {
                return Some(Event::Arrived);
            }
        }
        // Walked off the edge of a window.
        if self.perch.is_some() && !support.under(self.x) {
            return self.fall();
        }
        None
    }

    /// No goal: slide to a stop (after a landing with sideways speed).
    fn coast(&mut self, env: &Env, dt: f32) {
        if self.vx == 0.0 {
            return;
        }
        self.vx = toward(self.vx, 0.0, BRAKE * 1.6 * env.ppm * dt);
        self.x = (self.x + self.vx * dt).clamp(env.walls.0, env.walls.1);
    }

    /// Launches a jump that peaks [`JUMP_CLEARANCE`] above the higher of the two ends and comes
    /// down on the goal.
    fn jump_to(&mut self, env: &Env, goal: Goal) {
        let ppm = env.ppm;
        let g = GRAVITY * ppm;
        let top = goal.perch.and_then(|id| env.perch(id)).map(|s| s.top).unwrap_or(env.floor);
        let apex = self.y.min(top) - JUMP_CLEARANCE * ppm;
        let up = self.y - apex;
        let down = top - apex;
        let flight = (2.0 * up / g).sqrt() + (2.0 * down / g).sqrt();
        self.vy = -(2.0 * g * up).sqrt();
        self.vx = (goal.x - self.x) / flight.max(0.1);
        self.mode = BodyMode::Air;
        self.perch = None;
        self.jump = Some(goal.perch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PPM: f32 = 200.0;
    const DT: f32 = 1.0 / 30.0;
    const FLOOR: f32 = 1000.0;
    const PACE: Pace = Pace { walk: 1.0, jog: 1.75 };

    fn env(perches: &[Surface]) -> Env<'_> {
        Env { floor: FLOOR, perches, walls: (0.0, 1920.0), ceiling: 300.0, ppm: PPM }
    }

    fn run(body: &mut Body, env: &Env, goal: Option<Goal>, seconds: f32) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..(seconds / DT) as usize {
            events.extend(body.step(env, goal, PACE, DT));
        }
        events
    }

    fn standing_at(x: f32) -> Body {
        let mut body = Body::default();
        body.place(x, FLOOR);
        body
    }

    #[test]
    fn a_drop_falls_to_the_taskbar_at_the_speed_gravity_gives() {
        let mut body = Body::default();
        body.place(500.0, FLOOR - 2.0 * PPM);
        body.mode = BodyMode::Air;
        let events = run(&mut body, &env(&[]), None, 2.0);
        assert_eq!(body.mode, BodyMode::Ground);
        assert_eq!(body.y, FLOOR);
        let Some(Event::Landed { speed }) = events.first() else { panic!("{events:?}") };
        let expected = (2.0 * GRAVITY * 2.0).sqrt();
        assert!((speed - expected).abs() < 0.4, "landed at {speed} m/s, expected {expected}");
    }

    #[test]
    fn placed_above_the_floor_she_falls_and_below_it_she_stands_on_it() {
        let mut body = standing_at(500.0);
        body.y = FLOOR - 300.0;
        assert_eq!(body.step(&env(&[]), None, PACE, DT), vec![Event::Fell]);
        assert_eq!(body.mode, BodyMode::Air);

        let mut body = Body::default();
        body.place(500.0, FLOOR + 80.0);
        body.mode = BodyMode::Air;
        let events = body.step(&env(&[]), None, PACE, DT);
        assert!(matches!(events[..], [Event::Landed { .. }]));
        assert_eq!(body.y, FLOOR);
    }

    #[test]
    fn walking_accelerates_cruises_brakes_and_stops_on_the_spot() {
        let mut body = standing_at(200.0);
        let goal = Some(Goal { x: 800.0, perch: None });
        let mut fastest = 0.0f32;
        let mut previous_v = 0.0f32;
        let mut arrived = false;
        for _ in 0..300 {
            let events = body.step(&env(&[]), goal, PACE, DT);
            fastest = fastest.max(body.vx);
            // Speed changes gradually: no instant start, and only a tiny settle at the end.
            let limit = if body.x == 800.0 { 0.15 * PPM } else { BRAKE * PPM * DT + 1e-3 };
            assert!((body.vx - previous_v).abs() <= limit, "{previous_v} -> {} at {}", body.vx, body.x);
            assert!(body.x <= 800.0, "overshot to {}", body.x);
            previous_v = body.vx;
            if events.contains(&Event::Arrived) {
                arrived = true;
                break;
            }
        }
        assert!(arrived);
        assert_eq!(body.x, 800.0);
        assert!((fastest - PACE.walk * PPM).abs() < 1.0, "cruises at walking pace, got {fastest}");
    }

    #[test]
    fn long_trips_are_jogged() {
        let mut body = standing_at(0.0);
        run(&mut body, &env(&[]), Some(Goal { x: 1900.0, perch: None }), 1.5);
        assert!(body.vx > PACE.walk * PPM * 1.3, "should be jogging, at {}", body.vx);
    }

    #[test]
    fn a_throw_bounces_off_the_wall_and_lands() {
        let mut body = standing_at(1700.0);
        body.hold(1700.0, 700.0, DT);
        for i in 1..=6 {
            body.hold(1700.0 + i as f32 * 60.0, 700.0 - i as f32 * 10.0, DT);
        }
        body.release(PPM);
        assert!(body.vx > 1000.0 && body.vy < 0.0);
        let events = run(&mut body, &env(&[]), None, 3.0);
        assert!(body.x < 1920.0);
        assert!(matches!(events.last(), Some(Event::Landed { .. })));
        assert_eq!(body.mode, BodyMode::Ground);
        // She slides out the leftover speed, then stops.
        assert_eq!(body.vx, 0.0);
    }

    #[test]
    fn she_jumps_up_onto_a_window_and_rides_it() {
        let window = Surface { left: 900.0, right: 1500.0, top: 600.0, perch: Some(7) };
        let mut body = standing_at(300.0);
        let goal = Some(Goal { x: 1200.0, perch: Some(7) });
        let events = run(&mut body, &env(&[window]), goal, 6.0);
        assert!(events.iter().any(|e| matches!(e, Event::Landed { .. })), "{events:?}");
        assert_eq!(events.last(), Some(&Event::Arrived));
        assert_eq!((body.perch, body.y, body.x), (Some(7), 600.0, 1200.0));

        // The user drags the window: she goes with it.
        let moved = Surface { left: 950.0, right: 1550.0, top: 450.0, perch: Some(7) };
        body.step(&env(&[moved]), None, PACE, DT);
        assert_eq!((body.y, body.mode), (450.0, BodyMode::Ground));

        // It is minimized: she drops to the floor.
        let events = run(&mut body, &env(&[]), None, 2.0);
        assert_eq!(events.first(), Some(&Event::Fell));
        assert_eq!((body.perch, body.y), (None, FLOOR));
    }

    #[test]
    fn getting_down_she_walks_off_the_edge_then_carries_on() {
        let window = Surface { left: 900.0, right: 1500.0, top: 600.0, perch: Some(7) };
        let mut body = standing_at(1200.0);
        body.y = 600.0;
        body.perch = Some(7);
        let goal = Some(Goal { x: 1300.0, perch: None });
        let events = run(&mut body, &env(&[window]), goal, 8.0);
        assert!(events.contains(&Event::Fell), "{events:?}");
        assert_eq!(events.last(), Some(&Event::Arrived));
        assert_eq!((body.perch, body.y, body.x), (None, FLOOR, 1300.0));
    }

    #[test]
    fn falling_past_a_window_does_not_catch_on_it() {
        let window = Surface { left: 0.0, right: 1920.0, top: 600.0, perch: Some(7) };
        let mut body = Body::default();
        body.place(500.0, 400.0);
        body.mode = BodyMode::Air;
        run(&mut body, &env(&[window]), None, 2.0);
        assert_eq!((body.perch, body.y), (None, FLOOR), "only an aimed jump lands on a window");
    }

    #[test]
    fn the_ceiling_keeps_her_on_screen() {
        let mut body = standing_at(500.0);
        body.mode = BodyMode::Air;
        body.vy = -5000.0;
        let mut highest = f32::MAX;
        for _ in 0..90 {
            body.step(&env(&[]), None, PACE, DT);
            highest = highest.min(body.y);
        }
        assert!(highest >= 300.0);
    }
}
