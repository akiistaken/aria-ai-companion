//! Facial animation: the emotion's expression blend, blinking, lip-sync and (for models whose eyes
//! are driven by blend shapes rather than bones) gaze.
//!
//! An emotion asks for a [`Face`]; [`FaceRig`] eases toward it, layers the blink and the mouth on
//! top and sends the result to `bevy_vrm1` as a single `SetExpressions` (it replaces every
//! override, so everything has to go in one event). Only expressions the loaded model declares are
//! sent: a custom VRM without `relaxed` or `ou` simply does without.

use bevy::prelude::*;
use bevy_vrm1::prelude::*;

use crate::spring::{approach, Rng};

/// The expression channels this crate drives, by VRM 1.0 preset name.
pub const CHANNELS: [&str; 17] = [
    "happy", "angry", "sad", "relaxed", "surprised", "blink", "blinkLeft", "blinkRight", "aa", "ih",
    "ou", "ee", "oh", "lookUp", "lookDown", "lookLeft", "lookRight",
];

const HAPPY: usize = 0;
const ANGRY: usize = 1;
const SAD: usize = 2;
const RELAXED: usize = 3;
const SURPRISED: usize = 4;
const BLINK: usize = 5;
const BLINK_LEFT: usize = 6;
const BLINK_RIGHT: usize = 7;
const AA: usize = 8;
const IH: usize = 9;
const OU: usize = 10;
const EE: usize = 11;
const OH: usize = 12;
const LOOK_UP: usize = 13;
const LOOK_DOWN: usize = 14;
const LOOK_LEFT: usize = 15;
const LOOK_RIGHT: usize = 16;

/// What an emotion wants the face to do, each weight `0..1`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Face {
    pub happy: f32,
    pub angry: f32,
    pub sad: f32,
    pub relaxed: f32,
    pub surprised: f32,
    /// Eyelids held closed by the emotion (crying, dozing); automatic blinking comes on top.
    pub eyes_closed: f32,
    /// A wink of the model's left eye.
    pub wink: f32,
    /// Mouth shapes the emotion itself makes (a laugh, a yawn, a pout). Speech is added later.
    pub aa: f32,
    pub ih: f32,
    pub ou: f32,
    pub oh: f32,
}

impl Face {
    pub fn blend(&mut self, other: &Face, t: f32) {
        let mix = |a: &mut f32, b: f32| *a += (b - *a) * t;
        mix(&mut self.happy, other.happy);
        mix(&mut self.angry, other.angry);
        mix(&mut self.sad, other.sad);
        mix(&mut self.relaxed, other.relaxed);
        mix(&mut self.surprised, other.surprised);
        mix(&mut self.eyes_closed, other.eyes_closed);
        mix(&mut self.wink, other.wink);
        mix(&mut self.aa, other.aa);
        mix(&mut self.ih, other.ih);
        mix(&mut self.ou, other.ou);
        mix(&mut self.oh, other.oh);
    }
}

/// Lip-sync input for one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Speech {
    /// Open-mouth level measured from the playing voice, `0..1`.
    pub aa: f32,
    /// Rounded-mouth level, `0..1`.
    pub oh: f32,
}

/// Per-model facial state.
pub struct FaceRig {
    /// Which of [`CHANNELS`] the loaded model declares.
    available: [bool; CHANNELS.len()],
    /// Eyes are moved by `lookUp`/`lookLeft`/… expressions instead of eye bones.
    gaze_by_expression: bool,
    current: Face,
    blink_timer: f32,
    next_blink: f32,
    /// A second blink right after the first, now and then.
    double_blink: bool,
    last_sent: [f32; CHANNELS.len()],
    rng: Rng,
}

impl Default for FaceRig {
    fn default() -> Self {
        Self {
            available: [true; CHANNELS.len()],
            gaze_by_expression: false,
            current: Face::default(),
            blink_timer: 0.0,
            next_blink: 2.5,
            double_blink: false,
            last_sent: [-1.0; CHANNELS.len()],
            rng: Rng::default(),
        }
    }
}

/// One blink: closing takes a third of it, opening the rest.
const BLINK_SECS: f32 = 0.16;

