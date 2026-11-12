//! Full-body emotions.
//!
//! Each emotion is a function from time to an [`Act`]: the upper-body pose, the face, where the
//! eyes go, and what the feet and knees add on top of the animator's standing or walking legs.
//! Nothing here is keyframed. Hands are placed with IK on landmarks measured off the loaded model
//! (chin, eyes, hips, knees), and every distance is in the model's own units, so the same code
//! reads on a tall adult rig and on a chibi.
//!
//! The style is anime: big clear silhouettes, an anticipation or hop on entry, rhythmic secondary
//! motion (a sob, a foot tap, a laugh shaking the shoulders), and the persona tints how lively the
//! idle is.
//!
//! | Module | What |
//! |---|---|
//! | this one | the types, the building blocks, and the feelings (happy, sad, thinking, …) |
//! | [`moments`] | idle moments she does on her own: stretch, look around, hum, twirl, fix her hair, clap |
//! | [`dance`] | the dance library and the groove, on the music's beat |

mod dance;
mod moments;

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;

use crate::face::Face;
use crate::ik;
use crate::pose::{hand_aim, Arm, HandShape, Landmarks, Pose};
use crate::rig::{Bone, Rig, Side};
use crate::spring::{bump, smoothstep};

/// The emotions the app can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Emote {
    #[default]
    Idle,
    Talking,
    Happy,
    Laugh,
    Angry,
    Annoyed,
    Sad,
    Cry,
    Thinking,
    Surprised,
    Shy,
    Dizzy,
    Sleepy,
    Dance,
    Groove,
    Sit,
    Wave,
    Stretch,
    LookAround,
    Hum,
    Twirl,
    FixHair,
    Clap,
}

impl Emote {
    /// Every emote, in the order of `aria_protocol::EMOTIONS`.
    #[cfg(test)]
    pub const ALL: [Emote; 23] = [
        Emote::Idle, Emote::Talking, Emote::Happy, Emote::Laugh, Emote::Angry, Emote::Annoyed,
        Emote::Sad, Emote::Cry, Emote::Thinking, Emote::Surprised, Emote::Shy, Emote::Dizzy,
        Emote::Sleepy, Emote::Dance, Emote::Groove, Emote::Sit, Emote::Wave, Emote::Stretch,
        Emote::LookAround, Emote::Hum, Emote::Twirl, Emote::FixHair, Emote::Clap,
    ];

    /// Parses the wire name. `walking` was an emotion in older builds; walking now comes from
    /// actually moving, so it and anything unknown mean "no particular emotion".
    pub fn parse(name: &str) -> Emote {
        match name.trim().to_ascii_lowercase().as_str() {
            "talking" => Emote::Talking,
            "happy" => Emote::Happy,
            "laugh" | "laughing" => Emote::Laugh,
            "angry" => Emote::Angry,
            "annoyed" | "pout" => Emote::Annoyed,
            "sad" => Emote::Sad,
            "cry" | "crying" => Emote::Cry,
            "thinking" | "think" => Emote::Thinking,
            "surprised" | "surprise" => Emote::Surprised,
            "shy" | "love" => Emote::Shy,
            "dizzy" => Emote::Dizzy,
            "sleepy" => Emote::Sleepy,
            "dance" => Emote::Dance,
            "groove" => Emote::Groove,
            "sit" => Emote::Sit,
            "wave" => Emote::Wave,
            "stretch" => Emote::Stretch,
            "look_around" | "look around" => Emote::LookAround,
            "hum" | "humming" => Emote::Hum,
            "twirl" | "spin" => Emote::Twirl,
            "fix_hair" | "fix hair" => Emote::FixHair,
            "clap" | "clapping" => Emote::Clap,
            _ => Emote::Idle,
        }
    }

    /// Emotions busy enough to deserve the higher frame rate.
    pub fn is_lively(self) -> bool {
        matches!(
            self,
            Emote::Dance
                | Emote::Groove
                | Emote::Dizzy
                | Emote::Laugh
                | Emote::Happy
                | Emote::Surprised
                | Emote::Wave
                | Emote::Angry
                | Emote::Twirl
                | Emote::Clap
                | Emote::Stretch
        )
    }
}

/// The persona's idle habit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Habit {
    /// Arms relaxed at the sides.
    #[default]
    Relaxed,
    /// One hand on the hip, chin up.
    HandOnHip,
    /// Both hands on the hips, feet apart.
    HandsOnHips,
    /// Hands clasped behind the back, head tilted.
    HandsBehind,
    /// Hands folded in front.
    HandsFolded,
    /// Fists up at the chest, bouncing on her toes.
    Pumped,
    /// Loose arms that swing with a big sway.
    Swingy,
}

/// How the current persona moves.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub habit: Habit,
    /// Gesture size and tempo, about 0.6 (reserved) to 1.5 (hyper).
    pub energy: f32,
    /// Idle side-to-side sway.
    pub sway: f32,
    /// Vertical bounce in idle and walking.
    pub bounce: f32,
    /// Stance width multiplier.
    pub stance: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style { habit: Habit::Relaxed, energy: 1.0, sway: 1.0, bounce: 1.0, stance: 1.0 }
    }
}

impl Style {
    /// The style for one of the app's personas; anything unknown gets the neutral style.
    pub fn for_persona(persona: &str) -> Style {
        let (habit, energy, sway, bounce, stance) = match persona.trim().to_ascii_lowercase().as_str() {
            "tsundere" => (Habit::HandOnHip, 1.0, 0.8, 0.9, 1.1),
            "yandere" => (Habit::HandsBehind, 0.8, 1.5, 0.7, 0.85),
            "tomboy" => (Habit::HandsOnHips, 1.2, 1.0, 1.2, 1.5),
            "bakadere" => (Habit::Swingy, 1.25, 1.9, 1.4, 1.0),
            "kuudere" => (Habit::Relaxed, 0.6, 0.35, 0.55, 0.9),
            "genki" => (Habit::Pumped, 1.5, 1.2, 1.7, 1.15),
            "meganekko" => (Habit::HandsFolded, 0.85, 0.6, 0.8, 0.85),
            _ => return Style::default(),
        };
        Style { habit, energy, sway, bounce, stance }
    }
}

