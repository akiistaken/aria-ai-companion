//! Scripted preview runs, for checking animation without the app or a hand on the mouse.
//!
//! ```text
//! ARIA_AVATAR_SHOTS=<folder> [ARIA_AVATAR_SCRIPT="happy@ground@0,0@2@4; idle@ground@160,0@2@6"] aria-avatar.exe
//! ```
//!
//! Each `;`-separated step is `emotion@mode@vx,vy@seconds@shots[@persona[@bpm@dbfs]]`: act out `emotion` in
//! body `mode` (`ground`, `air`, `held`) while the window travels at `vx,vy` pixels per second, for
//! `seconds`, saving `shots` evenly spaced frames. Every step becomes one contact-sheet PNG in the
//! folder, with the floor drawn as a line. The run shows no window and uses a fixed 30 FPS clock,
//! played a few times faster than real time, so the same script always animates the same way.

use std::path::PathBuf;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use aria_protocol::BodyMode;

use crate::animator::Animator;
use crate::behaviors::AvatarState;
use crate::emotes::{Emote, Style};

/// Fixed time step of a scripted run.
pub const STEP_SECS: f32 = 1.0 / 30.0;
/// Frames shown before the script starts, so the model and its spring bones have settled.
const WARM_UP_FRAMES: u32 = 45;
/// Real time given to each frame. Captures come back from the GPU a frame or two late, so the
/// run cannot simply spin; this is still about three times faster than real time.
pub const FRAME_PAUSE: std::time::Duration = std::time::Duration::from_millis(11);

/// A tour of everything, used when no script is given.
const DEFAULT_SCRIPT: &str = "idle@ground@0,0@1.5@2; talking@ground@0,0@2@3; happy@ground@0,0@2@4; \
    laugh@ground@0,0@2@4; angry@ground@0,0@2.4@4; annoyed@ground@0,0@2@3; sad@ground@0,0@2@3; \
    cry@ground@0,0@2@3; thinking@ground@0,0@2@3; surprised@ground@0,0@1.2@4; shy@ground@0,0@2@3; \
    dizzy@ground@0,0@2@4; sleepy@ground@0,0@9@4; dance@ground@0,0@6@6; groove@ground@0,0@2@3; \
    sit@ground@0,0@2.5@4; wave@ground@0,0@1.5@3; idle@ground@150,0@3@8; idle@ground@-150,0@2@6; \
    idle@ground@0,0@1.5@4; idle@air@0,500@1.2@4; idle@ground@0,0@1@5; idle@held@120,-40@1.5@4";

#[derive(Clone, Debug)]
struct Step {
    emotion: String,
    mode: BodyMode,
    velocity: Vec2,
    seconds: f32,
    shots: u32,
    persona: String,
    music: Option<(f32, f32)>,
}

impl Step {
    fn parse(text: &str) -> Option<Step> {
        let mut fields = text.trim().split('@').map(str::trim);
        let emotion = fields.next().filter(|e| !e.is_empty())?.to_string();
        let mode = match fields.next()? {
            "air" => BodyMode::Air,
            "held" => BodyMode::Held,
            _ => BodyMode::Ground,
        };
        let (vx, vy) = fields.next()?.split_once(',')?;
        Some(Step {
            emotion,
            mode,
            velocity: Vec2::new(vx.trim().parse().ok()?, vy.trim().parse().ok()?),
            seconds: fields.next()?.parse().ok()?,
            shots: fields.next()?.parse().ok()?,
            persona: fields.next().unwrap_or("").to_string(),
            music: match fields.next() {
                Some(bpm) => {
                    let bpm: f32 = bpm.parse().ok()?;
                    let dbfs: f32 = fields.next()?.parse().ok()?;
                    if !bpm.is_finite() || bpm <= 0.0 || !dbfs.is_finite() { return None; }
                    Some((bpm, ((dbfs + 48.0) / 42.0).clamp(0.0, 1.0)))
                }
                None => None,
            },
        })
    }
}

/// The scripted run in progress.
#[derive(Resource)]
pub struct Demo {
    folder: PathBuf,
    steps: Vec<Step>,
    index: usize,
    /// Frames into the current step.
    frame: u32,
    warm_up: u32,
    /// A frame with the model in it has come back: she is loaded and the shaders are compiled.
    rendering: bool,
    window: Vec2,
    /// A shot of the current step is due: the next presented frame is kept.
    shot_due: bool,
    shots: Vec<Vec<u8>>,
    size: (u32, u32),
    /// Pixels from the bottom of the frame to the floor, for the guide line.
    floor: f32,
}

