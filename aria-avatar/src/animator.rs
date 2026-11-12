//! The per-frame animation pass: turns "what she feels" and "how the window is moving" into one
//! pose.
//!
//! Layers, bottom to top:
//!
//! 1. **Emotion** ([`crate::emotes`]): upper body, face and what the feet add. Changing emotion
//!    cross-fades from a snapshot of the old pose with a slight overshoot.
//! 2. **Locomotion** ([`crate::locomotion`]): feet planted or stepping on the ground with leg IK,
//!    hips riding on them, arms swinging with the stride; or the falling / held pose when she is
//!    not on the ground.
//! 3. **Attention**: head and eyes following the cursor.
//! 4. **Follow-through**: springs for leaning into acceleration, the crouch of a landing and the
//!    pendulum swing of being carried. The model's own spring bones (hair, clothes) get theirs from
//!    the root being carried through the world by the window's real movement.

use bevy::prelude::*;

use aria_protocol::{BodyMode, PeekSide};

use crate::emotes::{self, Act, Ctx, Emote, FootAdjust, Style};
use crate::face::{FaceRig, Speech, CHANNELS};
use crate::ik;
use crate::locomotion::{air_pose, held_pose, AirInput, FootPose, Gait, GaitInput, HeldInput, Motion};
use crate::pose::{Arm, HandShape, Pose};
use crate::rig::{Bone, Rig, Side};
use crate::spring::{approach, approach_factor, ease_out_back, smoothstep, smootherstep, Spring};

/// How far she turns toward the direction she walks in. Short of a full profile, so the face stays
/// readable and she keeps half an eye on the viewer.
const WALK_YAW: f32 = 1.25;
/// Seconds an emotion takes to blend in.
const EMOTE_BLEND_SECS: f32 = 0.42;
/// Landing faster than this (m/s) leaves her seeing stars for a moment.
const HARD_LANDING: f32 = 6.5;
/// Her own tempo when no music sets one (dancing on request, humming).
const DEFAULT_BPM: f32 = 122.0;
/// Above this loudness the music gets a dance, below it a groove.
const DANCE_ENERGY: f32 = 0.5;

/// A measured beat from the app: tempo, and where in the beat the music is right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatSync {
    pub bpm: f32,
    /// 0 on the beat, rising to 1 just before the next.
    pub phase: f32,
}

/// Counts beats. Runs freely at its own tempo and is pulled (smoothly, never jumping) onto the
/// music's beat whenever a [`BeatSync`] arrives.
#[derive(Clone, Copy, Debug)]
pub struct BeatClock {
    pub beats: f64,
    pub bpm: f32,
    /// The tempo the music asked for last; she keeps it between measurements.
    target_bpm: f32,
    /// Phase correction still to be taken out, in beats.
    pending: f32,
}

impl Default for BeatClock {
    fn default() -> Self {
        Self { beats: 0.0, bpm: DEFAULT_BPM, target_bpm: DEFAULT_BPM, pending: 0.0 }
    }
}

impl BeatClock {
    /// Advances by `dt`. `music` says whether music is playing: without it she drifts back to her
    /// own tempo.
    pub fn step(&mut self, dt: f32, sync: Option<BeatSync>, music: bool) {
        if let Some(sync) = sync {
            self.target_bpm = sync.bpm.clamp(60.0, 200.0);
            // Phase error, wrapped to the nearer beat, measured against where the clock will be
            // once the corrections still pending are in.
            let have = (self.beats as f32 + self.pending).rem_euclid(1.0);
            self.pending += (sync.phase - have + 0.5).rem_euclid(1.0) - 0.5;
        }
        if !music {
            self.target_bpm = DEFAULT_BPM;
        }
        // Tempo eases toward the target (a jump would make every step lurch).
        self.bpm += (self.target_bpm - self.bpm) * crate::spring::approach_factor(1.5, dt);
        // The correction goes in over about a quarter of a second: a nudge, never a skip.
        let take = self.pending * crate::spring::approach_factor(4.0, dt);
        self.pending -= take;
        self.beats += (dt * self.bpm / 60.0 + take) as f64;
    }

    /// Beats as `f32`, for the emotes (kept small so `f32` keeps its precision).
    pub fn beat(&self) -> f32 {
        self.beats.rem_euclid(4096.0) as f32
    }
}

/// Everything the animator needs from the outside for one frame.
pub struct Input {
    pub emote: Emote,
    pub style: Style,
    pub mode: BodyMode,
    /// Smoothed cursor direction, `-1..1` (x toward screen right, y up).
    pub look: Vec2,
    pub speech: Speech,
    /// Loudness of the music she is listening to, when there is any.
    pub music: f32,
    /// The emotion is only "music is playing" (nothing else asked for), so she may dance or groove
    /// as the music's loudness suggests.
    pub music_fill: bool,
    /// A fresh beat measurement this frame.
    pub sync: Option<BeatSync>,
    /// 1 right after a head-pat, fading to 0.
    pub headpat: f32,
    pub peeking: PeekSide,
}

/// The result of one frame.
pub struct Frame {
    pub pose: Pose,
    /// Root rotation, and translation relative to the world offset in [`Motion::world`].
    pub root: Transform,
    pub expressions: [f32; CHANNELS.len()],
    /// Where the eyes point, `-1..1`.
    pub gaze: Vec2,
    /// How far the hips are below standing height; the camera follows this in cropped framings.
    pub drop: f32,
    /// Something is moving fast enough to deserve the higher frame rate.
    pub busy: bool,
}

