//! Moving through the desktop: how fast the window is travelling, where the feet go, and what the
//! body does when it is not standing on anything.
//!
//! The app owns the physics and simply moves the window. The renderer **measures** that movement
//! ([`Motion`]) instead of being told a speed, so the animation can never disagree with what the
//! window is really doing: no walking on the spot, no gliding.
//!
//! # Walking
//!
//! [`Gait`] keeps each foot at a point on the ground. A planted foot moves backward under the body
//! by exactly the distance the window moved forward, so it stays on the same desktop pixel. The
//! step cycle advances with distance travelled rather than with time, and stride length grows with
//! the square root of speed (as it does in people), so cadence and stride both fall to zero as she
//! slows down and the same code covers a creep, a walk and a jog. When she stops, or turns on the
//! spot, feet left out of place take a short settling step home.

use std::f32::consts::PI;

use bevy::prelude::*;

use crate::face::Face;
use crate::pose::{Arm, HandShape, Pose};
use crate::rig::Side;
use crate::spring::{approach_factor, smoothstep, smootherstep};

/// Gravity, for turning a speed into a dimensionless gait speed.
const GRAVITY: f32 = 9.81;

/// How much of the window's travel the model's spring bones get to feel, sideways and vertically.
/// VRM springs have no air to speak of: at the full distance a stroll blows her hair out
/// horizontal. A fall keeps more of it, since hair flying up is the point.
const SPRING_BONE_SHARE: Vec2 = Vec2::new(0.3, 0.45);

/// Tracks the avatar window across the desktop, in model metres.
#[derive(Default)]
pub struct Motion {
    last_px: Option<IVec2>,
    last_viewport: (u32, u32),
    /// How far the window moved this frame (x toward screen right, y up).
    pub delta: Vec2,
    /// Smoothed velocity in m/s.
    pub velocity: Vec2,
    /// Smoothed acceleration in m/s².
    pub accel: Vec2,
    /// Position the model root and camera are carried to, so the model's spring bones (hair,
    /// skirt, anything that jiggles) feel the window's real motion. Lightly smoothed: the window
    /// moves in steps that do not line up with rendered frames.
    pub world: Vec2,
    world_raw: Vec2,
}

impl Motion {
    /// Feeds this frame's window position (top-left, physical pixels).
    pub fn update(&mut self, window: Option<IVec2>, viewport: (u32, u32), pixels_per_metre: f32, dt: f32) {
        let moved = match (window, self.last_px) {
            (Some(now), Some(before)) => (now - before).as_vec2(),
            _ => Vec2::ZERO,
        };
        self.last_px = window.or(self.last_px);
        // A resize re-seats the window, and a jump of more than a window's size in one frame is a
        // teleport (a preset position, a monitor change). Neither is her moving.
        let resized = std::mem::replace(&mut self.last_viewport, viewport) != viewport;
        let limit = viewport.0.max(viewport.1) as f32 * 1.5;
        let moved = if resized || moved.length() > limit { Vec2::ZERO } else { moved };

        let dt = dt.max(1e-4);
        self.delta = Vec2::new(moved.x, -moved.y) / pixels_per_metre.max(1.0);
        let previous = self.velocity;
        self.velocity = self.velocity.lerp(self.delta / dt, approach_factor(13.0, dt));
        self.accel = self.accel.lerp((self.velocity - previous) / dt, approach_factor(9.0, dt));
        self.world_raw += self.delta * SPRING_BONE_SHARE;
        self.world = self.world.lerp(self.world_raw, approach_factor(22.0, dt));
    }
}

/// One foot's place on the ground. Positions are relative to the point under the body, in stage
/// axes that do not turn with her: x toward screen right, y toward the viewer.
#[derive(Clone, Copy, Debug, Default)]
struct FootState {
    pos: Vec2,
    /// Which way the foot points, as a world yaw.
    yaw: f32,
    swing: Option<Swing>,
    landed: f32,
}

