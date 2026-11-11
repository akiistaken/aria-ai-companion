//! Avatar behaviour: model lifecycle, camera framing, look-at, expressions, blink, lip-sync and
//! posing.
//!
//! Tuning values (framing distances, blink timing, smoothing rates, expression weights) are ported
//! from the reference stage at `docs/reference/vtuber_stage.html` and must stay in sync with it.

use std::collections::BTreeMap;

use bevy::prelude::*;
use bevy_vrm1::prelude::*;

use aria_protocol::{Event, PeekSide};

use crate::link::AvatarLink;
use crate::rig::Rig;

/// The model shipped with the app, relative to the asset root.
pub const BUNDLED_MODEL: &str = "models/aria_default.vrm";

/// Everything the main app can influence, plus the animation state derived from it.
#[derive(Resource)]
pub struct AvatarState {
    // ---- Driven by the app ----
    /// Path waiting to be loaded from `LoadModel` (or the command line).
    pub pending_model: Option<String>,
    pub emotion: String,
    pub facing_right: bool,
    pub moving: bool,
    pub talking: bool,
    pub look_x: f32,
    pub look_y: f32,
    pub mouth_aa: f32,
    pub mouth_oh: f32,
    /// Seconds since the last `SetMouth`; the measured envelope wins while it is fresh.
    pub mouth_age: f32,
    pub peeking: PeekSide,
    pub music_active: bool,
    pub music_energy: f32,
    pub target_inertia_x: f32,
    pub target_inertia_y: f32,
    pub props: BTreeMap<String, bool>,
    pub lighting: String,
    pub framing: String,
    pub zoom: f32,
    pub y_offset: f32,
    /// Requested frame rate; 0 pauses rendering.
    pub fps: u32,
    /// Window size in physical pixels.
    pub viewport: (u32, u32),
    /// Latest cursor sample, consumed by the gesture detectors.
    pub pointer: Option<crate::input::PointerSample>,

    // ---- Animation state ----
    pub smoothed_look_x: f32,
    pub smoothed_look_y: f32,
    pub inertia_x: f32,
    pub inertia_y: f32,
    pub headpat_active: f32,
    pub emotion_elapsed: f32,
    last_emotion: String,
    blink_timer: f32,
    next_blink: f32,
    body_yaw: f32,
    last_expressions: [f32; 7],

    // ---- Model ----
    pub root: Option<Entity>,
    pub rig: Option<Rig>,
    props_pending: bool,
    load_handle: Option<Handle<bevy::gltf::Gltf>>,
    loading_path: String,
    pub look_target: Option<Entity>,
}

impl Default for AvatarState {
    fn default() -> Self {
        Self {
            pending_model: None,
            emotion: "idle".into(),
            facing_right: false,
            moving: false,
            talking: false,
            look_x: 0.0,
            look_y: 0.0,
            mouth_aa: 0.0,
            mouth_oh: 0.0,
            mouth_age: f32::MAX,
            peeking: PeekSide::None,
            music_active: false,
            music_energy: 0.0,
            target_inertia_x: 0.0,
            target_inertia_y: 0.0,
            props: BTreeMap::new(),
            lighting: "auto".into(),
            framing: "half".into(),
            zoom: 1.0,
            y_offset: 0.0,
            fps: 30,
            viewport: (crate::DEFAULT_W, crate::DEFAULT_H),
            pointer: None,
            smoothed_look_x: 0.0,
            smoothed_look_y: 0.0,
            inertia_x: 0.0,
            inertia_y: 0.0,
            headpat_active: 0.0,
            emotion_elapsed: 0.0,
            last_emotion: "idle".into(),
            blink_timer: 0.0,
            next_blink: 2.2,
            body_yaw: 0.0,
            last_expressions: [-1.0; 7],
            root: None,
            rig: None,
            props_pending: false,
            load_handle: None,
            loading_path: String::new(),
            look_target: None,
        }
    }
}

impl AvatarState {
    /// Initial state that loads `model` on the first frame.
    pub fn with_model(model: String) -> Self {
        Self {
            pending_model: Some(model),
            ..Self::default()
        }
    }

    /// The reference renders at 30 FPS idle and 45 FPS while something is moving. The app's
    /// requested rate is the base; the boost only applies at the normal (30+) rate, never in game
    /// mode.
    pub fn effective_fps(&self) -> u32 {
        let busy = self.talking
            || self.moving
            || matches!(self.emotion.as_str(), "dance" | "dizzy" | "walking")
            || self.headpat_active > 0.0;
        if self.fps >= 30 && busy {
            self.fps.max(45)
        } else {
            self.fps
        }
    }
}

#[derive(Component)]
pub struct AvatarCamera;

/// The entity the VRM's eyes follow; a child of the camera.
#[derive(Component)]
pub struct LookTarget;

/// Cheap pseudo-random `0..1`. Blink timing does not justify a dependency.
fn rand_unit() -> f32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let mut x = nanos ^ (nanos << 13) ^ (nanos >> 7);
    x ^= x << 17;
    (x % 10_000) as f32 / 10_000.0
}

