//! The socket link to the main Aria app.
//!
//! Wire format: one JSON object per line over TCP. The main app binds a port, spawns this
//! process with `--port/--token/--model`, and this side connects back, sends `Hello`, then
//! streams `Event`s and consumes `Command`s.
//!
//! Unknown or malformed lines are logged and ignored: a bad message must never take the renderer
//! down.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Commands the main app sends to the avatar.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Command {
    LoadModel { path: String },
    SetState {
        emotion: String,
        facing_right: bool,
        moving: bool,
        talking: bool,
    },
    SetLook { x: f32, y: f32 },
    SetMouth { aa: f32, oh: f32 },
    SetPeeking { side: String },
    SetMusicVibe { active: bool, energy: f32 },
    SetDragInertia { x: f32, y: f32 },
    TriggerHeadpat,
    SetProps {
        props: std::collections::BTreeMap<String, bool>,
    },
    SetLighting { preset: String },
    SetCameraFraming { framing: String, zoom: f32, y_offset: f32 },
    SetFps { fps: u32 },
    Shutdown,
}

/// Events the avatar sends to the main app.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Event {
    Hello { token: String },
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
    Poke,
    Shake,
    Headpat,
    MiddleClick,
    ContextMenu,
    Error { msg: String },
}

#[derive(Debug, Clone)]
pub struct Args {
    pub port: u16,
    pub token: String,
    pub model: String,
    /// Sent as `ppid` so the child can exit when the parent disappears.
    pub parent_pid: u32,
}

impl Args {
    pub fn parse() -> Self {
        let mut args = Args {
            port: 0,
            token: String::new(),
            model: String::new(),
            parent_pid: 0,
        };
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            let mut take = |slot: &mut String| {
                if let Some(v) = it.next() {
                    *slot = v;
                }
            };
            match arg.as_str() {
                "--port" => {
                    let mut v = String::new();
                    take(&mut v);
                    args.port = v.parse().unwrap_or(0);
                }
                "--token" => take(&mut args.token),
                "--model" => take(&mut args.model),
                "--ppid" => {
                    let mut v = String::new();
                    take(&mut v);
                    args.parent_pid = v.parse().unwrap_or(0);
                }
                _ => {}
            }
        }
        args
    }
}

/// Shared handle to the outgoing writer thread plus the incoming command queue.
#[derive(Resource)]
pub struct AvatarLink {
    pub commands: Mutex<Receiver<Command>>,
    outbox: Sender<Event>,
    args: Args,
    connected: bool,
    /// Lines pushed before the socket is up.
    pending: Arc<Mutex<VecDeque<Event>>>,
}

impl AvatarLink {
    pub fn new(args: Args) -> Self {
        let (out_tx, out_rx) = mpsc::channel::<Event>();
        let (in_tx, in_rx) = mpsc::channel::<Command>();
        let pending: Arc<Mutex<VecDeque<Event>>> = Arc::new(Mutex::new(VecDeque::new()));

        // Standalone mode (`--model` only) is the Phase 3 spike: no socket, no supervision.
        if args.port != 0 {
            spawn_connection(args.clone(), in_tx, out_rx, pending.clone());
        } else {
            log::info!("avatar running standalone (no --port given)");
        }

        let connected = args.port == 0;
        Self {
            commands: Mutex::new(in_rx),
            outbox: out_tx,
            args,
            connected,
            pending,
        }
    }

    pub fn send(&self, event: Event) {
        if self.connected {
            let _ = self.outbox.send(event);
        } else if let Ok(mut queue) = self.pending.lock() {
            // Bounded: a dead app must not grow this forever.
            if queue.len() < 256 {
                queue.push_back(event);
            }
        }
    }

    pub fn mark_connected(&mut self) {
        self.connected = true;
        if let Ok(mut queue) = self.pending.lock() {
            for event in queue.drain(..) {
                let _ = self.outbox.send(event);
            }
        }
    }

    pub fn shutdown_requested(&self) -> bool {
        self.args.port == 0 && !self.args.token.is_empty()
    }

    pub fn token(&self) -> &str {
        &self.args.token
    }
}

