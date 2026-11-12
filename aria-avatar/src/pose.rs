//! A pose in normalized space, and the anatomical vocabulary poses are written in.
//!
//! Raw Euler angles on root-aligned bones are easy to get wrong (the previous finger code bent
//! every finger backward, and "elbow" rotations were twists about the forearm's own axis). So a
//! pose is described by what the body does: how far an arm hangs down, how much an elbow bends,
//! how tightly a finger curls. Each helper is written once for the **left** side and mirrored
//! across the body's midline for the right.
//!
//! Axes, for reference: +X is the model's left, +Y up, +Z the way she faces. In the T-pose rest the
//! left arm points along +X with the palm down, the legs along -Y.

use bevy::prelude::*;

use crate::ik;
use crate::rig::{Bone, Fk, Rig, Side};

/// One frame's normalized local rotation for every humanoid bone, plus the hips' offset from rest.
#[derive(Clone)]
pub struct Pose {
    pub rot: [Quat; Bone::COUNT],
    /// Hips translation from the rest position, in model space (metres).
    pub hips: Vec3,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            rot: [Quat::IDENTITY; Bone::COUNT],
            hips: Vec3::ZERO,
        }
    }
}

/// Mirrors a rotation across the body's midline (the YZ plane).
pub fn mirror(q: Quat) -> Quat {
    Quat::from_xyzw(q.x, -q.y, -q.z, q.w)
}

/// `q` as authored for the left side, or its mirror image for the right.
fn sided(side: Side, q: Quat) -> Quat {
    match side {
        Side::Left => q,
        Side::Right => mirror(q),
    }
}

/// Shoulder and elbow angles, in radians.
#[derive(Clone, Copy, Debug, Default)]
pub struct Arm {
    /// How far the upper arm hangs below horizontal: 0 is the T-pose, π/2 straight down, negative
    /// raises it above the shoulder.
    pub down: f32,
    /// Swing of a hanging arm forward (positive) or back.
    pub swing: f32,
    /// Sweep of a raised arm forward around the vertical axis.
    pub yaw: f32,
    /// Rotation about the arm's own axis: positive turns the elbow's bend toward the body.
    pub twist: f32,
    /// Elbow bend, 0 = straight.
    pub bend: f32,
    /// Forearm roll: positive turns the palm of a hanging arm to face backward.
    pub pronate: f32,
}

impl Arm {
    /// A relaxed arm hanging at the side.
    pub const HANG: Arm = Arm { down: 1.33, swing: 0.02, yaw: 0.0, twist: 0.3, bend: 0.22, pronate: 0.0 };
}

/// How closed each finger is, 0 = straight … 1 = fist, in the order thumb, index, middle, ring,
/// little; `spread` fans the fingers apart (negative squeezes them together).
#[derive(Clone, Copy, Debug)]
pub struct HandShape {
    pub curl: [f32; 5],
    pub spread: f32,
}

impl HandShape {
    pub const RELAXED: HandShape = HandShape { curl: [0.25, 0.28, 0.36, 0.44, 0.5], spread: 0.15 };
    pub const OPEN: HandShape = HandShape { curl: [0.0, 0.02, 0.02, 0.04, 0.06], spread: 0.9 };
    pub const FLAT: HandShape = HandShape { curl: [0.1, 0.03, 0.03, 0.04, 0.06], spread: 0.0 };
    pub const FIST: HandShape = HandShape { curl: [0.85, 1.0, 1.0, 1.0, 1.0], spread: 0.0 };
    pub const LOOSE_FIST: HandShape = HandShape { curl: [0.5, 0.72, 0.78, 0.82, 0.85], spread: 0.0 };
    pub const POINT: HandShape = HandShape { curl: [0.7, 0.0, 1.0, 1.0, 1.0], spread: 0.0 };

    /// Blends toward `other`.
    pub fn mix(self, other: HandShape, t: f32) -> HandShape {
        let mut curl = self.curl;
        for (c, o) in curl.iter_mut().zip(other.curl) {
            *c += (o - *c) * t;
        }
        HandShape { curl, spread: self.spread + (other.spread - self.spread) * t }
    }

    /// Curls every finger a little more (or less) than the shape says.
    pub fn tighten(mut self, amount: f32) -> HandShape {
        for c in &mut self.curl {
            *c = (*c + amount).clamp(0.0, 1.0);
        }
        self
    }
}

impl Pose {
    pub fn set(&mut self, bone: Bone, rotation: Quat) {
        self.rot[bone.index()] = rotation;
    }

    /// Rotates a bone further, in its parent's frame.
    pub fn add(&mut self, bone: Bone, rotation: Quat) {
        let slot = &mut self.rot[bone.index()];
        *slot = (rotation * *slot).normalize();
    }