/// Animation state that lives across frames.
#[derive(Resource)]
pub struct Animator {
    pub motion: Motion,
    /// Stands in for the real window position in scripted preview runs (see `crate::demo`).
    pub simulated_window: Option<IVec2>,
    gait: Gait,
    pub face: FaceRig,

    emote: Emote,
    emote_time: f32,
    /// Snapshot being blended away from, with the blend progress `0..1`.
    from: Option<Act>,
    blend: f32,
    last_act: Option<Act>,

    mode: BodyMode,
    mode_time: f32,
    ground: f32,
    air: f32,
    held: f32,

    /// Which way the body faces.
    yaw: Spring,
    facing: f32,
    /// Landing dip of the hips (negative = lower), in metres.
    crouch: Spring,
    /// Lean into acceleration while grounded.
    lean: Spring,
    /// Body swing when held or airborne.
    swing: Spring,
    /// Smoothed shift of the hips toward the supporting foot.
    shift: Vec2,
    /// Arms thrown out for balance after a landing, fading.
    landing: f32,
    /// Seconds of seeing stars left after a hard landing.
    stun: f32,
    /// Peak downward speed (m/s) of the fall in progress.
    fall_speed: f32,
    struggle: f32,
    talk: f32,
    drop: f32,
    beat: BeatClock,
    /// Music fill-in: dancing (true) or grooving, with hysteresis on the loudness.
    music_dancing: bool,
    /// The upper body as last shown, for the follow-through filter.
    shown: Option<Pose>,
}

impl Default for Animator {
    fn default() -> Self {
        Self {
            motion: Motion::default(),
            simulated_window: None,
            gait: Gait::default(),
            face: FaceRig::default(),
            emote: Emote::Idle,
            emote_time: 0.0,
            from: None,
            blend: 1.0,
            last_act: None,
            mode: BodyMode::Ground,
            mode_time: 0.0,
            ground: 1.0,
            air: 0.0,
            held: 0.0,
            yaw: Spring::default(),
            facing: 1.0,
            crouch: Spring::default(),
            lean: Spring::default(),
            swing: Spring::default(),
            shift: Vec2::ZERO,
            landing: 0.0,
            stun: 0.0,
            fall_speed: 0.0,
            struggle: 0.0,
            talk: 0.0,
            drop: 0.0,
            beat: BeatClock::default(),
            music_dancing: false,
            shown: None,
        }
    }
}

/// How quickly each bone follows its target in the follow-through filter (1/s). The trunk lags a
/// little, the arms a little more, the hands least: that ordering is what reads as weight and
/// follow-through. Legs, feet and hips are not filtered at all: the feet must stay exactly where
/// IK put them, or they would slide.
fn follow_rate(bone: Bone) -> Option<f32> {
    use Bone::*;
    match bone {
        Hips | LeftUpperLeg | RightUpperLeg | LeftLowerLeg | RightLowerLeg | LeftFoot | RightFoot | LeftToes
        | RightToes => None,
        Spine | Chest | UpperChest => Some(26.0),
        Neck | Head | Jaw | LeftEye | RightEye => Some(30.0),
        LeftShoulder | RightShoulder => Some(24.0),
        LeftUpperArm | RightUpperArm => Some(20.0),
        LeftLowerArm | RightLowerArm => Some(22.0),
        LeftHand | RightHand => Some(26.0),
        _ => Some(34.0),
    }
}

/// Stage axes (x toward screen right, y toward the viewer) to her own ground axes (x her left,
/// y forward), for a body turned by `yaw`.
fn to_local(yaw: f32, stage: Vec2) -> Vec2 {
    let (sin, cos) = yaw.sin_cos();
    Vec2::new(stage.x * cos - stage.y * sin, stage.x * sin + stage.y * cos)
}

/// Where one foot has to go, in model space.
struct FootTarget {
    ankle: Vec3,
    rotation: Quat,
    knee: Vec3,
    /// Heel-raise angle the toes must flatten out.
    toes: f32,
}

impl Animator {
    /// Forgets everything about the previous model.
    pub fn reset_for_model(&mut self) {
        let motion = std::mem::take(&mut self.motion);
        *self = Self { motion, simulated_window: self.simulated_window, ..Self::default() };
    }

    fn change_mode(&mut self, mode: BodyMode, leg: f32) {
        let previous = std::mem::replace(&mut self.mode, mode);
        self.mode_time = 0.0;
        if mode != BodyMode::Ground {
            self.fall_speed = 0.0;
            return;
        }
        // Touching down: the knees take the fall, arms go out, and a long drop dazes her.
        let impact = if previous == BodyMode::Air { self.fall_speed } else { 0.4 };
        self.crouch.kick(-(impact * 0.38).min(4.0 * leg));
        self.landing = self.landing.max((impact / 3.5).min(1.0));
        if impact > HARD_LANDING {
            self.stun = 1.8;
        }
        self.gait = Gait::default();
    }

    /// The emotion being acted out right now.
    #[cfg(test)]
    pub fn emote(&self) -> Emote {
        self.emote
    }

    /// Eases the upper body toward `target` (see [`follow_rate`]); legs and hips pass through.
    fn follow_through(&mut self, target: Pose, dt: f32) -> Pose {
        let Some(mut shown) = self.shown.take() else {
            self.shown = Some(target.clone());
            return target;
        };
        for bone in Bone::ALL {
            let i = bone.index();
            shown.rot[i] = match follow_rate(*bone) {
                Some(rate) => shown.rot[i].slerp(target.rot[i], approach_factor(rate, dt)).normalize(),
                None => target.rot[i],
            };
        }
        shown.hips = target.hips;
        self.shown = Some(shown.clone());
        shown
    }

