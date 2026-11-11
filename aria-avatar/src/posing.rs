//! Procedural posing, ported from `animate()` in `docs/reference/vtuber_stage.html`.
//!
//! Every rotation is a normalized-bone Euler angle (three.js `XYZ` order) and every constant is
//! copied from the reference verbatim; [`crate::rig::Rig`] maps them onto the raw skeleton. Bones a
//! branch does not mention return to rest each frame, so switching emotion never leaves a limb
//! stranded in the previous pose.

use bevy::prelude::*;

use aria_protocol::PeekSide;

use crate::behaviors::AvatarState;

/// One frame's pose in normalized space.
#[derive(Default)]
pub struct Pose {
    pub bones: Vec<(&'static str, Vec3)>,
    /// Hips offset from rest, in metres (the reference's `hips.position.y`).
    pub hips_offset_y: f32,
}

impl Pose {
    fn set(&mut self, bone: &'static str, x: f32, y: f32, z: f32) {
        self.bones.push((bone, Vec3::new(x, y, z)));
    }

    /// `applyHandPosing(side, curl, spread, wristX, wristY, wristZ)` from the reference.
    fn hand(&mut self, left: bool, curl: f32, spread: f32, wrist: Vec3) {
        let sign = if left { 1.0 } else { -1.0 };
        let (hand, fingers): (&'static str, [[&'static str; 3]; 5]) = if left {
            (
                "leftHand",
                [
                    ["leftThumbMetacarpal", "leftThumbProximal", "leftThumbDistal"],
                    ["leftIndexProximal", "leftIndexIntermediate", "leftIndexDistal"],
                    ["leftMiddleProximal", "leftMiddleIntermediate", "leftMiddleDistal"],
                    ["leftRingProximal", "leftRingIntermediate", "leftRingDistal"],
                    ["leftLittleProximal", "leftLittleIntermediate", "leftLittleDistal"],
                ],
            )
        } else {
            (
                "rightHand",
                [
                    ["rightThumbMetacarpal", "rightThumbProximal", "rightThumbDistal"],
                    ["rightIndexProximal", "rightIndexIntermediate", "rightIndexDistal"],
                    ["rightMiddleProximal", "rightMiddleIntermediate", "rightMiddleDistal"],
                    ["rightRingProximal", "rightRingIntermediate", "rightRingDistal"],
                    ["rightLittleProximal", "rightLittleIntermediate", "rightLittleDistal"],
                ],
            )
        };
        self.set(hand, wrist.x, wrist.y, wrist.z);

        // Thumb.
        let t_curl = curl * 0.45;
        let [a, b, c] = fingers[0];
        self.set(a, 0.0, sign * 0.12, sign * (t_curl * 0.25));
        self.set(b, 0.0, 0.0, sign * (t_curl * 0.45));
        self.set(c, 0.0, 0.0, sign * (t_curl * 0.35));

        // index, middle, ring, little: (curl weight, spread direction)
        let tuning = [(0.90, 0.06), (1.0, 0.0), (1.05, -0.04), (1.10, -0.08)];
        for (joints, (weight, spread_dir)) in fingers[1..].iter().zip(tuning) {
            let base = curl * weight;
            let spread_y = spread_dir * spread;
            self.set(joints[0], 0.0, sign * spread_y, sign * (base * 0.38));
            self.set(joints[1], 0.0, 0.0, sign * (base * 0.42));
            self.set(joints[2], 0.0, 0.0, sign * (base * 0.25));
        }
    }
}

/// The emotion branch the reference would pick this frame.
fn grooving(state: &AvatarState) -> bool {
    let emo = state.emotion.as_str();
    emo == "groove" || (state.music_active && emo == "idle")
}

