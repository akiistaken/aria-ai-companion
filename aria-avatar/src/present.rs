//! Offscreen render target, readback presentation, viewport size and pause.
//!
//! The scene renders into an `Image` instead of the window's swap chain (which can only be
//! `Opaque`), and every captured frame is handed to the layered-window presenter. The captured
//! frame's alpha channel doubles as the hit mask for click-through: a pixel the model covers is
//! exactly a pixel with alpha, so hit-testing is pixel-accurate for any model, prop or pose.

use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::Extent3d;
use bevy::render::renderer::RenderDevice;

use crate::behaviors::{AvatarCamera, AvatarState};
use crate::hearts::Hearts;
use crate::layered::LayeredSurface;

/// The offscreen image the 3D camera renders into.
#[derive(Resource, Clone)]
pub struct OffscreenTarget(pub Handle<Image>);

/// Owns the layered window and turns captured frames into desktop pixels.
#[derive(Resource, Default)]
pub struct Presenter {
    surface: Option<LayeredSurface>,
    capture: Option<(Entity, u32, u32)>,
    paused: bool,
    logged: bool,
    pub captures: u64,
}

impl Presenter {
    pub fn set_surface(&mut self, surface: LayeredSurface) {
        self.surface = Some(surface);
    }
}

/// Alpha of the last presented frame, for hit-testing the cursor against the model.
#[derive(Resource, Default)]
pub struct HitMask {
    pub width: u32,
    pub height: u32,
    alpha: Vec<u8>,
}

impl HitMask {
    /// True when any pixel within `radius` of `(x, y)` is part of the model.
    pub fn hit(&self, x: f32, y: f32, radius: i32) -> bool {
        if self.alpha.is_empty() {
            return false;
        }
        let (cx, cy) = (x.round() as i32, y.round() as i32);
        if cx < -radius
            || cy < -radius
            || cx >= self.width as i32 + radius
            || cy >= self.height as i32 + radius
        {
            return false;
        }
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let (px, py) = (cx + dx, cy + dy);
                if px < 0 || py < 0 || px >= self.width as i32 || py >= self.height as i32 {
                    continue;
                }
                if self.alpha[(py as u32 * self.width + px as u32) as usize] > 24 {
                    return true;
                }
            }
        }
        false
    }

    fn update(&mut self, rgba: &[u8], width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.alpha.clear();
        self.alpha.extend(rgba.as_chunks::<4>().0.iter().map(|px| px[3]));
    }
}

/// Latest captured frame, handed from the observer to a normal system.
///
/// A Bevy observer cannot touch `Presenter` here: the capture fires during rendering, while the
/// world is mid-update, so a `ResMut` there is silently unavailable. The image is parked and a
/// regular `Update` system presents it.
#[derive(Resource, Default)]
pub struct LatestFrame(pub Option<CapturedFrame>);