    /// With music playing and nothing else going on: dance when it is loud, groove when it is
    /// soft, with a little hysteresis so a quiet bar does not flip her back and forth.
    fn music_emote(&mut self, input: &Input) -> Emote {
        if !input.music_fill {
            self.music_dancing = false;
            return input.emote;
        }
        let threshold = if self.music_dancing { DANCE_ENERGY - 0.1 } else { DANCE_ENERGY + 0.05 };
        self.music_dancing = input.music >= threshold;
        if self.music_dancing {
            Emote::Dance
        } else {
            Emote::Groove
        }
    }

    /// Evaluates the current emotion, cross-fading from the previous one.
    fn act(&mut self, rig: &Rig, input: &Input, t: f32, dt: f32) -> Act {
        self.stun = (self.stun - dt).max(0.0);
        let chosen = self.music_emote(input);
        let wanted = if self.stun > 0.0 { Emote::Dizzy } else { chosen };
        if wanted != self.emote {
            self.from = self.last_act.take();
            self.emote = wanted;
            self.emote_time = 0.0;
            self.blend = 0.0;
        }
        self.emote_time += dt;
        self.blend = (self.blend + dt / EMOTE_BLEND_SECS).min(1.0);
        self.beat.step(dt, input.sync, input.music_fill || input.sync.is_some());

        let ctx = Ctx {
            rig,
            t,
            e: self.emote_time,
            style: input.style,
            talk: self.talk,
            energy: input.music,
            beat: self.beat.beat(),
        };
        let mut act = emotes::act(self.emote, &ctx);
        if self.blend >= 1.0 {
            self.from = None;
        } else if let Some(from) = self.from.as_ref() {
            // Big moves (into a dance, a stretch, a sit) settle with a slight anime overshoot;
            // everything else eases in without one, which reads smoother.
            let curve = if self.emote.is_lively() { ease_out_back(self.blend) } else { smootherstep(self.blend) };
            let mut mixed = from.clone();
            mixed.blend(&act, curve);
            act = mixed;
        }
        self.last_act = Some(act.clone());
        act
    }

