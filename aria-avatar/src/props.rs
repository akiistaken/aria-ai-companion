//! Wardrobe props, ported from `attachWardrobePropsToVRM()` and the prop animation block of the
//! reference stage.
//!
//! Every prop is built from primitives with the reference's exact dimensions, transforms and
//! materials, and hung off a normalized-space attachment frame of its bone (see
//! [`crate::rig::Rig::attachment_frame`]), so the numbers mean the same thing as in three.js. All
//! thirteen are built once per model and shown or hidden by `SetProps`.
//!
//! three.js primitives map onto Bevy as: `ConeGeometry` -> `Cone`, `CylinderGeometry` ->
//! `ConicalFrustum`/`Cylinder`, `BoxGeometry` -> `Cuboid`, `SphereGeometry` -> `Sphere`. Bevy's torus
//! lies in a different plane and cannot do partial arcs, so `TorusGeometry` is generated here with
//! three's own formula.

use std::f32::consts::{PI, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::behaviors::AvatarState;
use crate::rig::Rig;

/// Names in the order the app's wardrobe uses.
pub const PROP_NAMES: [&str; 13] = [
    "cat_ears",
    "headset",
    "glasses",
    "halo",
    "angel_wings",
    "witch_hat",
    "sunglasses",
    "devil_horns",
    "cyber_sword",
    "fox_tail",
    "coffee_mug",
    "game_controller",
    "smartphone",
];

/// The root entity of one prop; its visibility follows `AvatarState::props`.
#[derive(Component)]
pub struct Prop(pub &'static str);

/// Per-frame motion of a prop part, relative to its built transform.
#[derive(Component, Clone, Copy)]
pub enum PropAnim {
    CatEar { left: bool },
    Halo,
    Wing { left: bool },
    WitchHat,
    FoxTail,
    Sword,
    Mug,
    Phone,
}

fn euler(x: f32, y: f32, z: f32) -> Quat {
    Quat::from_euler(EulerRot::XYZ, x, y, z)
}

fn at(x: f32, y: f32, z: f32) -> Transform {
    Transform::from_xyz(x, y, z)
}

fn at_rot(pos: [f32; 3], rot: [f32; 3]) -> Transform {
    Transform::from_xyz(pos[0], pos[1], pos[2]).with_rotation(euler(rot[0], rot[1], rot[2]))
}

/// three.js `TorusGeometry(radius, tube, radialSegments, tubularSegments, arc)`.
fn torus(radius: f32, tube: f32, radial: u32, tubular: u32, arc: f32) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for j in 0..=radial {
        for i in 0..=tubular {
            let u = i as f32 / tubular as f32 * arc;
            let v = j as f32 / radial as f32 * TAU;
            let p = Vec3::new(
                (radius + tube * v.cos()) * u.cos(),
                (radius + tube * v.cos()) * u.sin(),
                tube * v.sin(),
            );
            let center = Vec3::new(radius * u.cos(), radius * u.sin(), 0.0);
            positions.push(p.to_array());
            normals.push((p - center).normalize_or_zero().to_array());
            uvs.push([i as f32 / tubular as f32, j as f32 / radial as f32]);
        }
    }
    let mut indices = Vec::new();
    let row = tubular + 1;
    for j in 1..=radial {
        for i in 1..=tubular {
            let a = row * j + i - 1;
            let b = row * (j - 1) + i - 1;
            let c = row * (j - 1) + i;
            let d = row * j + i;
            indices.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

fn cone(radius: f32, height: f32, segments: u32) -> Mesh {
    Cone { radius, height }.mesh().resolution(segments).build()
}

fn cylinder(top: f32, bottom: f32, height: f32, segments: u32) -> Mesh {
    if (top - bottom).abs() < 1e-6 {
        Cylinder::new(top, height).mesh().resolution(segments).build()
    } else {
        ConicalFrustum {
            radius_top: top,
            radius_bottom: bottom,
            height,
        }
        .mesh()
        .resolution(segments)
        .build()
    }
}

/// Material shorthand: `(color, roughness, metalness, emissive, emissive intensity)`.
fn material(
    color: u32,
    roughness: f32,
    metalness: f32,
    emissive: Option<(u32, f32)>,
) -> StandardMaterial {
    let srgb = |rgb: u32| Color::srgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    StandardMaterial {
        base_color: srgb(color),
        perceptual_roughness: roughness,
        metallic: metalness,
        emissive: emissive
            .map(|(rgb, k)| srgb(rgb).to_linear() * k)
            .unwrap_or(LinearRgba::BLACK),
        ..default()
    }
}

/// Spawns prop parts under one parent.
struct Builder<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    meshes: &'a mut Assets<Mesh>,
    materials: &'a mut Assets<StandardMaterial>,
}

impl Builder<'_, '_, '_> {
    fn group(&mut self, parent: Entity, transform: Transform) -> Entity {
        self.commands
            .spawn((transform, Visibility::Inherited, ChildOf(parent)))
            .id()
    }

    fn part(
        &mut self,
        parent: Entity,
        mesh: Mesh,
        material: &Handle<StandardMaterial>,
        transform: Transform,
    ) -> Entity {
        let mesh = self.meshes.add(mesh);
        self.commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material.clone()),
                transform,
                ChildOf(parent),
            ))
            .id()
    }

    fn mat(&mut self, m: StandardMaterial) -> Handle<StandardMaterial> {
        self.materials.add(m)
    }

    /// A prop root: hidden until enabled.
    fn prop(&mut self, frame: Entity, name: &'static str, transform: Transform) -> Entity {
        self.commands
            .spawn((transform, Visibility::Hidden, Prop(name), ChildOf(frame)))
            .id()
    }
}