    /// Blends every bone and the hips offset toward `other`. `t` may leave `0..1`, which
    /// extrapolates: that is how a transition overshoots and settles.
    pub fn blend(&mut self, other: &Pose, t: f32) {
        if t == 0.0 {
            return;
        }
        for (mine, theirs) in self.rot.iter_mut().zip(other.rot.iter()) {
            *mine = mine.slerp(*theirs, t);
        }
        self.hips = self.hips.lerp(other.hips, t);
    }

    /// Blends only the listed bones toward `other`.
    pub fn blend_bones(&mut self, other: &Pose, bones: impl IntoIterator<Item = Bone>, t: f32) {
        for bone in bones {
            let i = bone.index();
            self.rot[i] = self.rot[i].slerp(other.rot[i], t);
        }
    }

    // ---- Trunk ---------------------------------------------------------------------------------

    /// Bends the whole spine: `forward` leans toward the viewer, `twist` turns the shoulders toward
    /// the model's left, `side` leans toward the model's left. Spread over spine, chest and upper
    /// chest so the back curves instead of hinging.
    pub fn spine(&mut self, forward: f32, twist: f32, side: f32) {
        for (bone, share) in [(Bone::Spine, 0.3), (Bone::Chest, 0.35), (Bone::UpperChest, 0.35)] {
            self.add(bone, Quat::from_euler(EulerRot::YXZ, twist * share, forward * share, -side * share));
        }
    }

    /// Turns the head: `pitch` nods down, `yaw` looks toward the model's left (screen right),
    /// `roll` tips the head toward the model's left shoulder. Shared 40/60 with the neck.
    pub fn head(&mut self, pitch: f32, yaw: f32, roll: f32) {
        for (bone, share) in [(Bone::Neck, 0.4), (Bone::Head, 0.6)] {
            self.add(bone, Quat::from_euler(EulerRot::YXZ, yaw * share, pitch * share, -roll * share));
        }
    }

    /// Tilts and turns the pelvis, with the same sign conventions as [`Self::spine`].
    pub fn pelvis(&mut self, forward: f32, twist: f32, side: f32) {
        self.add(Bone::Hips, Quat::from_euler(EulerRot::YXZ, twist, forward, -side));
    }

    // ---- Arms ----------------------------------------------------------------------------------

    /// Raises (`up`) and rolls forward (`forward`) one shoulder.
    pub fn shrug(&mut self, side: Side, up: f32, forward: f32) {
        let q = Quat::from_rotation_z(up) * Quat::from_rotation_y(-forward);
        self.set(Bone::LeftShoulder.on(side), sided(side, q));
    }

    /// Poses the upper arm and elbow from anatomical angles.
    pub fn arm(&mut self, side: Side, arm: Arm) {
        let upper = Quat::from_rotation_y(-arm.yaw)
            * Quat::from_rotation_x(-arm.swing)
            * Quat::from_rotation_z(-arm.down)
            * Quat::from_rotation_x(arm.twist);
        let lower = Quat::from_rotation_y(-arm.bend) * Quat::from_rotation_x(arm.pronate);
        self.set(Bone::LeftUpperArm.on(side), sided(side, upper));
        self.set(Bone::LeftLowerArm.on(side), sided(side, lower));
    }

    /// Bends the wrist: `flex` toward the palm, `deviate` toward the thumb, `roll` about the
    /// forearm.
    pub fn wrist(&mut self, side: Side, flex: f32, deviate: f32, roll: f32) {
        let q = Quat::from_rotation_x(roll) * Quat::from_rotation_y(-deviate) * Quat::from_rotation_z(-flex);
        self.set(Bone::LeftHand.on(side), sided(side, q));
    }

    /// Curls and spreads the fingers.
    pub fn fingers(&mut self, side: Side, shape: HandShape) {
        // The thumb folds across the palm rather than curling down like the fingers: it drops
        // below the palm plane first, then its two outer joints close toward the index finger.
        let thumb = shape.curl[0];
        let spread = shape.spread;
        let thumb_joints = [
            Quat::from_rotation_x(0.55 * thumb) * Quat::from_rotation_y(0.35 * thumb - 0.25 * spread),
            Quat::from_rotation_y(0.75 * thumb),
            Quat::from_rotation_y(0.85 * thumb),
        ];
        for (joint, q) in thumb_joints.into_iter().enumerate() {
            self.set(Bone::finger(side, 0, joint), sided(side, q));
        }
        // Fan of the four fingers about the middle one, index toward the thumb.
        const FAN: [f32; 4] = [-0.14, -0.02, 0.09, 0.2];
        for finger in 1..5 {
            let curl = shape.curl[finger];
            let knuckle = Quat::from_rotation_y(FAN[finger - 1] * spread) * Quat::from_rotation_z(-1.4 * curl);
            self.set(Bone::finger(side, finger, 0), sided(side, knuckle));
            self.set(Bone::finger(side, finger, 1), sided(side, Quat::from_rotation_z(-1.65 * curl)));
            self.set(Bone::finger(side, finger, 2), sided(side, Quat::from_rotation_z(-1.1 * curl)));
        }
    }

