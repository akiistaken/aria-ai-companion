//! Dancing and grooving, on the beat.
//!
//! Everything here runs on [`Ctx::beat`], the animator's beat clock, which the app keeps locked
//! to the music it hears (or to her own tempo when nothing plays), so steps land on the beat
//! whatever the song's tempo. Whole beats are the downbeats: bounces bottom out and claps land
//! there.
//!
//! **Dance** is a library of routines, each a 16-beat phrase. She changes routine every phrase,
//! cross-fading over the last beat, and the order is a deterministic shuffle so no two phrases in
//! a row repeat. Reserved personas only pick from the subtler routines.
//!
//! **Groove** is what she does while just listening: three lighter loops (head nod, sway and
//! snap, step-touch) that change every eight beats.

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;

use super::{chest, fract, hands_together, neutral, sx, Act, Ctx};
use crate::face::Face;
use crate::pose::{hand_aim, Arm, HandShape, Landmarks, Pose};
use crate::rig::Side;
use crate::spring::{bump, smoothstep};

/// Beats per dance phrase, and per groove loop.
const PHRASE: f32 = 16.0;
const GROOVE_LOOP: f32 = 8.0;

/// 1 on the beat, easing to 0 halfway between: the bounce that bottoms out on the downbeat.
fn on_beat(b: f32) -> f32 {
    0.5 + 0.5 * (TAU * b).cos()
}

/// A short accent that rises into the beat and releases smoothly, including across beat zero.
fn hit(b: f32) -> f32 {
    on_beat(b).powi(6)
}

/// -1..1 over two beats: which way a side-to-side move is going.
fn sway(b: f32) -> f32 {
    (PI * b).sin()
}

/// A turn `progress` of the way round (0..1), back to 0 once complete.
fn full_turn(progress: f32) -> f32 {
    if progress >= 1.0 {
        0.0
    } else {
        TAU * progress.max(0.0)
    }
}

/// How big the moves are: louder music and livelier personas dance bigger.
fn size(ctx: &Ctx) -> f32 {
    (0.15 + 0.85 * ctx.energy.clamp(0.0, 1.0))
        * (0.55 + 0.45 * smoothstep(85.0, 145.0, ctx.bpm))
        * ctx.style.energy.clamp(0.75, 1.3)
}

/// A soft, absorbed expression for ballads, an alert smile for faster/louder music.
/// Accents are small enough that she never mouths every kick like a syllable.
fn listening_face(ctx: &Ctx) -> Face {
    let level = ctx.energy.clamp(0.0, 1.0);
    let tempo = smoothstep(78.0, 145.0, ctx.bpm);
    let excitement = tempo * level;
    let accent = on_beat(ctx.beat).powi(3) * excitement;
    let absorbed = (1.0 - tempo) * level;
    Face {
        happy: 0.12 + 0.3 * level + 0.32 * excitement + 0.12 * accent,
        relaxed: 0.2 + 0.5 * (1.0 - tempo) * (1.0 - 0.3 * level),
        eyes_closed: absorbed * (0.12 + 0.16 * on_beat(ctx.beat / 4.0)),
        surprised: 0.13 * accent,
        aa: 0.045 * accent,
        ..default()
    }
}

/// Integer hash for the deterministic routine shuffle.
fn hash(n: i64) -> u32 {
    let mut x = (n as u32).wrapping_mul(0x9E37_79B9) ^ ((n >> 32) as u32);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

type Routine = fn(&Ctx, f32) -> Act;

/// Every routine, energetic ones last.
const ROUTINES: &[(&str, Routine)] = &[
    ("side_tap", side_tap),
    ("idol_heart", idol_heart),
    ("chest_pulse", chest_pulse),
    ("shimmy", shimmy),
    ("concert_wave", concert_wave),
    ("bounce_clap", bounce_clap),
    ("step_kick", step_kick),
    ("spin_pose", spin_pose),
];
/// Reserved personas keep to the first few.
const CALM_ROUTINES: usize = 4;

/// Phrases are dealt in decks: each deck is a shuffle of every routine in the pool, so all of
/// them get danced, none twice in a row (the first of a deck is swapped if it would repeat the
/// last of the deck before).
fn deck(d: i64, pool: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..pool).collect();
    for i in (1..pool).rev() {
        let j = hash(d.wrapping_mul(131).wrapping_add(i as i64)) as usize % (i + 1);
        order.swap(i, j);
    }
    order
}

