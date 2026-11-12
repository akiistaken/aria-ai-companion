//! Message types shared by the Aria app and the avatar renderer.
//!
//! Wire format: **one JSON object per line** over a local TCP socket. Each message is a tagged
//! enum: `{"kind":"set_state","data":{...}}`. Unknown kinds and malformed lines are the
//! receiver's problem to ignore, never to panic on.
//!
//! The variants mirror the old WebView JS API so behaviour maps 1:1.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Where a peeking avatar is poking out from.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PeekSide {
    #[default]
    None,
    Left,
    Right,
}

impl PeekSide {
    pub fn as_str(self) -> &'static str {
        match self {
            PeekSide::None => "none",
            PeekSide::Left => "left",
            PeekSide::Right => "right",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text {
            "left" => PeekSide::Left,
            "right" => PeekSide::Right,
            _ => PeekSide::None,
        }
    }
}

/// What is holding the avatar up. The app owns the desktop physics (gravity, the taskbar as the
/// ground, throwing); the renderer picks the matching body animation and derives speed from how
/// far its own window actually moved.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BodyMode {
    /// Standing or walking on the ground (the taskbar, or a window she is perched on).
    #[default]
    Ground,
    /// Falling, jumping or thrown.
    Air,
    /// Held by the cursor.
    Held,
}

/// Every full-body emotion the renderer can act out, in the spelling used on the wire and in the
/// `[EMOTION: …]` tags. Unknown names fall back to `idle` on the renderer side.
pub const EMOTIONS: &[&str] = &[
    "idle", "talking", "happy", "laugh", "angry", "annoyed", "sad", "cry", "thinking",
    "surprised", "shy", "dizzy", "sleepy", "dance", "groove", "sit", "wave",
];

/// Commands sent from the app to the avatar renderer.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Command {
    /// Load a VRM from an absolute filesystem path.
    LoadModel { path: String },
    /// What she feels (one of [`EMOTIONS`]) and whether her voice is playing. Walking is not part
    /// of it: the renderer sees its own window move and animates from that.
    SetState { emotion: String, talking: bool },
    /// Whether she is on the ground, in the air or held (see [`BodyMode`]).
    SetBody { mode: BodyMode },
    /// The persona (`tsundere`, `genki`, …) that styles idle posture and how lively she moves.
    SetMood { persona: String },
    /// Normalised `-1..1` against the avatar window centre.
    SetLook { x: f32, y: f32 },
    /// Lip-sync envelope, `0..1`.
    SetMouth { aa: f32, oh: f32 },
    SetPeeking { side: PeekSide },
    SetMusicVibe { active: bool, energy: f32 },
    TriggerHeadpat,
    SetProps { props: BTreeMap<String, bool> },
    /// `auto` | `day` | `dawn` | `sunset` | `night`.
    SetLighting { preset: String },
    SetCameraFraming {
        framing: String,
        zoom: f32,
        y_offset: f32,
    },
    /// 30 normal, 10 in game mode, 0 = paused (nothing rendered, window hidden).
    SetFps { fps: u32 },
    /// Where the avatar window goes, in physical screen pixels, and the size it draws at (a
    /// layered window shows its bitmap 1:1). The renderer applies the position only until it has
    /// an owner (see `SetOwner`); after that the app moves the window itself, in the same
    /// `DeferWindowPos` batch as its own window.
    SetWindowRect {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// One cursor sample, in physical pixels relative to the avatar window's top-left corner.
    ///
    /// The avatar window is layered and gets no input of its own, so the app feeds it the global
    /// cursor position and button state.
    SetPointer {
        x: f32,
        y: f32,
        /// Primary (left) button.
        pressed: bool,
        #[serde(default)]
        right: bool,
        #[serde(default)]
        middle: bool,
    },
    /// Whether clicks should reach the desktop: false over empty space.
    SetClickThrough { enabled: bool },
    /// Makes the avatar window an owned window of the app's mascot window (`hwnd`).
    ///
    /// Owned windows share their owner's z-order band and are hidden and minimized with it, so
    /// the avatar and its chat chrome behave as one window. From then on the renderer stops
    /// positioning its window and `UpdateLayeredWindow` keeps whatever position the app set.
    SetOwner { hwnd: u64 },
    Shutdown,
}

