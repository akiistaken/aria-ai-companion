//! Props: accessories the user loads from their own files and attaches to a bone.
//!
//! There are no built-in props. Every prop is a glTF/GLB (or VRM) the user picks, hung off a
//! humanoid bone through the same normalized attachment frame the rig uses
//! ([`crate::rig::Rig::attachment_frame`]), so it follows that bone exactly as hair does.
//!
//! A prop is described by [`PropSpec`], which the app owns and persists. The renderer builds one
//! child scene per spec the first time it sees it, and shows or hides it by `enabled`.
//!
//! Offsets are in the **model's own units** (metres, model space) at the bone, so a hat placed on
//! one rig sits right on another of the same proportions.

use std::collections::BTreeMap;

use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;

use crate::behaviors::AvatarState;
use crate::rig::{Bone, Rig};

/// Parses a wire bone name into a [`Bone`].
pub fn bone_named(name: &str) -> Option<Bone> {
    let wanted = name.trim();
    Bone::ALL
        .iter()
        .copied()
        .find(|bone| bone.name().eq_ignore_ascii_case(wanted))
}

/// One attached prop, as the app describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct PropSpec {
    /// Stable id, so the same prop is not rebuilt every frame.
    pub id: String,
    /// Absolute path of the glTF/GLB to show.
    pub path: String,
    /// Humanoid bone it hangs from.
    pub bone: String,
    /// Offset from the bone, in model metres.
    pub offset: Vec3,
    /// Rotation, in degrees about the model's own axes at the bone.
    pub rotation: Vec3,
    /// Uniform scale, 1 = as authored.
    pub scale: f32,
    pub enabled: bool,
}

impl PropSpec {
    /// Normalises a spec coming off the wire: finite numbers, a bone the rig may have.
    pub fn sanitized(&self) -> PropSpec {
        let finite = |v: Vec3| Vec3::new(v.x, v.y, v.z);
        let mut scale = self.scale;
        if !scale.is_finite() || scale <= 0.0 {
            scale = 1.0;
        }
        PropSpec {
            id: self.id.clone(),
            path: self.path.clone(),
            bone: self.bone.clone(),
            offset: finite(self.offset).clamp(Vec3::splat(-2.0), Vec3::splat(2.0)),
            rotation: finite(self.rotation),
            scale: scale.clamp(0.01, 20.0),
            enabled: self.enabled,
        }
    }
}

/// A prop's root entity in the scene, for a future per-prop animation to find.
#[derive(Component)]
pub struct AttachedProp;

/// What has been built, so unchanged props are not rebuilt every frame.
#[derive(Resource, Default)]
pub struct Props {
    /// The spec each live prop was built from.
    pub built: BTreeMap<String, BuiltProp>,
}

pub struct BuiltProp {
    /// The spec it was built from, so a change of transform or bone is noticed.
    pub spec: PropSpec,
    /// The entity under the bone frame that the scene is spawned into.
    pub root: Entity,
}