/// What an emotion adds to one foot while she stands.
#[derive(Clone, Copy, Debug, Default)]
pub struct FootAdjust {
    /// Shift on the ground in her own frame: x toward her left, y forward (metres).
    pub offset: Vec2,
    /// Whole foot off the ground (metres).
    pub lift: f32,
    /// Heel raised with the ball of the foot planted (radians).
    pub heel: f32,
    /// Toes raised with the heel planted (radians).
    pub toe: f32,
    /// Extra toe-out (negative turns the toes in).
    pub yaw: f32,
}

impl FootAdjust {
    fn blend(&mut self, other: &FootAdjust, t: f32) {
        self.offset = self.offset.lerp(other.offset, t);
        self.lift += (other.lift - self.lift) * t;
        self.heel += (other.heel - self.heel) * t;
        self.toe += (other.toe - self.toe) * t;
        self.yaw += (other.yaw - self.yaw) * t;
    }
}

/// Everything one emotion asks of the body this frame.
#[derive(Clone)]
pub struct Act {
    pub pose: Pose,
    pub face: Face,
    /// Where the eyes drift on their own, `-1..1` (x toward screen right, y up).
    pub gaze: Vec2,
    /// How much the cursor steers head and eyes: 1 follows it, 0 ignores it.
    pub attention: f32,
    pub feet: [FootAdjust; 2],
    /// Extra knee bend: the hips sink by this much (metres).
    pub crouch: f32,
    /// The whole body off the ground (metres), for hops.
    pub hop: f32,
    /// 0 standing … 1 sitting on the floor.
    pub sit: f32,
    /// Extra turn of the whole body (radians), for spins.
    pub spin: f32,
    /// How much of the walking arm swing replaces this pose's arms while she moves.
    pub walk_arms: f32,
    /// Knees drawn together (1) or apart (-1).
    pub knees_in: f32,
    /// Stance width multiplier.
    pub stance: f32,
}

impl Default for Act {
    fn default() -> Self {
        Act {
            pose: Pose::default(),
            face: Face::default(),
            gaze: Vec2::ZERO,
            attention: 1.0,
            feet: [FootAdjust::default(); 2],
            crouch: 0.0,
            hop: 0.0,
            sit: 0.0,
            spin: 0.0,
            walk_arms: 1.0,
            knees_in: 0.0,
            stance: 1.0,
        }
    }
}

impl Act {
    /// Blends toward `other`; `t` may overshoot 1 for the pose (see [`Pose::blend`]), everything
    /// else is clamped so a face or a foot never extrapolates.
    pub fn blend(&mut self, other: &Act, t: f32) {
        self.pose.blend(&other.pose, t);
        let t = t.clamp(0.0, 1.0);
        self.face.blend(&other.face, t);
        self.gaze = self.gaze.lerp(other.gaze, t);
        for (mine, theirs) in self.feet.iter_mut().zip(other.feet.iter()) {
            mine.blend(theirs, t);
        }
        let mix = |a: &mut f32, b: f32| *a += (b - *a) * t;
        mix(&mut self.attention, other.attention);
        mix(&mut self.crouch, other.crouch);
        mix(&mut self.hop, other.hop);
        mix(&mut self.sit, other.sit);
        mix(&mut self.spin, other.spin);
        mix(&mut self.walk_arms, other.walk_arms);
        mix(&mut self.knees_in, other.knees_in);
        mix(&mut self.stance, other.stance);
    }
}

/// Inputs shared by every emotion.
pub struct Ctx<'a> {
    pub rig: &'a Rig,
    /// Seconds since the renderer started; drives the continuous rhythms.
    pub t: f32,
    /// Seconds since this emotion began; drives entries (a hop, a spin, a stomp).
    pub e: f32,
    pub style: Style,
    /// How loudly she is speaking right now, `0..1`.
    pub talk: f32,
    /// Loudness of the music she is grooving to, `0..1`.
    pub energy: f32,
    /// Beats counted by the animator's beat clock: whole numbers fall on the music's beats (or on
    /// her own tempo when nothing is playing). Dances and the groove run on this, not on `t`.
    pub beat: f32,
    /// Smoothed tempo; scales the choreography and the listening expression.
    pub bpm: f32,
}

impl Ctx<'_> {
    /// The model's size relative to a 1.6 m figure.
    fn u(&self) -> f32 {
        self.rig.metrics.unit
    }
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

/// Half-wave rectified sine: the positive humps only.
fn pulse(x: f32) -> f32 {
    x.sin().max(0.0)
}

/// A point in front of the chest, relative to the midpoint between the shoulders.
fn chest(lm: &Landmarks, local: Vec3) -> Vec3 {
    let mid = (lm.pos(Bone::LeftUpperArm) + lm.pos(Bone::RightUpperArm)) * 0.5;
    mid + lm.rot(Bone::UpperChest) * local
}

/// `v` with its x component mirrored for the right side.
fn sx(side: Side, x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x * side.sign(), y, z)
}

/// Brings both hands together in front of the chest (`apart` is the gap between the wrists),
/// palms facing each other: a clap, a prayer, a heart shape when the fingers curl.
fn hands_together(pose: &mut Pose, ctx: &Ctx, height: f32, forward: f32, apart: f32, shape: HandShape) {
    let u = ctx.u();
    let lm = Landmarks::of(ctx.rig, pose);
    for side in Side::BOTH {
        let target = chest(&lm, sx(side, (0.012 + apart * 0.5) * u, height * u, forward * u));
        let hand = hand_aim(side, sx(side, -0.25, 0.95, 0.15), sx(side, -1.0, 0.0, 0.0));
        pose.reach(ctx.rig, side, target, sx(side, 0.75, -0.6, -0.2), Some(hand));
        pose.fingers(side, shape);
    }
}

