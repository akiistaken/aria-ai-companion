//! The socket link to the main Aria app.
//!
//! Wire format: one JSON object per line over TCP, using the shared `aria-protocol` types. The main
//! app binds a port, spawns this process with `--port/--token/--model`, and this side connects
//! back, sends `Hello`, then streams `Event`s and consumes `Command`s.
//!
//! Unknown or malformed lines are logged and ignored: a bad message must never take the renderer
//! down.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use aria_protocol::{Command, Event};
use bevy::prelude::*;

#[derive(Debug, Clone)]
pub struct Args {
    pub port: u16,
    pub token: String,
    pub model: String,
}

impl Args {
    pub fn parse() -> Self {
        let mut args = Args {
            port: 0,
            token: String::new(),
            model: String::new(),
        };
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--port" => {
                    if let Some(v) = it.next() {
                        args.port = v.parse().unwrap_or(0);
                    }
                }
                "--token" => {
                    if let Some(v) = it.next() {
                        args.token = v;
                    }
                }
                "--model" => {
                    if let Some(v) = it.next() {
                        args.model = v;
                    }
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
    /// Events queued before the socket is up.
    pending: Arc<Mutex<VecDeque<Event>>>,
    connected: bool,
    /// Test-only tap on the outgoing events.
    #[cfg(test)]
    sent: Mutex<Option<Receiver<Event>>>,
}

impl AvatarLink {
    pub fn new(args: Args) -> Self {
        let (out_tx, out_rx) = mpsc::channel::<Event>();
        let (in_tx, in_rx) = mpsc::channel::<Command>();
        let pending: Arc<Mutex<VecDeque<Event>>> = Arc::new(Mutex::new(VecDeque::new()));

        // Standalone runs (`--model` only) never connect anywhere.
        let connected = args.port == 0;
        if args.port != 0 {
            spawn_connection(args.clone(), in_tx, out_rx, pending.clone());
        } else {
            log::info!("avatar running standalone (no --port given)");
        }

        Self {
            commands: Mutex::new(in_rx),
            outbox: out_tx,
            pending,
            connected,
            #[cfg(test)]
            sent: Mutex::new(None),
        }
    }

    /// A connected link with no socket, whose events can be read back with [Self::drain_sent].
    #[cfg(test)]
    pub fn detached() -> Self {
        let (out_tx, out_rx) = mpsc::channel::<Event>();
        let (_in_tx, in_rx) = mpsc::channel::<Command>();
        Self {
            commands: Mutex::new(in_rx),
            outbox: out_tx,
            pending: Arc::new(Mutex::new(VecDeque::new())),
            connected: true,
            sent: Mutex::new(Some(out_rx)),
        }
    }

    #[cfg(test)]
    pub fn drain_sent(&self) -> Vec<Event> {
        self.sent
            .lock()
            .unwrap()
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default()
    }

    /// Marks the link usable and flushes anything queued before the socket existed.
    ///
    /// `Ready` is produced before the renderer has necessarily finished connecting, so it lands in
    /// the queue; without this flush the app would never learn the window handle.
    fn mark_connected(&mut self) {
        if self.connected {
            return;
        }
        self.connected = true;
        if let Ok(mut queue) = self.pending.lock() {
            for event in queue.drain(..) {
                let _ = self.outbox.send(event);
            }
        }
    }

