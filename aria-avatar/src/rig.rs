//! A three-vrm style *normalized* humanoid rig on top of `bevy_vrm1`'s raw bones.
//!
//! Every pose in this crate is written against virtual bones whose rest rotation is identity and
//! whose axes line up with the model root (+Y up, +Z the way the model faces, +X the model's left).
//! `bevy_vrm1` exposes the raw glTF bones instead, whose rest orientation is whatever the exporter
//! wrote, so three-vrm's `VRMHumanoidRig.update()` mapping is applied when a pose is written:
//!
//! ```text
//! raw_local = P⁻¹ · N · P · rest_local      where P = the parent's rest rotation in root space
//! ```
//!
//! `bevy_vrm1` snapshots each bone's rest transforms (`RestTransform` / `RestGlobalTransform`), so
//! the mapping is exact for any model.
//!
//! # Any VRM, not just the bundled one
//!
//! Only 15 humanoid bones are mandatory in VRM; chest, neck, shoulders, toes and every finger are
//! optional. Nothing here assumes they exist:
//!
//! * a rotation posed on a **missing** bone is folded into its children (a model without a chest
//!   still leans its shoulders and head by the chest's share), which [`Fk`] does for free by
//!   walking the fixed humanoid hierarchy rather than the model's own;
//! * every length a pose needs (leg and arm segments, hip width, where the chin is) is measured off
//!   the rest pose in [`Metrics`], so IK targets land on the body whatever its proportions.

use bevy::ecs::world::EntityRef;
use bevy::prelude::*;
// Not the whole prelude: it exports a marker component per bone (`Hips`, `Head`, …) that would
// collide with the `Bone` variants used unqualified below.
use bevy_vrm1::prelude::{RestGlobalTransform, RestTransform};