impl FaceRig {
    /// Reads what the model can do off its VRM root. Returns the declared expression names.
    pub fn bind(&mut self, world: &World, root: Entity) -> Vec<String> {
        *self = Self::default();
        let mut names = Vec::new();
        if let Some(settings) = world.get::<ExpressionSettings>(root) {
            for (slot, name) in self.available.iter_mut().zip(CHANNELS) {
                *slot = settings.0.contains_key(name);
            }
            names = settings.0.keys().cloned().collect();
            names.sort();
        }
        self.gaze_by_expression = world
            .get::<LookAtProperties>(root)
            .is_some_and(|look| look.r#type == LookAtType::Expression);
        names
    }

    /// Advances the blink clock and returns how closed the eyelids are from blinking alone.
    fn blink(&mut self, dt: f32) -> f32 {
        self.blink_timer += dt;
        if self.blink_timer < self.next_blink {
            return 0.0;
        }
        let phase = (self.blink_timer - self.next_blink) / BLINK_SECS;
        if phase >= 1.0 {
            self.blink_timer = 0.0;
            self.next_blink = if self.double_blink {
                self.double_blink = false;
                0.12
            } else {
                self.double_blink = self.rng.unit() < 0.18;
                self.rng.range(2.0, 5.5)
            };
            return 0.0;
        }
        if phase < 0.35 {
            phase / 0.35
        } else {
            1.0 - (phase - 0.35) / 0.65
        }
    }

    /// Computes this frame's expression weights. `gaze` is where the eyes point, `-1..1` on each
    /// axis (x toward screen right, y up), used only by expression-gaze models.
    pub fn update(&mut self, target: &Face, speech: Speech, gaze: Vec2, dt: f32) -> [f32; CHANNELS.len()] {
        // Emotions fade in and out; a face that snaps reads as a glitch.
        let rate = 11.0;
        let ease = |a: &mut f32, b: f32| *a = approach(*a, b, rate, dt);
        let c = &mut self.current;
        ease(&mut c.happy, target.happy);
        ease(&mut c.angry, target.angry);
        ease(&mut c.sad, target.sad);
        ease(&mut c.relaxed, target.relaxed);
        ease(&mut c.surprised, target.surprised);
        ease(&mut c.eyes_closed, target.eyes_closed);
        ease(&mut c.wink, target.wink);
        // Mouth shapes follow faster: a laugh is a rhythm.
        c.aa = approach(c.aa, target.aa, 22.0, dt);
        c.ih = approach(c.ih, target.ih, 22.0, dt);
        c.ou = approach(c.ou, target.ou, 22.0, dt);
        c.oh = approach(c.oh, target.oh, 22.0, dt);
        let c = self.current;

        // A smile already narrows the eyes; a full blink on top of it would squash them shut twice
        // over, so blinking fades as the smile grows.
        let blink = self.blink(dt) * (1.0 - 0.7 * c.happy);
        let lids = c.eyes_closed.max(blink).clamp(0.0, 1.0);

        let mut w = [0.0; CHANNELS.len()];
        w[HAPPY] = c.happy;
        w[ANGRY] = c.angry;
        w[SAD] = c.sad;
        w[RELAXED] = c.relaxed;
        w[SURPRISED] = c.surprised;
        w[BLINK] = lids;
        if c.wink > 0.01 && lids < c.wink {
            if self.available[BLINK_LEFT] {
                w[BLINK_LEFT] = c.wink;
            } else {
                w[BLINK] = lids.max(c.wink * 0.5);
            }
        }
        // Speech on top of the emotion's own mouth shape; whichever opens wider wins.
        w[AA] = c.aa.max(speech.aa);
        w[OH] = c.oh.max(speech.oh);
        w[IH] = c.ih;
        w[OU] = c.ou;
        // A little `ih` between open vowels keeps speech from looking like a flapping hinge.
        if speech.aa > 0.02 {
            w[IH] = w[IH].max((0.35 - (speech.aa - 0.35).abs()).max(0.0) * 0.6);
        }
        if !self.available[OU] {
            w[OH] = w[OH].max(w[OU]);
        }
        if !self.available[IH] {
            w[EE] = w[IH];
        }
        if self.gaze_by_expression {
            w[LOOK_RIGHT] = gaze.x.max(0.0);
            w[LOOK_LEFT] = (-gaze.x).max(0.0);
            w[LOOK_UP] = gaze.y.max(0.0);
            w[LOOK_DOWN] = (-gaze.y).max(0.0);
        }
        // `blinkRight` is reserved for a future right-eye wink; keep it explicitly open.
        w[BLINK_RIGHT] = 0.0;
        for (weight, available) in w.iter_mut().zip(self.available) {
            *weight = if available { weight.clamp(0.0, 1.0) } else { 0.0 };
        }
        w
    }

    /// Sends `weights` to the model, unless nothing visible changed since the last send.
    pub fn send(&mut self, weights: [f32; CHANNELS.len()], root: Entity, commands: &mut Commands) {
        let changed = weights
            .iter()
            .zip(self.last_sent.iter())
            .any(|(a, b)| (a - b).abs() > 0.004);
        if !changed {
            return;
        }
        self.last_sent = weights;
        commands.trigger(SetExpressions::from_iter(
            root,
            CHANNELS
                .into_iter()
                .zip(weights)
                .zip(self.available)
                .filter_map(|(pair, available)| available.then_some(pair)),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(rig: &mut FaceRig, target: &Face, speech: Speech) -> [f32; CHANNELS.len()] {
        let mut weights = [0.0; CHANNELS.len()];
        // Short of the first blink.
        for _ in 0..40 {
            weights = rig.update(target, speech, Vec2::ZERO, 1.0 / 30.0);
        }
        weights
    }

    #[test]
    fn a_silent_neutral_face_keeps_its_mouth_shut() {
        let mut rig = FaceRig::default();
        let weights = settle(&mut rig, &Face::default(), Speech::default());
        for channel in [AA, IH, OU, EE, OH] {
            assert_eq!(weights[channel], 0.0, "{} should be closed", CHANNELS[channel]);
        }
    }

    #[test]
    fn speech_opens_the_mouth_and_emotion_eases_in() {
        let mut rig = FaceRig::default();
        let happy = Face { happy: 1.0, ..default() };
        let first = rig.update(&happy, Speech { aa: 0.8, oh: 0.1 }, Vec2::ZERO, 1.0 / 30.0);
        assert_eq!(first[AA], 0.8, "speech is not smoothed again, it already is an envelope");
        assert!(first[HAPPY] > 0.0 && first[HAPPY] < 0.6, "emotions fade in, got {}", first[HAPPY]);
        let later = settle(&mut rig, &happy, Speech::default());
        assert!(later[HAPPY] > 0.98);
        assert_eq!(later[AA], 0.0);
    }

    #[test]
    fn missing_expressions_are_never_sent_and_fall_back() {
        let mut rig = FaceRig::default();
        rig.available[OU] = false;
        rig.available[RELAXED] = false;
        let weights = settle(&mut rig, &Face { ou: 0.6, relaxed: 1.0, ..default() }, Speech::default());
        assert_eq!(weights[OU], 0.0);
        assert_eq!(weights[RELAXED], 0.0);
        assert!(weights[OH] > 0.5, "`oh` stands in for a missing `ou`");
    }

    #[test]
    fn blinks_happen_and_reopen() {
        let mut rig = FaceRig { rng: Rng::seeded(7), ..default() };
        let mut closed_frames = 0;
        let mut peak = 0.0f32;
        for _ in 0..600 {
            let w = rig.update(&Face::default(), Speech::default(), Vec2::ZERO, 1.0 / 60.0);
            peak = peak.max(w[BLINK]);
            if w[BLINK] > 0.5 {
                closed_frames += 1;
            }
        }
        assert!(peak > 0.9, "the eyes should close fully during a blink");
        assert!(closed_frames > 3 && closed_frames < 120, "closed for {closed_frames} of 600 frames");
    }

    #[test]
    fn expression_gaze_models_get_look_weights() {
        let mut rig = FaceRig { gaze_by_expression: true, ..default() };
        let w = rig.update(&Face::default(), Speech::default(), Vec2::new(0.5, -0.25), 1.0 / 30.0);
        assert_eq!((w[LOOK_RIGHT], w[LOOK_LEFT]), (0.5, 0.0));
        assert_eq!((w[LOOK_UP], w[LOOK_DOWN]), (0.0, 0.25));
    }
}