/// Connects, then owns both directions of the socket for the life of the process.
fn spawn_connection(
    args: Args,
    in_tx: Sender<Command>,
    out_rx: Receiver<Event>,
    pending: Arc<Mutex<VecDeque<Event>>>,
) {
    std::thread::spawn(move || {
        let addr = format!("127.0.0.1:{}", args.port);
        let stream = match TcpStream::connect(&addr) {
            Ok(s) => s,
            Err(e) => {
                log::error!("avatar could not connect to {addr}: {e}");
                std::process::exit(2);
            }
        };
        let _ = stream.set_nodelay(true);
        log::info!("avatar connected to {addr}");

        // The socket closing is the app's shutdown signal.
        let mut reader = BufReader::new(match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                log::error!("avatar could not clone the socket: {e}");
                std::process::exit(2);
            }
        });
        let mut writer = stream;

        let hello = serde_json::to_string(&Event::Hello {
            token: args.token.clone(),
        })
        .unwrap_or_default();
        if writeln!(writer, "{hello}").is_err() {
            std::process::exit(2);
        }
        // Flush anything queued while connecting (notably `Ready`).
        if let Ok(mut queue) = pending.lock() {
            for event in queue.drain(..) {
                if let Ok(line) = serde_json::to_string(&event) {
                    let _ = writeln!(writer, "{line}");
                }
            }
        }

        let write_handle = std::thread::spawn(move || {
            while let Ok(event) = out_rx.recv() {
                let Ok(line) = serde_json::to_string(&event) else {
                    continue;
                };
                if writeln!(writer, "{line}").is_err() {
                    break;
                }
            }
        });

        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    log::info!("avatar link closed by the app; exiting");
                    std::process::exit(0);
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Command>(trimmed) {
                        Ok(Command::Shutdown) => {
                            log::info!("avatar shutdown requested");
                            std::process::exit(0);
                        }
                        Ok(command) => {
                            if in_tx.send(command).is_err() {
                                break;
                            }
                        }
                        Err(e) => log::warn!("ignoring unparseable command ({e}): {trimmed}"),
                    }
                }
                Err(e) => {
                    log::warn!("avatar link read error: {e}");
                    break;
                }
            }
        }
        drop(write_handle);
        std::process::exit(0);
    });
}

/// Drains socket commands into the ECS once per frame, so game logic stays single-threaded.
pub fn pump_link(
    mut link: ResMut<AvatarLink>,
    mut state: ResMut<crate::behaviors::AvatarState>,
    mut pending_model: ResMut<crate::behaviors::PendingModel>,
    mut commands: Commands,
) {
    let drained: Vec<Command> = {
        let rx = link.commands.lock().unwrap_or_else(|e| e.into_inner());
        rx.try_iter().collect()
    };
    for command in drained {
        match command {
            Command::LoadModel { path } => {
                pending_model.0 = Some(path);
            }
            Command::SetState {
                emotion,
                facing_right,
                moving,
                talking,
            } => {
                state.emotion = emotion;
                state.facing_right = facing_right;
                state.moving = moving;
                state.talking = talking;
            }
            Command::SetLook { x, y } => {
                state.look_x = x;
                state.look_y = y;
            }
            Command::SetMouth { aa, oh } => {
                state.mouth_aa = aa;
                state.mouth_oh = oh;
            }
            Command::SetPeeking { side } => state.peeking = side,
            Command::SetMusicVibe { active, energy } => {
                state.music_active = active;
                state.music_energy = energy;
            }
            Command::SetDragInertia { x, y } => {
                state.inertia_x = x;
                state.inertia_y = y;
            }
            Command::TriggerHeadpat => state.headpat_at = Some(std::time::Instant::now()),
            Command::SetProps { props } => state.props = props,
            Command::SetLighting { preset } => state.lighting = preset,
            Command::SetCameraFraming {
                framing,
                zoom,
                y_offset,
            } => {
                state.framing = framing;
                state.zoom = zoom;
                state.y_offset = y_offset;
            }
            Command::SetFps { fps } => state.fps = fps,
            Command::Shutdown => {
                commands.write_message(AppExit::Success);
            }
        }
    }
}