    /// Places a hand with IK. `target` is where the wrist goes and `elbow` the direction the elbow
    /// points, both in model space; `hand` is the model-space rotation of the hand (see
    /// [`hand_aim`]), or `None` to let it follow the forearm. The trunk must be posed first.
    pub fn reach(&mut self, rig: &Rig, side: Side, target: Vec3, elbow: Vec3, hand: Option<Quat>) {
        let (upper, lower, wrist) = (
            Bone::LeftUpperArm.on(side),
            Bone::LeftLowerArm.on(side),
            Bone::LeftHand.on(side),
        );
        let fk = rig.fk(self);
        let limb = ik::two_bone(
            fk.pos[upper.index()],
            target,
            rig.rest(lower) - rig.rest(upper),
            rig.rest(wrist) - rig.rest(lower),
            Vec3::NEG_Z,
            elbow,
        );
        self.set(upper, fk.frame[upper.index()].inverse() * limb.upper);
        self.set(lower, limb.upper.inverse() * limb.lower);
        self.set(wrist, hand.map(|h| limb.lower.inverse() * h).unwrap_or(Quat::IDENTITY));
    }

    // ---- Legs (forward kinematics; grounded legs are solved with IK by the animator) -----------

    /// Poses a leg from joint angles: `swing` lifts the thigh forward, `spread` moves it outward,
    /// `twist` turns the knee outward, `knee` bends it, `point` points the toes down.
    pub fn leg(&mut self, side: Side, swing: f32, spread: f32, twist: f32, knee: f32, point: f32) {
        let thigh = Quat::from_rotation_x(-swing) * Quat::from_rotation_z(spread) * Quat::from_rotation_y(twist);
        self.set(Bone::LeftUpperLeg.on(side), sided(side, thigh));
        self.set(Bone::LeftLowerLeg.on(side), Quat::from_rotation_x(knee));
        self.set(Bone::LeftFoot.on(side), Quat::from_rotation_x(point));
    }
}

/// Model-space rotation that points a hand's fingers along `fingers` with the palm facing `palm`.
pub fn hand_aim(side: Side, fingers: Vec3, palm: Vec3) -> Quat {
    ik::aim(Vec3::X * side.sign(), Vec3::NEG_Y, fingers, palm)
}

/// Where the body's landmarks are for a pose, so hands can be sent to them.
pub struct Landmarks {
    pub fk: Fk,
}

impl Landmarks {
    pub fn of(rig: &Rig, pose: &Pose) -> Self {
        Self { fk: rig.fk(pose) }
    }

    pub fn pos(&self, bone: Bone) -> Vec3 {
        self.fk.pos[bone.index()]
    }

    pub fn rot(&self, bone: Bone) -> Quat {
        self.fk.rot[bone.index()]
    }

    /// A point given in a bone's own (normalized) space, e.g. `rig.metrics.chin` on the head.
    pub fn on(&self, bone: Bone, local: Vec3) -> Vec3 {
        self.pos(bone) + self.rot(bone) * local
    }

    /// A direction given in a bone's space.
    pub fn dir(&self, bone: Bone, local: Vec3) -> Vec3 {
        self.rot(bone) * local
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-3
    }

    /// Where a bone's own axis and its child end up after `pose`, using the synthetic rig.
    fn joint(rig: &Rig, pose: &Pose, bone: Bone) -> Vec3 {
        rig.fk(pose).pos[bone.index()]
    }

    #[test]
    fn a_hanging_arm_points_down_on_both_sides() {
        let rig = Rig::synthetic(&[]);
        let mut pose = Pose::default();
        for side in Side::BOTH {
            pose.arm(side, Arm { down: FRAC_PI_2, ..default() });
        }
        for side in Side::BOTH {
            let shoulder = joint(&rig, &pose, Bone::LeftUpperArm.on(side));
            let wrist = joint(&rig, &pose, Bone::LeftHand.on(side));
            let dir = (wrist - shoulder).normalize();
            assert!(close(dir, Vec3::NEG_Y), "{side:?} arm points {dir:?}");
        }
    }

