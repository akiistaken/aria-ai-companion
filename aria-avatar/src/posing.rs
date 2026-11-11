//! Procedural bone posing: breathing, sway, walk/dance/sit branches, head look, peeking lean.
//!
//! Every numeric constant here is ported from `docs/reference/vtuber_stage.html` (the `animate()`
//! emotion branches). They are hand-tuned, so they are copied verbatim rather than re-derived.
//!
//! The rotations are applied to the *normalized* humanoid bone nodes, which is what the reference
//! does: VRM applies the humanoid retargeting on top, so the same numbers produce the same pose
//! regardless of the model's rest skeleton.

use bevy::prelude::*;
use bevy_vrm1::prelude::*;

use aria_protocol::PeekSide;

use crate::behaviors::AvatarState;

/// A bone's local rotation, keyed by VRM humanoid bone name.
pub fn pose_for(state: &AvatarState, t: f32, breath: f32, sway: f32) -> Vec<(&'static str, Vec3)> {
    let mut pose: Vec<(&'static str, Vec3)> = Vec::new();
    let emo = state.emotion.as_str();
    let walk_cycle = t * 8.5;
    let stride = walk_cycle.sin();
    let moving = state.moving || emo == "walking";

    // Torso: breathing plus a slow sway; `dizzy` wobbles harder.
    pose.push(("spine", Vec3::new(breath * 0.025, 0.0, if emo == "dizzy" { (t * 9.0).sin() * 0.11 } else { sway * 0.025 })));
    pose.push(("chest", Vec3::new(breath * 0.03, 0.0, 0.0)));

    // Neck and head follow the cursor, plus emotion-specific shake.
    let mut head = Vec3::new(
        if emo == "sleepy" {
            0.18 + (t * 1.2).sin() * 0.08
        } else {
            -state.smoothed_look_y * 0.18 + (t * 1.8).sin() * 0.025
        },
        state.smoothed_look_x * 0.28,
        if emo == "dizzy" {
            (t * 14.0).cos() * 0.22
        } else {
            (if emo == "idle" || emo == "happy" { 0.07 } else { -0.04 }) * sway
        },
    );
    if emo == "annoyed" {
        head.y += (t * 26.0).sin() * 0.14;
    } else if emo == "dizzy" {
        head.y += (t * 14.0).sin() * 0.24;
    }
    match state.peeking {
        PeekSide::Left => head.z += 0.09,
        PeekSide::Right => head.z -= 0.09,
        PeekSide::None => {}
    }
    pose.push(("neck", Vec3::new(-state.smoothed_look_y * 0.14, state.smoothed_look_x * 0.22, 0.0)));
    pose.push(("head", head));

    if moving {
        pose.push(("hips", Vec3::new(0.0, stride * 0.08, 0.0)));
        pose.push(("leftUpperArm", Vec3::new(-stride * 0.45, 0.0, -1.18)));
        pose.push(("rightUpperArm", Vec3::new(stride * 0.45, 0.0, 1.18)));
        pose.push(("leftLowerArm", Vec3::new(-0.35, 0.0, 0.0)));
        pose.push(("rightLowerArm", Vec3::new(-0.35, 0.0, 0.0)));
        pose.push(("leftUpperLeg", Vec3::new(stride * 0.52, 0.0, 0.0)));
        pose.push(("rightUpperLeg", Vec3::new(-stride * 0.52, 0.0, 0.0)));
        pose.push(("leftLowerLeg", Vec3::new((-stride).max(0.0) * 0.55, 0.0, 0.0)));
        pose.push(("rightLowerLeg", Vec3::new(stride.max(0.0) * 0.55, 0.0, 0.0)));
    } else {
        match emo {
            "dance" => {
                let beat = t * 7.5;
                pose.push(("hips", Vec3::new(0.0, (beat * 0.5).cos() * 0.09, 0.0)));
                pose.push(("leftUpperArm", Vec3::new(0.3, 0.2, -0.75 + beat.sin() * 0.25)));
                pose.push(("rightUpperArm", Vec3::new(0.3, -0.2, 0.75 + beat.cos() * 0.25)));
                pose.push(("leftLowerArm", Vec3::new(-0.85, 0.3, 0.2)));
                pose.push(("rightLowerArm", Vec3::new(-0.85, -0.3, -0.2)));
                pose.push(("leftUpperLeg", Vec3::new(beat.sin() * 0.25, 0.0, -0.05)));
                pose.push(("rightUpperLeg", Vec3::new(-beat.sin() * 0.25, 0.0, 0.05)));
            }
            "groove" => {
                let beat = t * 5.0;
                pose.push(("hips", Vec3::new(0.0, (beat * 0.5).cos() * 0.05, 0.0)));
                pose.push(("leftUpperArm", Vec3::new(0.15, 0.1, -0.85 + beat.sin() * 0.15)));
                pose.push(("rightUpperArm", Vec3::new(0.15, -0.1, 0.85 + beat.cos() * 0.15)));
            }
            "sit" => {
                pose.push(("leftUpperArm", Vec3::new(0.55, 0.0, -0.55)));
                pose.push(("rightUpperArm", Vec3::new(0.55, 0.0, 0.55)));
                pose.push(("leftLowerArm", Vec3::new(-0.25, 0.12, 0.05)));
                pose.push(("rightLowerArm", Vec3::new(-0.25, -0.12, -0.05)));
            }
            "happy" => {
                pose.push(("leftUpperArm", Vec3::new(0.2, 0.0, -0.35)));
                pose.push(("rightUpperArm", Vec3::new(0.2, 0.0, 0.35)));
                pose.push(("leftLowerArm", Vec3::new(-0.2, 0.15, 0.18)));
                pose.push(("rightLowerArm", Vec3::new(-0.2, -0.15, -0.18)));
            }
            "annoyed" => {
                pose.push(("leftUpperArm", Vec3::new(0.0, 0.0, -0.95)));
                pose.push(("rightUpperArm", Vec3::new(0.0, 0.0, 0.95)));
                pose.push(("leftLowerArm", Vec3::new(-0.95, 0.1, 0.2)));
                pose.push(("rightLowerArm", Vec3::new(-0.95, -0.1, -0.2)));
            }
            "talking" => {
                pose.push(("leftUpperArm", Vec3::new(0.0, 0.0, -0.4)));
                pose.push(("rightUpperArm", Vec3::new(0.0, 0.0, 0.4)));
                pose.push(("leftLowerArm", Vec3::new(-0.4, 0.35, 0.05)));
                pose.push(("rightLowerArm", Vec3::new(-0.25 + (t * 4.5).sin() * 0.2, -0.2, -0.15)));
            }
            _ => {
                // Idle: arms relaxed with a breath-driven curl.
                pose.push(("leftUpperArm", Vec3::new(0.0, 0.0, -0.45 - breath * 0.02)));
                pose.push(("rightUpperArm", Vec3::new(0.0, 0.0, 0.45 + breath * 0.02)));
                pose.push(("leftLowerArm", Vec3::new(-0.45, 0.08, 0.04)));
                pose.push(("rightLowerArm", Vec3::new(-0.45, -0.08, -0.04)));
            }
        }
    }
    pose
}

/// The pose body rotation (yaw for facing, roll for peeking and drag inertia).
pub fn body_rotation(state: &AvatarState, t: f32, current_yaw: f32, dt: f32) -> Vec3 {
    let mut target_yaw = 0.0;
    if state.peeking != PeekSide::None {
        target_yaw = if state.peeking == PeekSide::Left { -0.38 } else { 0.38 };
    } else if state.moving || state.emotion == "walking" {
        target_yaw = if state.facing_right { 0.38 } else { -0.38 };
    }
    let yaw = current_yaw + (target_yaw - current_yaw) * (dt * 7.0).min(1.0);
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
    .clamp(Vec3::splat(-0.5), Vec3::splat(0.5))
}

/// The scale head-pat wobble adds to the head pose.
pub fn headpat_wobble(effect: f32, t: f32) -> Vec3 {
    if effect <= 0.0 {
        return Vec3::ZERO;
    }
    Vec3::new(
        0.08 * effect,
        0.0,
        (t * 7.5).sin() * 0.08 * effect,
    )
}

/// How long a head pat keeps wobbling after the event, in seconds.
pub const HEADPAT_DECAY: f32 = 2.6;

/// Applies a pose list to the model's bones.
///
/// The reference applies these rotations to the *normalized* humanoid node, which starts at
/// identity. Here they are composed onto the bone's rest rotation instead, because the model's bone
/// transforms carry its own rest orientation (a T-pose rig does not have identity arms). Composing
/// keeps the "same numbers, same pose" property without overwriting that rest data.
pub fn apply_pose<F: bevy::ecs::query::QueryFilter>(
    root: Entity,
    pose: &[(&'static str, Vec3)],
    vrm: &VrmBones,
    transforms: &mut Query<&mut Transform, F>,
) {
    for (name, rotation) in pose {
        let Some(entity) = vrm.entity_for(root, name) else {
            continue;
        };
        let Ok(mut transform) = transforms.get_mut(entity) else {
            continue;
        };
        let rest = vrm.rest.get(name).copied().unwrap_or(Quat::IDENTITY);
        let animated = Quat::from_euler(EulerRot::XYZ, rotation.x, rotation.y, rotation.z);
        // Limbs are posed absolutely: the reference drives the normalized humanoid node, whose rest
        // orientation is identity, so the raw numbers are the whole pose. Spine/neck/head compose
        // onto their rest rotation, because those carry the rig's own tilt.
        transform.rotation = match *name {
            "spine" | "chest" | "neck" | "head" | "hips" => rest * animated,
            _ => animated,
        };
    }
}

/// Reads the bone entities off a VRM root.
///
/// Only the bones the pose functions actually drive are read; a model missing one (an expression
/// preset the model does not ship) is skipped silently rather than treated as an error.
pub fn read_bones(
    root: Entity,
    q: &Query<(
        Option<&SpineBoneEntity>,
        Option<&ChestBoneEntity>,
        Option<&NeckBoneEntity>,
        Option<&HeadBoneEntity>,
        Option<&HipsBoneEntity>,
        Option<&LeftUpperArmBoneEntity>,
        Option<&RightUpperArmBoneEntity>,
        Option<&LeftLowerArmBoneEntity>,
        Option<&RightLowerArmBoneEntity>,
        Option<&LeftUpperLegBoneEntity>,
        Option<&RightUpperLegBoneEntity>,
        Option<&LeftLowerLegBoneEntity>,
        Option<&RightLowerLegBoneEntity>,
    )>,
) -> VrmBones {
    let mut bones = VrmBones::default();
    let Ok(row) = q.get(root) else { return bones };
    let pairs: [(&'static str, Option<Entity>); 13] = [
        ("spine", row.0.map(|h| h.0)),
        ("chest", row.1.map(|h| h.0)),
        ("neck", row.2.map(|h| h.0)),
        ("head", row.3.map(|h| h.0)),
        ("hips", row.4.map(|h| h.0)),
        ("leftUpperArm", row.5.map(|h| h.0)),
        ("rightUpperArm", row.6.map(|h| h.0)),
        ("leftLowerArm", row.7.map(|h| h.0)),
        ("rightLowerArm", row.8.map(|h| h.0)),
        ("leftUpperLeg", row.9.map(|h| h.0)),
        ("rightUpperLeg", row.10.map(|h| h.0)),
        ("leftLowerLeg", row.11.map(|h| h.0)),
        ("rightLowerLeg", row.12.map(|h| h.0)),
    ];
    for (name, entity) in pairs {
        if let Some(entity) = entity {
            bones.map.insert(name, entity);
        }
    }
    bones
}

/// The bone entity handles and rest rotations for one model.
#[derive(Default, Clone)]
pub struct VrmBones {
    map: std::collections::HashMap<&'static str, Entity>,
    /// The bone rotation before any procedural posing, so poses compose onto the rig's own rest
    /// orientation instead of replacing it.
    pub rest: std::collections::HashMap<&'static str, Quat>,
}

impl VrmBones {
    pub fn entity_for(&self, _root: Entity, name: &str) -> Option<Entity> {
        self.map.get(name).copied()
    }

    /// Every resolved bone, as (name, entity) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, Entity)> + '_ {
        self.map.iter().map(|(name, entity)| (*name, *entity))
    }
}