/// Builds the pose for time `t` (seconds since start).
pub fn pose_for(state: &AvatarState, t: f32) -> Pose {
    let mut pose = Pose::default();
    let emo = state.emotion.as_str();
    let smx = state.smoothed_look_x;
    let smy = state.smoothed_look_y;
    let breath = (t * 2.2).sin();
    let sway = (t * 1.3).sin();
    let emo_elapsed = state.emotion_elapsed;
    let headpat = state.headpat_active;

    pose.set(
        "spine",
        breath * 0.025,
        0.0,
        if emo == "dizzy" { (t * 9.0).sin() * 0.11 } else { sway * 0.025 },
    );
    pose.set("chest", breath * 0.03, 0.0, 0.0);
    pose.set("neck", -smy * 0.14, smx * 0.22, 0.0);

    let head_shake = if emo == "annoyed" && emo_elapsed < 1.2 {
        (t * 26.0).sin() * 0.14
    } else if emo == "dizzy" {
        (t * 14.0).sin() * 0.24
    } else {
        0.0
    };
    let mut head = Vec3::new(
        if emo == "sleepy" {
            0.18 + (t * 1.2).sin() * 0.08
        } else {
            -smy * 0.18 + (t * 1.8).sin() * 0.025
        },
        smx * 0.28 + head_shake,
        if emo == "dizzy" {
            (t * 14.0).cos() * 0.22
        } else {
            (if emo == "idle" || emo == "happy" { 0.07 } else { -0.04 }) * sway
        },
    );
    if headpat > 0.0 {
        head.z += (t * 7.5).sin() * 0.08 * headpat;
        head.x += 0.08 * headpat;
    }
    match state.peeking {
        PeekSide::Left => head.z += 0.09,
        PeekSide::Right => head.z -= 0.09,
        PeekSide::None => {}
    }

    let mut hips = Vec3::ZERO;
    let talking = emo == "talking" || state.talking;

    if state.moving || emo == "walking" {
        let walk_cycle = t * 8.5;
        let stride = walk_cycle.sin();
        pose.hips_offset_y = walk_cycle.cos().abs() * 0.035;
        hips.y = stride * 0.08;
        pose.set("leftUpperArm", -stride * 0.45, 0.0, -1.18);
        pose.set("rightUpperArm", stride * 0.45, 0.0, 1.18);
        pose.set("leftLowerArm", -0.35, 0.0, 0.0);
        pose.set("rightLowerArm", -0.35, 0.0, 0.0);
        pose.set("leftUpperLeg", stride * 0.52, 0.0, 0.0);
        pose.set("rightUpperLeg", -stride * 0.52, 0.0, 0.0);
        pose.set("leftLowerLeg", (-stride).max(0.0) * 0.55, 0.0, 0.0);
        pose.set("rightLowerLeg", stride.max(0.0) * 0.55, 0.0, 0.0);
        let curl = 0.42 + walk_cycle.sin() * 0.08;
        pose.hand(true, curl, 0.35, Vec3::new(-stride * 0.22 + 0.08, -stride * 0.04, 0.06));
        pose.hand(false, curl, 0.35, Vec3::new(stride * 0.22 + 0.08, stride * 0.04, -0.06));
    } else if emo == "dance" {
        let beat = t * 7.5;
        pose.hips_offset_y = beat.sin().abs() * 0.045;
        hips.z = (beat * 0.5).cos() * 0.09;
        pose.set("leftUpperArm", 0.3, 0.2, -0.75 + beat.sin() * 0.25);
        pose.set("rightUpperArm", 0.3, -0.2, 0.75 + beat.cos() * 0.25);
        pose.set("leftLowerArm", -0.85, 0.3, 0.2);
        pose.set("rightLowerArm", -0.85, -0.3, -0.2);
        pose.set("leftUpperLeg", beat.sin() * 0.25, 0.0, -0.05);
        pose.set("rightUpperLeg", -beat.sin() * 0.25, 0.0, 0.05);
        let curl = 0.35 + (beat * 2.0).sin() * 0.25;
        pose.hand(true, curl, 0.5, Vec3::new(beat.sin() * 0.3, 0.15, 0.2));
        pose.hand(false, curl, 0.5, Vec3::new(beat.cos() * 0.3, -0.15, -0.2));
    } else if emo == "sit" {
        pose.hips_offset_y = -0.12;
        hips.z = sway * 0.025;
        pose.set("leftUpperArm", 0.18, 0.1, -1.12);
        pose.set("rightUpperArm", 0.18, -0.1, 1.12);
        pose.set("leftLowerArm", -0.35, 0.2, 0.1);
        pose.set("rightLowerArm", -0.35, -0.2, -0.1);
        pose.set("leftUpperLeg", 1.22, 0.06, -0.05);
        pose.set("rightUpperLeg", 1.22, -0.06, 0.05);
        pose.set("leftLowerLeg", -1.25 + (t * 2.8).sin() * 0.18, 0.0, 0.0);
        pose.set("rightLowerLeg", -1.25 - (t * 2.8).sin() * 0.18, 0.0, 0.0);
        pose.hand(true, 0.55, 0.25, Vec3::new(0.25, 0.12, 0.05));
        pose.hand(false, 0.55, 0.25, Vec3::new(0.25, -0.12, -0.05));
    } else if grooving(state) {
        // Music vibe: the reference grooves at a fixed amplitude; here the loudness the app
        // measures scales it, never below the reference's own 0.35 floor.
        let energy = if state.music_active {
            state.music_energy.clamp(0.35, 1.0)
        } else {
            1.0
        };
        let beat = t * 6.8;
        pose.hips_offset_y = beat.sin().abs() * 0.03 * energy;
        hips.z = (beat * 0.5).sin() * 0.07 * energy;
        head.x += beat.sin().abs() * 0.14 * energy;
        pose.set("leftUpperArm", 0.2, 0.15, -0.98 + beat.sin() * 0.12 * energy);
        pose.set("rightUpperArm", 0.2, -0.15, 0.98 - beat.sin() * 0.12 * energy);
        pose.set("leftLowerArm", -0.65, 0.2, 0.15);
        pose.set("rightLowerArm", -0.65, -0.2, -0.15);
        let curl = 0.38 + beat.sin() * 0.18;
        pose.hand(true, curl, 0.4, Vec3::new(0.15, (beat * 0.5).sin() * 0.2, 0.1));
        pose.hand(false, curl, 0.4, Vec3::new(0.15, -(beat * 0.5).sin() * 0.2, -0.1));
    } else if emo == "happy" {
        let bounce = (t * 5.2).sin();
        pose.hips_offset_y = bounce.abs() * 0.02;
        hips.z = bounce * 0.05;
        pose.set("leftUpperArm", 0.25, 0.35, -0.92);
        pose.set("rightUpperArm", 0.25, -0.35, 0.92);
        pose.set("leftLowerArm", -0.95, 0.45, 0.35);
        pose.set("rightLowerArm", -0.95, -0.45, -0.35);
        let curl = 0.22 + (t * 6.0).sin() * 0.12;
        pose.hand(true, curl, 0.65, Vec3::new(-0.2, 0.15, 0.18));
        pose.hand(false, curl, 0.65, Vec3::new(-0.2, -0.15, -0.18));
    } else if emo == "dizzy" {
        hips.z = (t * 8.0).sin() * 0.08;
        pose.set("leftUpperArm", 0.2, 0.0, -0.95 + (t * 10.0).sin() * 0.15);
        pose.set("rightUpperArm", 0.2, 0.0, 0.95 + (t * 10.0).cos() * 0.15);
        pose.set("leftLowerArm", -0.4, 0.0, 0.0);
        pose.set("rightLowerArm", -0.4, 0.0, 0.0);
        let curl = 0.4 + (t * 8.0).sin() * 0.2;
        pose.hand(true, curl, 0.3, Vec3::new((t * 10.0).sin() * 0.3, 0.0, 0.2));
        pose.hand(false, curl, 0.3, Vec3::new((t * 10.0).cos() * 0.3, 0.0, -0.2));
    } else if emo == "annoyed" {
        hips.z = 0.04;
        pose.set("leftUpperArm", 0.15, 0.25, -0.85);
        pose.set("rightUpperArm", 0.15, -0.25, 0.85);
        pose.set("leftLowerArm", -0.65, 0.5, 0.45);
        pose.set("rightLowerArm", -0.65, -0.5, -0.45);
        pose.set("leftUpperLeg", 0.0, 0.0, -0.04);
        pose.set("rightUpperLeg", 0.0, 0.0, 0.04);
        pose.hand(true, 0.95, 0.1, Vec3::new(0.15, 0.1, 0.2));
        pose.hand(false, 0.95, 0.1, Vec3::new(0.15, -0.1, -0.2));
    } else if talking {
        let wave = (t * 4.5).sin();
        pose.set("leftUpperArm", 0.12, 0.0, -1.16 + breath * 0.03);
        pose.set("rightUpperArm", 0.28 + wave * 0.08, -0.15, 0.98);
        pose.set("leftLowerArm", -0.22, 0.0, 0.0);
        pose.set("rightLowerArm", -0.75 + wave * 0.18, -0.3, -0.2);
        let curl = 0.32 + (t * 5.0).sin() * 0.22;
        pose.hand(true, 0.4, 0.35, Vec3::new(0.05, 0.05, 0.05));
        pose.hand(false, curl, 0.5, Vec3::new(-0.25 + wave * 0.2, -0.2, -0.15));
    } else {
        hips.z = sway * 0.02;
        pose.set("leftUpperArm", 0.06, 0.0, -1.20 + breath * 0.025);
        pose.set("rightUpperArm", 0.06, 0.0, 1.20 - breath * 0.025);
        pose.set("leftLowerArm", -0.18, 0.12, 0.08);
        pose.set("rightLowerArm", -0.18, -0.12, -0.08);
        pose.set("leftUpperLeg", 0.0, 0.0, -0.02);
        pose.set("rightUpperLeg", 0.0, 0.0, 0.02);
        let curl = 0.45 + (t * 2.2).sin() * 0.05;
        pose.hand(true, curl, 0.3, Vec3::new(0.08, 0.04, 0.04));
        pose.hand(false, curl, 0.3, Vec3::new(0.08, -0.04, -0.04));
    }

    pose.set("hips", hips.x, hips.y, hips.z);
    pose.set("head", head.x, head.y, head.z);
    pose
}