/// Evaluates an emotion.
pub fn act(emote: Emote, ctx: &Ctx) -> Act {
    match emote {
        Emote::Idle => idle(ctx),
        Emote::Talking => talking(ctx),
        Emote::Happy => happy(ctx),
        Emote::Laugh => laugh(ctx),
        Emote::Angry => angry(ctx),
        Emote::Annoyed => annoyed(ctx),
        Emote::Sad => sad(ctx),
        Emote::Cry => cry(ctx),
        Emote::Thinking => thinking(ctx),
        Emote::Surprised => surprised(ctx),
        Emote::Shy => shy(ctx),
        Emote::Dizzy => dizzy(ctx),
        Emote::Sleepy => sleepy(ctx),
        Emote::Dance => dance::dance(ctx),
        Emote::Groove => dance::groove(ctx),
        Emote::Sit => sit(ctx),
        Emote::Wave => wave(ctx),
        Emote::Stretch => moments::stretch(ctx),
        Emote::LookAround => moments::look_around(ctx),
        Emote::Hum => moments::hum(ctx),
        Emote::Twirl => moments::twirl(ctx),
        Emote::FixHair => moments::fix_hair(ctx),
        Emote::Clap => moments::clap(ctx),
    }
}

// ---- Building blocks ----------------------------------------------------------------------------

/// Standing at ease: breathing, a slow sway, arms hanging. Every emotion starts from this.
fn neutral(ctx: &Ctx) -> Act {
    let mut act = Act::default();
    let t = ctx.t;
    let breath = (t * 1.9).sin();
    let sway = (t * 0.85).sin() * ctx.style.sway;
    let pose = &mut act.pose;
    pose.spine(0.015 + breath * 0.012, 0.0, sway * 0.014);
    pose.pelvis(0.0, 0.0, -sway * 0.012);
    pose.head(-breath * 0.008, 0.0, sway * 0.018 + (t * 0.53).sin() * 0.012);
    for side in Side::BOTH {
        pose.shrug(side, 0.01 + breath * 0.014, 0.0);
        pose.arm(side, Arm { down: Arm::HANG.down - breath * 0.012, ..Arm::HANG });
        pose.wrist(side, 0.08, 0.0, 0.0);
        pose.fingers(side, HandShape::RELAXED.tighten((t * 0.7 + side.sign()).sin() * 0.03));
    }
    act.stance = ctx.style.stance;
    act
}

/// Rests a hand on the hip, elbow out.
fn hand_on_hip(pose: &mut Pose, ctx: &Ctx, side: Side) {
    let (m, u) = (&ctx.rig.metrics, ctx.u());
    let lm = Landmarks::of(ctx.rig, pose);
    let target = lm.on(Bone::Hips, sx(side, m.hip_half_width * 1.8 + 0.035 * u, 0.075 * u, 0.015 * u));
    let hand = hand_aim(
        side,
        lm.dir(Bone::Hips, sx(side, -0.3, -0.8, 0.5)),
        lm.dir(Bone::Hips, sx(side, -1.0, 0.1, -0.15)),
    );
    pose.reach(ctx.rig, side, target, sx(side, 1.0, -0.1, -0.75), Some(hand));
    pose.fingers(side, HandShape::RELAXED.tighten(0.08));
}

/// Brings the wrist to a point on the face (`local` is in head space), palm toward the face.
fn hand_to_face(pose: &mut Pose, ctx: &Ctx, side: Side, local: Vec3, fingers: Vec3, elbow: Vec3) {
    let lm = Landmarks::of(ctx.rig, pose);
    let hand = hand_aim(side, lm.dir(Bone::Head, fingers), lm.dir(Bone::Head, Vec3::new(0.0, 0.1, -1.0)));
    pose.reach(ctx.rig, side, lm.on(Bone::Head, local), elbow, Some(hand));
}

// ---- Emotions -----------------------------------------------------------------------------------

fn idle(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, u) = (ctx.t, ctx.u());
    match ctx.style.habit {
        Habit::Relaxed => {}
        Habit::HandOnHip => {
            // Weight on one leg, a hand on the hip, chin up and turned a little away.
            act.pose.pelvis(0.0, 0.0, 0.05);
            act.pose.spine(-0.02, -0.04, -0.03);
            act.pose.head(-0.06, -0.1, 0.04);
            act.pose.hips.x += 0.018 * u;
            hand_on_hip(&mut act.pose, ctx, Side::Left);
        }
        Habit::HandsOnHips => {
            act.pose.spine(-0.03, 0.0, 0.0);
            act.pose.head(-0.04, 0.0, 0.0);
            for side in Side::BOTH {
                hand_on_hip(&mut act.pose, ctx, side);
            }
        }
        Habit::HandsBehind => {
            act.pose.spine(0.07, 0.0, 0.0);
            act.pose.head(-0.02, 0.0, 0.13 + (t * 0.6).sin() * 0.03);
            let lm = Landmarks::of(ctx.rig, &act.pose);
            for side in Side::BOTH {
                let target = lm.on(Bone::Hips, sx(side, 0.045 * u, 0.035 * u, -0.135 * u));
                let hand = hand_aim(side, sx(side, -0.8, -0.55, 0.0), Vec3::NEG_Z);
                act.pose.reach(ctx.rig, side, target, sx(side, 1.0, -0.15, -0.35), Some(hand));
                act.pose.fingers(side, HandShape::RELAXED);
            }
        }
        Habit::HandsFolded => {
            act.pose.spine(-0.015, 0.0, 0.0);
            let lm = Landmarks::of(ctx.rig, &act.pose);
            for side in Side::BOTH {
                let target = lm.on(Bone::Hips, sx(side, 0.04 * u, -0.035 * u, 0.125 * u));
                let hand = hand_aim(side, sx(side, -0.65, -0.7, 0.25), Vec3::new(0.0, 0.15, -1.0));
                act.pose.reach(ctx.rig, side, target, sx(side, 0.8, -0.5, -0.5), Some(hand));
                act.pose.fingers(side, HandShape::RELAXED.tighten(-0.1));
            }
        }
        Habit::Pumped => {
            // Fists up, bouncing on her toes.
            let bounce = (t * 5.6).sin() * 0.5 + 0.5;
            let lm = Landmarks::of(ctx.rig, &act.pose);
            for side in Side::BOTH {
                let lift = 0.012 * u * (t * 5.6 + side.sign()).sin();
                let target = chest(&lm, sx(side, 0.125 * u, -0.045 * u + lift, 0.155 * u));
                let hand = hand_aim(side, sx(side, -0.2, 0.95, 0.2), sx(side, -1.0, 0.0, -0.2));
                act.pose.reach(ctx.rig, side, target, sx(side, 0.5, -1.0, -0.15), Some(hand));
                act.pose.fingers(side, HandShape::FIST);
                act.feet[side.index()].heel = bounce * 0.22;
            }
            act.crouch = 0.008 * u * (1.0 - bounce);
        }
        Habit::Swingy => {
            let swing = (t * 2.1).sin();
            act.pose.head(0.0, 0.0, swing * 0.07);
            act.pose.spine(0.0, swing * 0.05, 0.0);
            for side in Side::BOTH {
                let arm = Arm { swing: swing * 0.2 * side.sign(), down: 1.25, ..Arm::HANG };
                act.pose.arm(side, arm);
            }
        }
    }
    act
}