/// Which routine plays in phrase `n`.
fn routine_for(n: i64, calm: bool) -> usize {
    let pool = if calm { CALM_ROUTINES } else { ROUTINES.len() };
    let (d, i) = (n.div_euclid(pool as i64), n.rem_euclid(pool as i64) as usize);
    let mut order = deck(d, pool);
    if order[0] == deck(d - 1, pool)[pool - 1] {
        order.swap(0, 1);
    }
    order[i]
}

/// The name of the routine playing at `beat`.
#[cfg(test)]
fn routine_name(beat: f32, calm: bool) -> &'static str {
    ROUTINES[routine_for((beat / PHRASE).floor() as i64, calm)].0
}

pub fn dance(ctx: &Ctx) -> Act {
    if ctx.bpm < 94.0 {
        return groove(ctx);
    }
    let mut act = dance_phrase(ctx, true);
    // A continuous transition between smaller and bigger routines, so a volume accent or
    // tempo correction cannot abruptly switch the arms to another pose.
    let drive = if ctx.style.energy < 0.8 { 0.0 } else {
        smoothstep(108.0, 135.0, ctx.bpm) * smoothstep(0.5, 0.85, ctx.energy)
    };
    if drive > 0.0 {
        act.blend(&dance_phrase(ctx, false), drive);
    }
    act.face = listening_face(ctx);
    act.attention = 0.3;
    act.walk_arms = 0.4;
    act
}

fn dance_phrase(ctx: &Ctx, calm: bool) -> Act {
    let phrase = (ctx.beat / PHRASE).floor();
    let local = ctx.beat - phrase * PHRASE;
    let (_, current) = ROUTINES[routine_for(phrase as i64, calm)];
    let mut act = current(ctx, local);
    // The last beat of a phrase flows into the next routine.
    let fade = smoothstep(PHRASE - 1.0, PHRASE, local);
    if fade > 0.0 {
        let (_, next) = ROUTINES[routine_for(phrase as i64 + 1, calm)];
        act.blend(&next(ctx, local - PHRASE), fade);
    }
    act
}

// ---- Routines -----------------------------------------------------------------------------------
// Each takes the beat within its phrase (`b`, 0..16; slightly negative while cross-fading in).

/// Side taps, the right foot then the left, under continuous forearm rolls.
fn side_tap(ctx: &Ctx, b: f32) -> Act {
    let mut act = neutral(ctx);
    let (u, k) = (ctx.u(), size(ctx));
    let two = fract(b / 2.0);
    let (right_out, left_out) = (bump(two * 2.0), bump(two * 2.0 - 1.0));
    let lr = left_out - right_out;
    let step_hop = bump(fract(two * 4.0));
    act.feet[Side::Right.index()].offset.x = -0.16 * u * right_out * k;
    act.feet[Side::Right.index()].lift = if two < 0.5 { 0.035 * u * step_hop } else { 0.0 };
    act.feet[Side::Left.index()].offset.x = 0.16 * u * left_out * k;
    act.feet[Side::Left.index()].lift = if two >= 0.5 { 0.035 * u * step_hop } else { 0.0 };
    act.pose.hips.x -= 0.035 * u * lr * k;
    act.pose.pelvis(0.0, 0.1 * sway(b), -0.1 * lr * k);
    act.pose.spine(-0.02, -0.14 * sway(b) * k, 0.07 * lr);
    act.pose.head(0.06 * on_beat(b), 0.0, 0.08 * sway(b));
    act.crouch = 0.035 * u * on_beat(b) * k;

    let mut arms = Pose::default();
    for side in Side::BOTH {
        // A continuous forearm roll with wrists following a quarter beat behind.
        let phase = PI * b * 0.5 + if side == Side::Left { 0.0 } else { PI };
        let roll = phase.sin();
        let arm = Arm { down: 0.95 - 0.22 * roll * k, swing: 0.3 + 0.14 * phase.cos() * k,
            yaw: 0.12, twist: 0.2 + 0.22 * roll * k, bend: 1.25 + 0.28 * phase.cos() * k, pronate: -0.35 };
        arms.arm(side, arm);
        arms.wrist(side, 0.12 * (phase - 0.4).sin(), 0.08 * (phase - 0.4).cos(), 0.0);
        arms.fingers(side, HandShape::RELAXED.mix(HandShape::LOOSE_FIST, 0.3 + 0.2 * roll));
        act.pose.blend_bones(&arms, super::arm_bones(side), 1.0);
    }
    act
}