/// The model root's Euler rotation: yaw toward the cursor / walk direction / peek side, the dance
/// spin, and roll/pitch from peeking and drag inertia. `current_yaw` is last frame's yaw.
pub fn body_rotation(state: &AvatarState, current_yaw: f32, dt: f32) -> Vec3 {
    let ease = (dt * 7.0).min(1.0);
    let yaw = match state.peeking {
        PeekSide::Left => current_yaw + (0.38 - current_yaw) * ease,
        PeekSide::Right => current_yaw + (-0.38 - current_yaw) * ease,
        PeekSide::None if state.emotion == "dance" && state.emotion_elapsed < 4.5 => {
            // One and a half turns on entering a dance.
            (state.emotion_elapsed * std::f32::consts::PI * 1.6) % std::f32::consts::TAU
        }
        PeekSide::None => {
            let target = if state.moving || state.emotion == "walking" {
                if state.facing_right { 0.38 } else { -0.38 }
            } else {
                state.smoothed_look_x * 0.18
            };
            // After a spin the stored yaw can be anywhere in 0..2π; take the short way back.
            let mut delta = target - current_yaw;
            while delta > std::f32::consts::PI {
                delta -= std::f32::consts::TAU;
            }
            while delta < -std::f32::consts::PI {
                delta += std::f32::consts::TAU;
            }
            current_yaw + delta * ease
        }
    };
    let peek_lean = match state.peeking {
        PeekSide::Left => 0.14,
        PeekSide::Right => -0.14,
        PeekSide::None => 0.0,
    };
    Vec3::new(
        state.inertia_y * 0.16,
        yaw,
        -state.inertia_x * 0.22 + peek_lean,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_pose_has_arms_down_and_both_hands() {
        let state = AvatarState::default();
        let pose = pose_for(&state, 0.0);
        let left = pose.bones.iter().find(|(n, _)| *n == "leftUpperArm").unwrap().1;
        assert!((left.z + 1.20).abs() < 0.05, "left arm should hang at about -1.2 rad");
        // hand + 3 thumb + 4x3 fingers per side
        let finger_count = pose.bones.iter().filter(|(n, _)| n.contains("Thumb") || n.contains("Index") || n.contains("Middle") || n.contains("Ring") || n.contains("Little")).count();
        assert_eq!(finger_count, 30);
    }

    #[test]
    fn body_yaw_unwinds_after_a_spin() {
        let mut state = AvatarState::default();
        state.emotion = "idle".into();
        let wanted = body_rotation(&state, 6.0, 0.1);
        // From 6.0 rad the short way to ~0 is forward through 2π, not back through π.
        assert!(wanted.y > 6.0);
    }
}
