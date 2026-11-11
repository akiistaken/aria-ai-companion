//! A three-vrm style *normalized* humanoid rig on top of `bevy_vrm1`'s raw bones.
//!
//! The reference stage drives `vrm.humanoid.getNormalizedBoneNode(..)`: virtual bones whose rest
//! rotation is identity and whose axes line up with the model root. Every pose number in
//! `posing.rs` is expressed in that space. `bevy_vrm1` exposes the raw glTF bones instead, whose
//! rest orientation is whatever the exporter wrote, so writing those numbers straight onto the raw
//! bones gives wrong poses (a T-pose on the bundled model).
//!
//! three-vrm's `VRMHumanoidRig.update()` maps a normalized local rotation `N` onto a raw bone as
//!
//! ```text
//! raw_local = P⁻¹ · N · P · rest_local      where P = the parent's rest rotation in root space
//! ```
//!
//! and that is what [`Rig::apply`] does. `bevy_vrm1` already snapshots each bone's rest local and
//! root-relative transforms (`RestTransform` / `RestGlobalTransform`), so nothing has to be
//! captured by hand and the mapping is exact for any model.

use std::collections::HashMap;

use bevy::ecs::world::EntityRef;
use bevy::prelude::*;
use bevy_vrm1::prelude::*;

/// One raw bone and the rest data needed to map normalized rotations onto it.
#[derive(Clone, Copy, Debug)]
pub struct RigBone {
    pub entity: Entity,
    rest_local: Transform,
    /// Rest rotation of the parent, relative to the VRM root (`P` above).
    parent_rest: Quat,
    /// Rest rotation and scale of this bone relative to the VRM root.
    pub rest_world_rotation: Quat,
    pub rest_world_scale: Vec3,
    pub rest_world_translation: Vec3,
}

/// Every resolved humanoid bone of one loaded model.
#[derive(Clone, Default)]
pub struct Rig {
    bones: HashMap<&'static str, RigBone>,
    /// Approximate model height and vertical centre, in root space (used for camera framing).
    pub height: f32,
    pub center_y: f32,
}

macro_rules! bone_table {
    ($($component:ident => $name:literal),* $(,)?) => {
        /// The humanoid bones this crate drives, read off the VRM root's `*BoneEntity` holders.
        fn bone_entities(root: &EntityRef) -> Vec<(&'static str, Entity)> {
            let mut found = Vec::new();
            $(
                if let Some(holder) = root.get::<$component>() {
                    found.push(($name, holder.0));
                }
            )*
            found
        }
    };
}

bone_table!(
    HipsBoneEntity => "hips",
    SpineBoneEntity => "spine",
    ChestBoneEntity => "chest",
    UpperChestBoneEntity => "upperChest",
    NeckBoneEntity => "neck",
    HeadBoneEntity => "head",
    LeftUpperArmBoneEntity => "leftUpperArm",
    RightUpperArmBoneEntity => "rightUpperArm",
    LeftLowerArmBoneEntity => "leftLowerArm",
    RightLowerArmBoneEntity => "rightLowerArm",
    LeftHandBoneEntity => "leftHand",
    RightHandBoneEntity => "rightHand",
    LeftUpperLegBoneEntity => "leftUpperLeg",
    RightUpperLegBoneEntity => "rightUpperLeg",
    LeftLowerLegBoneEntity => "leftLowerLeg",
    RightLowerLegBoneEntity => "rightLowerLeg",
    LeftThumbMetacarpalBoneEntity => "leftThumbMetacarpal",
    LeftThumbProximalBoneEntity => "leftThumbProximal",
    LeftThumbDistalBoneEntity => "leftThumbDistal",
    LeftIndexProximalBoneEntity => "leftIndexProximal",
    LeftIndexIntermediateBoneEntity => "leftIndexIntermediate",
    LeftIndexDistalBoneEntity => "leftIndexDistal",
    LeftMiddleProximalBoneEntity => "leftMiddleProximal",
    LeftMiddleIntermediateBoneEntity => "leftMiddleIntermediate",
    LeftMiddleDistalBoneEntity => "leftMiddleDistal",
    LeftRingProximalBoneEntity => "leftRingProximal",
    LeftRingIntermediateBoneEntity => "leftRingIntermediate",
    LeftRingDistalBoneEntity => "leftRingDistal",
    LeftLittleProximalBoneEntity => "leftLittleProximal",
    LeftLittleIntermediateBoneEntity => "leftLittleIntermediate",
    LeftLittleDistalBoneEntity => "leftLittleDistal",
    RightThumbMetacarpalBoneEntity => "rightThumbMetacarpal",
    RightThumbProximalBoneEntity => "rightThumbProximal",
    RightThumbDistalBoneEntity => "rightThumbDistal",
    RightIndexProximalBoneEntity => "rightIndexProximal",
    RightIndexIntermediateBoneEntity => "rightIndexIntermediate",
    RightIndexDistalBoneEntity => "rightIndexDistal",
    RightMiddleProximalBoneEntity => "rightMiddleProximal",
    RightMiddleIntermediateBoneEntity => "rightMiddleIntermediate",
    RightMiddleDistalBoneEntity => "rightMiddleDistal",
    RightRingProximalBoneEntity => "rightRingProximal",
    RightRingIntermediateBoneEntity => "rightRingIntermediate",
    RightRingDistalBoneEntity => "rightRingDistal",
    RightLittleProximalBoneEntity => "rightLittleProximal",
    RightLittleIntermediateBoneEntity => "rightLittleIntermediate",
    RightLittleDistalBoneEntity => "rightLittleDistal",
);