fn talking(ctx: &Ctx) -> Act {
    let mut act = idle(ctx);
    let t = ctx.t;
    // With no voice (speech bubbles only) she still gestures a little.
    let voice = ctx.talk.max(0.35) * ctx.style.energy.clamp(0.7, 1.3);
    let beat = (t * 2.4).sin() * voice;
    // Drift between gesturing with one hand and with both.
    let both = 0.5 * smoothstep(-0.2, 0.5, (t * 0.47).sin());
    act.pose.head(beat * 0.03, 0.0, (t * 1.1).sin() * 0.03);
    act.pose.spine(0.01 + beat * 0.012, (t * 0.8).sin() * 0.03, 0.0);

    let mut gesture = Pose::default();
    for side in Side::BOTH {
        let phase = if side == Side::Right { 0.0 } else { 1.3 };
        let wave = (t * 2.4 + phase).sin() * voice;
        let present = smoothstep(-0.7, 0.7, (t * 0.8 + phase).sin());
        gesture.arm(
            side,
            Arm {
                down: 1.16 - 0.16 * present,
                swing: 0.42 + wave * 0.08,
                yaw: 0.0,
                twist: 0.12,
                bend: 0.85 + 0.45 * present + wave * 0.1,
                pronate: -0.65 * present,
            },
        );
        gesture.wrist(side, -0.08 + (t * 2.4 + phase - 0.5).sin() * voice * 0.08, 0.05 * wave, 0.0);
        gesture.fingers(side, HandShape::RELAXED.mix(HandShape::OPEN, 0.4 * present));
        let weight = if side == Side::Right { 1.0 } else { both };
        act.pose.blend_bones(&gesture, arm_bones(side), weight);
    }
    act.walk_arms = 0.6;
    act
}

fn happy(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, e, u) = (ctx.t, ctx.e, ctx.u());
    let tempo = 5.4 * ctx.style.energy.clamp(0.8, 1.25);
    let bounce = (t * tempo).sin() * 0.5 + 0.5;
    let tilt = (t * tempo * 0.5).sin();
    act.pose.spine(-0.04, 0.0, tilt * 0.045);
    act.pose.head(-0.05, 0.0, tilt * 0.13);
    let lm = Landmarks::of(ctx.rig, &act.pose);
    for side in Side::BOTH {
        act.pose.shrug(side, 0.07, 0.05);
        // Fists tucked up under the chin.
        let target = chest(&lm, sx(side, 0.075 * u, -0.005 * u + bounce * 0.012 * u, 0.15 * u));
        let hand = hand_aim(side, sx(side, -0.35, 0.9, 0.1), sx(side, -0.85, 0.0, -0.5));
        act.pose.reach(ctx.rig, side, target, sx(side, 0.4, -1.0, -0.1), Some(hand));
        act.pose.fingers(side, HandShape::LOOSE_FIST);
        act.feet[side.index()].heel = bounce * 0.3;
    }
    // A little jump for joy on the way in.
    act.hop = 0.075 * u * bump(e / 0.42);
    act.crouch = 0.012 * u * (1.0 - bounce);
    act.face = Face { happy: 1.0, ..default() };
    act.walk_arms = 0.65;
    act
}

fn laugh(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, u, m) = (ctx.t, ctx.u(), &ctx.rig.metrics);
    let shake = (t * 13.0).sin();
    // Rocks back with the laugh, then folds forward.
    let fold = pulse(t * 1.7);
    act.pose.spine(-0.1 + 0.16 * fold, 0.0, (t * 2.1).sin() * 0.03);
    act.pose.head(-0.16 + 0.2 * fold, 0.0, (t * 2.1).sin() * 0.06);
    for side in Side::BOTH {
        act.pose.shrug(side, 0.06 + 0.035 * shake, 0.04);
    }
    // One hand in front of the mouth, the other on the belly.
    hand_to_face(
        &mut act.pose,
        ctx,
        Side::Right,
        m.mouth + Vec3::new(-0.035 * u, -0.045 * u, 0.045 * u),
        Vec3::new(0.55, 0.8, 0.1),
        Vec3::new(-0.6, -1.0, 0.1),
    );
    act.pose.fingers(Side::Right, HandShape::FLAT.tighten(0.12));
    let lm = Landmarks::of(ctx.rig, &act.pose);
    let belly = lm.on(Bone::Spine, Vec3::new(0.03 * u, 0.02 * u, 0.125 * u));
    let hand = hand_aim(Side::Left, Vec3::new(-0.9, -0.2, 0.25), Vec3::new(0.0, 0.1, -1.0));
    act.pose.reach(ctx.rig, Side::Left, belly, Vec3::new(0.9, -0.4, -0.4), Some(hand));
    act.pose.fingers(Side::Left, HandShape::FLAT.tighten(0.2));

    act.crouch = 0.014 * u * (shake * 0.5 + 0.5);
    act.face = Face { happy: 1.0, aa: 0.35 + 0.3 * pulse(t * 13.0), ..default() };
    act.attention = 0.5;
    act.walk_arms = 0.5;
    act
}