    /// Advances the animation by `dt` and returns the frame to show.
    pub fn step(&mut self, rig: &Rig, input: &Input, t: f32, dt: f32) -> Frame {
        let m = rig.metrics;
        let (leg, u) = (m.leg(), m.unit);
        if self.mode == BodyMode::Air {
            // The fastest she fell, not the speed at the instant the ground arrives: by then the
            // window has already stopped and the measured speed is collapsing.
            self.fall_speed = self.fall_speed.max(-self.motion.velocity.y);
        }
        if input.mode != self.mode {
            self.change_mode(input.mode, leg);
        }
        self.mode_time += dt;
        // Landing snaps in fast (the ground does not wait); leaving it eases.
        let toward = |value: f32, on: bool, rate: f32| approach(value, if on { 1.0 } else { 0.0 }, rate, dt);
        self.ground = toward(self.ground, input.mode == BodyMode::Ground, 22.0);
        self.air = toward(self.air, input.mode == BodyMode::Air, 14.0);
        self.held = toward(self.held, input.mode == BodyMode::Held, 14.0);
        let grounded = input.mode == BodyMode::Ground;

        self.talk = approach(self.talk, input.speech.aa.max(input.speech.oh), 12.0, dt);
        let velocity = self.motion.velocity;
        let pace = Gait::pace(velocity.x, leg);
        let walk = smoothstep(0.04, 0.22, pace) * self.ground;
        let run = smoothstep(0.7, 1.1, pace) * self.ground;

        // ---- Emotion ----------------------------------------------------------------------
        let mut act = self.act(rig, input, t, dt);
        // Nobody sits while walking or in mid-air.
        let sit = act.sit * self.ground * (1.0 - walk);

        // ---- Facing -----------------------------------------------------------------------
        if grounded && pace > 0.05 {
            self.facing = velocity.x.signum();
        }
        let attention = act.attention * (1.0 - 0.5 * walk);
        let yaw_target = match (input.peeking, input.mode) {
            (PeekSide::Left, _) => 0.38,
            (PeekSide::Right, _) => -0.38,
            (PeekSide::None, BodyMode::Ground) => {
                let turn = smoothstep(0.03, 0.16, pace);
                self.facing * WALK_YAW * turn + input.look.x * 0.18 * attention * (1.0 - turn)
            }
            (PeekSide::None, BodyMode::Air) => velocity.x.signum() * 0.45 * smoothstep(0.6, 2.5, velocity.x.abs()),
            (PeekSide::None, BodyMode::Held) => 0.0,
        };
        self.yaw.step(yaw_target, 10.5, 1.0, dt);
        let yaw = self.yaw.value;

        // ---- Follow-through springs -------------------------------------------------------
        self.crouch.step(0.0, 13.0, 0.5, dt);
        self.landing = approach(self.landing, 0.0, 4.5, dt);
        let lean_target = if grounded { (self.motion.accel.x * 0.022).clamp(-0.16, 0.16) } else { 0.0 };
        self.lean.step(lean_target, 9.0, 0.55, dt);
        match input.mode {
            BodyMode::Held => {
                // A pendulum hanging from the hand: the body trails the hand's acceleration.
                self.swing.kick(-self.motion.accel.x * 1.1 * dt);
                self.swing.step(0.0, 7.5, 0.22, dt);
                let shaken = (self.motion.accel.length() / 45.0).min(1.0);
                self.struggle = approach(self.struggle, shaken, if shaken > self.struggle { 14.0 } else { 1.6 }, dt);
            }
            BodyMode::Air => {
                self.swing.step((velocity.x * 0.1).clamp(-0.5, 0.5), 8.0, 0.6, dt);
                self.struggle = approach(self.struggle, 0.0, 3.0, dt);
            }
            BodyMode::Ground => {
                self.swing.step(0.0, 16.0, 0.8, dt);
                self.struggle = approach(self.struggle, 0.0, 3.0, dt);
            }
        }
        self.swing.value = self.swing.value.clamp(-1.1, 1.1);

        // ---- Upper body layers ------------------------------------------------------------
        let mut pose = std::mem::take(&mut act.pose);
        let mut face = act.face;

        // Feet first: the walk's upper body is driven by where they are.
        let half_width = (m.hip_half_width * 1.15 * act.stance).max(0.04 * u);
        let feet = self.gait.update(&GaitInput {
            dx: if grounded { self.motion.delta.x } else { 0.0 },
            dt,
            speed: if grounded { velocity.x } else { 0.0 },
            yaw,
            leg,
            half_width,
            toe_out: 0.13,
            bounce: input.style.bounce,
        });
        let local = [0, 1].map(|i| to_local(yaw, feet[i].ground));
        let (left, right) = (Side::Left.index(), Side::Right.index());

        if walk > 0.001 {
            // Pelvis follows the feet, shoulders counter it, arms swing against the legs.
            let across = local[left] - local[right];
            let twist = (-0.45 * across.y.atan2(across.x.max(0.05))).clamp(-0.3, 0.3) * walk;
            let roll = ((feet[left].height - feet[right].height) / leg * 0.9).clamp(-0.12, 0.12) * walk;
            // 0 with both feet down, 1 at the top of a step: drives the little bounces of a walk.
            let lift = ((feet[left].height.max(feet[right].height)) / (0.1 * leg)).min(1.0);
            pose.pelvis(0.0, twist, roll);
            pose.spine(0.05 * walk + 0.16 * run, -twist * 1.6, -roll * 0.9);
            // The head stays level and steady against the trunk, with a small nod each step.
            pose.head(0.035 * walk * (lift - 0.5), twist * 0.9, roll * 0.6);
            let lead = ((local[right].y - local[left].y) / leg * 1.35).clamp(-0.8, 0.8);
            let mut arms = Pose::default();
            for side in Side::BOTH {
                let swing = lead * side.sign();
                // Shoulders ride up a touch with each step.
                arms.shrug(side, 0.03 * lift * walk, 0.02 * swing);
                arms.arm(
                    side,
                    Arm {
                        down: 1.3 - 0.1 * run - 0.06 * swing.abs(),
                        swing,
                        yaw: 0.0,
                        twist: 0.3,
                        bend: 0.3 + 0.45 * swing.max(0.0) + 1.0 * run,
                        pronate: 0.0,
                    },
                );
                // Loose wrists trail the swing.
                arms.wrist(side, 0.1 - 0.22 * swing, 0.0, 0.0);
                arms.fingers(side, HandShape::RELAXED.mix(HandShape::LOOSE_FIST, run));
                let bones = emotes::arm_bones(side).chain([Bone::LeftShoulder.on(side)]);
                pose.blend_bones(&arms, bones, walk * act.walk_arms);
            }
        }
        // Leaning into a start and rocking back out of a stop, in whichever direction she faces.
        let lean = to_local(yaw, Vec2::new(self.lean.value * self.ground, 0.0));
        pose.spine(lean.y, 0.0, lean.x);
        // Turned away to walk, she still keeps her face toward the viewer a little.
        pose.head(0.0, -yaw * 0.3, 0.0);

        // Balance after a landing.
        if self.landing > 0.01 {
            let mut balance = Pose::default();
            for side in Side::BOTH {
                balance.arm(side, Arm { down: 0.7, swing: 0.2, yaw: 0.1, twist: 0.0, bend: 0.55, pronate: 0.0 });
                balance.fingers(side, HandShape::OPEN);
                pose.blend_bones(&balance, emotes::arm_bones(side), self.landing * 0.85 * self.ground);
            }
            pose.spine(0.18 * self.landing * self.ground, 0.0, 0.0);
        }

        // Off the ground the whole body belongs to the fall or the hand.
        if self.air > 0.002 {
            let (airborne, air_face) = air_pose(&AirInput { t, vy: velocity.y, airtime: self.mode_time });
            pose.blend(&airborne, self.air);
            face.blend(&air_face, self.air);
        }
        if self.held > 0.002 {
            let (hanging, held_face) = held_pose(&HeldInput {
                t,
                held_for: self.mode_time,
                swing: self.swing.value,
                swing_rate: self.swing.velocity,
                struggle: self.struggle,
            });
            pose.blend(&hanging, self.held);
            face.blend(&held_face, self.held);
        }

        // Attention: head and a little of the spine follow the cursor.
        pose.head(-input.look.y * 0.18 * attention, input.look.x * 0.3 * attention, 0.0);
        pose.spine(0.0, input.look.x * 0.06 * attention, 0.0);
        match input.peeking {
            PeekSide::Left => {
                pose.spine(0.0, 0.0, -0.14);
                pose.head(0.0, 0.0, -0.09);
            }
            PeekSide::Right => {
                pose.spine(0.0, 0.0, 0.14);
                pose.head(0.0, 0.0, 0.09);
            }
            PeekSide::None => {}
        }
        if input.headpat > 0.0 {
            let pat = input.headpat;
            pose.head(0.08 * pat, 0.0, (t * 7.5).sin() * 0.08 * pat);
            face.happy = face.happy.max(0.95 * pat);
            face.relaxed = face.relaxed.max(0.85 * pat);
            face.eyes_closed = face.eyes_closed.max(0.55 * pat);
        }

        // ---- Hips -------------------------------------------------------------------------
        let standing = rig.rest(Bone::Hips);
        let middle = (local[left] + local[right]) * 0.5;
        let support = match (feet[left].planted, feet[right].planted) {
            (true, false) => local[left],
            (false, true) => local[right],
            _ => middle,
        };
        // Weight rides between the feet and over whichever one is carrying her.
        let idle_sway = (t * 0.45).sin() * 0.012 * u * input.style.sway * (1.0 - walk);
        let shift = middle * 0.5 + (support - middle) * 0.3 + Vec2::new(idle_sway, 0.0);
        self.shift = self.shift.lerp(shift, crate::spring::approach_factor(10.0, dt));
        // Legs spread apart lower the hips, like the top of a pendulum: that is the walk's bob.
        let spread = (feet[left].ground.distance(feet[right].ground) * 0.5).min(leg * 0.9);
        let bob = (leg - (leg * leg - spread * spread).sqrt()) * input.style.bounce.clamp(0.7, 1.6);
        let dip = (-self.crouch.value).clamp(-0.02 * leg, 0.38 * leg);
        // Walking on soft knees: a straight-legged walk is what looks stiff. Running sinks more.
        let soft_knees = (0.028 * walk + 0.035 * run) * leg;
        let lowered = 0.012 * leg + bob + act.crouch + dip + soft_knees;

        // ---- Feet -------------------------------------------------------------------------
        let layout = (sit > 0.001).then(|| emotes::sit_layout(rig));
        let mut targets: [Option<FootTarget>; 2] = [None, None];
        let mut rise = 0.0;
        for side in Side::BOTH {
            let i = side.index();
            let adjust = act.feet[i];
            let (target, lifted) = foot_target(&m, side, &feet[i], local[i], yaw, &adjust, act.knees_in);
            rise += lifted * 0.5;
            targets[i] = Some(target);
        }
        let mut hips = standing
            + pose.hips
            + Vec3::new(self.shift.x, rise - lowered + act.hop, self.shift.y) * self.ground;
        for target in targets.iter_mut().flatten() {
            target.ankle.y += act.hop;
        }
        if let Some(layout) = layout.as_ref() {
            hips = hips.lerp(standing + layout.hips + pose.hips, sit);
            for side in Side::BOTH {
                let i = side.index();
                let target = targets[i].as_mut().expect("both feet have targets");
                target.ankle = target.ankle.lerp(layout.ankles[i], sit);
                let seated = Quat::from_rotation_x(-act.feet[i].toe);
                target.rotation = target.rotation.slerp(seated, sit);
                target.knee = target.knee.lerp(emotes::sit_knee_pole(side), sit).normalize_or(Vec3::Z);
                target.toes *= 1.0 - sit;
            }
        }
        // Straight legs cannot stretch: sink the hips until every foot can reach its spot.
        let hips_rotation = pose.rot[Bone::Hips.index()];
        let mut sink = 0.0f32;
        for side in Side::BOTH {
            let joint = hips + hips_rotation * (rig.rest(Bone::LeftUpperLeg.on(side)) - standing);
            let ankle = targets[side.index()].as_ref().expect("both feet have targets").ankle;
            let horizontal = Vec2::new(ankle.x - joint.x, ankle.z - joint.z).length_squared();
            let reach = leg * 0.998;
            let allowed = ankle.y + (reach * reach - horizontal).max(0.0).sqrt();
            sink = sink.max(joint.y - allowed);
        }
        hips.y -= sink * self.ground;
        pose.hips = hips - standing;

        // ---- Legs -------------------------------------------------------------------------
        let fk = rig.fk(&pose);
        for side in Side::BOTH {
            let target = targets[side.index()].take().expect("both feet have targets");
            let (upper, lower, foot, toes) = (
                Bone::LeftUpperLeg.on(side),
                Bone::LeftLowerLeg.on(side),
                Bone::LeftFoot.on(side),
                Bone::LeftToes.on(side),
            );
            let limb = ik::two_bone(
                fk.pos[upper.index()],
                target.ankle,
                rig.rest(lower) - rig.rest(upper),
                rig.rest(foot) - rig.rest(lower),
                Vec3::Z,
                target.knee,
            );
            let solved = [
                (upper, fk.frame[upper.index()].inverse() * limb.upper),
                (lower, limb.upper.inverse() * limb.lower),
                (foot, limb.lower.inverse() * target.rotation),
                (toes, Quat::from_rotation_x(-target.toes)),
            ];
            // In the air the legs keep the forward-kinematic pose already blended in.
            for (bone, rotation) in solved {
                let slot = &mut pose.rot[bone.index()];
                *slot = slot.slerp(rotation, self.ground);
            }
        }

        // ---- Follow-through ---------------------------------------------------------------
        // Every bone above the hips eases toward its target instead of snapping to it. That is
        // what takes the stiffness out: gesture changes, hand IK switching targets and dance
        // figures all flow, and arms trail the trunk a little.
        let pose = self.follow_through(pose, dt);

        // ---- Root -------------------------------------------------------------------------
        // Tilt about the chest when carried, about the middle of the body when flying.
        let tilt = Quat::from_rotation_z(self.swing.value * (self.air + self.held).min(1.0));
        let pivot = Vec3::Y * m.height * (0.55 * self.air + 0.8 * self.held) / (self.air + self.held).max(1e-3);
        let root = Transform {
            translation: pivot - tilt * pivot,
            rotation: tilt * Quat::from_rotation_y(yaw + act.spin),
            scale: Vec3::ONE,
        };

        // ---- Face -------------------------------------------------------------------------
        let gaze = (input.look * attention + act.gaze).clamp(Vec2::splat(-1.0), Vec2::splat(1.0));
        let expressions = self.face.update(&face, input.speech, gaze, dt);

        self.drop = approach(self.drop, (standing.y - hips.y).max(0.0) * self.ground, 9.0, dt);
        let busy = !grounded
            || pace > 0.03
            || self.emote.is_lively()
            || self.blend < 1.0
            || self.landing > 0.05
            || self.crouch.velocity.abs() > 0.05;
        Frame { pose, root, expressions, gaze, drop: self.drop, busy }
    }
}