pub struct CapturedFrame {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

/// Packs a mapped GPU texture, whose rows are padded to 256 bytes, into RGBA8.
fn pack_rgba(data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let row = width as usize * 4;
    let stride = RenderDevice::align_copy_bytes_per_row(row);
    if row == 0 || height == 0 || data.len() < stride * height as usize {
        return None;
    }
    let mut rgba = Vec::with_capacity(row * height as usize);
    for pixels in data.chunks_exact(stride).take(height as usize) {
        rgba.extend_from_slice(&pixels[..row]);
    }
    Some(rgba)
}

/// Keeps one readback entity for the current viewport, using Bevy's pooled staging buffers.
/// A frame is copied directly from the target after rendering, with no screenshot texture/blit
/// pass or per-frame entities. Both handoff slots keep only the latest completed frame.
pub fn request_capture(
    mut commands: Commands,
    target: Option<Res<OffscreenTarget>>,
    mut presenter: ResMut<Presenter>,
    images: Res<Assets<Image>>,
) {
    let Some(target) = target else {
        return;
    };
    if presenter.paused || presenter.surface.is_none() {
        if let Some((entity, _, _)) = presenter.capture.take() {
            commands.entity(entity).despawn();
        }
        return;
    }
    let Some(image) = images.get(&target.0) else { return };
    let (width, height) = (image.width(), image.height());
    if presenter.capture.is_some_and(|(_, w, h)| w == width && h == height) {
        return;
    }
    if let Some((entity, _, _)) = presenter.capture.take() {
        commands.entity(entity).despawn();
    }
    let entity = commands
        .spawn(Readback::texture(target.0.clone()))
        .observe(move |captured: On<ReadbackComplete>, mut latest: ResMut<LatestFrame>| {
            if let Some(rgba) = pack_rgba(&captured.data, width, height) {
                latest.0 = Some(CapturedFrame { rgba, width, height });
            }
        })
        .id();
    presenter.capture = Some((entity, width, height));
}

/// Presents the most recent capture, painting the head-pat hearts over it first.
pub fn present_latest(
    time: Res<Time>,
    mut presenter: ResMut<Presenter>,
    mut latest: ResMut<LatestFrame>,
    mut hit: ResMut<HitMask>,
    mut hearts: ResMut<Hearts>,
    demo: Option<ResMut<crate::demo::Demo>>,
    state: Res<AvatarState>,
) {
    hearts.step(time.delta_secs().min(0.1));
    let Some(CapturedFrame { mut rgba, width, height }) = latest.0.take() else { return };
    // A resized target may still have an old capture on the GPU. Never resize the window back
    // to the old bitmap, and do not replace the hit mask with obsolete geometry.
    if state.viewport != (width, height) { return; }
    if !presenter.logged {
        presenter.logged = true;
        let covered = rgba.as_chunks::<4>().0.iter().filter(|px| px[3] > 0).count();
        log::info!(
            "first frame {width}x{height} RGBA8, {covered} of {} pixels covered",
            width * height
        );
    }
    presenter.captures += 1;
    // The mask is the model only; hearts are decoration and must not catch clicks.
    hit.update(&rgba, width, height);
    if !hearts.is_empty() {
        hearts.paint(&mut rgba, width, height);
    }
    if let Some(mut demo) = demo {
        // A scripted preview keeps its frames as pictures and never shows a window.
        demo.offer(&rgba, width, height);
        return;
    }
    if presenter.paused {
        return;
    }
    if let Some(surface) = presenter.surface.as_mut() {
        surface.present_rgba(rgba, width, height);
    }
}

/// Resizes the render target to the window size the app asked for.
pub fn apply_viewport(
    state: Res<AvatarState>,
    target: Option<Res<OffscreenTarget>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(target) = target else { return };
    let (w, h) = state.viewport;
    let Some(image) = images.get(&target.0) else { return };
    if image.width() == w && image.height() == h {
        return;
    }
    if let Some(mut image) = images.get_mut(&target.0) {
        log::info!("viewport {}x{} -> {w}x{h}", image.width(), image.height());
        image.resize(Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        });
    }
}

/// `SetFps 0` pauses: the camera stops rendering and the window hides. Anything else resumes.
pub fn apply_pause(
    state: Res<AvatarState>,
    mut presenter: ResMut<Presenter>,
    mut cameras: Query<&mut Camera, With<AvatarCamera>>,
) {
    let paused = state.fps == 0;
    if presenter.paused == paused {
        return;
    }
    presenter.paused = paused;
    log::info!("rendering {}", if paused { "paused" } else { "resumed" });
    for mut camera in cameras.iter_mut() {
        camera.is_active = !paused;
    }
    if let Some(surface) = presenter.surface.as_mut() {
        if paused {
            surface.hide();
        } else {
            surface.show();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{pack_rgba, HitMask};

    #[test]
    fn readback_strips_padding_without_changing_color_or_alpha() {
        for width in [1, 64, 65, 260] {
            let row = width as usize * 4;
            let stride = bevy::render::renderer::RenderDevice::align_copy_bytes_per_row(row);
            let pixels: Vec<u8> = (0..row * 3).map(|i| (i % 251) as u8).collect();
            let mut padded = vec![255; stride * 3];
            for y in 0..3 {
                padded[y * stride..y * stride + row].copy_from_slice(&pixels[y * row..(y + 1) * row]);
            }
            assert_eq!(pack_rgba(&padded, width, 3), Some(pixels));
            assert!(pack_rgba(&padded[..padded.len() - 1], width, 3).is_none());
        }
        assert!(pack_rgba(&[], 0, 3).is_none());
        assert!(pack_rgba(&[], 3, 0).is_none());
    }

    #[test]
    fn hit_mask_finds_covered_pixels_with_a_margin() {
        let (w, h) = (10u32, 10u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        // One opaque pixel at (5, 5).
        rgba[((5 * w + 5) * 4 + 3) as usize] = 255;
        let mut mask = HitMask::default();
        mask.update(&rgba, w, h);
        assert!(mask.hit(5.0, 5.0, 0));
        assert!(!mask.hit(8.0, 8.0, 0));
        assert!(mask.hit(7.0, 5.0, 2));
        assert!(!mask.hit(-50.0, 5.0, 2));
    }
}