/// The idol routine: hips pop on the beat, hands make a heart at the chest, then over the head.
fn idol_heart(ctx: &Ctx, b: f32) -> Act {
    let mut act = neutral(ctx);
    let (u, k) = (ctx.u(), size(ctx));
    let pop = hit(b);
    let side_to_side = sway(b);
    act.pose.hips.x += 0.03 * u * side_to_side * k;
    act.pose.pelvis(0.0, 0.0, 0.09 * side_to_side * k);
    act.pose.spine(-0.03, 0.04 * side_to_side, -0.06 * side_to_side * k);
    act.pose.head(-0.04, 0.0, 0.14 * side_to_side + 0.04 * pop);
    act.crouch = 0.02 * u * on_beat(b) * k;
    // Weight shifts onto one leg and the other heel lifts, alternating every beat.
    let lifted = if side_to_side > 0.0 { Side::Right } else { Side::Left };
    act.feet[lifted.index()].heel = 0.35 * side_to_side.abs() * k;

    // Four beats at the chest, four overhead.
    let high = smoothstep(3.6, 4.4, fract(b / 8.0) * 8.0) * (1.0 - smoothstep(7.6, 8.0, fract(b / 8.0) * 8.0));
    let heart = HandShape { curl: [0.25, 0.55, 0.6, 0.65, 0.7], spread: 0.0 };
    let mut at_chest = act.pose.clone();
    hands_together(&mut at_chest, ctx, 0.0 + 0.015 * pop, 0.2, 0.0, heart);
    let mut overhead = act.pose.clone();
    for side in Side::BOTH {
        overhead.arm(side, Arm { down: -1.15, swing: 0.0, yaw: 0.25, twist: -1.3, bend: 1.55, pronate: 0.0 });
        overhead.fingers(side, heart);
    }
    act.pose = at_chest;
    for side in Side::BOTH {
        act.pose.blend_bones(&overhead, super::arm_bones(side), high);
    }
    act.face.happy = 1.0;
    act.knees_in = 0.4;
    act
}

/// A two-handed chest pulse with alternating shoulder accents and soft heel lifts.
/// Elbows stay close; the wrists open together instead of one arm sweeping up and down.
fn chest_pulse(ctx: &Ctx, b: f32) -> Act {
    let mut act = neutral(ctx);
    let (u, k) = (ctx.u(), size(ctx));
    let pulse = on_beat(b);
    let travel = sway(b / 2.0);
    let open = 0.5 - 0.5 * (PI * b * 0.5).cos();
    act.pose.hips.x += 0.025 * u * travel * k;
    act.pose.pelvis(0.0, 0.06 * travel * k, -0.045 * travel * k);
    act.pose.spine(-0.025 + 0.045 * pulse * k, -0.08 * travel * k, 0.025 * travel);
    act.pose.head(0.04 * pulse * k, 0.0, -0.04 * travel);
    act.crouch = 0.025 * u * pulse * k;
    for side in Side::BOTH {
        act.pose.shrug(side, 0.025 + 0.025 * sway(b) * side.sign() * k, 0.0);
        act.pose.arm(side, Arm {
            down: 1.05 - 0.12 * open * k, swing: 0.2 + 0.04 * pulse * k,
            yaw: 0.1, twist: 0.2 + 0.12 * open * k,
            bend: 1.35 + 0.12 * pulse * k, pronate: -0.25,
        });
        act.pose.wrist(side, -0.08 + 0.1 * open, 0.08 * open, 0.0);
        act.pose.fingers(side, HandShape::RELAXED.mix(HandShape::OPEN, 0.65 * open));
        act.feet[side.index()].heel = 0.18 * (travel * side.sign()).max(0.0) * k;
    }
    act
}