#[derive(Clone, Copy, Debug)]
struct Swing {
    from: Vec2,
    from_yaw: f32,
    /// Cycle count when the foot left the ground.
    start: f32,
    lift: f32,
}

/// Where one foot is this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct FootPose {
    /// Point on the ground under the ankle (stage axes).
    pub ground: Vec2,
    /// Height of the sole above the ground.
    pub height: f32,
    /// Positive: heel raised on the ball of the foot. Negative: toes raised on the heel.
    pub pitch: f32,
    /// World yaw of the foot.
    pub yaw: f32,
    pub planted: bool,
}

/// What [`Gait::update`] needs each frame.
pub struct GaitInput {
    /// Horizontal distance the body moved this frame (metres, toward screen right).
    pub dx: f32,
    pub dt: f32,
    /// Smoothed horizontal speed in m/s.
    pub speed: f32,
    /// Which way the body faces (0 = the viewer, positive = screen right).
    pub yaw: f32,
    /// Hip joint to ankle.
    pub leg: f32,
    /// Half the distance between the feet when standing.
    pub half_width: f32,
    /// Toe-out of a standing foot.
    pub toe_out: f32,
    /// Style multiplier on how high the feet lift.
    pub bounce: f32,
}

/// The step cycle.
#[derive(Default)]
pub struct Gait {
    feet: [FootState; 2],
    /// Cycles walked; one cycle is a step with each foot.
    phase: f32,
    /// Current stride (distance covered by one cycle) in metres.
    pub stride: f32,
    placed: bool,
    stationary_for: f32,
}

/// Fraction of a cycle each foot spends on the ground.
const DUTY: f32 = 0.62;
/// Cycles per second used to finish or correct a step when distance is not driving the cycle.
const SETTLE_RATE: f32 = 1.7;

/// Smooth Hermite tracks through toe-off, early swing, passing, extension and heel strike.
/// Monotone tangents avoid overshooting the floor while preserving velocity between keys.
fn foot_track(keys: &[(f32, f32)], t: f32) -> f32 {
    let slope = |i: usize| {
        if i == 0 || i + 1 == keys.len() { return 0.0; }
        let a = (keys[i].1 - keys[i - 1].1) / (keys[i].0 - keys[i - 1].0);
        let b = (keys[i + 1].1 - keys[i].1) / (keys[i + 1].0 - keys[i].0);
        if a * b <= 0.0 { 0.0 } else { (a + b).signum() * ((a + b).abs() * 0.5).min(3.0 * a.abs().min(b.abs())) }
    };
    let i = keys.windows(2).position(|pair| t <= pair[1].0).unwrap_or(keys.len() - 2);
    let (x, a) = keys[i];
    let (end, b) = keys[i + 1];
    let span = end - x;
    let u = ((t - x) / span).clamp(0.0, 1.0);
    let (u2, u3) = (u * u, u * u * u);
    (2.0 * u3 - 3.0 * u2 + 1.0) * a + (u3 - 2.0 * u2 + u) * span * slope(i)
        + (-2.0 * u3 + 3.0 * u2) * b + (u3 - u2) * span * slope(i + 1)
}

const SWING_HEIGHT: &[(f32, f32)] = &[(0.0, 0.0), (0.14, 0.42), (0.42, 1.0), (0.68, 0.72), (0.86, 0.18), (1.0, 0.0)];
const SWING_PITCH: &[(f32, f32)] = &[(0.0, 0.4), (0.16, 0.48), (0.42, 0.12), (0.68, -0.1), (0.86, -0.23), (1.0, -0.23)];

impl Gait {
    /// Stride length for a speed: about 1.9 leg lengths times the square root of the Froude-style
    /// speed `v / sqrt(g L)`. That is a 1.25 leg-length stride at a normal walk, a little shorter
    /// and quicker than an adult's, which is the light, bouncy step anime characters walk with.
    pub fn stride_for(speed: f32, leg: f32) -> f32 {
        let froude = speed.abs() / (GRAVITY * leg).sqrt();
        (1.9 * froude.sqrt() * leg).min(1.7 * leg)
    }