/// Spawns, updates and removes props to match what the app asked for.
pub fn sync_props(
    state: Res<AvatarState>,
    mut props: ResMut<Props>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    if state.rig.is_none() {
        return;
    }
    let wanted: Vec<PropSpec> = state.props.iter().map(PropSpec::sanitized).collect();

    // Remove what is gone or off.
    let stale: Vec<String> = props
        .built
        .iter()
        .filter(|(id, built)| {
            let spec = wanted.iter().find(|s| &s.id == *id);
            spec.is_none_or(|s| !s.enabled || s.path != built.spec.path || s.bone != built.spec.bone)
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in stale {
        if let Some(built) = props.built.remove(&id) {
            commands.entity(built.root).despawn();
        }
    }

    let rig = state.rig.clone().expect("checked above");
    for spec in wanted.into_iter().filter(|s| s.enabled) {
        match props.built.get_mut(&spec.id) {
            // Already built from the same file on the same bone: move it if the transform changed.
            Some(built) => {
                if built.spec.offset == spec.offset
                    && built.spec.rotation == spec.rotation
                    && built.spec.scale == spec.scale
                {
                    continue;
                }
                let transform = local_transform(&spec);
                commands.entity(built.root).insert(transform);
                built.spec = spec;
            }
            None => {
                let Some((frame, mut frame_transform)) = attach_frame(&rig, &spec.bone) else {
                    log::warn!("prop {}: bone {} is not in this model", spec.id, spec.bone);
                    continue;
                };
                // The attachment frame carries the bone's rest orientation and scale, so a prop
                // offset is authored in model space and applies to any rig.
                frame_transform.scale *= rig.metrics.unit;
                let anchor = commands.spawn((frame_transform, Visibility::Inherited, ChildOf(frame))).id();
                let root = commands
                    .spawn((
                        local_transform(&spec),
                        Visibility::Inherited,
                        AttachedProp,
                        ChildOf(anchor),
                    ))
                    .id();
                let handle: Handle<bevy::world_serialization::WorldAsset> =
                    asset_server.load(format!("{}#Scene0", spec.path));
                commands.entity(root).insert(WorldAssetRoot(handle));
                props.built.insert(spec.id.clone(), BuiltProp { spec, root });
            }
        }
    }
}

/// The prop's own transform under its bone frame.
fn local_transform(spec: &PropSpec) -> Transform {
    Transform {
        translation: spec.offset,
        rotation: Quat::from_euler(
            EulerRot::YXZ,
            spec.rotation.y.to_radians(),
            spec.rotation.x.to_radians(),
            spec.rotation.z.to_radians(),
        ),
        scale: Vec3::splat(spec.scale),
    }
}

/// The bone's normalized attachment frame, the same one hair and props use.
fn attach_frame(rig: &Rig, bone: &str) -> Option<(Entity, Transform)> {
    let bone = bone_named(bone)?;
    rig.attachment_frame(bone)
}

/// Unused today, kept so a future per-prop animation can find its root by id.
#[allow(dead_code)]
pub fn root_of(props: &Props, id: &str) -> Option<Entity> {
    props.built.get(id).map(|built| built.root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(bone: &str) -> PropSpec {
        PropSpec {
            id: "hat".into(),
            path: "C:/props/hat.glb".into(),
            bone: bone.into(),
            offset: Vec3::Y * 0.1,
            rotation: Vec3::ZERO,
            scale: 1.0,
            enabled: true,
        }
    }

    #[test]
    fn bone_names_round_trip() {
        // Every bone the app offers must resolve on the renderer side.
        for (name, _) in [
            ("head", ""), ("upperChest", ""), ("chest", ""), ("spine", ""), ("hips", ""),
            ("leftHand", ""), ("rightHand", ""), ("leftUpperArm", ""), ("rightUpperArm", ""),
            ("leftShoulder", ""), ("rightShoulder", ""), ("leftUpperLeg", ""), ("rightUpperLeg", ""),
        ] {
            let bone = bone_named(name).unwrap_or_else(|| panic!("{name} is not a bone"));
            assert_eq!(bone.name(), name);
        }
        assert!(bone_named("not_a_bone").is_none());
        assert_eq!(bone_named(" HEAD ").map(|b| b.name()), Some("head"));
    }

    #[test]
    fn bad_numbers_are_repaired_rather_than_shown() {
        let mut bad = spec("head");
        bad.offset = Vec3::new(f32::NAN, 99.0, -99.0);
        bad.scale = 0.0;
        let fixed = bad.sanitized();
        assert!(fixed.offset.is_finite());
        assert!(fixed.offset.y <= 2.0 && fixed.offset.z >= -2.0);
        assert!(fixed.scale > 0.0);
    }

    #[test]
    fn a_props_offset_is_in_model_metres_at_the_bone() {
        let transform = local_transform(&spec("head"));
        assert_eq!(transform.translation, Vec3::Y * 0.1);
        assert_eq!(transform.scale, Vec3::ONE);
    }
}
