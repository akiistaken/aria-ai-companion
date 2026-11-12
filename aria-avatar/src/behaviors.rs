//! Avatar behaviour: what the app has asked for, the model's lifecycle, and the system that runs
//! the animator and writes its result into the scene.

use std::collections::BTreeMap;

use bevy::prelude::*;
use bevy_vrm1::prelude::{Initialized, LookAt, VrmHandle};

use aria_protocol::{BodyMode, Event, PeekSide};

use crate::animator::{Animator, Framing, Input};
use crate::emotes::{Emote, Style};
use crate::face::Speech;
use crate::link::AvatarLink;
use crate::rig::Rig;
use crate::spring::approach_factor;

/// The model shipped with the app, relative to the asset root.
pub const BUNDLED_MODEL: &str = "models/aria_default.vrm";

/// A lip-sync envelope older than this is stale: the voice has stopped.
const MOUTH_FRESH_SECS: f32 = 0.3;

/// Everything the main app can influence, plus the model's lifecycle.
#[derive(Resource)]
pub struct AvatarState {
    // ---- Driven by the app ----
    /// Path waiting to be loaded from `LoadModel` (or the command line).
    pub pending_model: Option<String>,
    pub emote: Emote,
    pub style: Style,
    pub body: BodyMode,
    pub talking: bool,
    pub look_x: f32,
    pub look_y: f32,
    pub mouth_aa: f32,
    pub mouth_oh: f32,
    /// Seconds since the last `SetMouth`; the mouth only moves while the envelope is fresh.
    pub mouth_age: f32,
    pub peeking: PeekSide,
    pub music_active: bool,
    pub music_energy: f32,
    /// A beat measurement waiting for the animator's beat clock.
    pub music_sync: Option<crate::animator::BeatSync>,
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
    pub headpat_active: f32,

    // ---- Derived each frame ----
    smoothed_look: Vec2,
    /// The animator has something moving that deserves the higher frame rate.
    busy: bool,
    /// How fast she is being carried, `-1..1` per axis; wings and tails trail it.
    pub drift: Vec2,
    /// Last `Stage` geometry reported to the app.
    stage_reported: Option<(f32, f32)>,

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
            emote: Emote::Idle,
            style: Style::default(),
            body: BodyMode::Ground,
            talking: false,
            look_x: 0.0,
            look_y: 0.0,
            mouth_aa: 0.0,
            mouth_oh: 0.0,
            mouth_age: f32::MAX,
            peeking: PeekSide::None,
            music_active: false,
            music_energy: 0.0,
            music_sync: None,
            props: BTreeMap::new(),
            lighting: "auto".into(),
            framing: "full".into(),
            zoom: 1.0,
            y_offset: 0.0,
            fps: 30,
            viewport: (crate::DEFAULT_W, crate::DEFAULT_H),
            pointer: None,
            headpat_active: 0.0,
            smoothed_look: Vec2::ZERO,
            busy: false,
            drift: Vec2::ZERO,
            stage_reported: None,
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

    /// 30 FPS at rest and 60 while something is moving: walking, dancing, talking, a gesture
    /// blending in. The app's requested rate is the base; the boost only applies at the normal
    /// (30+) rate, never in game mode.
    pub fn effective_fps(&self) -> u32 {
        let busy = self.talking || self.busy || self.headpat_active > 0.0 || self.music_active;
        if self.fps >= 30 && busy {
            self.fps.max(60)
        } else {
            self.fps
        }
    }

    /// Pixels from the bottom of the window up to the floor she stands on, as last reported.
    pub fn floor_offset(&self) -> f32 {
        self.stage_reported.map(|stage| stage.0).unwrap_or(0.0)
    }

    /// The emotion to act out: an explicit one wins, otherwise speech and music fill the silence.
    /// The flag says the music filled it in (the animator then picks dance or groove by loudness).
    fn current_emote(&self) -> (Emote, bool) {
        match self.emote {
            Emote::Idle if self.talking => (Emote::Talking, false),
            Emote::Idle if self.music_active => (Emote::Groove, true),
            emote => (emote, false),
        }
    }
}

#[derive(Component)]
pub struct AvatarCamera;