/// Shoulders shimmying twice a beat, a low groove, hands loose at the sides.
fn shimmy(ctx: &Ctx, b: f32) -> Act {
    let mut act = neutral(ctx);
    let (u, k) = (ctx.u(), size(ctx));
    let shake = (TAU * 2.0 * b).sin();
    let travel = sway(b / 2.0);
    act.pose.spine(0.08, 0.16 * shake * k, 0.03 * travel);
    act.pose.pelvis(0.06, -0.05 * shake, 0.0);
    act.pose.head(-0.02, -0.08 * shake, 0.05 * travel);
    act.pose.hips.x += 0.025 * u * travel;
    for side in Side::BOTH {
        act.pose.shrug(side, 0.06 + 0.06 * shake * side.sign() * k, 0.04);
        act.pose.arm(side, Arm { down: 0.95, swing: 0.2, yaw: 0.2, twist: 0.2, bend: 1.2, pronate: 0.0 });
        act.pose.wrist(side, -0.3, 0.0, 0.0);
        act.pose.fingers(side, HandShape::OPEN.mix(HandShape::RELAXED, 0.4));
    }
    act.crouch = (0.05 + 0.015 * on_beat(b)) * u * k;
    act.stance = 1.3;
    act.face = Face { happy: 0.6, relaxed: 0.3, ..default() };
    act
}

/// Arms up, swaying overhead like at a concert, over a step-touch.
fn concert_wave(ctx: &Ctx, b: f32) -> Act {
    let mut act = neutral(ctx);
    let (u, k) = (ctx.u(), size(ctx));
    let side_to_side = sway(b / 2.0);
    let step = sway(b);
    act.pose.hips.x += 0.04 * u * step * k;
    act.pose.pelvis(0.0, 0.0, 0.07 * step);
    act.pose.spine(-0.06, 0.0, -0.1 * side_to_side * k);
    act.pose.head(-0.12, 0.0, -0.08 * side_to_side);
    let (moving, planted) = if step > 0.0 { (Side::Right, Side::Left) } else { (Side::Left, Side::Right) };
    act.feet[moving.index()].offset.x = moving.sign() * 0.05 * u * step.abs() * k;
    act.feet[moving.index()].heel = 0.25 * step.abs();
    act.feet[planted.index()].heel = 0.0;
    act.crouch = 0.03 * u * on_beat(b) * k;
    for side in Side::BOTH {
        let lean = side_to_side * side.sign();
        act.pose.arm(side, Arm { down: -1.05 - 0.2 * lean, swing: 0.0, yaw: 0.15, twist: -1.25, bend: 0.25 + 0.15 * lean.max(0.0), pronate: 0.0 });
        act.pose.wrist(side, 0.0, 0.25 * side_to_side, 0.0);
        act.pose.fingers(side, HandShape::OPEN);
    }
    act.gaze = Vec2::new(0.0, 0.35);
    act
}

/// Bouncing on every beat and clapping on two and four, with a hop on the phrase's last bar.
fn bounce_clap(ctx: &Ctx, b: f32) -> Act {
    let mut act = neutral(ctx);
    let (u, k) = (ctx.u(), size(ctx));
    // Claps on beats 2 and 4 (1 and 3 counting from zero).
    let phase = (b - 1.0).rem_euclid(2.0);
    let distance = phase.min(2.0 - phase);
    let closing = 1.0 - smoothstep(0.04, 0.32, distance);
    let apart = 0.03 + 0.17 * (1.0 - closing);
    hands_together(&mut act.pose, ctx, -0.01 + 0.03 * closing, 0.2, apart, HandShape::FLAT);
    act.pose.spine(0.02 + 0.04 * hit(b), 0.0, 0.03 * sway(b));
    act.pose.head(0.06 * hit(b), 0.0, 0.06 * sway(b));
    act.crouch = 0.045 * u * on_beat(b) * k;
    for side in Side::BOTH {
        act.feet[side.index()].heel = 0.25 * (1.0 - on_beat(b)) * k;
    }
    // Little jumps through the last bar of the phrase.
    if b >= PHRASE - 4.0 {
        act.hop = 0.06 * u * bump(fract(b)) * k;
    }
    act.face = Face { happy: 1.0, aa: 0.25 * closing, ..default() };
    act
}