/// Loads requested models, replacing the previous one, and falls back to the bundled model when a
/// custom one is missing or fails to load.
pub fn manage_model(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut state: ResMut<AvatarState>,
    link: Res<AvatarLink>,
) {
    // A load that failed: report it and fall back once.
    if let Some(handle) = state.load_handle.as_ref() {
        if let Some(bevy::asset::LoadState::Failed(error)) = asset_server.get_load_state(handle.id()) {
            let path = state.loading_path.clone();
            log::error!("avatar model {path} failed to load: {error}");
            link.send(Event::ModelLoaded {
                ok: false,
                error: Some(error.to_string()),
                expressions: Vec::new(),
            });
            state.load_handle = None;
            if path != BUNDLED_MODEL {
                state.pending_model = Some(BUNDLED_MODEL.into());
            }
        }
    }

    let Some(path) = state.pending_model.take() else {
        return;
    };
    if let Some(old) = state.root.take() {
        commands.entity(old).despawn();
    }
    state.rig = None;

    let path = if path == BUNDLED_MODEL || std::path::Path::new(&path).is_file() {
        path
    } else {
        log::warn!("avatar model {path} not found; using the bundled model");
        BUNDLED_MODEL.to_string()
    };
    log::info!("loading avatar model {path}");
    let handle: Handle<bevy::gltf::Gltf> = asset_server.load(path.clone());
    let mut root = commands.spawn((VrmHandle(handle.clone()), Transform::default()));
    if let Some(target) = state.look_target {
        root.insert(LookAt::Target(target));
    }
    state.root = Some(root.id());
    state.load_handle = Some(handle);
    state.loading_path = path;
}

/// Resolves the rig once `bevy_vrm1` has finished setting the model up, and tells the app the
/// model is in.
///
/// Exclusive because it reads arbitrary components off the VRM root, which a `Query<EntityRef>`
/// cannot do next to `ResMut<AvatarState>` (resources are entities in this Bevy version).
pub fn resolve_rig(world: &mut World) {
    let (root, resolved) = {
        let state = world.resource::<AvatarState>();
        (state.root, state.rig.is_some())
    };
    let Some(root) = root.filter(|_| !resolved) else { return };
    let Some(rig) = Rig::resolve(world, root) else { return };
    log::info!("rig resolved: {} bones, height {:.2} m", rig.len(), rig.height);
    {
        let mut state = world.resource_mut::<AvatarState>();
        state.rig = Some(rig);
        state.props_pending = true;
        state.load_handle = None;
        state.last_expressions = [-1.0; 7];
    }
    world.resource::<AvatarLink>().send(Event::ModelLoaded {
        ok: true,
        error: None,
        expressions: Vec::new(),
    });
}

/// Builds the wardrobe for a newly resolved rig.
pub fn build_props(
    mut state: ResMut<AvatarState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !state.props_pending {
        return;
    }
    state.props_pending = false;
    if let Some(rig) = state.rig.as_ref() {
        crate::props::build_props(rig, &mut commands, &mut meshes, &mut materials);
    }
}

/// Camera framing, after `applyVRMCameraFraming()` in the reference.
///
/// With the 28° lens the reference's "half" (1.55 × height) actually showed knees-up, which spends
/// half of the small stage on legs. "half" is now a real waist-up shot; "full" and "face" keep the
/// reference numbers.
fn frame_camera(state: &AvatarState, transform: &mut Transform) {
    let (height, center) = state
        .rig
        .as_ref()
        .map(|r| (r.height, r.center_y))
        .unwrap_or((1.5, 0.8));
    let (mut target_y, mut dist) = match state.framing.as_str() {
        "full" => (center - height * 0.02, height * 2.38),
        "face" => (center + height * 0.38, height * 0.78),
        _ => (center + height * 0.22, height * 1.22),
    };
    let zoom = if state.zoom.is_finite() && state.zoom > 0.0 { state.zoom } else { 1.0 };
    dist *= zoom;
    target_y += state.y_offset * height;
    transform.translation = Vec3::new(0.0, target_y, dist);
    transform.look_at(Vec3::new(0.0, target_y, 0.0), Vec3::Y);
}