fn angry(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, e, u) = (ctx.t, ctx.e, ctx.u());
    // Trembling comes in bursts.
    let burst = pulse(t * 2.2).powi(3);
    let shake = (t * 38.0).sin() * burst;
    act.pose.spine(0.14, 0.0, 0.0);
    act.pose.pelvis(0.04, 0.0, 0.0);
    act.pose.head(0.1, shake * 0.025, 0.0);
    for side in Side::BOTH {
        act.pose.shrug(side, 0.1, 0.0);
        // Stiff arms thrust down and back, fists clenched.
        act.pose.arm(
            side,
            Arm { down: 1.16 + shake * 0.02, swing: -0.2, yaw: 0.0, twist: 0.1, bend: 0.12, pronate: 0.3 },
        );
        act.pose.wrist(side, -0.3, 0.0, 0.0);
        act.pose.fingers(side, HandShape::FIST);
    }
    // Stomps a foot: three in a row on the way in, then one now and then.
    let cycle = e / 0.62;
    let stomping = cycle < 3.0 || fract(e / 3.4) < 0.62 / 3.4;
    if stomping {
        let p = fract(if cycle < 3.0 { cycle } else { fract(e / 3.4) * 3.4 / 0.62 });
        act.feet[Side::Right.index()].lift = 0.1 * u * bump(p / 0.55);
        act.crouch = 0.022 * u * bump((p - 0.55) / 0.45);
    }
    act.face = Face { angry: 1.0, ih: 0.22, ..default() };
    act.stance = ctx.style.stance.max(1.0) * 1.25;
    act.walk_arms = 0.4;
    act
}

fn annoyed(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, u) = (ctx.t, ctx.u());
    // Arms folded, weight on one hip, face turned away with the odd glance back.
    act.pose.pelvis(0.0, 0.0, 0.05);
    act.pose.hips.x += 0.015 * u;
    act.pose.spine(-0.03, -0.1, -0.03);
    let glance = smoothstep(0.75, 0.95, (t * 0.9).sin());
    act.pose.head(-0.07, -0.42 + glance * 0.12, 0.06);
    let lm = Landmarks::of(ctx.rig, &act.pose);
    for side in Side::BOTH {
        // Each wrist crosses to the far side of the chest; the right arm lies on top.
        let (across, drop, out) = match side {
            Side::Right => (-0.1, -0.19, 0.135),
            Side::Left => (-0.085, -0.235, 0.105),
        };
        let target = chest(&lm, sx(side, across * u, drop * u, out * u));
        let hand = hand_aim(side, sx(side, -1.0, 0.22, -0.1), Vec3::new(0.0, 0.1, -1.0));
        act.pose.reach(ctx.rig, side, target, sx(side, 0.8, -0.7, 0.1), Some(hand));
        act.pose.fingers(side, HandShape::RELAXED.tighten(0.15));
    }
    // An impatient toe tap.
    act.feet[Side::Right.index()].toe = 0.4 * pulse(t * 7.5);
    act.feet[Side::Right.index()].offset = Vec2::new(-0.02 * u, 0.04 * u);
    act.gaze = Vec2::new(0.25 + glance * 0.6, 0.1);
    act.attention = 0.15;
    act.face = Face { angry: 0.55, eyes_closed: 0.18, ou: 0.15, ..default() };
    act.walk_arms = 0.25;
    act
}

fn sad(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, u) = (ctx.t, ctx.u());
    // A slow sigh every few seconds.
    let sigh = bump(fract(t / 5.5) / 0.4);
    act.pose.spine(0.19 + sigh * 0.03, 0.0, (t * 0.7).sin() * 0.03);
    act.pose.head(0.36, (t * 0.5).sin() * 0.06, 0.05);
    for side in Side::BOTH {
        act.pose.shrug(side, -0.05 + sigh * 0.06, 0.25);
        act.pose.arm(side, Arm { down: 1.43, swing: 0.07, yaw: 0.0, twist: 0.45, bend: 0.08, pronate: 0.0 });
        act.pose.fingers(side, HandShape::RELAXED);
        act.feet[side.index()].yaw = -0.2;
    }
    act.knees_in = 0.5;
    act.crouch = 0.012 * u;
    act.stance = 0.85;
    act.gaze = Vec2::new(0.0, -0.6);
    act.attention = 0.15;
    act.face = Face { sad: 1.0, ..default() };
    act.walk_arms = 0.3;
    act
}

fn cry(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, u, m) = (ctx.t, ctx.u(), &ctx.rig.metrics);
    // Sobs hitch the shoulders in fits.
    let sob = pulse(t * 9.0).powi(2) * (0.6 + 0.4 * (t * 1.3).sin());
    act.pose.spine(0.16 + 0.03 * sob, 0.0, 0.0);
    act.pose.head(0.3, 0.0, (t * 1.1).sin() * 0.04);
    for side in Side::BOTH {
        act.pose.shrug(side, 0.07 + 0.05 * sob, 0.2);
        // Fists rubbing at the eyes.
        let rub = (t * 6.0 + side.sign()).sin() * 0.008 * u;
        let eye = m.eye + sx(side, m.eye_spacing * 0.5 + 0.022 * u + rub, -0.062 * u, 0.03 * u);
        hand_to_face(&mut act.pose, ctx, side, eye, sx(side, -0.3, 0.95, -0.05), sx(side, 0.45, -1.0, 0.3));
        act.pose.fingers(side, HandShape::LOOSE_FIST);
        act.feet[side.index()].yaw = -0.24;
    }
    act.knees_in = 0.85;
    act.crouch = 0.028 * u;
    act.stance = 0.8;
    act.attention = 0.0;
    act.gaze = Vec2::new(0.0, -0.4);
    act.face = Face { sad: 1.0, eyes_closed: 0.95, oh: 0.22 + 0.2 * sob, ..default() };
    act.walk_arms = 0.1;
    act
}

fn thinking(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, u, m) = (ctx.t, ctx.u(), &ctx.rig.metrics);
    let hmm = (t * 0.9).sin();
    act.pose.pelvis(0.0, 0.0, -0.04);
    act.pose.hips.x -= 0.014 * u;
    act.pose.spine(0.02, 0.06, 0.03);
    act.pose.head(-0.05 + hmm * 0.025, 0.14, -0.13);
    // Right hand to the chin, index finger along the cheek and tapping.
    hand_to_face(
        &mut act.pose,
        ctx,
        Side::Right,
        m.chin + Vec3::new(-0.03 * u, -0.07 * u, 0.04 * u),
        Vec3::new(0.2, 0.95, 0.0),
        Vec3::new(-0.3, -1.0, 0.25),
    );
    let tap = pulse(t * 5.0) * smoothstep(0.2, 0.8, (t * 0.7).sin());
    let mut shape = HandShape { curl: [0.2, 0.06 + tap * 0.18, 0.85, 0.92, 0.95], spread: 0.0 };
    shape.curl[0] = 0.15;
    act.pose.fingers(Side::Right, shape);
    // Left forearm across the body, cupping the right elbow.
    let lm = Landmarks::of(ctx.rig, &act.pose);
    let elbow = lm.pos(Bone::RightLowerArm);
    let hand = hand_aim(Side::Left, Vec3::new(-1.0, 0.15, 0.15), Vec3::new(0.0, 0.75, -0.65));
    let target = elbow + Vec3::new(0.07 * u, -0.035 * u, 0.0);
    act.pose.reach(ctx.rig, Side::Left, target, Vec3::new(0.7, -0.7, -0.35), Some(hand));
    act.pose.fingers(Side::Left, HandShape::RELAXED.tighten(0.1));

    act.gaze = Vec2::new(0.45 + (t * 0.4).sin() * 0.15, 0.55);
    act.attention = 0.2;
    act.face = Face { relaxed: 0.15, ou: 0.14, ..default() };
    act.walk_arms = 0.2;
    act
}