/// Events sent from the avatar renderer to the app.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Event {
    /// First line after connecting; the app drops connections with a wrong token.
    Hello { token: String },
    /// The window exists and the renderer is live. `hwnd` is an integer so the app can
    /// `SetWindowPos` it directly.
    Ready { hwnd: u64, adapter: String },
    ModelLoaded {
        ok: bool,
        error: Option<String>,
        expressions: Vec<String>,
    },
    /// How the model sits in the avatar window, so the app can stand her on the taskbar and move
    /// her at a speed that matches her size. Sent whenever framing, size or model changes.
    Stage {
        /// Pixels from the bottom of the avatar window up to the soles of her feet (0 when the
        /// framing crops the legs, so the window's bottom edge rests on the ground instead).
        ground_offset: f32,
        /// Window pixels per metre of model.
        pixels_per_metre: f32,
    },
    DragStart,
    DragMove,
    DragEnd,
    /// Double-click on the model.
    Poke,
    Shake,
    Headpat,
    MiddleClick,
    ContextMenu,
    /// The cursor moved on or off the model, so the app can toggle click-through.
    HoverChanged { over: bool },
    Error { msg: String },
}

impl Command {
    /// Serialises to a single line without the trailing newline.
    pub fn to_line(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }
}

impl Event {
    pub fn to_line(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }

    pub fn from_line(line: &str) -> Option<Event> {
        serde_json::from_str(line).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip_as_tagged_json() {
        let command = Command::SetState {
            emotion: "happy".into(),
            talking: true,
        };
        let line = command.to_line().expect("serialises");
        assert!(line.contains("\"kind\":\"set_state\""));
        let back: Command = serde_json::from_str(&line).expect("parses");
        match back {
            Command::SetState { emotion, talking } => {
                assert_eq!(emotion, "happy");
                assert!(talking);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn events_round_trip_and_reject_garbage() {
        let event = Event::Headpat;
        let line = event.to_line().expect("serialises");
        assert!(matches!(Event::from_line(&line), Some(Event::Headpat)));

        // A bad line must be ignorable, not a panic.
        assert!(Event::from_line("{\"kind\":\"nope\"}").is_none());
        assert!(Event::from_line("not json at all").is_none());
    }

    #[test]
    fn old_pointer_lines_still_parse() {
        // Older senders omit the extra buttons; they must default to released.
        let line = r#"{"kind":"set_pointer","data":{"x":1.0,"y":2.0,"pressed":true}}"#;
        match serde_json::from_str::<Command>(line).expect("parses") {
            Command::SetPointer { pressed, right, middle, .. } => {
                assert!(pressed);
                assert!(!right && !middle);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn body_mode_and_stage_use_snake_case_json() {
        let line = Command::SetBody { mode: BodyMode::Air }.to_line().expect("serialises");
        assert_eq!(line, r#"{"kind":"set_body","data":{"mode":"air"}}"#);
        let stage = Event::Stage { ground_offset: 23.5, pixels_per_metre: 190.0 };
        match Event::from_line(&stage.to_line().expect("serialises")) {
            Some(Event::Stage { ground_offset, pixels_per_metre }) => {
                assert_eq!(ground_offset, 23.5);
                assert_eq!(pixels_per_metre, 190.0);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn peek_side_round_trips() {
        for side in [PeekSide::None, PeekSide::Left, PeekSide::Right] {
            assert_eq!(PeekSide::parse(side.as_str()), side);
        }
        assert_eq!(PeekSide::parse("bogus"), PeekSide::None);
    }
}
