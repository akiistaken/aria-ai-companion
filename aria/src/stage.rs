//! Avatar stage: a WebView2 child window parked in the Slint layout slot, driven by JS calls.
//!
//! Slint has no WebGL surface, and the Live2D / VRM renderers need one, so the character keeps
//! its existing renderer and Slint owns everything around it.

use serde::Serialize;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

pub struct Stage {
    webview: Option<wry::WebView>,
    ready: bool,
    last_emotion: String,
    last_facing_right: bool,
    last_moving: bool,
    last_talking: bool,
    last_peeking: String,
    last_scale: f32,
}

pub type StageHandle = Rc<RefCell<Stage>>;

pub fn new_handle() -> StageHandle {
    Rc::new(RefCell::new(Stage {
        webview: None,
        ready: false,
        last_emotion: String::new(),
        last_facing_right: false,
        last_moving: false,
        last_talking: false,
        last_peeking: String::new(),
        last_scale: 0.0,
    }))
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct StageRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl StageRect {
    pub fn to_wry(self, scale: f32) -> wry::Rect {
        wry::Rect {
            position: wry::dpi::PhysicalPosition::new(
                (self.x * scale).round() as i32,
                (self.y * scale).round() as i32,
            )
            .into(),
            size: wry::dpi::PhysicalSize::new(
                (self.width * scale).max(1.0).round() as i32,
                (self.height * scale).max(1.0).round() as i32,
            )
            .into(),
        }
    }
}

fn escape_js(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', " ")
}

impl Stage {
    /// Builds the child WebView inside the Slint window and loads the stage page.
    pub fn attach<W: wry::raw_window_handle::HasWindowHandle>(
        &mut self,
        window: &W,
        rect: StageRect,
        scale: f32,
        on_event: Box<dyn Fn(String)>,
    ) {
        if self.webview.is_some() {
            return;
        }
        let base = crate::assets::start().to_string();
        if base.is_empty() {
            log::error!("stage unavailable: no asset server");
            return;
        }

        let model_url = resolve_model_url(&base);
        let mode = crate::config::get().avatar_mode;
        let settings = crate::config::get();
        let stage_url = format!(
            "{base}/assets/vtuber_stage.html?mode={mode}&url={}&framing={}&zoom={:.3}&yoff={:.3}",
            urlencode(&model_url),
            settings.body_framing,
            settings.camera_zoom,
            settings.camera_y_offset
        );

        let script = include_str!("stage_bridge.js");
        let builder = wry::WebViewBuilder::new()
            .with_transparent(true)
            .with_devtools(false)
            .with_autoplay(true)
            .with_url(&stage_url)
            .with_bounds(rect.to_wry(scale))
            .with_initialization_script(script)
            .with_ipc_handler(move |request| {
                on_event(request.body().to_string());
            });

        let builder = builder
            .with_background_color((0, 0, 0, 0))
            .with_initialization_script(
                "window.addEventListener('error',function(e){\
                   window.ipc&&window.ipc.postMessage(JSON.stringify({kind:'js_error',\
                   data:{msg:String(e.message),src:String(e.filename),line:e.lineno}}));});",
            );
        match builder.build_as_child(window) {
            Ok(webview) => {
                log::info!("avatar stage attached ({mode})");
                self.webview = Some(webview);
                self.ready = true;
                self.last_scale = scale;
                self.apply_settings();
            }
            Err(e) => log::error!("avatar stage failed: {e}"),
        }
    }

    pub fn set_bounds(&mut self, rect: StageRect, scale: f32) {
        if let Some(webview) = &self.webview {
            if (scale - self.last_scale).abs() > 0.01 {
                let _ = webview.set_bounds(rect.to_wry(scale));
                self.last_scale = scale;
            }
        }
    }

    fn run_js(&self, js: &str) {
        if let Some(webview) = &self.webview {
            if let Err(e) = webview.evaluate_script(js) {
                log::debug!("stage js failed: {e}");
            }
        }
    }

    pub fn set_state(&mut self, emotion: &str, facing_right: bool, moving: bool, talking: bool) {
        if self.last_emotion == emotion
            && self.last_facing_right == facing_right
            && self.last_moving == moving
            && self.last_talking == talking
        {
            return;
        }
        self.last_emotion = emotion.to_string();
        self.last_facing_right = facing_right;
        self.last_moving = moving;
        self.last_talking = talking;
        let js = format!(
            "window.setAvatarState && window.setAvatarState('{}', {}, {}, {});",
            escape_js(emotion),
            facing_right,
            moving,
            talking
        );
        self.run_js(&js);
    }

    pub fn set_mouse_look(&mut self, nx: f32, ny: f32) {
        let js = format!(
            "window.setMouseLook && window.setMouseLook({nx:.3}, {ny:.3});"
        );
        self.run_js(&js);
    }

    pub fn set_peeking(&mut self, side: &str) {
        if self.last_peeking == side {
            return;
        }
        self.last_peeking = side.to_string();
        let js = format!(
            "window.setPeekingMode && window.setPeekingMode('{}');",
            escape_js(side)
        );
        self.run_js(&js);
    }

    pub fn set_music_vibe(&mut self, active: bool, energy: f32) {
        let js = format!(
            "window.setMusicVibe && window.setMusicVibe({active}, {energy:.2});"
        );
        self.run_js(&js);
    }

    pub fn trigger_headpat(&mut self) {
        self.run_js("window.triggerHeadpat && window.triggerHeadpat();");
    }

    pub fn set_drag_inertia(&mut self, vx: f32, vy: f32) {
        let js = format!("window.setDragInertia && window.setDragInertia({vx:.3}, {vy:.3});");
        self.run_js(&js);
    }

    #[allow(dead_code)] // TODO(wire-up): the avatar process renders audio state itself.
    pub fn play_audio(&mut self, url: &str) {
        let js = format!(
            "if (window.playVoiceAudio) {{ window.playVoiceAudio('{}', false); }} else {{ window.__pendingAudioUrl = '{}'; }}",
            escape_js(url),
            escape_js(url)
        );
        self.run_js(&js);
    }

    pub fn stop_audio(&mut self) {
        self.run_js("window.__pendingAudioUrl = null; window.stopVoiceAudio && window.stopVoiceAudio();");
    }

    #[allow(dead_code)] // TODO(wire-up): audio and render pausing move to the avatar process.
    pub fn set_render_paused(&mut self, paused: bool) {
        let js = format!("window.setRenderPaused && window.setRenderPaused({paused});");
        self.run_js(&js);
    }

    pub fn switch_mode(&mut self, mode: &str, url: &str) {
        let js = format!(
            "window.setAvatarMode && window.setAvatarMode('{}', '{}');",
            escape_js(mode),
            escape_js(url)
        );
        self.run_js(&js);
        self.apply_settings();
    }

    /// Pushes persona-independent settings (props, lighting, spectrum) to the stage.
    pub fn apply_settings(&mut self) {
        let settings = crate::config::get();
        let props = serde_json::Value::Object(settings.avatar_props.clone());
        self.run_js(&format!(
            "window.setAvatarProps && window.setAvatarProps({});",
            props
        ));
        self.run_js(&format!(
            "window.setStageLightingPreset && window.setStageLightingPreset('{}');",
            escape_js(&settings.stage_lighting)
        ));
        self.run_js(&format!(
            "window.setMusicSpectrumEnabled && window.setMusicSpectrumEnabled({});",
            settings.music_spectrum_enabled
        ));
        self.run_js(&format!(
            "window.setCameraFraming && window.setCameraFraming('{}', {:.3}, {:.3});",
            escape_js(&settings.body_framing),
            settings.camera_zoom,
            settings.camera_y_offset
        ));
    }

    pub fn shutdown(&mut self) {
        self.ready = false;
        self.webview = None;
    }
}

fn urlencode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Resolves the settings' avatar source into a URL the stage page can load.
pub fn resolve_model_url(base: &str) -> String {
    let settings = crate::config::get();
    let (source, mode) = match settings.avatar_mode.as_str() {
        "vrm" => (settings.custom_vrm_path.clone(), "vrm"),
        "puppet" => (settings.puppet_source.clone(), "puppet"),
        _ => (settings.live2d_source.clone(), "live2d"),
    };

    if source.starts_with("http://") || source.starts_with("https://") {
        return source;
    }
    if source.starts_with("/assets/") {
        return format!("{base}{}", urlencode(&source));
    }
    if !source.trim().is_empty() && Path::new(&source).exists() {
        return crate::assets::register_custom(Path::new(&source));
    }
    let fallback = match mode {
        "vrm" => "/assets/models/aria_default.vrm".to_string(),
        "puppet" => "/assets/models/silver_witch.png".to_string(),
        _ => settings.live2d_source,
    };
    if fallback.starts_with("/assets/") {
        format!("{base}{}", urlencode(&fallback))
    } else {
        format!("{base}/assets/models/aria_default.vrm")
    }
}
