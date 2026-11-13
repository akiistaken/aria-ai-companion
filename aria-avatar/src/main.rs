//! Aria avatar renderer.
//!
//! A headless Bevy app that loads a VRM 1.0 model with `bevy_vrm1` (MToon shading, spring bones,
//! look-at, node constraints), renders it offscreen and presents it through a layered Win32 window
//! with per-pixel alpha. The main app drives it over a local socket and owns the window's position
//! and size; nothing here handles desktop geometry.

// Release builds must not open a console window of their own next to the avatar.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod animator;
mod behaviors;
mod demo;
mod emotes;
mod face;
mod hearts;
mod ik;
mod input;
mod layered;
mod lighting;
mod link;
mod locomotion;
mod pose;
mod present;
mod props;
mod rig;
mod spring;


use bevy::prelude::*;
use bevy_vrm1::prelude::*;
use std::path::PathBuf;

use link::AvatarLink;

/// Default window size in physical pixels: the app's stage slot at scale 1. The app sends the real
/// size with `SetViewport` as soon as it connects.
pub const DEFAULT_W: u32 = 260;
pub const DEFAULT_H: u32 = 370;

/// Vertical field of view of the reference stage's camera; the framing distances assume it.
pub(crate) const FOV_DEGREES: f32 = 28.0;

fn main() {
    let args = link::Args::parse();

    // Asset root: the bundled models live next to the exe (`<exe>/assets`) in a release layout,
    // at `<workspace>/assets` or `<workspace>/aria-avatar/assets` in dev. Absolute model paths
    // bypass this entirely.
    let asset_root = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("assets")))
        .into_iter()
        .chain(std::env::current_dir().ok().map(|d| d.join("assets")))
        .chain(std::env::current_dir().ok().map(|d| d.join("aria-avatar").join("assets")))
        .find(|p| p.join(behaviors::BUNDLED_MODEL).is_file())
        .unwrap_or_else(|| PathBuf::from("assets"));

    let initial_model = if args.model.trim().is_empty() {
        behaviors::BUNDLED_MODEL.to_string()
    } else {
        args.model.clone()
    };

    let mut state = behaviors::AvatarState::with_model(initial_model);
    // Standalone only: a scripted preview run, or single-state overrides.
    let demo = (args.port == 0).then(demo::Demo::from_env).flatten();
    if args.port == 0 {
        apply_dev_overrides(&mut state);
    }

    let mut app = App::new();
    app.add_plugins(
        // No window at all: Bevy renders headless into an offscreen image, and presentation
        // happens through a plain Win32 layered window. A real Bevy window would create a swap
        // chain that can only be `Opaque`.
        DefaultPlugins
            .set(bevy::asset::AssetPlugin {
                file_path: asset_root.to_string_lossy().to_string(),
                // The user may pick a VRM from anywhere on disk (`custom_vrm_path`), which is an
                // "unapproved" absolute path by Bevy's default policy. This process renders one
                // explicitly supplied model for its own user, so allowing it is intended.
                unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow,
                ..default()
            })
            .set(bevy::log::LogPlugin {
                filter: "info,wgpu_core=warn,wgpu_hal=warn,naga=warn".into(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            // One small model at 30 FPS does not need a worker per core in every pool; idle
            // workers still cost wake-ups. Three threads keep render extraction parallel.
            .set(TaskPoolPlugin {
                task_pool_options: TaskPoolOptions::with_num_threads(3),
            })
            // Rendering is driven by `run_loop`, not by a window event loop.
            .disable::<bevy::winit::WinitPlugin>(),
    )
    .add_plugins(VrmPlugin)
    .insert_resource(ClearColor(Color::NONE))
    .insert_resource(state)
    .init_resource::<present::Presenter>()
    .init_resource::<present::LatestFrame>()
    .init_resource::<present::HitMask>()
    .init_resource::<hearts::Hearts>()
    .init_resource::<input::GestureState>()
    .init_resource::<animator::Animator>()
    .init_resource::<props::Props>()
    .insert_resource(AvatarLink::new(args))
    .set_runner(run_loop)
    .add_systems(
        Startup,
        (setup_layered, setup_camera, lighting::spawn_lights).chain(),
    )
    .add_systems(
        Update,
        (
            // Handshake first, so the app learns the window handle as early as possible.
            link::announce_ready,
            link::pump_link,
            behaviors::spawn_look_target,
            behaviors::manage_model,
            behaviors::resolve_rig,
            behaviors::forget_props,
            present::apply_viewport,
            present::apply_pause,
            lighting::apply_lighting,
            demo::drive.run_if(resource_exists::<demo::Demo>),
            behaviors::animate,
            props::sync_props,
            input::track_pointer,
            present::present_latest,
            present::request_capture,
        )
            .chain(),
    );
    if let Some(demo) = demo {
        // A fixed clock makes a scripted run repeatable and independent of how fast it renders.
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(demo::STEP_SECS),
        ))
        .insert_resource(demo);
    }
    app.run();
}