/// The per-frame animation pass.
#[allow(clippy::type_complexity)]
pub fn animate(
    time: Res<Time>,
    mut state: ResMut<AvatarState>,
    mut commands: Commands,
    mut parts: ParamSet<(
        Query<&mut Transform, With<AvatarCamera>>,
        Query<&mut Transform, With<LookTarget>>,
        Query<&mut Transform, (Without<AvatarCamera>, Without<LookTarget>)>,
    )>,
) {
    let dt = time.delta_secs().min(0.1);
    let t = time.elapsed_secs();
    let s = &mut *state;

    // ---- Smoothing (reference rates) -------------------------------------------------
    let look = (dt * 8.0).min(1.0);
    s.smoothed_look_x += (s.look_x - s.smoothed_look_x) * look;
    s.smoothed_look_y += (s.look_y - s.smoothed_look_y) * look;
    let inertia = (dt * 10.0).min(1.0);
    s.inertia_x += (s.target_inertia_x - s.inertia_x) * inertia;
    s.inertia_y += (s.target_inertia_y - s.inertia_y) * inertia;
    if s.headpat_active > 0.0 {
        s.headpat_active = (s.headpat_active - dt * 0.38).max(0.0);
    }
    if s.emotion != s.last_emotion {
        s.last_emotion.clone_from(&s.emotion);
        s.emotion_elapsed = 0.0;
    } else {
        s.emotion_elapsed += dt;
    }
    s.mouth_age += dt;

    // ---- Blink: next in 2.2..5.4 s, lasting 0.14 s -------------------------------------
    s.blink_timer += dt;
    let mut blink = 0.0;
    if s.blink_timer >= s.next_blink {
        let elapsed = s.blink_timer - s.next_blink;
        if elapsed < 0.14 {
            blink = ((elapsed / 0.14) * std::f32::consts::PI).sin();
        } else {
            s.blink_timer = 0.0;
            s.next_blink = 2.2 + rand_unit() * 3.2;
        }
    }

    // ---- Mouth: the app's measured TTS envelope, else the procedural flap ---------------
    let (aa, oh) = if s.mouth_age < 0.3 {
        (s.mouth_aa, s.mouth_oh)
    } else if s.talking {
        (
            ((t * 17.5).sin() * 0.55 + (t * 11.0).cos() * 0.2).max(0.0),
            ((t * 9.5 + 1.0).sin() * 0.3).max(0.0),
        )
    } else {
        (0.0, 0.0)
    };

    // ---- Camera and look target ---------------------------------------------------------
    if let Ok(mut camera) = parts.p0().single_mut() {
        frame_camera(s, &mut camera);
    }
    if let Ok(mut target) = parts.p1().single_mut() {
        // Camera-local: the eyes follow the cursor around the viewer.
        target.translation = Vec3::new(s.smoothed_look_x * 0.85, s.smoothed_look_y * 0.55, -1.0);
    }

    let (Some(root), Some(rig)) = (s.root, s.rig.as_ref()) else {
        return;
    };

    // ---- Pose -------------------------------------------------------------------------
    let pose = crate::posing::pose_for(s, t);
    {
        let mut transforms = parts.p2();
        // Bones a branch leaves out go back to rest, so no limb is stranded in an old pose.
        for (name, _) in rig.bones() {
            if !pose.bones.iter().any(|(n, _)| *n == name) {
                rig.apply(name, Quat::IDENTITY, &mut transforms);
            }
        }
        for (name, euler) in &pose.bones {
            rig.apply(
                name,
                Quat::from_euler(EulerRot::XYZ, euler.x, euler.y, euler.z),
                &mut transforms,
            );
        }
        rig.offset_hips(Vec3::new(0.0, pose.hips_offset_y, 0.0), &mut transforms);

        let body = crate::posing::body_rotation(s, s.body_yaw, dt);
        s.body_yaw = body.y;
        if let Ok(mut root_transform) = transforms.get_mut(root) {
            root_transform.rotation = Quat::from_euler(EulerRot::XYZ, body.x, body.y, body.z);
        }
    }

    // ---- Expressions (one `SetExpressions` per frame: it replaces every override) -------
    let emo = s.emotion.as_str();
    let full_happy = emo == "happy" || emo == "dance";
    let mild_happy = emo == "talking" || emo == "walking";
    let mut happy = if full_happy { 0.88 } else if mild_happy { 0.35 } else { 0.0 };
    let mut relaxed = if emo == "idle" || emo == "sleepy" { 0.55 } else { 0.0 };
    let mut blink_w = if emo == "sleepy" { blink.max(0.68) } else { blink };
    if s.headpat_active > 0.0 {
        happy = f32::max(0.96 * s.headpat_active, happy);
        relaxed = f32::max(0.88 * s.headpat_active, relaxed);
        blink_w = f32::max(0.55 * s.headpat_active, blink_w);
    }
    let weights = [
        blink_w,
        aa.clamp(0.0, 1.0),
        if emo == "dizzy" { oh.max(0.55) } else { oh }.clamp(0.0, 1.0),
        if emo == "annoyed" { 0.85 } else { 0.0 },
        happy,
        relaxed,
        if emo == "dizzy" { 0.8 } else { 0.0 },
    ];
    let changed = weights
        .iter()
        .zip(s.last_expressions.iter())
        .any(|(a, b)| (a - b).abs() > 0.004);
    if changed {
        s.last_expressions = weights;
        let names = ["blink", "aa", "oh", "angry", "happy", "relaxed", "surprised"];
        commands.trigger(SetExpressions::from_iter(
            root,
            names.into_iter().zip(weights),
        ));
    }
}

/// Spawns the look target under the camera. Runs once after the camera exists.
pub fn spawn_look_target(
    mut commands: Commands,
    mut state: ResMut<AvatarState>,
    cameras: Query<Entity, With<AvatarCamera>>,
) {
    if state.look_target.is_some() {
        return;
    }
    let Ok(camera) = cameras.single() else { return };
    let target = commands
        .spawn((LookTarget, Transform::from_xyz(0.0, 0.0, -1.0), ChildOf(camera)))
        .id();
    state.look_target = Some(target);
}