/// Kicks forward, alternating legs each beat, fists pumping.
fn step_kick(ctx: &Ctx, b: f32) -> Act {
    let mut act = neutral(ctx);
    let (u, k) = (ctx.u(), size(ctx));
    let kicker = if (b.floor() as i32).rem_euclid(2) == 0 { Side::Right } else { Side::Left };
    let kick = bump(fract(b) * 1.6).max(0.0);
    let foot = &mut act.feet[kicker.index()];
    foot.offset.y = 0.16 * u * kick * k;
    foot.lift = 0.09 * u * kick * k;
    foot.toe = 0.3 * kick;
    act.pose.spine(-0.06 * kick, 0.08 * kicker.sign() * kick, 0.0);
    act.pose.head(0.04 * hit(b), 0.0, 0.0);
    act.crouch = 0.03 * u * on_beat(b) * k;
    let mut arms = Pose::default();
    for side in Side::BOTH {
        // The arm opposite the kicking leg punches up.
        let punch = if side == kicker { 0.0 } else { kick };
        arms.arm(side, Arm { down: 0.45 - 1.3 * punch, swing: 0.4 - 0.3 * punch, yaw: 0.3, twist: -0.4 - 0.8 * punch, bend: 1.7 - 1.1 * punch, pronate: 0.0 });
        arms.fingers(side, HandShape::FIST);
        act.pose.blend_bones(&arms, super::arm_bones(side), 1.0);
    }
    act.face = Face { happy: 0.9, ..default() };
    act
}

/// Step-touch for four beats, a full turn over the next two, then a pose held for two.
fn spin_pose(ctx: &Ctx, b: f32) -> Act {
    let mut act = neutral(ctx);
    let (u, k) = (ctx.u(), size(ctx));
    let bar8 = fract(b / 8.0) * 8.0;
    let step = sway(b) * (1.0 - smoothstep(3.5, 4.0, bar8));
    act.pose.hips.x += 0.035 * u * step * k;
    act.pose.pelvis(0.0, 0.0, 0.07 * step);
    act.crouch = 0.03 * u * on_beat(b) * k;
    act.spin = full_turn(smoothstep(4.0, 6.0, bar8));
    // Arms out for the turn, then one up and one on the hip to land.
    let mut arms = Pose::default();
    for side in Side::BOTH {
        arms.arm(side, Arm { down: 0.3, swing: 0.1, yaw: 0.1, twist: -0.2, bend: 0.3, pronate: 0.0 });
        arms.fingers(side, HandShape::OPEN);
    }
    let open = bump((bar8 - 3.6) / 2.8);
    for side in Side::BOTH {
        act.pose.blend_bones(&arms, super::arm_bones(side), open);
    }
    let pose_in = smoothstep(5.8, 6.2, bar8) * (1.0 - smoothstep(7.6, 8.0, bar8));
    if pose_in > 0.0 {
        let mut strike = act.pose.clone();
        strike.arm(Side::Right, Arm { down: -1.1, swing: 0.0, yaw: 0.3, twist: -1.3, bend: 0.2, pronate: 0.0 });
        strike.fingers(Side::Right, HandShape::POINT);
        super::hand_on_hip(&mut strike, ctx, Side::Left);
        act.pose.blend(&strike, pose_in);
    }
    act.face = Face { happy: 1.0, ..default() };
    act
}

// ---- Groove -------------------------------------------------------------------------------------

type Loop = fn(&Ctx, f32, f32) -> Act;

const GROOVES: &[Loop] = &[nod, sway_snap, step_touch];