/// Standalone runs (no `--port`) can preview states without the app:
/// `ARIA_AVATAR_EMOTION=happy`, `ARIA_AVATAR_PERSONA=genki`, `ARIA_AVATAR_BODY=held`,
/// `ARIA_AVATAR_PROPS=halo,cat_ears`, `ARIA_AVATAR_FRAMING=full`, `ARIA_AVATAR_LIGHTING=sunset`,
/// `ARIA_AVATAR_SIZE=260x370`. For scripted runs that save pictures, see [`demo`].
fn apply_dev_overrides(state: &mut behaviors::AvatarState) {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    if let Some(emotion) = var("ARIA_AVATAR_EMOTION") {
        state.emote = emotes::Emote::parse(&emotion);
    }
    if let Some(persona) = var("ARIA_AVATAR_PERSONA") {
        state.style = emotes::Style::for_persona(&persona);
    }
    match var("ARIA_AVATAR_BODY").as_deref() {
        Some("air") => state.body = aria_protocol::BodyMode::Air,
        Some("held") => state.body = aria_protocol::BodyMode::Held,
        _ => {}
    }
    // `ARIA_AVATAR_PROPS=hat.glb@head@0,0.1,0` — a preview of a loaded prop without the app.
    if let Some(props) = var("ARIA_AVATAR_PROPS") {
        for entry in props.split(';').map(str::trim).filter(|e| !e.is_empty()) {
            let mut fields = entry.split('@');
            let (Some(path), Some(bone)) = (fields.next(), fields.next()) else { continue };
            let offset = fields
                .next()
                .and_then(|v| {
                    let parts: Option<Vec<f32>> = v.split(',').map(|n| n.trim().parse().ok()).collect();
                    parts.and_then(|p| (p.len() == 3).then(|| [p[0], p[1], p[2]]))
                })
                .unwrap_or([0.0; 3]);
            let scale = fields.next().and_then(|v| v.trim().parse().ok()).unwrap_or(1.0);
            state.props.push(crate::props::PropSpec {
                id: path.to_string(),
                path: path.to_string(),
                bone: bone.trim().to_string(),
                offset: Vec3::from(offset),
                rotation: Vec3::ZERO,
                scale,
                enabled: true,
            });
        }
    }
    if let Some(framing) = var("ARIA_AVATAR_FRAMING") {
        state.framing = framing;
    }
    if let Some(lighting) = var("ARIA_AVATAR_LIGHTING") {
        state.lighting = lighting;
    }
    if let Some((w, h)) = var("ARIA_AVATAR_SIZE").and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
    }) {
        state.viewport = (w, h);
    }
}

/// Drives the headless app at its target frame rate.
///
/// Without `WinitPlugin` nothing else pumps the schedule, so this replaces the window event loop.
/// Headless rendering has no vsync, which is why the pace is explicit.
fn run_loop(mut app: App) -> AppExit {
    app.finish();
    app.cleanup();

    // A scripted preview runs on its own fixed clock, faster than real time.
    let scripted = app.world().contains_resource::<demo::Demo>();
    let mut last = std::time::Instant::now();
    loop {
        app.update();
        if scripted {
            std::thread::sleep(demo::FRAME_PAUSE);
            continue;
        }

        let fps = app
            .world()
            .get_resource::<behaviors::AvatarState>()
            .map(|s| s.effective_fps())
            .unwrap_or(30);
        let target = if fps > 0 {
            std::time::Duration::from_secs_f32(1.0 / fps as f32)
        } else {
            // Paused: nothing renders; keep draining commands at a relaxed pace.
            std::time::Duration::from_millis(100)
        };
        let elapsed = last.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
        last = std::time::Instant::now();
    }
}

/// Creates the layered presenter window and the offscreen render target.
fn setup_layered(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut presenter: ResMut<present::Presenter>,
) {
    let Some(surface) = layered::LayeredSurface::spawn(DEFAULT_W, DEFAULT_H, "Aria Avatar") else {
        log::error!("could not create the avatar window for layered presentation");
        return;
    };

    // `new_target_texture` is the constructor that marks the image as a valid render target.
    let mut image = Image::new_target_texture(
        DEFAULT_W,
        DEFAULT_H,
        bevy::render::render_resource::TextureFormat::Rgba8Unorm,
        Some(bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb),
    );
    image.texture_descriptor.usage = bevy::render::render_resource::TextureUsages::COPY_SRC
        | bevy::render::render_resource::TextureUsages::COPY_DST
        | bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT
        | bevy::render::render_resource::TextureUsages::TEXTURE_BINDING;
    let handle = images.add(image);

    commands.insert_resource(present::OffscreenTarget(handle));
    presenter.set_surface(surface);
}

fn setup_camera(mut commands: Commands, target: Option<Res<present::OffscreenTarget>>) {
    let mut camera = commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: FOV_DEGREES.to_radians(),
            near: 0.1,
            far: 20.0,
            ..default()
        }),
        Transform::from_xyz(0.0, 1.15, 2.35).looking_at(Vec3::new(0.0, 1.15, 0.0), Vec3::Y),
        // Ambient light is a per-camera component in this Bevy version; `lighting` sets it.
        AmbientLight::default(),
        behaviors::AvatarCamera,
    ));
    // The camera receives its target in the same system that spawns it: a separate system would
    // not see the entity until deferred commands are applied.
    match target {
        Some(target) => {
            camera.insert(bevy::camera::RenderTarget::Image(
                bevy::camera::ImageRenderTarget {
                    handle: target.0.clone(),
                    scale_factor: 1.0,
                },
            ));
        }
        None => log::error!("no offscreen target when spawning the camera"),
    }
}