    /// Dimensionless speed: about 0.45 is a comfortable walk, above 0.8 a jog.
    pub fn pace(speed: f32, leg: f32) -> f32 {
        speed.abs() / (GRAVITY * leg).sqrt()
    }

    fn home(side: Side, yaw: f32, half_width: f32) -> Vec2 {
        // The body's left, turned by its yaw, in stage axes.
        Vec2::new(yaw.cos(), -yaw.sin()) * (side.sign() * half_width)
    }

    /// Puts both feet under the body, as after a landing.
    pub fn plant(&mut self, yaw: f32, half_width: f32, toe_out: f32) {
        for side in Side::BOTH {
            self.feet[side.index()] = FootState {
                pos: Self::home(side, yaw, half_width),
                yaw: yaw + side.sign() * toe_out,
                swing: None,
                landed: self.phase - DUTY * 0.5,
            };
        }
        self.placed = true;
    }

    pub fn update(&mut self, input: &GaitInput) -> [FootPose; 2] {
        if !self.placed {
            self.plant(input.yaw, input.half_width, input.toe_out);
        }
        let leg = input.leg.max(0.05);
        let pace = Self::pace(input.speed, leg);
        self.stationary_for = if input.dx.abs() > 1e-5 { 0.0 } else { self.stationary_for + input.dt };
        let moving = pace > 0.03 && self.stationary_for < 0.12;
        let stride = Self::stride_for(input.speed, leg);
        self.stride = stride;
        let direction = if input.speed >= 0.0 { 1.0 } else { -1.0 };
        let swing_len = 1.0 - DUTY;

        // The ground slides back under a planted foot by exactly the distance travelled.
        for foot in &mut self.feet {
            foot.pos.x -= input.dx;
            if let Some(swing) = foot.swing.as_mut() {
                swing.from.x -= input.dx;
            }
        }

        let homes = Side::BOTH.map(|side| Self::home(side, input.yaw, input.half_width));
        let tolerance = 0.07 * leg;
        let astray = |gait: &Gait, i: usize| gait.feet[i].pos.distance(homes[i]) > tolerance;
        let needs_settling = (0..2).any(|i| self.feet[i].swing.is_some() || astray(self, i));

        // Distance drives the cycle while she moves, so every step covers exactly one stride;
        // only when she has stopped does time finish the step in progress or correct her stance.
        let advance = if moving {
            input.dx.abs() / stride.max(0.25 * leg)
        } else if needs_settling {
            input.dt * SETTLE_RATE
        } else {
            0.0
        };
        self.phase += advance.min(0.3);

        let mut out = [FootPose::default(); 2];
        for side in Side::BOTH {
            let i = side.index();
            let other_planted = self.feet[1 - i].swing.is_none();
            // Each foot's turn to swing opens half a cycle after the other's.
            let turn = (self.phase - i as f32 * 0.5).rem_euclid(1.0);
            let lead = if moving { direction * DUTY * stride * 0.5 } else { 0.0 };
            let target = homes[i] + Vec2::new(lead, 0.0);
            let target_yaw = input.yaw + side.sign() * input.toe_out;
            let reach = (stride / leg).min(1.0);

            if self.feet[i].swing.is_none()
                && turn < swing_len
                && other_planted
                && (moving || astray(self, i))
            {
                let lift = leg * (0.035 + 0.085 * reach) * input.bounce.clamp(0.6, 1.6);
                self.feet[i].swing = Some(Swing {
                    from: self.feet[i].pos,
                    from_yaw: self.feet[i].yaw,
                    start: self.phase,
                    lift,
                });
            }

            let foot = &mut self.feet[i];
            if let Some(swing) = foot.swing {
                let u = (self.phase - swing.start) / swing_len;
                if u >= 1.0 {
                    foot.pos = target;
                    foot.yaw = target_yaw;
                    foot.swing = None;
                    foot.landed = self.phase;
                } else {
                    let ease = smootherstep(u);
                    foot.pos = swing.from.lerp(target, ease);
                    foot.yaw = swing.from_yaw + crate::spring::wrap_angle(target_yaw - swing.from_yaw) * ease;
                    out[i] = FootPose {
                        ground: foot.pos,
                        // Leaves the ground quickly and comes down gently.
                        height: swing.lift * foot_track(SWING_HEIGHT, u),
                        // Toes trail at lift-off and lead into the heel strike.
                        pitch: reach * foot_track(SWING_PITCH, u),
                        yaw: foot.yaw,
                        planted: false,
                    };
                    continue;
                }
            }
            // Planted: roll from the heel at the start of stance onto the ball at the end.
            let stance = ((self.phase - foot.landed) / DUTY).clamp(0.0, 1.0);
            let roll = if moving {
                reach * (0.4 * smoothstep(0.68, 1.0, stance) - 0.23 * (1.0 - smoothstep(0.0, 0.22, stance)))
            } else {
                0.0
            };
            out[i] = FootPose { ground: foot.pos, height: 0.0, pitch: roll, yaw: foot.yaw, planted: true };
        }
        out
    }
}