fn surprised(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (e, u) = (ctx.e, ctx.u());
    // A start that settles: `k` is 1 at the jolt and fades.
    let k = (-e * 2.0).exp();
    act.pose.spine(-0.04 - 0.12 * k, 0.0, 0.0);
    act.pose.head(-0.04 - 0.1 * k, 0.0, 0.0);
    for side in Side::BOTH {
        act.pose.shrug(side, 0.05 + 0.1 * k, 0.0);
        // Hands fly up, palms out, then hover in front of the chest.
        act.pose.arm(
            side,
            Arm {
                down: 1.0 - 0.45 * k,
                swing: 0.35,
                yaw: 0.0,
                twist: -0.5 - 0.5 * k,
                bend: 1.95 - 0.15 * k,
                pronate: -0.3,
            },
        );
        act.pose.wrist(side, -0.35, 0.0, 0.0);
        act.pose.fingers(side, HandShape::OPEN);
    }
    act.hop = 0.065 * u * bump(e / 0.34);
    act.stance = 1.0 + 0.3 * k;
    act.face = Face { surprised: 1.0, oh: 0.12 + 0.3 * k, ..default() };
    act.walk_arms = 0.5;
    act
}

fn shy(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, u) = (ctx.t, ctx.u());
    let twist = (t * 1.9).sin();
    act.pose.spine(0.05, 0.13 * twist, 0.0);
    act.pose.head(0.15, -0.1 - 0.1 * twist, 0.1);
    let lm = Landmarks::of(ctx.rig, &act.pose);
    for side in Side::BOTH {
        act.pose.shrug(side, 0.07, 0.12);
        // Index fingers poking together in front of the belly.
        let poke = (t * 3.2).sin() * 0.008 * u;
        let target = lm.on(Bone::Spine, sx(side, 0.07 * u + poke, 0.02 * u, 0.15 * u));
        let hand = hand_aim(side, sx(side, -1.0, 0.1, 0.25), Vec3::new(0.0, -0.55, -0.85));
        act.pose.reach(ctx.rig, side, target, sx(side, 0.75, -0.8, -0.2), Some(hand));
        act.pose.fingers(side, HandShape::POINT);
    }
    // One toe drawing circles behind her.
    let foot = &mut act.feet[Side::Right.index()];
    foot.heel = 0.55;
    foot.offset = Vec2::new(0.02 * u, -0.06 * u);
    foot.yaw = 0.25 * (t * 2.6).sin();
    act.knees_in = 0.5;
    // Looks away, peeking up now and then.
    let peek = smoothstep(0.6, 0.9, (t * 0.8).sin());
    act.gaze = Vec2::new(-0.35 + peek * 0.35, -0.5 + peek * 0.7);
    act.attention = 0.2;
    act.face = Face { happy: 0.45, relaxed: 0.4, eyes_closed: 0.1, ..default() };
    act.walk_arms = 0.3;
    act
}

fn dizzy(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, u) = (ctx.t, ctx.u());
    let w = t * 4.2;
    // The whole body orbits over unsteady feet.
    act.pose.hips += Vec3::new(w.cos() * 0.035, 0.0, w.sin() * 0.03) * u;
    act.pose.spine(0.05 + w.sin() * 0.07, 0.0, w.cos() * 0.09);
    act.pose.head((w + 1.0).sin() * 0.16, 0.0, (w + 1.0).cos() * 0.2);
    for side in Side::BOTH {
        // Arms out, see-sawing for balance.
        let see_saw = (w + side.sign() * 1.4).sin();
        act.pose.arm(
            side,
            Arm { down: 0.85 + 0.25 * see_saw, swing: 0.1, yaw: 0.15, twist: 0.0, bend: 0.5, pronate: 0.0 },
        );
        act.pose.fingers(side, HandShape::OPEN.mix(HandShape::RELAXED, 0.5));
        act.feet[side.index()].lift = 0.03 * u * pulse(w * 0.5 + side.sign() * PI * 0.5).powi(3);
    }
    act.crouch = 0.03 * u;
    act.stance = 1.4;
    act.gaze = Vec2::new((t * 7.0).cos(), (t * 7.0).sin()) * 0.7;
    act.attention = 0.0;
    act.face = Face { surprised: 0.35, oh: 0.5, eyes_closed: 0.3, ..default() };
    act.walk_arms = 0.3;
    act
}