/// Turns a gait foot plus the emotion's adjustment into an ankle target. Also returns how far the
/// ankle rose from rolling onto the ball of the foot, so the hips can rise with it.
fn foot_target(
    m: &crate::rig::Metrics,
    side: Side,
    foot: &FootPose,
    local: Vec2,
    body_yaw: f32,
    adjust: &FootAdjust,
    knees_in: f32,
) -> (FootTarget, f32) {
    let ground = local + adjust.offset;
    let yaw = crate::spring::wrap_angle(foot.yaw - body_yaw) + side.sign() * adjust.yaw;
    let facing = Quat::from_rotation_y(yaw);
    let forward = facing * Vec3::Z;
    // Positive pitch rolls the foot onto its ball, lifting the ankle up and forward; negative
    // raises the toes with the heel (and so the ankle) staying put.
    let pitch = foot.pitch + adjust.heel - adjust.toe;
    let heel = pitch.max(0.0);
    let (sin, cos) = heel.sin_cos();
    let (ball, height) = (m.ball_length, m.ankle_height);
    let along = ball - ball * cos + height * sin;
    let up = ball * sin + height * cos;
    let ankle = Vec3::new(ground.x, 0.0, ground.y) + forward * along + Vec3::Y * (up + foot.height + adjust.lift);
    // Knees follow the feet, a touch outward unless the emotion draws them together.
    let knee = (Quat::from_rotation_y(yaw * 0.7) * Vec3::Z + Vec3::X * side.sign() * (0.07 - 0.4 * knees_in))
        .normalize_or(Vec3::Z);
    (
        FootTarget { ankle, rotation: facing * Quat::from_rotation_x(pitch), knee, toes: heel },
        up - height,
    )
}