/// Left or right, as seen by the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::Left, Side::Right];

    /// +1 for the left side (the model's +X), -1 for the right.
    pub fn sign(self) -> f32 {
        match self {
            Side::Left => 1.0,
            Side::Right => -1.0,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

macro_rules! bones {
    ($( $variant:ident : $holder:ident = $name:literal, $parent:expr; )*) => {
        /// The 55 VRM humanoid bones. Parents always precede their children, and the right limb
        /// block mirrors the left one entry for entry (see [`Bone::on`]).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Bone { $($variant),* }

        impl Bone {
            pub const ALL: &'static [Bone] = &[$(Bone::$variant),*];
            pub const COUNT: usize = Bone::ALL.len();

            /// The VRM specification's name for the bone.
            pub fn name(self) -> &'static str {
                match self { $(Bone::$variant => $name),* }
            }

            /// Parent in the VRM humanoid hierarchy, whether or not a model has that bone.
            pub fn parent(self) -> Option<Bone> {
                match self { $(Bone::$variant => $parent),* }
            }
        }

        /// Reads the bone entities off the VRM root's `*BoneEntity` holders.
        fn bone_entities(root: &EntityRef) -> [Option<Entity>; Bone::COUNT] {
            [$(root.get::<bevy_vrm1::prelude::$holder>().map(|holder| holder.0)),*]
        }
    };
}

use Bone::*;

bones! {
    Hips: HipsBoneEntity = "hips", None;
    Spine: SpineBoneEntity = "spine", Some(Hips);
    Chest: ChestBoneEntity = "chest", Some(Spine);
    UpperChest: UpperChestBoneEntity = "upperChest", Some(Chest);
    Neck: NeckBoneEntity = "neck", Some(UpperChest);
    Head: HeadBoneEntity = "head", Some(Neck);
    LeftEye: LeftEyeBoneEntity = "leftEye", Some(Head);
    RightEye: RightEyeBoneEntity = "rightEye", Some(Head);
    Jaw: JawBoneEntity = "jaw", Some(Head);

    LeftShoulder: LeftShoulderBoneEntity = "leftShoulder", Some(UpperChest);
    LeftUpperArm: LeftUpperArmBoneEntity = "leftUpperArm", Some(LeftShoulder);
    LeftLowerArm: LeftLowerArmBoneEntity = "leftLowerArm", Some(LeftUpperArm);
    LeftHand: LeftHandBoneEntity = "leftHand", Some(LeftLowerArm);
    LeftThumbMetacarpal: LeftThumbMetacarpalBoneEntity = "leftThumbMetacarpal", Some(LeftHand);
    LeftThumbProximal: LeftThumbProximalBoneEntity = "leftThumbProximal", Some(LeftThumbMetacarpal);
    LeftThumbDistal: LeftThumbDistalBoneEntity = "leftThumbDistal", Some(LeftThumbProximal);
    LeftIndexProximal: LeftIndexProximalBoneEntity = "leftIndexProximal", Some(LeftHand);
    LeftIndexIntermediate: LeftIndexIntermediateBoneEntity = "leftIndexIntermediate", Some(LeftIndexProximal);
    LeftIndexDistal: LeftIndexDistalBoneEntity = "leftIndexDistal", Some(LeftIndexIntermediate);
    LeftMiddleProximal: LeftMiddleProximalBoneEntity = "leftMiddleProximal", Some(LeftHand);
    LeftMiddleIntermediate: LeftMiddleIntermediateBoneEntity = "leftMiddleIntermediate", Some(LeftMiddleProximal);
    LeftMiddleDistal: LeftMiddleDistalBoneEntity = "leftMiddleDistal", Some(LeftMiddleIntermediate);
    LeftRingProximal: LeftRingProximalBoneEntity = "leftRingProximal", Some(LeftHand);
    LeftRingIntermediate: LeftRingIntermediateBoneEntity = "leftRingIntermediate", Some(LeftRingProximal);
    LeftRingDistal: LeftRingDistalBoneEntity = "leftRingDistal", Some(LeftRingIntermediate);
    LeftLittleProximal: LeftLittleProximalBoneEntity = "leftLittleProximal", Some(LeftHand);
    LeftLittleIntermediate: LeftLittleIntermediateBoneEntity = "leftLittleIntermediate", Some(LeftLittleProximal);
    LeftLittleDistal: LeftLittleDistalBoneEntity = "leftLittleDistal", Some(LeftLittleIntermediate);
    LeftUpperLeg: LeftUpperLegBoneEntity = "leftUpperLeg", Some(Hips);
    LeftLowerLeg: LeftLowerLegBoneEntity = "leftLowerLeg", Some(LeftUpperLeg);
    LeftFoot: LeftFootBoneEntity = "leftFoot", Some(LeftLowerLeg);
    LeftToes: LeftToesBoneEntity = "leftToes", Some(LeftFoot);

    RightShoulder: RightShoulderBoneEntity = "rightShoulder", Some(UpperChest);
    RightUpperArm: RightUpperArmBoneEntity = "rightUpperArm", Some(RightShoulder);
    RightLowerArm: RightLowerArmBoneEntity = "rightLowerArm", Some(RightUpperArm);
    RightHand: RightHandBoneEntity = "rightHand", Some(RightLowerArm);
    RightThumbMetacarpal: RightThumbMetacarpalBoneEntity = "rightThumbMetacarpal", Some(RightHand);
    RightThumbProximal: RightThumbProximalBoneEntity = "rightThumbProximal", Some(RightThumbMetacarpal);
    RightThumbDistal: RightThumbDistalBoneEntity = "rightThumbDistal", Some(RightThumbProximal);
    RightIndexProximal: RightIndexProximalBoneEntity = "rightIndexProximal", Some(RightHand);
    RightIndexIntermediate: RightIndexIntermediateBoneEntity = "rightIndexIntermediate", Some(RightIndexProximal);
    RightIndexDistal: RightIndexDistalBoneEntity = "rightIndexDistal", Some(RightIndexIntermediate);
    RightMiddleProximal: RightMiddleProximalBoneEntity = "rightMiddleProximal", Some(RightHand);
    RightMiddleIntermediate: RightMiddleIntermediateBoneEntity = "rightMiddleIntermediate", Some(RightMiddleProximal);
    RightMiddleDistal: RightMiddleDistalBoneEntity = "rightMiddleDistal", Some(RightMiddleIntermediate);
    RightRingProximal: RightRingProximalBoneEntity = "rightRingProximal", Some(RightHand);
    RightRingIntermediate: RightRingIntermediateBoneEntity = "rightRingIntermediate", Some(RightRingProximal);
    RightRingDistal: RightRingDistalBoneEntity = "rightRingDistal", Some(RightRingIntermediate);
    RightLittleProximal: RightLittleProximalBoneEntity = "rightLittleProximal", Some(RightHand);
    RightLittleIntermediate: RightLittleIntermediateBoneEntity = "rightLittleIntermediate", Some(RightLittleProximal);
    RightLittleDistal: RightLittleDistalBoneEntity = "rightLittleDistal", Some(RightLittleIntermediate);
    RightUpperLeg: RightUpperLegBoneEntity = "rightUpperLeg", Some(Hips);
    RightLowerLeg: RightLowerLegBoneEntity = "rightLowerLeg", Some(RightUpperLeg);
    RightFoot: RightFootBoneEntity = "rightFoot", Some(RightLowerLeg);
    RightToes: RightToesBoneEntity = "rightToes", Some(RightFoot);
}

/// Number of bones in one side's limb block.
const SIDE_BLOCK: usize = RightShoulder as usize - LeftShoulder as usize;

impl Bone {
    pub fn index(self) -> usize {
        self as usize
    }

    /// The same bone on `side`. Meant for the `Left*` names; a bone of the trunk is returned as is.
    pub fn on(self, side: Side) -> Bone {
        let i = self.index();
        let left = LeftShoulder.index()..RightShoulder.index();
        match side {
            Side::Right if self == LeftEye => RightEye,
            Side::Right if left.contains(&i) => Bone::ALL[i + SIDE_BLOCK],
            _ => self,
        }
    }

    /// Finger joint: `finger` 0 = thumb … 4 = little, `joint` 0 = nearest the palm … 2 = tip.
    pub fn finger(side: Side, finger: usize, joint: usize) -> Bone {
        Bone::ALL[LeftThumbMetacarpal.index() + finger.min(4) * 3 + joint.min(2)].on(side)
    }
}

/// One raw bone and the rest data needed to map normalized rotations onto it.
#[derive(Clone, Copy, Debug)]
struct RigBone {
    entity: Entity,
    rest_local: Transform,
    /// Rest rotation of the raw parent node, relative to the VRM root (`P` above).
    parent_rest: Quat,
    /// Rest rotation and scale of this bone relative to the VRM root.
    rest_world_rotation: Quat,
    rest_world_scale: Vec3,
    /// Nearest ancestor (in the humanoid hierarchy) that the model actually has.
    anchor: Option<Bone>,
}

/// Body measurements taken off the rest pose, in metres and root space.
///
/// Poses are authored against these instead of constants, which is what makes a hand reach the
/// chin or a foot reach the floor on a model of any height or build.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// Estimated full height including hair.
    pub height: f32,
    /// `height / 1.6`: scales authored offsets that have no bone to measure.
    pub unit: f32,
    pub upper_leg: f32,
    pub lower_leg: f32,
    /// Ankle height above the floor when standing.
    pub ankle_height: f32,
    /// Horizontal distance from the ankle forward to the ball of the foot.
    pub ball_length: f32,
    /// Half the distance between the hip joints.
    pub hip_half_width: f32,
    pub upper_arm: f32,
    pub lower_arm: f32,
    /// Face landmarks relative to the head bone, in head space.
    pub eye: Vec3,
    pub mouth: Vec3,
    pub chin: Vec3,
    /// Distance between the eyes; the unit the face landmarks are built from.
    pub eye_spacing: f32,
}

