//! Aria avatar renderer.
//!
//! A Bevy app in its own transparent, frameless, always-on-top window. It loads a VRM 1.0 model
//! with `bevy_vrm1` (MToon shading, spring bones, look-at, node constraints) and is driven by the
//! main app over a local socket. The main app owns the window position: it moves this process's
//! HWND with `SetWindowPos`, so nothing here handles desktop geometry.

#[path = "behaviors.rs"]
mod behaviors;
#[path = "layered.rs"]
mod layered;
#[path = "link.rs"]
mod link;
#[path = "present.rs"]
mod present;

use bevy::prelude::*;
use bevy::window::{CompositeAlphaMode, WindowLevel};
use bevy_vrm1::prelude::*;
use std::path::PathBuf;

use link::AvatarLink;

/// Window size in physical pixels. The camera framing below is tuned for this aspect.
const WINDOW_W: f32 = 520.0;
const WINDOW_H: f32 = 740.0;

fn main() {
    let args = link::Args::parse();

    // Asset root: the bundled models live next to the exe (`<exe>/assets`) in a release layout
    // and at `<workspace>/aria-avatar/assets` in dev. Absolute model paths bypass this entirely.
    let asset_root = std::path::Path::new(&args.model)
        .parent()
        .map(|p| p.to_path_buf())
        .filter(|p| p.is_dir())
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|p| p.join("assets")))
                .filter(|p| p.is_dir())
        })
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.join("aria-avatar").join("assets"))
                .filter(|p| p.is_dir())
        })
        .unwrap_or_else(|| PathBuf::from("assets"));

    let mut app = App::new();
    app.add_plugins(
        // No `WindowPlugin` window at all: Bevy renders headless into an offscreen image, and the
        // presentation happens through a plain Win32 layered window. Adding a real Bevy window
        // would create a swap chain that can only be `Opaque` and would block on present when
        // nothing draws into it.
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
            // Rendering is driven by the app loop, not by a window event loop.
            .disable::<bevy::winit::WinitPlugin>(),
    )
    .add_plugins(VrmPlugin)
    .insert_resource(ClearColor(Color::NONE))
    .insert_resource(behaviors::AvatarState {
        initial_model: args.model.clone(),
        ..default()
    })
    .init_resource::<present::Presenter>()
    .init_resource::<present::LatestFrame>()
    .insert_resource(AvatarLink::new(args))
    .set_runner(run_loop)
    .add_systems(
        Startup,
        (setup_layered, setup_camera, setup_lights).chain(),
    )
    .add_systems(
        Update,
        (
            // Handshake first, so the app learns the window handle as early as possible.
            link::announce_ready,
            link::pump_link,
            behaviors::animate,
            present::present_latest,
            present::request_capture,
        )
            .chain(),
    )
    .run();
}

/// Drives the headless app at its target frame rate.
///
/// Without `WinitPlugin` nothing else pumps the schedule, so this replaces the window event loop.
/// Headless rendering has no vsync, which is why the pace is explicit.
fn run_loop(mut app: App) -> AppExit {
    app.finish();
    app.cleanup();


    let mut last = std::time::Instant::now();
    loop {
        app.update();

        let fps = app
            .world()
            .get_resource::<behaviors::AvatarState>()
            .map(|s| s.fps)
            .unwrap_or(30);
        if fps > 0 {
            let target = std::time::Duration::from_secs_f32(1.0 / fps as f32);
            let elapsed = last.elapsed();
            if elapsed < target {
                std::thread::sleep(target - elapsed);
            }
            last = std::time::Instant::now();
        } else {
            // Paused: keep processing commands without rendering work.
            std::thread::sleep(std::time::Duration::from_millis(100));
            last = std::time::Instant::now();
        }
    }
}

/// Turns the winit-created window into the layered presenter and starts offscreen rendering.
///
/// Runs on the first frame, by which point winit has created the native window.
fn setup_layered(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut presenter: ResMut<present::Presenter>,
) {
    log::info!("setup_layered: creating the avatar window");
    let Some(surface) = layered::LayeredSurface::spawn(WINDOW_W as u32, WINDOW_H as u32, "Aria Avatar")
    else {
        log::error!("could not create the avatar window for layered presentation");
        return;
    };
    log::info!("layered surface created");

    // The window can only be `Opaque`, so the scene renders into an image and is presented by the
    // layered window instead of by the swap chain. `new_target_texture` is the constructor that
    // marks the image as a valid render target.
    let mut image = Image::new_target_texture(
        WINDOW_W as u32,
        WINDOW_H as u32,
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
    // Framing from the reference `applyVRMCameraFraming()`; the height-dependent numbers are
    // applied once the model reports its metrics (see behaviors::frame_camera).
    let mut camera = commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 1.2, 2.8).looking_at(Vec3::new(0.0, 1.2, 0.0), Vec3::Y),
        // Ambient light is a per-camera component in this Bevy version.
        AmbientLight {
            color: Color::srgb(0.85, 0.88, 0.95),
            brightness: 220.0,
            ..default()
        },
        behaviors::AvatarCamera,
    ));
    // Rendering into an image is what lets the layered window composite the avatar with real
    // per-pixel alpha; the window's own swap chain can only be opaque.
    if let Some(target) = target {
        log::info!("camera rendering into the offscreen target");
        camera.insert(bevy::camera::RenderTarget::Image(
            bevy::camera::ImageRenderTarget {
                handle: target.0.clone(),
                scale_factor: 1.0,
            },
        ));
    } else {
        log::error!("no offscreen target when spawning the camera");
    }
}

fn setup_lights(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(2.5, 4.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