/// How the body carries itself in the air.
pub struct AirInput {
    pub t: f32,
    /// Vertical speed in m/s, up positive.
    pub vy: f32,
    /// Seconds since she left the ground.
    pub airtime: f32,
}

/// Falling, jumping or thrown: arms up and flailing on the way down, tucked on the way up.
pub fn air_pose(input: &AirInput) -> (Pose, Face) {
    let AirInput { t, vy, airtime } = *input;
    let fall = smoothstep(0.3, 2.5, -vy);
    let rise = smoothstep(0.3, 2.5, vy);
    let mut pose = Pose::default();
    pose.spine(0.1 * rise - 0.1 * fall, 0.0, 0.0);
    pose.head(0.12 * rise - 0.16 * fall, 0.0, 0.0);
    for side in Side::BOTH {
        let alt = side.sign();
        let flail = (t * 11.0 + alt * 0.9).sin() * fall;
        pose.shrug(side, 0.05 + 0.12 * fall, 0.0);
        pose.arm(
            side,
            Arm {
                down: 0.85 - 1.35 * fall + 0.3 * rise + 0.2 * flail,
                swing: 0.1,
                yaw: 0.15,
                twist: -0.7 * fall,
                bend: 0.55 + 0.25 * (t * 9.0 + alt).sin() * fall,
                pronate: 0.0,
            },
        );
        pose.wrist(side, -0.2 * fall, 0.0, 0.0);
        pose.fingers(side, HandShape::OPEN.mix(HandShape::RELAXED, 0.3));
        // Falling, the legs splay into a frog shape (thighs out, knees turned out, shins folded in),
        // which reads from the front, and paddle. Rising, they tuck up under her.
        let pedal = (t * 7.5 + alt * PI * 0.5).sin() * fall;
        pose.leg(
            side,
            0.3 + 0.3 * pedal + 0.5 * rise,
            0.08 + 0.3 * fall,
            0.55 * fall,
            0.6 + 0.6 * fall + 0.35 * pedal + 0.8 * rise,
            0.6,
        );
    }
    let scared = fall * smoothstep(0.15, 0.4, airtime);
    let face = Face { surprised: 0.35 + 0.65 * fall, aa: 0.6 * scared, ..default() };
    (pose, face)
}

/// How the body hangs when it is picked up.
pub struct HeldInput {
    pub t: f32,
    /// Seconds since she was picked up.
    pub held_for: f32,
    /// Pendulum angle of the body (radians) and its rate, from the animator's swing spring.
    pub swing: f32,
    pub swing_rate: f32,
    /// How hard she is being shaken, `0..1`.
    pub struggle: f32,
}

