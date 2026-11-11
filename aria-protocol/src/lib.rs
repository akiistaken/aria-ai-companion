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

/// Commands sent from the app to the avatar renderer.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Command {
    /// Load a VRM from an absolute filesystem path.
    LoadModel { path: String },
    SetState {
        emotion: String,
        facing_right: bool,
        moving: bool,
        talking: bool,
    },
    /// Normalised `-1..1` against the avatar window centre.
    SetLook { x: f32, y: f32 },
    /// Lip-sync envelope, `0..1`.
    SetMouth { aa: f32, oh: f32 },
    SetPeeking { side: PeekSide },
    SetMusicVibe { active: bool, energy: f32 },
    SetDragInertia { x: f32, y: f32 },
    TriggerHeadpat,
    SetProps { props: BTreeMap<String, bool> },
    /// `auto` | `day` | `dawn` | `sunset` | `night`.
    SetLighting { preset: String },
    SetCameraFraming {
        framing: String,
        zoom: f32,
        y_offset: f32,
    },
    /// 30 normal, 10 in game mode, 0 = paused.
    SetFps { fps: u32 },
    /// The window got or lost primary-button focus, forwarded from the app.
    ///
    /// The avatar window is layered and gets no input of its own, so the app feeds it the global
    /// cursor position and button state.
    SetPointer {
        x: f32,
        y: f32,
        pressed: bool,
    },
    /// Whether clicks should reach the desktop: false over empty space.
    SetClickThrough { enabled: bool },
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
    DragStart,
    DragMove,
    DragEnd,
    DragInertia { x: f32, y: f32 },
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
            facing_right: true,
            moving: false,
            talking: true,
        };
        let line = command.to_line().expect("serialises");
        assert!(line.contains("\"kind\":\"set_state\""));
        let back: Command = serde_json::from_str(&line).expect("parses");
        match back {
            Command::SetState { emotion, talking, .. } => {
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
    fn peek_side_round_trips() {
        for side in [PeekSide::None, PeekSide::Left, PeekSide::Right] {
            assert_eq!(PeekSide::parse(side.as_str()), side);
        }
        assert_eq!(PeekSide::parse("bogus"), PeekSide::None);
    }
}