/// The entity the VRM's eyes follow; a child of the camera.
#[derive(Component)]
pub struct LookTarget;

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
    // `Initialized` is the last thing `bevy_vrm1` adds: bone holders, rest snapshots and the
    // expression table are all in place once it is there.
    if world.get::<Initialized>(root).is_none() {
        return;
    }
    let Some(rig) = Rig::resolve(world, root) else { return };
    let m = &rig.metrics;
    log::info!(
        "rig resolved: {} of {} humanoid bones, height {:.2} m, leg {:.2} m, arm {:.2} m",
        rig.len(),
        crate::rig::Bone::COUNT,
        m.height,
        m.leg(),
        m.upper_arm + m.lower_arm,
    );
    let missing = rig.missing();
    if !missing.is_empty() {
        log::info!("optional bones not in this model (their motion folds into the parents): {}", missing.join(", "));
    }
    let expressions = world.resource_scope(|world, mut animator: Mut<Animator>| {
        animator.reset_for_model();
        animator.face.bind(world, root)
    });
    log::info!("expressions: {}", expressions.join(", "));
    {
        let mut state = world.resource_mut::<AvatarState>();
        state.rig = Some(rig);
        state.props_pending = true;
        state.load_handle = None;
        // A different model stands differently in the frame.
        state.stage_reported = None;
    }
    world.resource::<AvatarLink>().send(Event::ModelLoaded {
        ok: true,
        error: None,
        expressions,
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

/// The per-frame animation pass: measure the window, run the animator, write the result.
#[allow(clippy::type_complexity)]
pub fn animate(
    time: Res<Time>,
    mut state: ResMut<AvatarState>,
    mut animator: ResMut<Animator>,
    mut commands: Commands,
    link: Res<AvatarLink>,
    mut parts: ParamSet<(
        Query<&mut Transform, With<AvatarCamera>>,
        Query<&mut Transform, With<LookTarget>>,
        Query<&mut Transform, (Without<AvatarCamera>, Without<LookTarget>)>,
    )>,
) {
    let dt = time.delta_secs().clamp(1e-3, 0.1);
    let t = time.elapsed_secs();
    let s = &mut *state;
    let animator = &mut *animator;

    s.smoothed_look = s
        .smoothed_look
        .lerp(Vec2::new(s.look_x, s.look_y), approach_factor(8.0, dt));
    if s.headpat_active > 0.0 {
        s.headpat_active = (s.headpat_active - dt * 0.38).max(0.0);
    }
    s.mouth_age += dt;

    // ---- Where she stands in the window, and what that means on the desktop ------------------
    let height = s.rig.as_ref().map(|rig| rig.metrics.height).unwrap_or(1.6);
    let framing = Framing::new(
        &s.framing,
        height,
        s.viewport.1,
        crate::FOV_DEGREES.to_radians(),
        s.zoom,
        s.y_offset,
    );
    let report = (framing.ground_offset, framing.pixels_per_metre);
    let stale = s
        .stage_reported
        .is_none_or(|sent| (sent.0 - report.0).abs() > 0.25 || (sent.1 - report.1).abs() > 0.05);
    if stale {
        s.stage_reported = Some(report);
        link.send(Event::Stage {
            ground_offset: report.0,
            pixels_per_metre: report.1,
        });
    }

    let window = animator
        .simulated_window
        .or_else(|| crate::layered::window_origin().map(IVec2::from));
    animator.motion.update(window, s.viewport, framing.pixels_per_metre, dt);
    // The model and the camera travel together through the world, so the picture does not
    // change, but hair and clothes feel every start, stop and fall.
    let world = animator.motion.world.extend(0.0);
    s.drift = (animator.motion.velocity / 1.5).clamp(Vec2::splat(-1.0), Vec2::splat(1.0));

    // ---- Animate ----------------------------------------------------------------------------
    let sync = s.music_sync.take();
    let frame = s.rig.as_ref().zip(s.root).map(|(rig, root)| {
        let speech = if s.mouth_age < MOUTH_FRESH_SECS {
            Speech { aa: s.mouth_aa, oh: s.mouth_oh }
        } else {
            Speech::default()
        };
        let (emote, music_fill) = s.current_emote();
        let input = Input {
            emote,
            style: s.style,
            mode: s.body,
            look: s.smoothed_look,
            speech,
            // Without music, dancing on request goes full out.
            music: if s.music_active { s.music_energy } else { 1.0 },
            music_fill,
            sync,
            headpat: s.headpat_active,
            peeking: s.peeking,
        };
        let frame = animator.step(rig, &input, t, dt);
        {
            let mut transforms = parts.p2();
            rig.apply(&frame.pose, &mut transforms);
            if let Ok(mut transform) = transforms.get_mut(root) {
                transform.translation = world + frame.root.translation;
                transform.rotation = frame.root.rotation;
            }
        }
        animator.face.send(frame.expressions, root, &mut commands);
        frame
    });
    s.busy = frame.as_ref().is_some_and(|frame| frame.busy);
    let gaze = frame.as_ref().map(|frame| frame.gaze).unwrap_or(s.smoothed_look);
    let drop = frame.as_ref().map(|frame| frame.drop).unwrap_or(0.0);

    // ---- Camera and look target --------------------------------------------------------------
    if let Ok(mut camera) = parts.p0().single_mut() {
        let eye_height = framing.target_y - drop * framing.follow;
        camera.translation = world + Vec3::new(0.0, eye_height, framing.distance);
        camera.rotation = Quat::IDENTITY;
    }
    if let Ok(mut target) = parts.p1().single_mut() {
        // Camera-local: the eyes follow the cursor around the viewer.
        target.translation = Vec3::new(gaze.x * 0.85, gaze.y * 0.55, -1.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_and_music_only_fill_in_for_idle() {
        let mut state = AvatarState::default();
        assert_eq!(state.current_emote(), (Emote::Idle, false));
        state.music_active = true;
        assert_eq!(state.current_emote(), (Emote::Groove, true), "music fills the silence");
        state.talking = true;
        assert_eq!(state.current_emote(), (Emote::Talking, false), "speech beats music");
        state.emote = Emote::Sad;
        assert_eq!(state.current_emote(), (Emote::Sad, false), "an explicit emotion beats both");
    }

    #[test]
    fn the_frame_rate_boost_needs_the_normal_rate() {
        let mut state = AvatarState { busy: true, ..default() };
        assert_eq!(state.effective_fps(), 60);
        state.fps = 10;
        assert_eq!(state.effective_fps(), 10, "game mode stays slow");
        state.fps = 0;
        assert_eq!(state.effective_fps(), 0, "paused stays paused");
        state.busy = false;
        state.fps = 30;
        assert_eq!(state.effective_fps(), 30);
    }
}