impl Rig {
    /// Reads the rig off a VRM root. `None` until `bevy_vrm1` has attached the bone holders and
    /// snapshotted the rest transforms, which happens a few frames after the scene spawns.
    pub fn resolve(world: &World, root: Entity) -> Option<Rig> {
        let root = world.get_entity(root).ok()?;
        let entities = bone_entities(&root);
        if !entities.iter().any(|(name, _)| *name == "hips") {
            return None;
        }
        let mut bones = HashMap::new();
        for (name, entity) in entities {
            // Rest snapshots land together; a missing one means "not ready yet".
            let rest_local = world.get::<RestTransform>(entity)?;
            let rest_global = world.get::<RestGlobalTransform>(entity)?;
            let (scale, rotation, translation) = rest_global.0.to_scale_rotation_translation();
            let parent_rest = rotation * rest_local.0.rotation.inverse();
            bones.insert(
                name,
                RigBone {
                    entity,
                    rest_local: rest_local.0,
                    parent_rest,
                    rest_world_rotation: rotation,
                    rest_world_scale: scale,
                    rest_world_translation: translation,
                },
            );
        }

        // Framing metrics: VRM models stand on the origin, so the head bone gives the height. The
        // head bone sits at the skull base, about 85% of the full height including hair (checked
        // against the bundled model, whose hair top was clipped at 88%).
        let head_y = bones
            .get("head")
            .map(|b| b.rest_world_translation.y)
            .unwrap_or(1.4);
        let height = (head_y / 0.85).max(0.8);
        Some(Rig {
            bones,
            height,
            center_y: height * 0.5,
        })
    }

    pub fn len(&self) -> usize {
        self.bones.len()
    }

    /// Every bone, for resetting to rest.
    pub fn bones(&self) -> impl Iterator<Item = (&'static str, &RigBone)> {
        self.bones.iter().map(|(name, bone)| (*name, bone))
    }

    /// Writes a normalized local rotation onto the raw bone.
    pub fn apply<F: bevy::ecs::query::QueryFilter>(
        &self,
        name: &str,
        normalized: Quat,
        transforms: &mut Query<&mut Transform, F>,
    ) {
        let Some(bone) = self.bones.get(name) else {
            return;
        };
        let Ok(mut transform) = transforms.get_mut(bone.entity) else {
            return;
        };
        transform.rotation =
            bone.parent_rest.inverse() * normalized * bone.parent_rest * bone.rest_local.rotation;
    }

    /// Offsets the hips from rest by `offset` in normalized (root-aligned) space.
    pub fn offset_hips<F: bevy::ecs::query::QueryFilter>(
        &self,
        offset: Vec3,
        transforms: &mut Query<&mut Transform, F>,
    ) {
        let Some(bone) = self.bones.get("hips") else {
            return;
        };
        let Ok(mut transform) = transforms.get_mut(bone.entity) else {
            return;
        };
        let parent_scale = (bone.rest_world_scale / bone.rest_local.scale).max(Vec3::splat(1e-4));
        transform.translation =
            bone.rest_local.translation + (bone.parent_rest.inverse() * offset) / parent_scale;
    }

    /// The local transform that turns a child of `bone` into a normalized-space attachment point:
    /// axes aligned with the model root, unit scale, origin at the bone. Props hang off this.
    pub fn attachment_frame(&self, name: &str) -> Option<(Entity, Transform)> {
        let bone = self.bones.get(name)?;
        Some((
            bone.entity,
            Transform {
                translation: Vec3::ZERO,
                rotation: bone.rest_world_rotation.inverse(),
                scale: Vec3::ONE / bone.rest_world_scale.max(Vec3::splat(1e-4)),
            },
        ))
    }
}