impl Metrics {
    /// Hip joint to ankle with the leg straight.
    pub fn leg(&self) -> f32 {
        self.upper_leg + self.lower_leg
    }
}

/// Every resolved humanoid bone of one loaded model.
#[derive(Clone)]
pub struct Rig {
    bones: [Option<RigBone>; Bone::COUNT],
    /// Rest position of every bone in root space; a missing bone sits on its anchor.
    rest_pos: [Vec3; Bone::COUNT],
    pub metrics: Metrics,
}

/// Forward kinematics of a [`crate::pose::Pose`] in normalized (model) space.
pub struct Fk {
    /// Rotation of the frame a bone's own pose rotation is applied in.
    pub frame: [Quat; Bone::COUNT],
    /// Model-space rotation of each bone.
    pub rot: [Quat; Bone::COUNT],
    /// Model-space position of each bone's joint.
    pub pos: [Vec3; Bone::COUNT],
}

impl Rig {
    /// Reads the rig off a VRM root. `None` until `bevy_vrm1` has attached the bone holders and
    /// snapshotted the rest transforms, which happens a few frames after the scene spawns.
    pub fn resolve(world: &World, root: Entity) -> Option<Rig> {
        let root = world.get_entity(root).ok()?;
        let entities = bone_entities(&root);
        entities[Hips.index()]?;

        let mut bones = [None; Bone::COUNT];
        let mut rest_pos = [Vec3::ZERO; Bone::COUNT];
        for &bone in Bone::ALL {
            // Parents come first in `Bone::ALL`, so the anchor is already settled.
            let anchor = bone.parent().and_then(|parent| {
                bones[parent.index()]
                    .as_ref()
                    .map(|_| parent)
                    .or(Self::anchor_in(&bones, parent))
            });
            let Some(entity) = entities[bone.index()] else {
                rest_pos[bone.index()] = anchor.map(|a| rest_pos[a.index()]).unwrap_or(Vec3::ZERO);
                continue;
            };
            // Rest snapshots land together; a missing one means "not ready yet".
            let rest_local = world.get::<RestTransform>(entity)?.0;
            let rest_global = world.get::<RestGlobalTransform>(entity)?;
            let (scale, rotation, translation) = rest_global.0.to_scale_rotation_translation();
            rest_pos[bone.index()] = translation;
            bones[bone.index()] = Some(RigBone {
                entity,
                rest_local,
                parent_rest: rotation * rest_local.rotation.inverse(),
                rest_world_rotation: rotation,
                rest_world_scale: scale,
                anchor,
            });
        }

        let metrics = Metrics::measure(&bones, &rest_pos);
        Some(Rig { bones, rest_pos, metrics })
    }