/// Spawns the normalized attachment frame for a bone, if the model has it.
fn frame(b: &mut Builder, rig: &Rig, bones: &[&str]) -> Option<Entity> {
    let (bone, transform) = bones.iter().find_map(|name| rig.attachment_frame(name))?;
    Some(b.group(bone, transform))
}

/// Builds every prop for a freshly loaded model.
pub fn build_props(
    rig: &Rig,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let mut b = Builder {
        commands,
        meshes,
        materials,
    };

    if let Some(head) = frame(&mut b, rig, &["head"]) {
        // 1. Cat ears.
        let ears = b.prop(head, "cat_ears", Transform::IDENTITY);
        let outer = b.mat(material(0x1e293b, 0.4, 0.3, None));
        let inner = b.mat(material(0xf472b6, 1.0, 0.0, Some((0xdb2777, 0.6))));
        for left in [true, false] {
            let s = if left { 1.0 } else { -1.0 };
            let ear = b.part(ears, cone(0.048, 0.105, 4), &outer, at_rot([s * 0.085, 0.195, 0.01], [0.1, s * 0.25, -s * 0.32]));
            b.commands.entity(ear).insert(PropAnim::CatEar { left });
            b.part(ear, cone(0.028, 0.075, 4), &inner, at(0.0, -0.005, 0.014));
        }

        // 2. Gaming headset.
        let hs = b.prop(head, "headset", Transform::IDENTITY);
        let band = b.mat(material(0x0f172a, 0.3, 0.6, None));
        let glow = b.mat(material(0x38bdf8, 1.0, 0.0, Some((0x0284c7, 0.9))));
        b.part(hs, torus(0.118, 0.012, 10, 28, PI), &band, at(0.0, 0.085, 0.0));
        for s in [-1.0f32, 1.0] {
            b.part(hs, cylinder(0.042, 0.042, 0.028, 20), &band, at_rot([s * 0.115, 0.065, 0.005], [0.0, 0.0, PI * 0.5]));
            b.part(hs, torus(0.036, 0.005, 8, 20, TAU), &glow, at_rot([s * 0.130, 0.065, 0.005], [0.0, PI * 0.5, 0.0]));
        }

        // 3. Glasses.
        let gl = b.prop(head, "glasses", Transform::IDENTITY);
        let frame_mat = b.mat(material(0x38bdf8, 0.2, 0.8, None));
        for s in [1.0f32, -1.0] {
            let lens = at(s * 0.038, 0.072, 0.098).with_scale(Vec3::new(1.2, 0.85, 1.0));
            b.part(gl, torus(0.028, 0.0032, 8, 24, TAU), &frame_mat, lens);
        }
        b.part(gl, cylinder(0.0025, 0.0025, 0.022, 8), &frame_mat, at_rot([0.0, 0.074, 0.099], [0.0, 0.0, PI * 0.5]));

        // 4. Halo.
        let halo_mat = b.mat(material(0xfef08a, 0.1, 0.0, Some((0x38bdf8, 1.1))));
        let halo = b.prop(head, "halo", Transform::IDENTITY);
        let ring = b.part(halo, torus(0.11, 0.008, 12, 36, TAU), &halo_mat, at_rot([0.0, 0.26, -0.01], [PI * 0.46, 0.0, 0.0]));
        b.commands.entity(ring).insert(PropAnim::Halo);

        // 5. Witch hat.
        let hat_mat = b.mat(material(0x1e1b4b, 0.6, 0.2, None));
        let ribbon_mat = b.mat(material(0xf59e0b, 1.0, 0.0, Some((0xd97706, 0.5))));
        let hat_root = b.prop(head, "witch_hat", Transform::IDENTITY);
        let hat = b.group(hat_root, at_rot([0.0, 0.18, 0.01], [-0.12, 0.0, -0.06]));
        b.commands.entity(hat).insert(PropAnim::WitchHat);
        b.part(hat, cylinder(0.185, 0.19, 0.007, 32), &hat_mat, Transform::IDENTITY);
        b.part(hat, cone(0.115, 0.14, 24), &hat_mat, at(0.0, 0.07, 0.0));
        b.part(hat, cone(0.065, 0.13, 20), &hat_mat, at_rot([0.0, 0.18, -0.025], [-0.35, 0.0, 0.0]));
        b.part(hat, torus(0.105, 0.009, 8, 30, TAU), &ribbon_mat, at_rot([0.0, 0.012, 0.0], [PI * 0.5, 0.0, 0.0]));

        // 6. Shades.
        let shades = b.prop(head, "sunglasses", Transform::IDENTITY);
        let shade_mat = b.mat(material(0x020617, 0.1, 0.95, None));
        let rim_mat = b.mat(material(0x38bdf8, 1.0, 0.0, Some((0x0284c7, 1.2))));
        for s in [1.0f32, -1.0] {
            b.part(shades, Cuboid::new(0.045, 0.025, 0.004).into(), &shade_mat, at_rot([s * 0.036, 0.070, 0.104], [0.0, s * 0.08, 0.0]));
        }
        b.part(shades, Cuboid::new(0.098, 0.0035, 0.005).into(), &rim_mat, at(0.0, 0.083, 0.105));

        // 7. Devil horns.
        let horns = b.prop(head, "devil_horns", Transform::IDENTITY);
        let horn_mat = b.mat(material(0x1e1b4b, 0.3, 0.5, Some((0xf43f5e, 0.75))));
        let tip_mat = b.mat(material(0xf43f5e, 0.1, 0.0, Some((0xe11d48, 1.2))));
        for s in [1.0f32, -1.0] {
            let horn = b.part(horns, cone(0.024, 0.065, 12), &horn_mat, at_rot([s * 0.085, 0.18, 0.02], [0.2, s * 0.35, -s * 0.42]));
            b.part(horn, cone(0.015, 0.045, 12), &tip_mat, at_rot([0.0, 0.045, 0.008], [-0.25, 0.0, 0.0]));
        }
    }

    if let Some(chest) = frame(&mut b, rig, &["upperChest", "chest", "spine"]) {
        // 8. Angel wings.
        let wings = b.prop(chest, "angel_wings", Transform::IDENTITY);
        let mut wing_mat = material(0xffffff, 0.25, 0.12, Some((0x38bdf8, 0.65)));
        wing_mat.base_color.set_alpha(0.94);
        wing_mat.alpha_mode = AlphaMode::Blend;
        let wing_mat = b.mat(wing_mat);
        let bone_mat = b.mat(material(0xbae6fd, 0.15, 0.0, Some((0x0284c7, 0.95))));
        for left in [true, false] {
            let s = if left { 1.0 } else { -1.0 };
            let wing = b.group(wings, at(s * 0.045, 0.08, -0.085));
            b.commands.entity(wing).insert(PropAnim::Wing { left });
            b.part(wing, cylinder(0.009, 0.014, 0.24, 8), &bone_mat, at_rot([s * 0.09, 0.06, 0.0], [0.2, s * 0.2, -s * 0.95]));
            for i in 0..5 {
                let i = i as f32;
                let len = 0.24 - i * 0.032;
                let feather = at_rot(
                    [s * (0.055 + i * 0.038), -i * 0.032, -0.012],
                    [-0.1, s * 0.18, -s * (0.62 + i * 0.22)],
                )
                .with_scale(Vec3::new(1.0, 1.0, 0.28));
                b.part(wing, cone(0.022, len, 5), &wing_mat, feather);
            }
        }

        // 9. Cyber katana on the back.
        let sword_root = b.prop(chest, "cyber_sword", Transform::IDENTITY);
        let scab_mat = b.mat(material(0x0f172a, 0.3, 0.7, None));
        let blade_mat = b.mat(material(0x38bdf8, 1.0, 0.0, Some((0x0284c7, 1.2))));
        let gold = b.mat(material(0xf59e0b, 0.2, 0.8, None));
        let sword = b.group(sword_root, at_rot([0.04, 0.03, -0.11], [0.15, 0.18, 0.72]));
        b.commands.entity(sword).insert(PropAnim::Sword);
        let scabbard = b.part(sword, Cuboid::new(0.018, 0.62, 0.034).into(), &scab_mat, Transform::IDENTITY);
        b.part(scabbard, Cuboid::new(0.004, 0.61, 0.008).into(), &blade_mat, at(0.008, 0.0, 0.014));
        b.part(scabbard, cylinder(0.032, 0.032, 0.008, 8), &gold, at(0.0, 0.315, 0.0));
        b.part(scabbard, cylinder(0.013, 0.015, 0.17, 10), &scab_mat, at(0.0, 0.405, 0.0));
        b.part(scabbard, Sphere::new(0.016).mesh().uv(10, 10), &gold, at(0.0, 0.495, 0.0));
    }

    if let Some(hips) = frame(&mut b, rig, &["hips"]) {
        // 10. Fox tail.
        let tail_root = b.prop(hips, "fox_tail", Transform::IDENTITY);
        let fur = b.mat(material(0xf8fafc, 0.8, 0.0, Some((0xf472b6, 0.25))));
        let fur_tip = b.mat(material(0xffffff, 0.6, 0.0, Some((0x38bdf8, 0.5))));
        let tail = b.group(tail_root, at(0.0, -0.06, -0.08));
        b.commands.entity(tail).insert(PropAnim::FoxTail);
        b.part(tail, cylinder(0.028, 0.018, 0.14, 12), &fur, at_rot([0.0, -0.07, -0.05], [-0.75, 0.0, 0.0]));
        b.part(tail, cone(0.052, 0.18, 14), &fur, at_rot([0.0, -0.17, -0.14], [-1.45, 0.0, 0.0]));
        b.part(tail, cone(0.036, 0.14, 12), &fur_tip, at_rot([0.0, -0.12, -0.27], [-2.1, 0.0, 0.0]));
    }

    if let Some(right_hand) = frame(&mut b, rig, &["rightHand"]) {
        // 11. Coffee mug.
        let mug_root = b.prop(right_hand, "coffee_mug", Transform::IDENTITY);
        let mug_mat = b.mat(material(0xf8fafc, 0.2, 0.1, None));
        let coffee = b.mat(material(0x3e2723, 0.4, 0.0, None));
        let mug = b.group(mug_root, at_rot([0.035, 0.035, 0.025], [0.2, 0.0, -0.4]));
        b.commands.entity(mug).insert(PropAnim::Mug);
        b.part(mug, cylinder(0.024, 0.022, 0.054, 16), &mug_mat, Transform::IDENTITY);
        b.part(mug, cylinder(0.021, 0.021, 0.005, 16), &coffee, at(0.0, 0.023, 0.0));
        b.part(mug, torus(0.016, 0.004, 8, 16, PI), &mug_mat, at_rot([0.026, 0.0, 0.0], [0.0, 0.0, PI * 0.5]));

        // 12. Game controller.
        let pad = b.prop(right_hand, "game_controller", at_rot([0.045, 0.045, 0.035], [0.4, 0.2, -0.3]));
        let body = b.mat(material(0x0f172a, 0.35, 0.7, None));
        let led = b.mat(material(0x38bdf8, 1.0, 0.0, Some((0x0284c7, 1.2))));
        let btn = b.mat(material(0xf43f5e, 1.0, 0.0, Some((0xe11d48, 0.8))));
        b.part(pad, Cuboid::new(0.075, 0.038, 0.016).into(), &body, Transform::IDENTITY);
        b.part(pad, cylinder(0.007, 0.007, 0.008, 10), &led, at(-0.02, 0.008, 0.008));
        b.part(pad, cylinder(0.007, 0.007, 0.008, 10), &led, at(0.012, -0.004, 0.008));
        b.part(pad, cylinder(0.007, 0.007, 0.008, 10), &btn, at(0.022, 0.006, 0.008));
    }

    if let Some(left_hand) = frame(&mut b, rig, &["leftHand"]) {
        // 13. Smartphone.
        let phone_root = b.prop(left_hand, "smartphone", Transform::IDENTITY);
        let chassis = b.mat(material(0x020617, 0.2, 0.9, None));
        let screen = b.mat(material(0x38bdf8, 0.1, 0.0, Some((0x0284c7, 0.95))));
        let phone = b.group(phone_root, at_rot([-0.035, 0.035, 0.03], [0.4, -0.3, 0.5]));
        b.commands.entity(phone).insert(PropAnim::Phone);
        b.part(phone, Cuboid::new(0.038, 0.072, 0.0045).into(), &chassis, Transform::IDENTITY);
        b.part(phone, Cuboid::new(0.034, 0.066, 0.001).into(), &screen, at(0.0, 0.0, 0.0028));
    }
}