/// Grooving to the music while she listens.
pub fn groove(ctx: &Ctx) -> Act {
    let n = (ctx.beat / GROOVE_LOOP).floor();
    let local = ctx.beat - n * GROOVE_LOOP;
    let en = (0.1 + 0.9 * ctx.energy.clamp(0.0, 1.0)) * ctx.style.energy.clamp(0.8, 1.2);
    let pick = |n: f32| hash(n as i64 ^ 0x5eed) as usize % GROOVES.len();
    let mut act = GROOVES[pick(n)](ctx, ctx.beat, en);
    let fade = smoothstep(GROOVE_LOOP - 1.0, GROOVE_LOOP, local);
    if fade > 0.0 {
        act.blend(&GROOVES[pick(n + 1.0)](ctx, ctx.beat, en), fade);
    }
    // Slow songs keep their own tempo, with the weight transfer stretched across four beats.
    // Blend by BPM rather than halving a changing beat count, which would jump the phase.
    act.blend(&slow_sway(ctx, ctx.beat, en), 1.0 - smoothstep(82.0, 104.0, ctx.bpm));
    act.face = listening_face(ctx);
    act.attention = 0.35;
    act.walk_arms = 0.5;
    act
}

fn slow_sway(ctx: &Ctx, b: f32, en: f32) -> Act {
    let mut act = neutral(ctx);
    let u = ctx.u();
    let phase = TAU * b / 4.0;
    let weight = phase.sin();
    act.pose.hips.x += 0.045 * u * weight * en;
    act.pose.pelvis(0.0, 0.035 * weight * en, 0.055 * weight * en);
    act.pose.spine(0.01, -0.045 * weight * en, -0.05 * weight * en);
    act.pose.head(0.055 * on_beat(b) * en, 0.025 * weight, 0.06 * (phase - 0.35).sin() * en);
    act.crouch = 0.012 * u * on_beat(b) * en;
    for side in Side::BOTH {
        let trail = (phase - 0.55).sin() * side.sign();
        act.pose.arm(side, Arm { down: 1.24, swing: 0.16 + 0.13 * trail * en,
            bend: 0.42 + 0.15 * trail * en, twist: 0.2, ..Arm::HANG });
        act.pose.wrist(side, 0.06 * (phase - 0.8).sin(), 0.04 * trail, 0.0);
        act.pose.fingers(side, HandShape::RELAXED);
        act.feet[side.index()].heel = 0.1 * (weight * side.sign()).max(0.0) * en;
    }
    act
}

/// Head nodding on the beat, shoulders bouncing, arms bent and swaying.
fn nod(ctx: &Ctx, b: f32, en: f32) -> Act {
    let mut act = super::idle(ctx);
    let u = ctx.u();
    let down = on_beat(b);
    let side_to_side = sway(b);
    act.pose.pelvis(0.0, 0.0, 0.05 * en * side_to_side);
    act.pose.spine(0.02 * down * en, 0.05 * en * side_to_side, -0.04 * en * side_to_side);
    act.pose.head(0.12 * en * down, 0.0, 0.05 * en * side_to_side);
    let mut arms = Pose::default();
    for side in Side::BOTH {
        let bob = (TAU * b + side.sign()).sin();
        arms.shrug(side, 0.04 * down * en, 0.0);
        arms.arm(side, Arm { down: 1.2, swing: 0.22, yaw: 0.0, twist: 0.1, bend: 1.3 + 0.15 * bob * en, pronate: 0.0 });
        arms.wrist(side, 0.12 * bob, 0.0, 0.0);
        arms.fingers(side, HandShape::LOOSE_FIST);
        act.pose.blend_bones(&arms, super::arm_bones(side), 1.0);
    }
    act.feet[Side::Right.index()].heel = 0.28 * en * (1.0 - down);
    act.crouch = 0.02 * u * en * down;
    act.face = Face { relaxed: 0.4, happy: 0.3, ..default() };
    act
}

