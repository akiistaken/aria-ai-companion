//! Offscreen render target and readback presentation.
//!
//! The scene renders into an `Image` instead of the window's swap chain (which can only be
//! `Opaque`), and every captured frame is handed to the layered-window presenter.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use crate::layered::LayeredSurface;

/// The offscreen image the 3D camera renders into.
#[derive(Resource, Clone)]
pub struct OffscreenTarget(pub Handle<Image>);

/// Owns the layered window and turns captured frames into desktop pixels.
#[derive(Resource, Default)]
pub struct Presenter {
    surface: Option<LayeredSurface>,
    pub in_flight: bool,
    shown: bool,
    logged: bool,
    requests: u32,
    captures: u32,
}

impl Presenter {
    pub fn set_surface(&mut self, surface: LayeredSurface) {
        self.surface = Some(surface);
    }

    pub fn ready(&self) -> bool {
        self.surface.is_some() && !self.in_flight
    }

    /// Uploads one captured frame. Returns false when presentation is not possible.
    pub fn present(&mut self, image: &Image) -> bool {
        let Some(surface) = self.surface.as_mut() else {
            log::warn!("present: no surface");
            return false;
        };

        if !self.shown {
            surface.show();
            self.shown = true;
        }
        let Some(data) = image.data.as_deref() else {
            log::warn!(
                "present: no image data ({}x{} {:?})",
                image.width(),
                image.height(),
                image.texture_descriptor.format
            );
            return false;
        };
        if !self.logged {
            self.logged = true;
            let corner = data.get(3).copied().unwrap_or(0);
            let mid = data.get((data.len() / 2 / 4) * 4 + 3).copied().unwrap_or(0);
            log::info!(
                "first frame: {}x{} bytes={} corner_alpha={corner} centre_alpha={mid} fmt={:?}",
                image.width(),
                image.height(),
                data.len(),
                image.texture_descriptor.format
            );
        }
        surface.present_rgba(data);
        true
    }
}

/// Latest captured frame, handed from the observer to a normal system.
///
/// A Bevy observer cannot touch `Presenter` here: the capture fires during rendering, while the
/// world is mid-update, so a `ResMut` there is silently unavailable. The image is parked and a
/// regular `Update` system presents it.
#[derive(Resource, Default)]
pub struct LatestFrame(pub Option<Image>);

/// Spawns the next screenshot request when the previous one has been consumed.
pub fn request_capture(
    mut commands: Commands,
    target: Option<Res<OffscreenTarget>>,
    mut presenter: ResMut<Presenter>,
    latest: Res<LatestFrame>,
) {
    let Some(target) = target else {
        return;
    };
    if !presenter.ready() {
        return;
    }
    // Only request once the previous frame has been presented, so capture cannot outrun display.
    if latest.0.is_some() {
        return;
    }
    presenter.in_flight = true;
    presenter.requests += 1;
    if presenter.requests <= 2 || presenter.requests % 300 == 0 {
        log::info!("capture request #{}", presenter.requests);
    }
    commands
        .spawn(Screenshot::image(target.0.clone()))
        .observe(|captured: On<ScreenshotCaptured>, mut latest: ResMut<LatestFrame>| {
            latest.0 = Some(captured.image.clone());
        });
}

/// Presents the most recent capture, if any.
pub fn present_latest(mut presenter: ResMut<Presenter>, mut latest: ResMut<LatestFrame>) {
    let Some(image) = latest.0.take() else { return };
    presenter.in_flight = false;
    if presenter.captures == 0 {
        log::info!(
            "first frame {}x{} {:?}",
            image.width(),
            image.height(),
            image.texture_descriptor.format
        );
    }
    presenter.captures += 1;
    if presenter.captures <= 3 {
        let data = image.data.as_deref().unwrap_or(&[]);
        let opaque = data.chunks_exact(4).filter(|px| px[3] > 0).count();
        log::info!(
            "frame #{}: {}x{} nonzero_alpha_pixels={} of {}",
            presenter.captures,
            image.width(),
            image.height(),
            opaque,
            data.len() / 4
        );
    }
    presenter.present(&image);
}