fn sleepy(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, u, m) = (ctx.t, ctx.u(), &ctx.rig.metrics);
    // The head sinks slowly, then jerks back up.
    let p = fract(t / 4.4);
    let droop = smoothstep(0.0, 0.8, p) * (1.0 - smoothstep(0.82, 0.9, p));
    // A yawn every so often.
    let yawn = bump((fract(t / 11.0) * 11.0 - 6.0) / 2.4);
    act.pose.spine(0.08 + 0.07 * droop - 0.08 * yawn, 0.0, (t * 0.6).sin() * 0.05);
    act.pose.head(0.12 + 0.36 * droop - 0.3 * yawn, 0.0, (t * 0.6).sin() * 0.1);
    for side in Side::BOTH {
        act.pose.shrug(side, -0.03 + 0.1 * yawn, 0.15);
        act.pose.arm(side, Arm { down: 1.42, swing: 0.05, yaw: 0.0, twist: 0.4, bend: 0.08, pronate: 0.0 });
        act.pose.fingers(side, HandShape::RELAXED);
    }
    if yawn > 0.0 {
        // Covers the yawn with a hand.
        let mut covered = act.pose.clone();
        hand_to_face(
            &mut covered,
            ctx,
            Side::Right,
            m.mouth + Vec3::new(-0.035 * u, -0.05 * u, 0.05 * u),
            Vec3::new(0.55, 0.8, 0.1),
            Vec3::new(-0.6, -1.0, 0.1),
        );
        covered.fingers(Side::Right, HandShape::FLAT.tighten(0.1));
        act.pose.blend_bones(&covered, arm_bones(Side::Right), smoothstep(0.0, 0.5, yawn));
    }
    act.crouch = 0.008 * u;
    act.attention = 0.1;
    act.gaze = Vec2::new(0.0, -0.3);
    act.face = Face {
        relaxed: 0.5,
        eyes_closed: (0.55 + 0.4 * droop).max(yawn),
        aa: 0.85 * yawn,
        ..default()
    };
    act.walk_arms = 0.4;
    act
}

/// Pelvis roll-back of the floor sit, shared with [`sit_layout`].
const SIT_PELVIS_TILT: f32 = -0.32;

/// Where the body goes when she sits on the floor hugging her knees: computed from the leg
/// lengths, so the feet stay on the ground and the knees come up on any rig.
pub struct SitLayout {
    /// Hips offset from rest, in model space.
    pub hips: Vec3,
    /// Ankle targets, in model space.
    pub ankles: [Vec3; 2],
    /// Where the knees end up.
    pub knees: [Vec3; 2],
}

/// Direction the knees point while sitting.
pub fn sit_knee_pole(side: Side) -> Vec3 {
    Vec3::new(-0.14 * side.sign(), 1.0, 0.12)
}

pub fn sit_layout(rig: &Rig) -> SitLayout {
    let m = &rig.metrics;
    let u = m.unit;
    let tilt = Quat::from_rotation_x(SIT_PELVIS_TILT);
    // The hip joints rest about a hand's width above the floor.
    let joint_height = 0.105 * u;
    let joint_offset = |side: Side| tilt * (rig.rest(Bone::LeftUpperLeg.on(side)) - rig.rest(Bone::Hips));
    let mean_joint_y = (joint_offset(Side::Left).y + joint_offset(Side::Right).y) * 0.5;
    let hips_pos = Vec3::new(0.0, joint_height - mean_joint_y, -0.1 * u);
    let mut ankles = [Vec3::ZERO; 2];
    let mut knees = [Vec3::ZERO; 2];
    for side in Side::BOTH {
        let joint = hips_pos + joint_offset(side);
        // Feet far enough forward that the knees rise to about 45 degrees.
        let reach = 0.72 * m.leg();
        let rise = m.ankle_height - joint.y;
        let forward = (reach * reach - rise * rise).max(0.0).sqrt();
        let ankle = Vec3::new(side.sign() * m.hip_half_width * 1.55, m.ankle_height, joint.z + forward);
        let limb = ik::two_bone(
            joint,
            ankle,
            rig.rest(Bone::LeftLowerLeg.on(side)) - rig.rest(Bone::LeftUpperLeg.on(side)),
            rig.rest(Bone::LeftFoot.on(side)) - rig.rest(Bone::LeftLowerLeg.on(side)),
            Vec3::Z,
            sit_knee_pole(side),
        );
        ankles[side.index()] = ankle;
        knees[side.index()] = limb.mid;
    }
    SitLayout {
        hips: hips_pos - rig.rest(Bone::Hips),
        ankles,
        knees,
    }
}

fn sit(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, u) = (ctx.t, ctx.u());
    let layout = sit_layout(ctx.rig);
    // Pose the trunk where it will be once seated, so the hands find the knees.
    act.pose.hips = layout.hips;
    act.pose.pelvis(SIT_PELVIS_TILT, 0.0, 0.0);
    act.pose.spine(0.42 + (t * 1.9).sin() * 0.01, 0.0, (t * 0.8).sin() * 0.03);
    act.pose.head(-0.1, 0.0, (t * 0.8).sin() * 0.05);
    for side in Side::BOTH {
        let knee = layout.knees[side.index()];
        let target = knee + Vec3::new(side.sign() * 0.01 * u, 0.06 * u, 0.0);
        let hand = hand_aim(side, Vec3::new(0.0, -0.45, 0.9), Vec3::new(0.0, -0.9, -0.4));
        act.pose.reach(ctx.rig, side, target, sx(side, 0.9, -0.5, -0.3), Some(hand));
        act.pose.fingers(side, HandShape::RELAXED.tighten(-0.1));
        // Idly tapping her toes.
        act.feet[side.index()].toe = 0.28 * pulse(t * 2.4 + side.sign() * 1.2);
    }
    // The animator owns the hips while the sit blends in; it reads the layout itself.
    act.pose.hips = Vec3::ZERO;
    act.sit = 1.0;
    act.face = Face { relaxed: 0.45, ..default() };
    act.walk_arms = 1.0;
    act
}

fn wave(ctx: &Ctx) -> Act {
    let mut act = idle(ctx);
    let e = ctx.e;
    // Two small wrist waves, a held peace sign, then lower the hand before repeating.
    let cycle = e.rem_euclid(6.0);
    let raise = smoothstep(0.0, 0.55, cycle) * (1.0 - smoothstep(4.0, 4.7, cycle));
    let greeting = 1.0 - smoothstep(1.8, 2.3, cycle);
    let peace = smoothstep(1.8, 2.5, cycle);
    let swing = ((cycle - 0.55).max(0.0) * TAU * 1.3).sin() * greeting;
    act.pose.spine(-0.02, 0.0, 0.04);
    act.pose.head(-0.03, 0.0, -0.09);
    let mut arm = Pose::default();
    arm.shrug(Side::Right, 0.12, 0.0);
    arm.arm(
        Side::Right,
        Arm { down: 0.18, swing: 0.0, yaw: 0.2, twist: -1.35, bend: 1.25 + 0.06 * swing, pronate: 0.0 },
    );
    arm.wrist(Side::Right, 0.04, 0.22 * swing, 0.07 * swing);
    arm.fingers(Side::Right, HandShape::OPEN.mix(HandShape::PEACE, peace));
    let bones = arm_bones(Side::Right).chain([Bone::RightShoulder]);
    act.pose.blend_bones(&arm, bones, raise);
    act.face = Face { happy: 0.65, wink: 0.5 * bump((cycle - 2.5) / 0.5), ..default() };
    act.walk_arms = 0.5;
    act
}