    pub fn send(&self, event: Event) {
        if self.connected {
            let _ = self.outbox.send(event);
        } else if let Ok(mut queue) = self.pending.lock() {
            // Bounded, so a dead app cannot grow this forever.
            if queue.len() < 256 {
                queue.push_back(event);
            }
        }
    }

}

/// Connects, then owns both directions of the socket for the life of the process.
fn spawn_connection(
    args: Args,
    in_tx: Sender<Command>,
    out_rx: Receiver<Event>,
    pending: Arc<Mutex<VecDeque<Event>>>,
) {
    std::thread::Builder::new()
        .name("avatar-link".into())
        .spawn(move || {
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

            let mut reader = BufReader::new(match stream.try_clone() {
                Ok(s) => s,
                Err(e) => {
                    log::error!("avatar could not clone the socket: {e}");
                    std::process::exit(2);
                }
            });
            let mut writer = stream;

            let hello = Event::Hello {
                token: args.token.clone(),
            };
            if let Some(line) = hello.to_line() {
                if writeln!(writer, "{line}").is_err() {
                    std::process::exit(2);
                }
            }
            // Flush anything queued before the socket came up, notably `Ready`.
            if let Ok(mut queue) = pending.lock() {
                for event in queue.drain(..) {
                    if let Some(line) = event.to_line() {
                        let _ = writeln!(writer, "{line}");
                    }
                }
            }

            std::thread::spawn(move || {
                while let Ok(event) = out_rx.recv() {
                    let Some(line) = event.to_line() else { continue };
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
                        // The app closing the socket is our shutdown signal.
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
                                // Moves skip the frame loop: the window thread applies them
                                // within ~15 ms, so the avatar keeps pace with its chrome.
                                if let Command::SetWindowRect { x, y, .. } = &command {
                                    crate::layered::place(*x, *y);
                                }
                                if in_tx.send(command).is_err() {
                                    break;
                                }
                            }
                            Err(e) => log::warn!("ignoring bad command ({e}): {trimmed}"),
                        }
                    }
                    Err(e) => {
                        log::warn!("avatar link read error: {e}");
                        break;
                    }
                }
            }
            std::process::exit(0);
        })
        .ok();
}

/// Applies socket commands to the ECS once per frame, so game logic stays single-threaded.
pub fn pump_link(
    link: ResMut<AvatarLink>,
    mut state: ResMut<crate::behaviors::AvatarState>,
    mut hearts: ResMut<crate::hearts::Hearts>,
    mut commands: Commands,
) {
    let drained: Vec<Command> = {
        let rx = link.commands.lock().unwrap_or_else(|e| e.into_inner());
        rx.try_iter().collect()
    };
    for command in drained {
        match command {
            Command::LoadModel { path } => state.pending_model = Some(path),
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
                state.mouth_age = 0.0;
            }
            Command::SetPeeking { side } => state.peeking = side,
            Command::SetMusicVibe { active, energy } => {
                state.music_active = active;
                state.music_energy = energy;
            }
            Command::SetDragInertia { x, y } => {
                state.target_inertia_x = x;
                state.target_inertia_y = y;
            }
            Command::TriggerHeadpat => {
                state.headpat_active = 1.0;
                hearts.burst();
            }
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
            // 0 pauses; otherwise clamp to something a desktop pet can sensibly use.
            Command::SetFps { fps } => state.fps = if fps == 0 { 0 } else { fps.clamp(5, 60) },
            // The position was already applied by the socket thread; only the size matters here.
            Command::SetWindowRect { width, height, .. } => {
                state.viewport = (width.clamp(64, 2048), height.clamp(64, 2048));
            }
            Command::SetPointer {
                x,
                y,
                pressed,
                right,
                middle,
            } => {
                state.pointer = Some(crate::input::PointerSample {
                    position: Vec2::new(x, y),
                    left: pressed,
                    right,
                    middle,
                });
            }
            // The app applies click-through to the window itself; nothing to do here.
            Command::SetClickThrough { .. } => {}
            Command::Shutdown => {
                commands.write_message(AppExit::Success);
            }
        }
    }
}

/// Reports the window handle once, so the app can move the avatar.
///
/// Also flips the link to connected and flushes anything queued while the socket was coming up.
pub fn announce_ready(mut link: ResMut<AvatarLink>, mut done: Local<bool>) {
    if *done {
        return;
    }
    let hwnd = crate::layered::LayeredSurface::hwnd_value();
    if hwnd == 0 {
        return;
    }
    *done = true;
    link.mark_connected();
    link.send(Event::Ready {
        hwnd,
        adapter: "bevy".into(),
    });
    log::info!("announced Ready with hwnd={hwnd}");
}