impl Demo {
    /// Reads the run from the environment; `None` for a normal start.
    pub fn from_env() -> Option<Demo> {
        let folder = PathBuf::from(std::env::var_os("ARIA_AVATAR_SHOTS").filter(|v| !v.is_empty())?);
        let script = std::env::var("ARIA_AVATAR_SCRIPT")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SCRIPT.to_string());
        let steps: Vec<Step> = script
            .split(';')
            .filter(|s| !s.trim().is_empty())
            .filter_map(|text| {
                let step = Step::parse(text);
                if step.is_none() {
                    log::error!("demo: cannot parse step {text:?}");
                }
                step
            })
            .collect();
        if let Err(e) = std::fs::create_dir_all(&folder) {
            log::error!("demo: cannot create {}: {e}", folder.display());
            return None;
        }
        log::info!("demo: {} steps into {}", steps.len(), folder.display());
        Some(Demo {
            folder,
            steps,
            index: 0,
            frame: 0,
            warm_up: 0,
            rendering: false,
            // Away from the origin, as a real window would be.
            window: Vec2::new(800.0, 600.0),
            shot_due: false,
            shots: Vec::new(),
            size: (0, 0),
            floor: 0.0,
        })
    }

    /// Keeps a presented frame if a shot is due.
    pub fn offer(&mut self, rgba: &[u8], width: u32, height: u32) {
        // The first frames are empty while textures load and pipelines compile.
        self.rendering = self.rendering || rgba.as_chunks::<4>().0.iter().step_by(7).any(|px| px[3] > 0);
        if !self.shot_due {
            return;
        }
        self.shot_due = false;
        self.size = (width, height);
        self.shots.push(rgba.to_vec());
    }

    /// Writes the current step's shots side by side over a neutral backdrop.
    fn save_sheet(&mut self) {
        let Some(step) = self.steps.get(self.index) else { return };
        let (w, h) = self.size;
        let shots = std::mem::take(&mut self.shots);
        if shots.is_empty() || w == 0 || h == 0 {
            return;
        }
        let sheet_w = w * shots.len() as u32;
        let floor_row = h.saturating_sub(self.floor.round() as u32).min(h - 1);
        let mut data = vec![0u8; (sheet_w * h * 4) as usize];
        for (index, shot) in shots.iter().enumerate() {
            for y in 0..h {
                for x in 0..w {
                    let src = ((y * w + x) * 4) as usize;
                    let dst = ((y * sheet_w + index as u32 * w + x) * 4) as usize;
                    let alpha = shot[src + 3] as u32;
                    // Floor line, frame divider, then a flat backdrop.
                    let backdrop: [u32; 3] = if y == floor_row {
                        [70, 200, 120]
                    } else if x == 0 {
                        [90, 96, 110]
                    } else {
                        [44, 48, 58]
                    };
                    for c in 0..3 {
                        let colour = shot[src + c] as u32;
                        data[dst + c] = ((colour * alpha + backdrop[c] * (255 - alpha)) / 255) as u8;
                    }
                    data[dst + 3] = 255;
                }
            }
        }
        let name = format!(
            "{:02}_{}_{}{}.png",
            self.index,
            step.emotion,
            match step.mode {
                BodyMode::Ground => "ground",
                BodyMode::Air => "air",
                BodyMode::Held => "held",
            },
            if step.velocity == Vec2::ZERO { "" } else { "_moving" },
        );
        let path = self.folder.join(name);
        let image = Image::new(
            Extent3d { width: sheet_w, height: h, depth_or_array_layers: 1 },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        match image.try_into_dynamic() {
            Ok(picture) => match picture.save(&path) {
                Ok(()) => log::info!("demo: wrote {}", path.display()),
                Err(e) => log::error!("demo: cannot write {}: {e}", path.display()),
            },
            Err(e) => log::error!("demo: cannot convert the sheet: {e}"),
        }
    }
}

/// Plays the script: sets the state the app would have sent and moves the pretend window.
pub fn drive(mut demo: ResMut<Demo>, mut state: ResMut<AvatarState>, mut animator: ResMut<Animator>) {
    animator.simulated_window = Some(demo.window.round().as_ivec2());
    if state.rig.is_none() || !demo.rendering {
        return;
    }
    if demo.warm_up < WARM_UP_FRAMES {
        demo.warm_up += 1;
        return;
    }
    let Some(step) = demo.steps.get(demo.index).cloned() else {
        log::info!("demo: finished");
        std::process::exit(0);
    };
    state.emote = Emote::parse(&step.emotion);
    state.body = step.mode;
    state.style = Style::for_persona(&step.persona);
    state.music_active = step.music.is_some();
    if let Some((bpm, energy)) = step.music {
        state.music_energy = energy;
        state.music_sync = Some(crate::animator::BeatSync {
            bpm, phase: (demo.frame as f32 * STEP_SECS * bpm / 60.0).fract(),
        });
    }
    demo.window += step.velocity * STEP_SECS;
    animator.simulated_window = Some(demo.window.round().as_ivec2());

    demo.frame += 1;
    let frames = (step.seconds / STEP_SECS).round().max(1.0) as u32;
    // Shots are spread evenly, the last one on the step's final frame.
    let every = (frames / step.shots.max(1)).max(1);
    if step.shots > 0 && demo.frame.is_multiple_of(every) && demo.shots.len() < step.shots as usize {
        demo.shot_due = true;
    }
    if demo.frame >= frames + 3 {
        // Three frames of slack: a capture lands a frame or two after it is requested.
        demo.floor = state.floor_offset();
        demo.save_sheet();
        demo.index += 1;
        demo.frame = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_parse_and_reject_garbage() {
        let step = Step::parse(" happy@air@-120, 40@1.5@3@genki ").expect("parses");
        assert_eq!(step.emotion, "happy");
        assert_eq!(step.mode, BodyMode::Air);
        assert_eq!(step.velocity, Vec2::new(-120.0, 40.0));
        assert_eq!((step.seconds, step.shots), (1.5, 3));
        assert_eq!(step.persona, "genki");
        assert!(step.music.is_none());
        assert_eq!(Step::parse("idle@ground@0,0@8@4@@65@-27").unwrap().music, Some((65.0, 0.5)));
        assert!(Step::parse("idle@ground@0,0@8@4@@NaN@-27").is_none());
        assert!(Step::parse("happy@ground@0@1@1").is_none(), "velocity needs two numbers");
        assert!(Step::parse("").is_none());
        for text in DEFAULT_SCRIPT.split(';') {
            assert!(Step::parse(text).is_some(), "default script step {text:?}");
        }
    }
}