    fn anchor_in(bones: &[Option<RigBone>; Bone::COUNT], mut bone: Bone) -> Option<Bone> {
        // `bone` itself is missing; its own anchor was computed when it was visited.
        while let Some(parent) = bone.parent() {
            if bones[parent.index()].is_some() {
                return Some(parent);
            }
            bone = parent;
        }
        None
    }

    /// The humanoid bones this model does not have (they are optional in VRM).
    pub fn missing(&self) -> Vec<&'static str> {
        Bone::ALL
            .iter()
            .filter(|bone| self.bones[bone.index()].is_none())
            .map(|bone| bone.name())
            .collect()
    }

    /// How many humanoid bones the model has.
    pub fn len(&self) -> usize {
        self.bones.iter().flatten().count()
    }

    /// Rest position of a bone's joint in root space.
    pub fn rest(&self, bone: Bone) -> Vec3 {
        self.rest_pos[bone.index()]
    }

    /// Forward kinematics of `pose`: where every joint ends up, in model space.
    pub fn fk(&self, pose: &crate::pose::Pose) -> Fk {
        let mut fk = Fk {
            frame: [Quat::IDENTITY; Bone::COUNT],
            rot: [Quat::IDENTITY; Bone::COUNT],
            pos: [Vec3::ZERO; Bone::COUNT],
        };
        for &bone in Bone::ALL {
            let i = bone.index();
            // The frame comes from the humanoid parent even when the model lacks it: that is what
            // folds a missing bone's rotation into its children.
            let frame = bone.parent().map(|p| fk.rot[p.index()]).unwrap_or(Quat::IDENTITY);
            fk.frame[i] = frame;
            fk.rot[i] = (frame * pose.rot[i]).normalize();
            fk.pos[i] = match self.bones[i].as_ref().map(|b| b.anchor) {
                // The root of the skeleton.
                Some(None) => self.rest_pos[i] + pose.hips,
                Some(Some(anchor)) => {
                    let a = anchor.index();
                    fk.pos[a] + fk.rot[a] * (self.rest_pos[i] - self.rest_pos[a])
                }
                // Missing: collapsed onto its parent.
                None => bone.parent().map(|p| fk.pos[p.index()]).unwrap_or(self.rest_pos[i]),
            };
        }
        fk
    }

    /// Writes `pose` onto the raw skeleton.
    pub fn apply<F: bevy::ecs::query::QueryFilter>(
        &self,
        pose: &crate::pose::Pose,
        transforms: &mut Query<&mut Transform, F>,
    ) {
        let fk = self.fk(pose);
        for &bone in Bone::ALL {
            // Eyes belong to `bevy_vrm1`'s look-at, which runs later in the frame.
            if matches!(bone, LeftEye | RightEye) {
                continue;
            }
            let Some(raw) = self.bones[bone.index()].as_ref() else { continue };
            let Ok(mut transform) = transforms.get_mut(raw.entity) else { continue };
            // Normalized rotation relative to the nearest bone the model has.
            let normalized = match raw.anchor {
                Some(anchor) => fk.rot[anchor.index()].inverse() * fk.rot[bone.index()],
                None => fk.rot[bone.index()],
            };
            let rotation = raw.parent_rest.inverse() * normalized * raw.parent_rest * raw.rest_local.rotation;
            // Writing an unchanged value would still flag the transform as changed.
            if transform.rotation != rotation {
                transform.rotation = rotation;
            }
            if bone == Hips {
                let parent_scale = (raw.rest_world_scale / raw.rest_local.scale).max(Vec3::splat(1e-4));
                let translation =
                    raw.rest_local.translation + (raw.parent_rest.inverse() * pose.hips) / parent_scale;
                if transform.translation != translation {
                    transform.translation = translation;
                }
            }
        }
    }

    /// The local transform that turns a child of `bone` into a normalized-space attachment point:
    /// axes aligned with the model root, unit scale, origin at the bone. Props hang off this.
    pub fn attachment_frame(&self, bone: Bone) -> Option<(Entity, Transform)> {
        let raw = self.bones[bone.index()].as_ref()?;
        Some((
            raw.entity,
            Transform {
                translation: Vec3::ZERO,
                rotation: raw.rest_world_rotation.inverse(),
                scale: Vec3::ONE / raw.rest_world_scale.max(Vec3::splat(1e-4)),
            },
        ))
    }
}