/// Swaying side to side over two beats, snapping her fingers on the beat.
fn sway_snap(ctx: &Ctx, b: f32, en: f32) -> Act {
    let mut act = neutral(ctx);
    let u = ctx.u();
    let side_to_side = sway(b);
    act.pose.hips.x += 0.025 * u * side_to_side * en;
    act.pose.pelvis(0.0, 0.0, 0.07 * side_to_side * en);
    act.pose.spine(0.0, 0.0, -0.06 * side_to_side * en);
    act.pose.head(0.04 * on_beat(b) * en, 0.0, 0.1 * side_to_side * en);
    let snap = hit(b);
    let lm = Landmarks::of(ctx.rig, &act.pose);
    for side in Side::BOTH {
        // Hands at shoulder height, out to the sides; the snapping hand flicks on the beat.
        let target = chest(&lm, sx(side, 0.2 * u, 0.02 * u + 0.015 * u * snap, 0.1 * u));
        let hand = hand_aim(side, sx(side, -0.2, 0.95, 0.2), sx(side, -0.6, 0.0, -0.8));
        act.pose.reach(ctx.rig, side, target, sx(side, 0.8, -0.6, -0.3), Some(hand));
        let snapping = HandShape { curl: [0.55, 0.55 * (1.0 - snap), 0.85, 0.9, 0.95], spread: 0.0 };
        act.pose.fingers(side, snapping);
    }
    let lifted = if side_to_side > 0.0 { Side::Left } else { Side::Right };
    act.feet[lifted.index()].heel = 0.3 * side_to_side.abs() * en;
    act.crouch = 0.015 * u * on_beat(b) * en;
    act.face = Face { happy: 0.45, relaxed: 0.35, ..default() };
    act
}

