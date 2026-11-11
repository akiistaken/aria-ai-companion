//! Avatar behaviour: model loading, camera framing, expressions, blink, lip-sync, idle posing.
//!
//! The numeric tuning values (framing distances, blink timing, breathing amplitudes) are ported
//! from the WebView reference at `docs/reference/vtuber_stage.html` and must stay in sync with it.

use std::collections::BTreeMap;
use std::time::Instant;

use bevy::prelude::*;
use bevy_vrm1::prelude::*;



/// Everything the main app can influence.
#[derive(Resource)]
pub struct AvatarState {
    /// Path waiting to be loaded from `LoadModel`.
    pub pending_model: Option<String>,
    pub emotion: String,
    pub facing_right: bool,
    pub moving: bool,
    pub talking: bool,
    pub look_x: f32,
    pub look_y: f32,
    pub mouth_aa: f32,
    pub mouth_oh: f32,
    pub peeking: aria_protocol::PeekSide,
    pub music_active: bool,
    pub music_energy: f32,
    pub inertia_x: f32,
    pub inertia_y: f32,
    pub headpat_at: Option<Instant>,
    pub props: BTreeMap<String, bool>,
    pub lighting: String,
    pub framing: String,
    pub zoom: f32,
    pub y_offset: f32,
    pub fps: u32,

    // Runtime animation state, not driven from outside.
    pub blink_clock: f32,
    pub next_blink: f32,
    pub smoothed_look_x: f32,
    pub smoothed_look_y: f32,
    pub smoothed_aa: f32,
    pub smoothed_oh: f32,
    pub breath: f32,
    pub root: Option<Entity>,
    pub model_height: f32,
    pub model_center_y: f32,
    pub squashed: f32,
    /// Set once the first model load has been requested.
    pub loaded_any: bool,
    /// Model requested on the command line (standalone runs).
    pub initial_model: String,
}

impl Default for AvatarState {
    fn default() -> Self {
        Self {
            emotion: "idle".into(),
            facing_right: false,
            moving: false,
            talking: false,
            look_x: 0.0,
            look_y: 0.0,
            mouth_aa: 0.0,
            mouth_oh: 0.0,
            peeking: aria_protocol::PeekSide::None,
            music_active: false,
            music_energy: 0.0,
            inertia_x: 0.0,
            inertia_y: 0.0,
            headpat_at: None,
            props: BTreeMap::new(),
            lighting: "auto".into(),
            framing: "half".into(),
            zoom: 1.0,
            y_offset: 0.0,
            fps: 30,
            blink_clock: 0.0,
            next_blink: 2.2,
            smoothed_look_x: 0.0,
            smoothed_look_y: 0.0,
            smoothed_aa: 0.0,
            smoothed_oh: 0.0,
            breath: 0.0,
            root: None,
            model_height: 1.6,
            model_center_y: 0.9,
            squashed: 0.0,
            loaded_any: false,
            initial_model: String::new(),
            pending_model: None,
        }
    }
}

/// Spawns a VRM root, falling back to the bundled model when the path is unusable.
fn load_model(path: String, asset_server: &AssetServer, commands: &mut Commands) {
    let handle: Handle<bevy::gltf::Gltf> = if std::path::Path::new(&path).is_file() {
        log::info!("loading avatar model {path}");
        asset_server.load(path)
    } else {
        log::warn!("avatar model {path} not found; falling back to the bundled model");
        asset_server.load("models/aria_default.vrm")
    };
    commands.spawn((VrmHandle(handle), LookAt::Cursor));
}

#[derive(Component)]
pub struct AvatarCamera;

/// Marks the entity that `LookAt::Target` follows, i.e. the head-look target.
#[derive(Component)]
pub struct LookTarget;

/// Requests the camera framing be recomputed (model changed or framing setting changed).
#[derive(Component)]
pub struct FramingDirty;

pub fn frame_camera(
    state: &AvatarState,
    transform: &mut Transform,
) {
    // Ported from applyVRMCameraFraming().
    let height = state.model_height;
    let center = state.model_center_y;
    let mut target_y = center + height * 0.12;
    let mut dist = height * 1.55;

    match state.framing.as_str() {
        "full" => {
            target_y = center - height * 0.02;
            dist = height * 2.38;
        }
        "face" => {
            target_y = center + height * 0.38;
            dist = height * 0.78;
        }
        _ => {}
    }

    dist *= state.zoom;
    target_y += state.y_offset * height;

    transform.translation = Vec3::new(0.0, target_y, dist);
    transform.look_at(Vec3::new(0.0, target_y, 0.0), Vec3::Y);
}