/// Camera placement for a framing preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    /// Height the camera looks at and sits at.
    pub target_y: f32,
    /// Distance from the model.
    pub distance: f32,
    /// Window pixels per metre at the model.
    pub pixels_per_metre: f32,
    /// Pixels from the bottom of the window up to the floor under her feet; 0 when the framing
    /// crops the floor away.
    pub ground_offset: f32,
    /// How much the camera follows the hips down when she sits or crouches: cropped framings
    /// would otherwise lose her off the bottom edge.
    pub follow: f32,
}

impl Framing {
    /// `full` shows the whole body with headroom for raised arms and a strip of floor to stand on;
    /// `half` is waist-up and `face` a close-up.
    pub fn new(preset: &str, height: f32, viewport_height: u32, fov: f32, zoom: f32, y_offset: f32) -> Framing {
        let (target, distance, follow) = match preset {
            "full" => (0.57, 2.62, 0.0),
            "face" => (0.88, 0.78, 1.0),
            _ => (0.72, 1.22, 1.0),
        };
        let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
        let distance = distance * height * zoom;
        let target_y = (target + y_offset) * height;
        let half_visible = distance * (fov * 0.5).tan();
        let pixels_per_metre = viewport_height as f32 * 0.5 / half_visible.max(1e-3);
        Framing {
            target_y,
            distance,
            pixels_per_metre,
            ground_offset: ((half_visible - target_y) * pixels_per_metre).max(0.0),
            follow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn input(emote: Emote, mode: BodyMode) -> Input {
        Input {
            emote,
            style: Style::default(),
            mode,
            look: Vec2::ZERO,
            speech: Speech::default(),
            music: 0.0,
            music_fill: false,
            sync: None,
            headpat: 0.0,
            peeking: PeekSide::None,
        }
    }

    #[test]
    fn the_beat_clock_locks_onto_the_music_without_jumping() {
        let mut clock = BeatClock::default();
        let dt = 1.0 / 60.0;
        // The music is at 100 BPM and its beat falls a quarter-beat away from ours.
        let mut music_beats = 0.25f64;
        let mut biggest_step = 0.0f64;
        for frame in 0..60 * 8 {
            music_beats += (dt * 100.0 / 60.0) as f64;
            let sync = (frame % 60 == 0).then(|| BeatSync { bpm: 100.0, phase: music_beats.rem_euclid(1.0) as f32 });
            let before = clock.beats;
            clock.step(dt, sync, true);
            biggest_step = biggest_step.max(clock.beats - before);
        }
        let error = ((clock.beats - music_beats).rem_euclid(1.0) + 0.5).rem_euclid(1.0) - 0.5;
        assert!(error.abs() < 0.03, "phase error after 8 s: {error}");
        assert!((clock.bpm - 100.0).abs() < 1.0, "tempo {}", clock.bpm);
        assert!(biggest_step < 0.08, "the clock never skips: {biggest_step} beats in one frame");
    }

    #[test]
    fn loud_music_gets_a_dance_and_soft_music_a_groove() {
        let mut animator = Animator::default();
        let mut music = input(Emote::Groove, BodyMode::Ground);
        music.music_fill = true;
        music.music = 0.8;
        assert_eq!(animator.music_emote(&music), Emote::Dance);
        music.music = 0.48;
        assert_eq!(animator.music_emote(&music), Emote::Dance, "hysteresis keeps her dancing through a quiet bar");
        music.music = 0.2;
        assert_eq!(animator.music_emote(&music), Emote::Groove);
        let asked = input(Emote::Sad, BodyMode::Ground);
        assert_eq!(animator.music_emote(&asked), Emote::Sad, "an explicit emotion beats the music");
    }

    #[test]
    fn follow_through_eases_the_arms_but_never_the_legs() {
        let mut animator = Animator::default();
        let rest = Pose::default();
        animator.follow_through(rest.clone(), DT);
        let mut moved = rest.clone();
        moved.arm(Side::Left, Arm { down: -0.5, ..Arm::HANG });
        moved.leg(Side::Left, 0.6, 0.0, 0.0, 0.4, 0.0);
        let shown = animator.follow_through(moved.clone(), DT);
        let (arm, leg) = (Bone::LeftUpperArm.index(), Bone::LeftUpperLeg.index());
        assert_eq!(shown.rot[leg], moved.rot[leg], "legs pass straight through");
        let left_to_go = shown.rot[arm].angle_between(moved.rot[arm]);
        assert!(left_to_go > 0.1, "the arm is still on its way: {left_to_go}");
        let mut settled = shown;
        for _ in 0..60 {
            settled = animator.follow_through(moved.clone(), DT);
        }
        assert!(settled.rot[arm].angle_between(moved.rot[arm]) < 0.01, "and arrives within a second");
    }

    /// Runs `frames` frames with the window moving at `speed` px/s (200 px per metre).
    fn run(animator: &mut Animator, rig: &Rig, input: &Input, frames: usize, speed: Vec2, clock: &mut f32, px: &mut Vec2) -> Frame {
        let mut frame = None;
        for _ in 0..frames {
            *px += speed * DT;
            animator.motion.update(Some(px.round().as_ivec2()), (260, 370), 200.0, DT);
            *clock += DT;
            frame = Some(animator.step(rig, input, *clock, DT));
        }
        frame.expect("at least one frame")
    }

    fn ankles(rig: &Rig, frame: &Frame) -> [Vec3; 2] {
        let fk = rig.fk(&frame.pose);
        [fk.pos[Bone::LeftFoot.index()], fk.pos[Bone::RightFoot.index()]]
    }

    #[test]
    fn standing_keeps_both_feet_flat_on_the_floor() {
        let rig = Rig::synthetic(&[]);
        let mut animator = Animator::default();
        let (mut clock, mut px) = (0.0, Vec2::ZERO);
        let frame = run(&mut animator, &rig, &input(Emote::Idle, BodyMode::Ground), 120, Vec2::ZERO, &mut clock, &mut px);
        for ankle in ankles(&rig, &frame) {
            assert!((ankle.y - rig.metrics.ankle_height).abs() < 2e-3, "ankle at {ankle:?}");
        }
        assert!(frame.pose.hips.y < 0.0 && frame.pose.hips.y > -0.05, "knees soft, not crouched: {}", frame.pose.hips.y);
        assert!(frame.root.rotation.angle_between(Quat::IDENTITY) < 0.05);
        assert!(!frame.busy);
    }

    #[test]
    fn walking_turns_her_toward_travel_and_stopping_turns_her_back() {
        let rig = Rig::synthetic(&[]);
        let mut animator = Animator::default();
        let (mut clock, mut px) = (0.0, Vec2::ZERO);
        let ground = input(Emote::Idle, BodyMode::Ground);
        // 160 px/s is 0.8 m/s.
        let walking = run(&mut animator, &rig, &ground, 120, Vec2::new(160.0, 0.0), &mut clock, &mut px);
        let (yaw, _, _) = walking.root.rotation.to_euler(EulerRot::YXZ);
        assert!(yaw > 1.0, "should face screen right, yaw {yaw}");
        assert!(walking.busy);
        // Neither foot goes below the floor, and the legs never over-stretch.
        for ankle in ankles(&rig, &walking) {
            assert!(ankle.y > rig.metrics.ankle_height - 5e-3, "ankle under the floor: {ankle:?}");
        }
        let left = run(&mut animator, &rig, &ground, 120, Vec2::new(-160.0, 0.0), &mut clock, &mut px);
        let (yaw, _, _) = left.root.rotation.to_euler(EulerRot::YXZ);
        assert!(yaw < -1.0, "should face screen left, yaw {yaw}");

        let stopped = run(&mut animator, &rig, &ground, 240, Vec2::ZERO, &mut clock, &mut px);
        let (yaw, _, _) = stopped.root.rotation.to_euler(EulerRot::YXZ);
        assert!(yaw.abs() < 0.05, "back to facing the viewer, yaw {yaw}");
        assert!(!stopped.busy);
    }

    #[test]
    fn landing_bends_the_knees_and_a_long_fall_dazes_her() {
        let rig = Rig::synthetic(&[]);
        let mut animator = Animator::default();
        let (mut clock, mut px) = (0.0, Vec2::ZERO);
        run(&mut animator, &rig, &input(Emote::Idle, BodyMode::Ground), 30, Vec2::ZERO, &mut clock, &mut px);
        let standing = animator.step(&rig, &input(Emote::Idle, BodyMode::Ground), clock, DT).pose.hips.y;

        // Fall at 3 m/s (600 px/s), then land.
        let falling = run(&mut animator, &rig, &input(Emote::Idle, BodyMode::Air), 40, Vec2::new(0.0, 600.0), &mut clock, &mut px);
        assert!(falling.expressions[4] > 0.5, "falling looks surprised");
        let mut lowest = 0.0f32;
        for _ in 0..40 {
            let frame = run(&mut animator, &rig, &input(Emote::Idle, BodyMode::Ground), 1, Vec2::ZERO, &mut clock, &mut px);
            lowest = lowest.min(frame.pose.hips.y);
            for ankle in ankles(&rig, &frame) {
                assert!(ankle.y > rig.metrics.ankle_height - 0.03, "feet stay on the floor through the landing");
            }
        }
        assert!(lowest < standing - 0.03, "the landing should dip the hips: {lowest} vs {standing}");
        assert_eq!(animator.emote(), Emote::Idle, "a short drop does not daze her");

        run(&mut animator, &rig, &input(Emote::Idle, BodyMode::Air), 60, Vec2::new(0.0, 1600.0), &mut clock, &mut px);
        run(&mut animator, &rig, &input(Emote::Idle, BodyMode::Ground), 2, Vec2::ZERO, &mut clock, &mut px);
        assert_eq!(animator.emote(), Emote::Dizzy, "an 8 m/s landing leaves her dizzy");
    }

    #[test]
    fn off_the_ground_the_legs_bend_instead_of_standing() {
        let rig = Rig::synthetic(&[]);
        // Falling at 2.5 m/s (500 px/s at 200 px per metre); dangling from a still cursor.
        for (mode, window) in [(BodyMode::Air, Vec2::new(0.0, 500.0)), (BodyMode::Held, Vec2::ZERO)] {
            let mut animator = Animator::default();
            let (mut clock, mut px) = (0.0, Vec2::ZERO);
            run(&mut animator, &rig, &input(Emote::Idle, BodyMode::Ground), 30, Vec2::ZERO, &mut clock, &mut px);
            let frame = run(&mut animator, &rig, &input(Emote::Idle, mode), 60, window, &mut clock, &mut px);
            let fk = rig.fk(&frame.pose);
            for side in Side::BOTH {
                let hip = fk.pos[Bone::LeftUpperLeg.on(side).index()];
                let knee = fk.pos[Bone::LeftLowerLeg.on(side).index()];
                let ankle = fk.pos[Bone::LeftFoot.on(side).index()];
                assert!(knee.z > hip.z + 0.05, "{mode:?} {side:?}: the thigh should come forward");
                assert!(hip.y - ankle.y < rig.metrics.leg() * 0.95, "{mode:?} {side:?}: the leg should not be straight");
            }
        }
    }

    #[test]
    fn every_emotion_in_every_mode_gives_a_valid_frame() {
        for missing in [vec![], vec![Bone::Chest, Bone::UpperChest, Bone::Neck, Bone::LeftToes, Bone::RightToes]] {
            let rig = Rig::synthetic(&missing);
            for name in aria_protocol::EMOTIONS {
                for mode in [BodyMode::Ground, BodyMode::Air, BodyMode::Held] {
                    let mut animator = Animator::default();
                    let (mut clock, mut px) = (0.0, Vec2::ZERO);
                    let frame = run(&mut animator, &rig, &input(Emote::parse(name), mode), 45, Vec2::new(90.0, 30.0), &mut clock, &mut px);
                    assert!(frame.pose.rot.iter().all(|q| q.is_finite()), "{name} {mode:?}");
                    assert!(frame.pose.hips.is_finite() && frame.root.translation.is_finite(), "{name} {mode:?}");
                    assert!(frame.expressions.iter().all(|w| (0.0..=1.0).contains(w)), "{name} {mode:?}");
                }
            }
        }
    }

    #[test]
    fn sitting_lowers_the_hips_and_keeps_the_feet_down() {
        let rig = Rig::synthetic(&[]);
        let mut animator = Animator::default();
        let (mut clock, mut px) = (0.0, Vec2::ZERO);
        let frame = run(&mut animator, &rig, &input(Emote::Sit, BodyMode::Ground), 120, Vec2::ZERO, &mut clock, &mut px);
        assert!(frame.pose.hips.y < -0.5, "hips should be near the floor: {}", frame.pose.hips.y);
        assert!(frame.drop > 0.5);
        for ankle in ankles(&rig, &frame) {
            assert!((ankle.y - rig.metrics.ankle_height).abs() < 0.02, "ankle at {ankle:?}");
        }
        // Walking gets her back on her feet.
        let frame = run(&mut animator, &rig, &input(Emote::Sit, BodyMode::Ground), 90, Vec2::new(160.0, 0.0), &mut clock, &mut px);
        assert!(frame.pose.hips.y > -0.2, "she stands up to walk: {}", frame.pose.hips.y);
    }

    #[test]
    fn full_framing_leaves_floor_to_stand_on_and_cropped_framings_none() {
        let fov = 28f32.to_radians();
        let full = Framing::new("full", 1.6, 370, fov, 1.0, 0.0);
        assert!(full.ground_offset > 10.0 && full.ground_offset < 40.0, "{}", full.ground_offset);
        // The top of the head and a raised hand fit.
        let top = full.target_y + 185.0 / full.pixels_per_metre;
        assert!(top > 1.6 * 1.15, "headroom up to {top} m");
        let half = Framing::new("half", 1.6, 370, fov, 1.0, 0.0);
        assert_eq!(half.ground_offset, 0.0);
        assert!(half.pixels_per_metre > full.pixels_per_metre * 1.8);
        assert_eq!((full.follow, half.follow), (0.0, 1.0));
    }
}