/// Stepping side to side, one foot meeting the other each beat, arms swinging low.
fn step_touch(ctx: &Ctx, b: f32, en: f32) -> Act {
    let mut act = super::idle(ctx);
    let u = ctx.u();
    let step = sway(b);
    let (moving, _) = if step > 0.0 { (Side::Right, Side::Left) } else { (Side::Left, Side::Right) };
    act.feet[moving.index()].offset.x = moving.sign() * 0.06 * u * step.abs() * en;
    act.feet[moving.index()].heel = 0.3 * step.abs();
    act.pose.hips.x += 0.03 * u * step * en;
    act.pose.pelvis(0.0, 0.0, 0.06 * step * en);
    act.pose.spine(0.02, 0.06 * step * en, -0.03 * step);
    act.pose.head(0.06 * on_beat(b) * en, 0.0, 0.06 * step);
    let mut arms = Pose::default();
    for side in Side::BOTH {
        let swing = step * side.sign();
        arms.arm(side, Arm { down: 1.2, swing: 0.25 + 0.25 * swing * en, yaw: 0.0, twist: 0.25, bend: 0.6 + 0.3 * swing.max(0.0), pronate: 0.0 });
        arms.fingers(side, HandShape::RELAXED);
        act.pose.blend_bones(&arms, super::arm_bones(side), 1.0);
    }
    act.crouch = 0.025 * u * on_beat(b) * en;
    act.face = Face { relaxed: 0.3, happy: 0.4, ..default() };
    act
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emotes::Style;
    use crate::rig::Rig;

    #[test]
    fn routines_change_every_phrase_and_never_repeat_back_to_back() {
        for calm in [false, true] {
            let mut seen = std::collections::HashSet::new();
            let mut last = None;
            for n in 0..64 {
                let r = routine_for(n, calm);
                assert_ne!(Some(r), last, "phrase {n} repeats the previous routine");
                assert!(r < if calm { CALM_ROUTINES } else { ROUTINES.len() });
                seen.insert(r);
                last = Some(r);
            }
            let pool = if calm { CALM_ROUTINES } else { ROUTINES.len() };
            assert_eq!(seen.len(), pool, "every routine gets danced (calm: {calm})");
        }
        assert_eq!(routine_name(3.0, false), routine_name(15.9, false), "one routine per phrase");
    }

    #[test]
    fn every_routine_is_finite_across_its_phrase_and_the_crossfade() {
        let rig = Rig::synthetic(&[]);
        for (name, routine) in ROUTINES {
            for step in -8..=136 {
                let b = step as f32 * 0.125;
                let ctx = Ctx { rig: &rig, t: b * 0.5, e: 5.0, style: Style::default(), talk: 0.0, energy: 0.8, beat: b, bpm: 120.0 };
                let act = routine(&ctx, b);
                assert!(act.pose.rot.iter().all(|q| q.is_finite() && q.is_normalized()), "{name} at beat {b}");
                assert!(act.pose.hips.is_finite() && act.hop >= 0.0 && act.crouch.is_finite(), "{name} at beat {b}");
            }
        }
    }

    #[test]
    fn bounces_bottom_out_on_the_beat() {
        let rig = Rig::synthetic(&[]);
        let at = |b: f32| {
            let ctx = Ctx { rig: &rig, t: 0.0, e: 5.0, style: Style::default(), talk: 0.0, energy: 0.8, beat: b, bpm: 120.0 };
            side_tap(&ctx, b).crouch
        };
        assert!(at(4.0) > at(4.5), "lowest on the beat, highest between");
    }

    #[test]
    fn hand_accents_are_continuous_across_beats() {
        let rig = Rig::synthetic(&[]);
        for b in [0.0, 1.0, 2.0, 3.0, 4.0] {
            assert!((hit(b - 0.001) - hit(b + 0.001)).abs() < 0.01);
            let ctx = super::super::tests::ctx(&rig, 4.0, Style::default());
            let before = bounce_clap(&ctx, b - 0.001);
            let after = bounce_clap(&ctx, b + 0.001);
            for side in Side::BOTH {
                for bone in super::super::arm_bones(side) {
                    assert!(before.pose.rot[bone.index()].angle_between(after.pose.rot[bone.index()]) < 0.03);
                }
            }
        }
    }

    #[test]
    fn listening_expression_responds_to_both_tempo_and_level() {
        let rig = Rig::synthetic(&[]);
        let mut ctx = super::super::tests::ctx(&rig, 4.0, Style::default());
        ctx.bpm = 65.0;
        ctx.energy = 0.2;
        let soft = groove(&ctx).face;
        ctx.energy = 0.9;
        let slow_loud = groove(&ctx).face;
        ctx.bpm = 145.0;
        let fast = groove(&ctx).face;
        assert!(slow_loud.happy > soft.happy);
        assert!(fast.happy > slow_loud.happy);
        assert!(soft.relaxed > fast.relaxed && slow_loud.eyes_closed > fast.eyes_closed);
        ctx.beat += 0.5;
        assert!(groove(&ctx).face.surprised < fast.surprised, "accents land on the beat");
    }

    #[test]
    fn slow_songs_keep_their_feet_down_and_arms_below_the_shoulders() {
        let rig = Rig::synthetic(&[]);
        for bpm in [45.0, 65.0, 80.0] {
            for step in 0..128 {
                let mut ctx = super::super::tests::ctx(&rig, 5.0, Style::default());
                ctx.bpm = bpm;
                ctx.energy = 1.0;
                ctx.beat = step as f32 / 8.0;
                let act = dance(&ctx);
                assert_eq!(act.hop, 0.0);
                let lm = Landmarks::of(&rig, &act.pose);
                for side in Side::BOTH {
                    use crate::rig::Bone;
                    assert_eq!(act.feet[side.index()].lift, 0.0);
                    assert!(lm.pos(Bone::LeftHand.on(side)).y < lm.pos(Bone::LeftUpperArm.on(side)).y);
                }
                assert!(act.pose.rot.iter().all(|q| q.is_finite() && q.is_normalized()));
            }
        }
    }

    #[test]
    fn claps_land_on_two_and_four() {
        let rig = Rig::synthetic(&[]);
        let gap = |b: f32| {
            let ctx = Ctx { rig: &rig, t: 0.0, e: 5.0, style: Style::default(), talk: 0.0, energy: 0.8, beat: b, bpm: 120.0 };
            let lm = Landmarks::of(&rig, &bounce_clap(&ctx, b).pose);
            lm.pos(crate::rig::Bone::LeftHand).distance(lm.pos(crate::rig::Bone::RightHand))
        };
        assert!(gap(1.0) < gap(0.0) - 0.05, "hands meet on beat two: {} vs {}", gap(1.0), gap(0.0));
        assert!(gap(3.02) < gap(2.02) - 0.05, "and on beat four");
    }
}