    #[test]
    fn swinging_and_bending_move_the_hand_forward() {
        let rig = Rig::synthetic(&[]);
        for side in Side::BOTH {
            let mut pose = Pose::default();
            pose.arm(side, Arm { down: FRAC_PI_2, swing: 0.5, ..default() });
            let hand = joint(&rig, &pose, Bone::LeftHand.on(side));
            assert!(hand.z > 0.15, "{side:?} swing should bring the hand forward: {hand:?}");

            let mut pose = Pose::default();
            pose.arm(side, Arm { down: FRAC_PI_2, bend: FRAC_PI_2, ..default() });
            let elbow = joint(&rig, &pose, Bone::LeftLowerArm.on(side));
            let hand = joint(&rig, &pose, Bone::LeftHand.on(side));
            assert!(close((hand - elbow).normalize(), Vec3::Z), "{side:?} forearm: {:?}", hand - elbow);

            // Twisting inward brings a bent forearm across the body.
            let mut pose = Pose::default();
            pose.arm(side, Arm { down: FRAC_PI_2, twist: FRAC_PI_2, bend: FRAC_PI_2, ..default() });
            let elbow = joint(&rig, &pose, Bone::LeftLowerArm.on(side));
            let hand = joint(&rig, &pose, Bone::LeftHand.on(side));
            assert!(close((hand - elbow).normalize(), Vec3::X * -side.sign()), "{side:?}");
        }
    }

    #[test]
    fn fingers_curl_toward_the_palm() {
        let rig = Rig::synthetic(&[]);
        for side in Side::BOTH {
            let mut pose = Pose::default();
            pose.fingers(side, HandShape::FIST);
            let fk = rig.fk(&pose);
            // The palm faces down in the T-pose, so a curled fingertip drops below the knuckle.
            for finger in 1..5 {
                let knuckle = fk.pos[Bone::finger(side, finger, 0).index()];
                let tip = fk.pos[Bone::finger(side, finger, 2).index()];
                assert!(tip.y < knuckle.y - 0.01, "{side:?} finger {finger} bends the wrong way");
            }
        }
    }

    #[test]
    fn reach_puts_the_wrist_on_the_target() {
        let rig = Rig::synthetic(&[]);
        for side in Side::BOTH {
            let mut pose = Pose::default();
            pose.spine(0.2, 0.1, 0.0);
            let target = Vec3::new(side.sign() * 0.1, 1.2, 0.25);
            pose.reach(&rig, side, target, Vec3::new(side.sign(), -1.0, -0.3), None);
            let wrist = joint(&rig, &pose, Bone::LeftHand.on(side));
            assert!(wrist.distance(target) < 2e-3, "{side:?} wrist at {wrist:?}");
            let elbow = joint(&rig, &pose, Bone::LeftLowerArm.on(side));
            assert!(elbow.y < target.y, "{side:?} elbow should hang below the hand");
        }
    }

    #[test]
    fn legs_swing_forward_and_knees_fold_back() {
        let rig = Rig::synthetic(&[]);
        for side in Side::BOTH {
            let mut pose = Pose::default();
            pose.leg(side, 0.6, 0.2, 0.0, 0.0, 0.0);
            let knee = joint(&rig, &pose, Bone::LeftLowerLeg.on(side));
            let hip = joint(&rig, &pose, Bone::LeftUpperLeg.on(side));
            assert!(knee.z > hip.z + 0.1, "{side:?} thigh should swing forward");
            assert!((knee.x - hip.x) * side.sign() > 0.02, "{side:?} thigh should spread outward");

            let mut pose = Pose::default();
            pose.leg(side, 0.0, 0.0, 0.0, 1.0, 0.0);
            let knee = joint(&rig, &pose, Bone::LeftLowerLeg.on(side));
            let ankle = joint(&rig, &pose, Bone::LeftFoot.on(side));
            assert!(ankle.z < knee.z - 0.1, "{side:?} shin should fold back");
        }
    }

    #[test]
    fn blending_extrapolates_for_overshoot() {
        let mut a = Pose::default();
        let mut b = Pose::default();
        b.set(Bone::Head, Quat::from_rotation_y(1.0));
        b.hips = Vec3::Y;
        a.blend(&b, 1.2);
        let (axis, angle) = a.rot[Bone::Head.index()].to_axis_angle();
        assert!((angle * axis.y - 1.2).abs() < 1e-3);
        assert!((a.hips.y - 1.2).abs() < 1e-5);
    }

    #[test]
    fn hand_aim_points_fingers_and_palm() {
        for side in Side::BOTH {
            let q = hand_aim(side, Vec3::Y, Vec3::Z);
            assert!(close(q * (Vec3::X * side.sign()), Vec3::Y));
            assert!(close(q * Vec3::NEG_Y, Vec3::Z));
        }
    }
}
