//! Stage lighting presets, ported from `updateStageLightingPreset()` in the reference.
//!
//! Three directional lights (key, cool rim from behind, warm fill from below) plus the camera's
//! ambient term. `auto` follows the local clock: dawn 05-09, day 09-17, sunset 17-20, night after.
//! three.js intensities are unitless; they are scaled here so the reference's `day` key light
//! matches the 12 000 lx the stage was originally tuned with.

use bevy::prelude::*;

use crate::behaviors::{AvatarCamera, AvatarState};

/// three.js intensity 1.0 in Bevy lux / ambient brightness.
const LUX_PER_UNIT: f32 = 7_500.0;
const AMBIENT_PER_UNIT: f32 = 160.0;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum StageLight {
    Key,
    Rim,
    Fill,
}

struct Preset {
    ambient: (u32, f32),
    key: (u32, f32, [f32; 3]),
    rim: (u32, f32),
    fill: (u32, f32),
}

fn preset(name: &str) -> Preset {
    match name {
        "dawn" => Preset {
            ambient: (0xffe4e6, 1.20),
            key: (0xfef08a, 1.45, [2.0, 1.4, 2.0]),
            rim: (0x38bdf8, 0.8),
            fill: (0xf472b6, 0.45),
        },
        "sunset" => Preset {
            ambient: (0xfce7f3, 1.10),
            key: (0xf97316, 1.85, [-1.8, 1.2, 1.8]),
            rim: (0xa855f7, 1.25),
            fill: (0xec4899, 0.55),
        },
        "night" => Preset {
            ambient: (0x1e1b4b, 0.90),
            key: (0xfef08a, 1.35, [1.4, 1.8, 1.5]),
            rim: (0x00f0ff, 1.45),
            fill: (0xec4899, 0.65),
        },
        _ => Preset {
            ambient: (0xffffff, 1.35),
            key: (0xfff7ed, 1.6, [1.2, 2.2, 2.5]),
            rim: (0x38bdf8, 0.85),
            fill: (0xf472b6, 0.35),
        },
    }
}

fn hex(rgb: u32) -> Color {
    Color::srgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Resolves `auto` against the hour.
pub fn resolve(preset: &str, hour: u32) -> &'static str {
    match preset {
        "day" => "day",
        "dawn" => "dawn",
        "sunset" => "sunset",
        "night" => "night",
        _ => match hour {
            5..=8 => "dawn",
            9..=16 => "day",
            17..=19 => "sunset",
            _ => "night",
        },
    }
}

/// The local hour from the OS clock and time zone, without a date crate.
pub fn local_hour() -> u32 {
    let utc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let offset_secs = {
        #[cfg(windows)]
        {
            use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
            const TIME_ZONE_ID_DAYLIGHT: u32 = 2;
            let mut info = TIME_ZONE_INFORMATION::default();
            let id = unsafe { GetTimeZoneInformation(&mut info) };
            let west = if id == TIME_ZONE_ID_DAYLIGHT {
                info.Bias + info.DaylightBias
            } else {
                info.Bias
            };
            -(west as i64) * 60
        }
        #[cfg(not(windows))]
        {
            0
        }
    };
    ((utc + offset_secs).rem_euclid(86_400) / 3600) as u32
}

pub fn spawn_lights(mut commands: Commands) {
    for light in [StageLight::Key, StageLight::Rim, StageLight::Fill] {
        commands.spawn((
            DirectionalLight {
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::default(),
            light,
        ));
    }
}

/// Re-applies the preset when it changes, and once a minute so `auto` follows the clock.
pub fn apply_lighting(
    time: Res<Time>,
    state: Res<AvatarState>,
    mut applied: Local<Option<&'static str>>,
    mut setting: Local<String>,
    mut since_check: Local<f32>,
    mut lights: Query<(&StageLight, &mut DirectionalLight, &mut Transform)>,
    mut ambient: Query<&mut AmbientLight, With<AvatarCamera>>,
) {
    *since_check += time.delta_secs();
    if applied.is_some() && *setting == state.lighting && *since_check < 60.0 {
        return;
    }
    *since_check = 0.0;
    setting.clone_from(&state.lighting);
    let mode = resolve(&state.lighting, local_hour());
    if *applied == Some(mode) {
        return;
    }
    *applied = Some(mode);
    let p = preset(mode);
    log::info!("stage lighting: {mode} (setting {})", state.lighting);

    for (role, mut light, mut transform) in lights.iter_mut() {
        let ((color, intensity), position) = match role {
            StageLight::Key => ((p.key.0, p.key.1), Vec3::from(p.key.2)),
            StageLight::Rim => (p.rim, Vec3::new(-1.5, 1.8, -1.5)),
            StageLight::Fill => (p.fill, Vec3::new(1.5, -0.5, 1.0)),
        };
        light.color = hex(color);
        light.illuminance = intensity * LUX_PER_UNIT;
        *transform = Transform::from_translation(position).looking_at(Vec3::ZERO, Vec3::Y);
    }
    for mut amb in ambient.iter_mut() {
        amb.color = hex(p.ambient.0);
        amb.brightness = p.ambient.1 * AMBIENT_PER_UNIT;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_follows_the_clock() {
        assert_eq!(resolve("auto", 6), "dawn");
        assert_eq!(resolve("auto", 12), "day");
        assert_eq!(resolve("auto", 18), "sunset");
        assert_eq!(resolve("auto", 23), "night");
        assert_eq!(resolve("auto", 2), "night");
        assert_eq!(resolve("sunset", 12), "sunset");
    }
}