/// Shows exactly the props the app has switched on.
pub fn sync_visibility(state: Res<AvatarState>, mut props: Query<(&Prop, &mut Visibility)>) {
    for (prop, mut visibility) in props.iter_mut() {
        let on = state.props.get(prop.0).copied().unwrap_or(false);
        let wanted = if on { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// The reference's per-frame prop motion.
pub fn animate_props(
    time: Res<Time>,
    state: Res<AvatarState>,
    mut parts: Query<(&PropAnim, &mut Transform)>,
) {
    let t = time.elapsed_secs();
    let inertia_x = state.inertia_x;
    let inertia_y = state.inertia_y;
    for (anim, mut transform) in parts.iter_mut() {
        match *anim {
            PropAnim::CatEar { left } => {
                let twitch = if (t * 5.5).sin() > 0.88 { (t * 28.0).sin() * 0.14 } else { 0.0 };
                transform.rotation = if left {
                    euler(0.1, 0.25, -0.32 + twitch)
                } else {
                    euler(0.1, -0.25, 0.32 - twitch)
                };
            }
            PropAnim::Halo => {
                transform.translation.y = 0.26 + (t * 3.0).sin() * 0.012;
                transform.rotation = euler(PI * 0.46, 0.0, t * 1.2);
            }
            PropAnim::Wing { left } => {
                let flap = (t * 3.6).sin() * 0.18;
                let hover = (t * 1.8).sin() * 0.04;
                transform.rotation = if left {
                    euler(0.0, -0.28 + flap + inertia_x * 0.35, 0.08 + hover)
                } else {
                    euler(0.0, 0.28 - flap + inertia_x * 0.35, -0.08 - hover)
                };
            }
            PropAnim::WitchHat => {
                transform.rotation = euler(-0.12, 0.0, -0.06 + (t * 1.8).sin() * 0.03);
            }
            PropAnim::FoxTail => {
                let sway = (t * 2.4).sin() * 0.22;
                transform.rotation = euler(
                    -0.55 + (t * 4.8).sin() * 0.08 - inertia_y * 0.35,
                    0.0,
                    sway - inertia_x * 0.45,
                );
            }
            PropAnim::Sword => {
                transform.rotation = euler(0.15, 0.18 + (t * 1.5).sin() * 0.04, 0.72);
            }
            PropAnim::Mug => {
                transform.translation.y = 0.035 + (t * 2.5).sin() * 0.003;
            }
            PropAnim::Phone => {
                transform.rotation = euler(0.4, -0.3, 0.5 + (t * 1.8).sin() * 0.03);
            }
        }
    }
}