/// Cheap pseudo-random `0..1`. Blink timing does not justify a dependency.
fn rand_unit() -> f32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    // xorshift-style scramble so consecutive calls differ.
    let mut x = nanos ^ (nanos << 13) ^ (nanos >> 7);
    x ^= x << 17;
    (x % 10_000) as f32 / 10_000.0
}

/// The per-frame animation pass.
pub fn animate(
    time: Res<Time>,
    mut state: ResMut<AvatarState>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut link: ResMut<crate::link::AvatarLink>,
    vrms: Query<(Entity, Option<&Vrm>), With<VrmHandle>>,
    mut cameras: Query<&mut Transform, (With<AvatarCamera>, Without<LookTarget>)>,
    mut look_targets: Query<&mut Transform, With<LookTarget>>,
) {
    let dt = time.delta_secs().min(0.1);

    // ---- Model loading ----------------------------------------------------------------
    let requested = state.pending_model.take();
    if let Some(path) = requested {
        load_model(path, &asset_server, &mut commands);
    } else if !state.loaded_any {
        // First frame: load whatever the command line asked for, else the bundled model.
        state.loaded_any = true;
        let initial = if state.initial_model.is_empty() {
            "models/aria_default.vrm".to_string()
        } else {
            state.initial_model.clone()
        };
        load_model(initial, &asset_server, &mut commands);
    }

    // ---- Blink (reference: next blink 2.2..5.4 s, 0.14 s duration) ---------------------
    state.blink_clock += dt;
    let mut blink = 0.0;
    if state.blink_clock >= state.next_blink {
        let elapsed = state.blink_clock - state.next_blink;
        if elapsed < 0.14 {
            blink = ((elapsed / 0.14) * std::f32::consts::PI).sin();
        } else {
            state.blink_clock = 0.0;
            state.next_blink = 2.2 + rand_unit() * 3.2;
        }
    }

    // ---- Smoothed look and mouth ------------------------------------------------------
    let smooth = (dt * 8.0).min(1.0);
    state.smoothed_look_x += (state.look_x - state.smoothed_look_x) * smooth;
    state.smoothed_look_y += (state.look_y - state.smoothed_look_y) * smooth;

    let (mut aa, mut oh) = (state.mouth_aa, state.mouth_oh);
    if aa <= 0.001 && oh <= 0.001 && state.talking {
        // Procedural fallback when no lip-sync envelope arrives.
        let t = time.elapsed_secs();
        aa = (t * 17.5).sin().mul_add(0.55, (t * 11.0).cos() * 0.2).max(0.0);
        oh = (t * 9.5 + 1.0).sin().mul_add(0.3, 0.0).max(0.0);
    }
    let mouth_smooth = (dt * 14.0).min(1.0);
    state.smoothed_aa += (aa - state.smoothed_aa) * mouth_smooth;
    state.smoothed_oh += (oh - state.smoothed_oh) * mouth_smooth;

    // ---- Breathing --------------------------------------------------------------------
    state.breath += dt;
    let _breath = (state.breath * 1.6).sin() * 0.012;

    // ---- Look target ------------------------------------------------------------------
    for mut transform in look_targets.iter_mut() {
        transform.translation.x = state.smoothed_look_x * 0.85;
        transform.translation.y = 1.25 + state.smoothed_look_y * 0.55;
        transform.translation.z = -1.0;
    }

    // ---- Camera framing ---------------------------------------------------------------
    if let Ok(mut camera) = cameras.single_mut() {
        frame_camera(&state, &mut camera);
    }

    // ---- Expressions ------------------------------------------------------------------
    let Some(root) = state.root else {
        // The root becomes available once the model reports `Initialized`.
        for (entity, vrm) in vrms.iter() {
            if vrm.is_some() {
                state.root = Some(entity);
                break;
            }
        }
        return;
    };

    let emotion = state.emotion.clone();
    // `SetExpressions` replaces every override, so the full set is sent each change.
    let mut wanted: Vec<(&str, f32)> = Vec::new();
    match emotion.as_str() {
        "happy" | "dance" | "groove" => wanted.push(("happy", 0.85)),
        "annoyed" => wanted.push(("angry", 0.8)),
        "sleepy" | "sit" => wanted.push(("relaxed", 0.6)),
        "dizzy" => wanted.push(("surprised", 0.7)),
        _ => wanted.push(("neutral", 0.35)),
    }
    if blink > 0.01 {
        wanted.push(("blink", blink));
    }
    for (name, weight) in wanted {
        commands.trigger(SetExpressions::single(root, name, weight));
    }
    if state.smoothed_aa > 0.01 || state.smoothed_oh > 0.01 {
        commands.trigger(ModifyExpressions::mouth(root, "aa", state.smoothed_aa));
        commands.trigger(ModifyExpressions::mouth(root, "oh", state.smoothed_oh));
    }

    let _ = &mut link;
}