/// Dangling from the cursor: shoulders hunched up, legs trailing behind the swing and kicking when
/// she is shaken.
pub fn held_pose(input: &HeldInput) -> (Pose, Face) {
    let HeldInput { t, held_for, swing, swing_rate, struggle } = *input;
    let mut pose = Pose::default();
    pose.spine(0.06, 0.0, -swing * 0.25);
    pose.head(0.1, 0.0, swing * 0.3);
    let lag = (-swing_rate * 0.12).clamp(-0.5, 0.5);
    for side in Side::BOTH {
        let alt = side.sign();
        let kick = (t * 14.0 + alt * PI * 0.5).sin() * struggle;
        pose.shrug(side, 0.3, 0.05);
        // Arms droop from hunched shoulders, forearms hanging in toward the body.
        pose.arm(
            side,
            Arm {
                down: 1.12 - 0.3 * struggle + 0.12 * kick,
                swing: 0.1,
                yaw: 0.0,
                twist: 0.7,
                bend: 0.65 + 0.3 * struggle,
                pronate: 0.0,
            },
        );
        pose.wrist(side, 0.35, 0.0, 0.0);
        pose.fingers(side, HandShape::RELAXED.mix(HandShape::OPEN, struggle));
        // The anime dangle: knees together, shins splayed out to the sides, toes pointed, one
        // knee a little higher. The knees turn in (negative twist), so the bent shins swing
        // outward where the viewer can see them. They trail the swing and kick when shaken.
        let uneven = if side == Side::Left { 0.18 } else { -0.06 };
        pose.leg(
            side,
            0.28 + uneven + 0.35 * kick,
            -0.05 + lag * alt,
            -0.6 + 0.3 * struggle,
            1.0 + uneven + 0.4 * (t * 14.0 + alt * PI * 0.5 + 1.0).sin().max(0.0) * struggle,
            0.75,
        );
    }
    // A start at being grabbed, then a resigned pout; alarm while shaken.
    let startled = (1.0 - held_for / 0.7).clamp(0.0, 1.0).max(struggle);
    let face = Face {
        surprised: 0.25 + 0.6 * startled,
        oh: 0.15 + 0.25 * startled,
        ..default()
    };
    (pose, face)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEG: f32 = 0.78;

    fn input(dx: f32, dt: f32, speed: f32, yaw: f32) -> GaitInput {
        GaitInput { dx, dt, speed, yaw, leg: LEG, half_width: 0.09, toe_out: 0.1, bounce: 1.0 }
    }

    #[test]
    fn motion_measures_window_travel_in_metres() {
        let mut motion = Motion::default();
        let (viewport, ppm, dt) = ((260, 370), 200.0, 1.0 / 30.0);
        motion.update(Some(IVec2::new(100, 100)), viewport, ppm, dt);
        assert_eq!(motion.delta, Vec2::ZERO, "the first sample has nothing to compare with");
        // 4 px right and 2 px down per frame.
        for frame in 1..=60 {
            motion.update(Some(IVec2::new(100 + frame * 4, 100 + frame * 2)), viewport, ppm, dt);
        }
        assert!((motion.delta - Vec2::new(0.02, -0.01)).length() < 1e-6);
        assert!((motion.velocity - Vec2::new(0.6, -0.3)).length() < 0.01, "{:?}", motion.velocity);
        let carried = 1.2 * SPRING_BONE_SHARE.x;
        assert!((motion.world.x - carried).abs() < 0.03, "the world offset follows: {:?}", motion.world);
    }

    #[test]
    fn teleports_and_resizes_are_not_movement() {
        let mut motion = Motion::default();
        let dt = 1.0 / 30.0;
        motion.update(Some(IVec2::new(0, 0)), (260, 370), 200.0, dt);
        motion.update(Some(IVec2::new(1500, 0)), (260, 370), 200.0, dt);
        assert_eq!(motion.delta, Vec2::ZERO, "a jump across the screen is a teleport");
        motion.update(Some(IVec2::new(1510, 0)), (520, 740), 200.0, dt);
        assert_eq!(motion.delta, Vec2::ZERO, "a resize moves the window without moving her");
        motion.update(Some(IVec2::new(1520, 0)), (520, 740), 200.0, dt);
        assert!(motion.delta.x > 0.0);
    }

    #[test]
    fn a_planted_foot_stays_on_its_spot_while_walking() {
        let mut gait = Gait::default();
        let (dt, speed) = (1.0 / 60.0, 0.9);
        let mut travelled = 0.0;
        let mut previous: [Option<f32>; 2] = [None; 2];
        let mut steps = 0;
        let mut was_planted = [true; 2];
        for _ in 0..600 {
            let dx = speed * dt;
            travelled += dx;
            let feet = gait.update(&input(dx, dt, speed, 1.3));
            for (i, foot) in feet.iter().enumerate() {
                // Position on the desktop: the body's travel plus the foot's place under it.
                let world = travelled + foot.ground.x;
                if foot.planted {
                    if let (Some(before), true) = (previous[i], was_planted[i]) {
                        assert!((world - before).abs() < 1e-4, "foot {i} slid by {}", world - before);
                    }
                    previous[i] = Some(world);
                } else if was_planted[i] {
                    steps += 1;
                }
                was_planted[i] = foot.planted;
            }
        }
        // 9 m at a ~0.85 m stride: about 21 steps in all.
        assert!((16..=26).contains(&steps), "took {steps} steps");
    }

    #[test]
    fn feet_alternate_and_never_both_leave_the_ground() {
        let mut gait = Gait::default();
        let dt = 1.0 / 45.0;
        let mut last_swinger = None;
        let mut order = Vec::new();
        for _ in 0..500 {
            let feet = gait.update(&input(0.7 * dt, dt, 0.7, -1.3));
            assert!(feet[0].planted || feet[1].planted, "both feet in the air while walking");
            let swinger = feet.iter().position(|f| !f.planted);
            if swinger != last_swinger {
                if let Some(i) = swinger {
                    order.push(i);
                }
                last_swinger = swinger;
            }
        }
        assert!(order.len() > 6);
        assert!(order.windows(2).all(|w| w[0] != w[1]), "feet must alternate: {order:?}");
    }

    #[test]
    fn stopping_brings_the_feet_home_and_then_stands_still() {
        let mut gait = Gait::default();
        let dt = 1.0 / 60.0;
        for _ in 0..200 {
            gait.update(&input(0.9 * dt, dt, 0.9, 1.3));
        }
        // Stop, and turn back to face the viewer.
        let mut feet = [FootPose::default(); 2];
        for _ in 0..180 {
            feet = gait.update(&input(0.0, dt, 0.0, 0.0));
        }
        for side in Side::BOTH {
            let foot = feet[side.index()];
            assert!(foot.planted && foot.height == 0.0 && foot.pitch == 0.0);
            let home = Gait::home(side, 0.0, 0.09);
            assert!(foot.ground.distance(home) < 0.07 * LEG + 1e-4, "{side:?} foot at {:?}", foot.ground);
        }
        let phase = gait.phase;
        gait.update(&input(0.0, dt, 0.0, 0.0));
        assert_eq!(gait.phase, phase, "nothing left to correct, so the cycle rests");
    }

    #[test]
    fn stride_and_cadence_both_grow_with_speed() {
        let slow = Gait::stride_for(0.3, LEG);
        let walk = Gait::stride_for(1.2, LEG);
        let jog = Gait::stride_for(2.4, LEG);
        assert!(slow < walk && walk < jog);
        assert!((walk / LEG - 1.25).abs() < 0.2, "a normal walk strides about 1.25 leg lengths");
        assert!(0.3 / slow < 1.2 / walk, "cadence rises too");
        assert_eq!(Gait::stride_for(0.0, LEG), 0.0);
        assert!(Gait::stride_for(50.0, LEG) <= 1.7 * LEG + 1e-6);
    }

    #[test]
    fn swing_has_clearance_and_a_soft_landing() {
        for i in 0..=1000 {
            let u = i as f32 / 1000.0;
            let height = foot_track(SWING_HEIGHT, u);
            assert!((0.0..=1.001).contains(&height), "foot cannot penetrate ground: {height}");
        }
        assert!(foot_track(SWING_HEIGHT, 0.4) > 0.95);
        assert!(foot_track(SWING_HEIGHT, 0.999) < 0.0001);
        assert!((foot_track(SWING_PITCH, 1.0) + 0.23).abs() < 1e-5);
    }

    #[test]
    fn stale_speed_cannot_keep_an_idle_foot_suspended() {
        let mut gait = Gait::default();
        for _ in 0..90 { gait.update(&input(0.9 / 60.0, 1.0 / 60.0, 0.9, 1.3)); }
        for _ in 0..240 { gait.update(&input(0.0, 1.0 / 60.0, 0.9, 0.0)); }
        let phase = gait.phase;
        let feet = gait.update(&input(0.0, 1.0 / 60.0, 0.9, 0.0));
        assert!(feet.iter().all(|f| f.planted && f.height == 0.0));
        assert_eq!(phase, gait.phase);
    }

    /// A bent leg seen from the front looks straight unless something moves sideways, so the air
    /// and held shapes are checked in the picture plane (x), not in depth.
    #[test]
    fn airborne_and_dangling_legs_read_from_the_front() {
        use crate::rig::{Bone, Rig};
        let rig = Rig::synthetic(&[]);
        let (held, _) = held_pose(&HeldInput { t: 0.0, held_for: 2.0, swing: 0.0, swing_rate: 0.0, struggle: 0.0 });
        let (falling, _) = air_pose(&AirInput { t: 0.0, vy: -4.0, airtime: 0.5 });
        for (name, pose, knees_out) in [("held", &held, false), ("falling", &falling, true)] {
            let fk = rig.fk(pose);
            for side in Side::BOTH {
                let outward = |bone: Bone| fk.pos[bone.on(side).index()].x * side.sign();
                let (hip, knee, ankle) = (outward(Bone::LeftUpperLeg), outward(Bone::LeftLowerLeg), outward(Bone::LeftFoot));
                if knees_out {
                    assert!(knee > hip + 0.08, "{name} {side:?}: knees should splay out ({hip} -> {knee})");
                } else {
                    assert!(knee < hip + 0.03, "{name} {side:?}: knees should stay together ({hip} -> {knee})");
                    assert!(ankle > knee + 0.08, "{name} {side:?}: shins should splay out ({knee} -> {ankle})");
                }
            }
        }
    }

    #[test]
    fn air_and_held_poses_are_finite_and_read_as_intended() {
        let (falling, face) = air_pose(&AirInput { t: 1.0, vy: -4.0, airtime: 0.6 });
        assert!(falling.rot.iter().all(|q| q.is_finite()));
        assert!(face.surprised > 0.9 && face.aa > 0.3, "a long fall is alarming");
        let (_, face) = air_pose(&AirInput { t: 1.0, vy: 3.0, airtime: 0.1 });
        assert!(face.aa == 0.0, "no scream on the way up");

        let (held, face) = held_pose(&HeldInput { t: 2.0, held_for: 3.0, swing: 0.3, swing_rate: 1.0, struggle: 0.0 });
        assert!(held.rot.iter().all(|q| q.is_finite()));
        assert!(face.surprised < 0.4, "she calms down once held for a while");
    }
}