impl Metrics {
    fn measure(bones: &[Option<RigBone>; Bone::COUNT], rest: &[Vec3; Bone::COUNT]) -> Metrics {
        let has = |bone: Bone| bones[bone.index()].is_some();
        let at = |bone: Bone| rest[bone.index()];
        let span = |a: Bone, b: Bone| (has(a) && has(b)).then(|| at(a).distance(at(b))).filter(|d| *d > 1e-3);
        // Average both sides: a slightly asymmetric rest pose should not make her limp.
        let both = |a: Bone, b: Bone, fallback: f32| {
            let lengths: Vec<f32> = Side::BOTH.iter().filter_map(|s| span(a.on(*s), b.on(*s))).collect();
            if lengths.is_empty() {
                fallback
            } else {
                lengths.iter().sum::<f32>() / lengths.len() as f32
            }
        };

        // VRM models stand on the origin. The head bone sits at the skull base, about 85% of the
        // full height including hair (checked against the bundled model).
        let head_y = if has(Head) { at(Head).y } else { 1.4 };
        let height = (head_y / 0.85).max(0.3);
        let unit = height / 1.6;

        let upper_leg = both(LeftUpperLeg, LeftLowerLeg, 0.42 * unit);
        let lower_leg = both(LeftLowerLeg, LeftFoot, 0.42 * unit);
        let ankle_height = if has(LeftFoot) && has(RightFoot) {
            ((at(LeftFoot).y + at(RightFoot).y) * 0.5).clamp(0.02 * unit, 0.2 * unit)
        } else {
            0.08 * unit
        };
        let ball_length = Side::BOTH
            .iter()
            .filter(|s| has(LeftToes.on(**s)) && has(LeftFoot.on(**s)))
            .map(|s| (at(LeftToes.on(*s)).z - at(LeftFoot.on(*s)).z).abs())
            .fold(None, |acc: Option<f32>, v| Some(acc.map_or(v, |a| (a + v) * 0.5)))
            .filter(|v| *v > 0.02 * unit)
            .unwrap_or(0.11 * unit);
        let hip_half_width = if has(LeftUpperLeg) && has(RightUpperLeg) {
            ((at(LeftUpperLeg).x - at(RightUpperLeg).x).abs() * 0.5).max(0.03 * unit)
        } else {
            0.08 * unit
        };
        let upper_arm = both(LeftUpperArm, LeftLowerArm, 0.24 * unit);
        let lower_arm = both(LeftLowerArm, LeftHand, 0.23 * unit);

        // Face landmarks hang off the eyes when the model has eye bones (nearly all do); anime
        // faces vary too much in size for a fixed fraction of the height to find the chin.
        let (eye, eye_spacing) = if has(LeftEye) && has(RightEye) && has(Head) {
            let centre = (at(LeftEye) + at(RightEye)) * 0.5 - at(Head);
            let spacing = at(LeftEye).distance(at(RightEye)).clamp(0.03 * unit, 0.14 * unit);
            (centre, spacing)
        } else {
            (Vec3::new(0.0, 0.07, 0.07) * unit, 0.065 * unit)
        };
        // The eye bones sit inside the head; the surface of the face is in front of them.
        let face_z = eye.z + eye_spacing * 0.75;
        let mouth = Vec3::new(0.0, eye.y - eye_spacing * 0.95, face_z);
        let chin = Vec3::new(0.0, eye.y - eye_spacing * 1.45, face_z - eye_spacing * 0.15);

        Metrics {
            height,
            unit,
            upper_leg,
            lower_leg,
            ankle_height,
            ball_length,
            hip_half_width,
            upper_arm,
            lower_arm,
            eye: Vec3::new(0.0, eye.y, face_z),
            mouth,
            chin,
            eye_spacing,
        }
    }
}