/// Upper arm, forearm, hand and fingers of one side.
pub fn arm_bones(side: Side) -> impl Iterator<Item = Bone> {
    let first = Bone::LeftUpperArm.on(side).index();
    let last = Bone::LeftLittleDistal.on(side).index();
    Bone::ALL[first..=last].iter().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Emote; 23] = Emote::ALL;

    pub(super) fn ctx(rig: &Rig, t: f32, style: Style) -> Ctx<'_> {
        Ctx { rig, t, e: t, style, talk: 0.5, energy: 0.6, beat: t * 2.0, bpm: 120.0 }
    }

    #[test]
    fn every_wire_emotion_parses_to_its_own_emote() {
        let mut seen = Vec::new();
        for (name, expected) in aria_protocol::EMOTIONS.iter().zip(Emote::ALL) {
            let emote = Emote::parse(name);
            assert_eq!(emote, expected, "{name} is out of order in Emote::ALL");
            assert!(!seen.contains(&emote), "{name} parsed to an emote another name already uses");
            seen.push(emote);
        }
        assert_eq!(seen.len(), ALL.len(), "every emote needs a wire name");
        assert_eq!(Emote::parse("walking"), Emote::Idle);
        assert_eq!(Emote::parse("  HAPPY "), Emote::Happy);
    }

    #[test]
    fn every_emotion_stays_finite_on_full_and_minimal_rigs() {
        // The 15 mandatory bones only: no chest, neck, shoulders, toes, eyes or fingers.
        let optional: Vec<Bone> = Bone::ALL
            .iter()
            .copied()
            .filter(|b| {
                let name = b.name();
                ["chest", "upperChest", "neck", "jaw"].contains(&name)
                    || name.ends_with("Eye")
                    || name.ends_with("Shoulder")
                    || name.ends_with("Toes")
                    || name.contains("Thumb")
                    || name.contains("Index")
                    || name.contains("Middle")
                    || name.contains("Ring")
                    || name.contains("Little")
            })
            .collect();
        for rig in [Rig::synthetic(&[]), Rig::synthetic(&optional)] {
            for persona in ["", "tsundere", "yandere", "tomboy", "bakadere", "kuudere", "genki", "meganekko"] {
                for emote in ALL {
                    for step in 0..40 {
                        let act = act(emote, &ctx(&rig, step as f32 * 0.37, Style::for_persona(persona)));
                        assert!(act.pose.hips.is_finite(), "{emote:?}/{persona}");
                        assert!(act.pose.rot.iter().all(|q| q.is_finite() && q.is_normalized()), "{emote:?}/{persona}");
                        assert!(act.crouch.is_finite() && act.hop >= 0.0, "{emote:?}/{persona}");
                        let fk = rig.fk(&act.pose);
                        assert!(fk.pos.iter().all(|p| p.is_finite()), "{emote:?}/{persona}");
                    }
                }
            }
        }
    }

    #[test]
    fn hands_reach_their_landmarks() {
        let rig = Rig::synthetic(&[]);
        let m = &rig.metrics;

        // Thinking: the right wrist sits just under the chin, in front of the face.
        let act = thinking(&ctx(&rig, 1.0, Style::default()));
        let lm = Landmarks::of(&rig, &act.pose);
        let chin = lm.on(Bone::Head, m.chin);
        let wrist = lm.pos(Bone::RightHand);
        assert!(wrist.distance(chin) < 0.14, "wrist {wrist:?} vs chin {chin:?}");
        assert!(wrist.y < chin.y && wrist.z > lm.pos(Bone::Head).z);

        // Crying: both wrists are up at the face, either side of the nose.
        let act = cry(&ctx(&rig, 1.0, Style::default()));
        let lm = Landmarks::of(&rig, &act.pose);
        for side in Side::BOTH {
            let wrist = lm.pos(Bone::LeftHand.on(side));
            let eye = lm.on(Bone::Head, m.eye);
            assert!(wrist.distance(eye) < 0.16, "{side:?} wrist {wrist:?} vs eyes {eye:?}");
            assert!((wrist.x - eye.x) * side.sign() > 0.0, "{side:?} hand should stay on its own side");
        }

        // Hand on hip: the wrist is at the waist, outside the hip joint.
        let act = idle(&ctx(&rig, 1.0, Style::for_persona("tsundere")));
        let lm = Landmarks::of(&rig, &act.pose);
        let wrist = lm.pos(Bone::LeftHand);
        let hip = lm.pos(Bone::LeftUpperLeg);
        assert!(wrist.x > hip.x + 0.04 && (wrist.y - hip.y - 0.1).abs() < 0.12, "wrist {wrist:?} hip {hip:?}");
    }

    #[test]
    fn sitting_puts_the_feet_on_the_floor_and_the_knees_up() {
        let rig = Rig::synthetic(&[]);
        let layout = sit_layout(&rig);
        let hips_y = rig.rest(Bone::Hips).y + layout.hips.y;
        assert!(hips_y < 0.3, "the hips should be near the floor, got {hips_y}");
        for side in Side::BOTH {
            let (ankle, knee) = (layout.ankles[side.index()], layout.knees[side.index()]);
            assert!((ankle.y - rig.metrics.ankle_height).abs() < 1e-4);
            assert!(knee.y > hips_y + 0.1, "{side:?} knee should be above the hips: {knee:?}");
            assert!(ankle.z > 0.3, "{side:?} foot should be out in front: {ankle:?}");
        }
    }

    #[test]
    fn blending_acts_never_extrapolates_the_face() {
        let rig = Rig::synthetic(&[]);
        let mut a = idle(&ctx(&rig, 0.0, Style::default()));
        let b = happy(&ctx(&rig, 0.0, Style::default()));
        a.blend(&b, 1.3);
        assert_eq!(a.face.happy, 1.0);
        assert!(a.sit == 0.0 && a.walk_arms <= 1.0);
    }
}