#[cfg(test)]
impl Rig {
    /// A plausible 1.6 m skeleton with no entities behind it, for tests of the pose maths. Bones
    /// listed in `missing` are left out the way an optional bone is in a real file.
    pub fn synthetic(missing: &[Bone]) -> Rig {
        let mut rest_pos = [Vec3::ZERO; Bone::COUNT];
        let mut place = |bone: Bone, x: f32, y: f32, z: f32| rest_pos[bone.index()] = Vec3::new(x, y, z);
        place(Hips, 0.0, 0.90, 0.0);
        place(Spine, 0.0, 0.98, 0.0);
        place(Chest, 0.0, 1.08, 0.0);
        place(UpperChest, 0.0, 1.18, 0.0);
        place(Neck, 0.0, 1.30, 0.0);
        place(Head, 0.0, 1.36, 0.0);
        place(Jaw, 0.0, 1.36, 0.02);
        for side in Side::BOTH {
            let s = side.sign();
            place(LeftEye.on(side), s * 0.032, 1.43, 0.06);
            place(LeftShoulder.on(side), s * 0.03, 1.27, 0.0);
            place(LeftUpperArm.on(side), s * 0.13, 1.27, 0.0);
            place(LeftLowerArm.on(side), s * 0.37, 1.27, 0.0);
            place(LeftHand.on(side), s * 0.60, 1.27, 0.0);
            for finger in 0..5 {
                for joint in 0..3 {
                    let reach = 0.66 + joint as f32 * 0.03;
                    place(Bone::finger(side, finger, joint), s * reach, 1.27, 0.04 - finger as f32 * 0.02);
                }
            }
            place(LeftUpperLeg.on(side), s * 0.08, 0.86, 0.0);
            place(LeftLowerLeg.on(side), s * 0.08, 0.47, 0.0);
            place(LeftFoot.on(side), s * 0.08, 0.08, 0.0);
            place(LeftToes.on(side), s * 0.08, 0.02, 0.12);
        }

        let mut bones = [None; Bone::COUNT];
        for &bone in Bone::ALL {
            if missing.contains(&bone) {
                let anchor = Self::anchor_in(&bones, bone);
                rest_pos[bone.index()] = anchor.map(|a| rest_pos[a.index()]).unwrap_or(Vec3::ZERO);
                continue;
            }
            let anchor = bone
                .parent()
                .and_then(|p| bones[p.index()].as_ref().map(|_| p).or(Self::anchor_in(&bones, p)));
            bones[bone.index()] = Some(RigBone {
                entity: Entity::PLACEHOLDER,
                rest_local: Transform::IDENTITY,
                parent_rest: Quat::IDENTITY,
                rest_world_rotation: Quat::IDENTITY,
                rest_world_scale: Vec3::ONE,
                anchor,
            });
        }
        let metrics = Metrics::measure(&bones, &rest_pos);
        Rig { bones, rest_pos, metrics }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pose::Pose;

    #[test]
    fn the_right_block_mirrors_the_left() {
        assert_eq!(Bone::COUNT, 55);
        assert_eq!(LeftUpperArm.on(Side::Right), RightUpperArm);
        assert_eq!(LeftToes.on(Side::Right), RightToes);
        assert_eq!(LeftUpperArm.on(Side::Left), LeftUpperArm);
        assert_eq!(Head.on(Side::Right), Head);
        assert_eq!(LeftEye.on(Side::Right), RightEye, "the eyes sit outside the limb block");
        // Every left bone maps to the right bone of the same name.
        for &bone in Bone::ALL {
            if let Some(rest) = bone.name().strip_prefix("left") {
                assert_eq!(bone.on(Side::Right).name(), format!("right{rest}"), "{bone:?}");
            }
        }
        assert_eq!(Bone::finger(Side::Right, 4, 2), RightLittleDistal);
        assert_eq!(Bone::finger(Side::Left, 0, 0), LeftThumbMetacarpal);
        for &bone in Bone::ALL {
            if let Some(parent) = bone.parent() {
                assert!(parent.index() < bone.index(), "{bone:?} must come after {parent:?}");
            }
        }
    }

    #[test]
    fn rest_pose_fk_reproduces_the_rest_positions() {
        let rig = Rig::synthetic(&[]);
        let fk = rig.fk(&Pose::default());
        for &bone in Bone::ALL {
            assert!(fk.pos[bone.index()].distance(rig.rest(bone)) < 1e-5, "{bone:?}");
        }
        assert!((rig.metrics.upper_leg - 0.39).abs() < 1e-4);
        assert!((rig.metrics.hip_half_width - 0.08).abs() < 1e-4);
    }

    #[test]
    fn a_missing_bones_rotation_reaches_its_children() {
        let lean = Quat::from_rotation_x(0.4);
        let full = Rig::synthetic(&[]);
        let sparse = Rig::synthetic(&[Chest, UpperChest, Neck, LeftShoulder, RightShoulder]);
        assert_eq!(sparse.len(), full.len() - 5);

        let mut pose = Pose::default();
        pose.rot[Chest.index()] = lean;
        // With or without a chest bone, the head ends up leaning by the chest's rotation.
        for rig in [&full, &sparse] {
            let fk = rig.fk(&pose);
            assert!(fk.rot[Head.index()].angle_between(lean) < 1e-4);
            assert!(fk.rot[LeftUpperArm.index()].angle_between(lean) < 1e-4);
            assert!(fk.rot[Hips.index()].angle_between(Quat::IDENTITY) < 1e-4);
        }
    }
}
