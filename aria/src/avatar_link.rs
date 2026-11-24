//! Supervision and control of the `aria-avatar` renderer process.
//!
//! The renderer is a separate executable: Slint and Bevy each want to own an event loop and each
//! pin their own wgpu/winit versions, so they are two binaries talking over a local socket. This
//! module spawns it, keeps it alive, and exposes the same method names the old WebView `Stage` did,
//! so call sites in `app.rs` barely change.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aria_protocol::{BodyMode, Command, Event, PeekSide};

/// How many restarts are attempted before the app gives up and reports a status message.
const MAX_RESTARTS: u32 = 4;

struct Shared {
    writer: Mutex<Option<TcpStream>>,
    child: Mutex<Option<Child>>,
    hwnd: AtomicU64,
    alive: AtomicBool,
    token: String,
    model: String,
    /// The port the renderer must connect back to.
    port: u16,
    /// Set while the app is shutting down, so supervision does not restart the renderer.
    shutting_down: AtomicBool,
}

/// Handle to the avatar renderer. Cheap to clone (it is an `Arc` internally).
#[derive(Clone)]
pub struct AvatarLink {
    inner: Arc<Shared>,
    /// Commands are queued here and serialised by the writer thread.
    tx: Sender<Command>,
    /// Last values pushed, so unchanged state is not re-sent every frame.
    last: Arc<Mutex<LastState>>,
}

/// Last values pushed, so the 60 Hz motion tick only writes to the socket when something moved.
#[derive(Default, Clone, PartialEq)]
struct LastState {
    emotion: String,
    talking: bool,
    body: Option<BodyMode>,
    mood: String,
    peeking: PeekSide,
    look: (i32, i32),
    mouth: (i32, i32),
    pointer: Option<(i32, i32, bool, bool, bool)>,
    rect: Option<(i32, i32, u32, u32)>,
    fps: Option<u32>,
}

/// Quantises a `-1..1` value so jitter below 1% is not re-sent.
fn q(v: f32) -> i32 {
    (v * 100.0).round() as i32
}

impl AvatarLink {
    /// A link that is not connected yet.
    ///
    /// `App` is built before the renderer is spawned, because spawning needs the settings that
    /// live in the app; commands sent before the real link exists are dropped harmlessly.
    pub fn placeholder() -> Self {
        let (tx, rx) = mpsc::channel::<Command>();
        // Nothing reads `rx` until `start` replaces this handle; drain it so `send` never blocks.
        std::thread::spawn(move || while rx.recv().is_ok() {});
        Self {
            inner: Arc::new(Shared {
                writer: Mutex::new(None),
                child: Mutex::new(None),
                hwnd: AtomicU64::new(0),
                alive: AtomicBool::new(false),
                token: String::new(),
                model: String::new(),
                port: 0,
                shutting_down: AtomicBool::new(false),
            }),
            tx,
            last: Arc::new(Mutex::new(LastState::default())),
        }
    }

    /// Binds a local port, spawns the renderer and starts supervising it.
    ///
    /// `on_event` receives renderer events on a background thread; callers normally forward them to
    /// the UI thread with `slint::invoke_from_event_loop`.
    pub fn start(
        model: String,
        on_event: impl Fn(Event) + Send + 'static,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let token = random_token();

        let shared = Arc::new(Shared {
            writer: Mutex::new(None),
            child: Mutex::new(None),
            hwnd: AtomicU64::new(0),
            alive: AtomicBool::new(true),
            token: token.clone(),
            model,
            port,
            shutting_down: AtomicBool::new(false),
        });

        let (tx, rx) = mpsc::channel::<Command>();

        // Writer: pulls commands off the channel and writes one JSON line each.
        {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("avatar-writer".into())
                .spawn(move || {
                    while let Ok(command) = rx.recv() {
                        if matches!(command, Command::Shutdown) {
                            break;
                        }
                        let Some(line) = command.to_line() else { continue };
                        let mut guard = shared.writer.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(stream) = guard.as_mut() {
                            if writeln!(stream, "{line}").is_err() {
                                *guard = None;
                            }
                        }
                    }
                })
                .ok();
        }

        // Supervisor: waits for the renderer to connect, then forever restarts it if it dies.
        {
            let shared = shared.clone();
            let on_event = Arc::new(Mutex::new(on_event));
            std::thread::Builder::new()
                .name("avatar-supervisor".into())
                .spawn(move || {
                    supervise(listener, shared, on_event);
                })
                .ok();
        }

        Ok(Self {
            inner: shared,
            tx,
            last: Arc::new(Mutex::new(LastState::default())),
        })
    }

    /// The renderer's window handle, once it has connected and reported `Ready`.
    pub fn hwnd(&self) -> u64 {
        self.inner.hwnd.load(Ordering::SeqCst)
    }

    /// True once the renderer is connected.
    pub fn is_ready(&self) -> bool {
        self.hwnd() != 0
    }

    fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    // ---- Methods mirroring the old `Stage` API -------------------------------------------

    /// What she feels and whether her voice is playing, only sent when either changed. Walking
    /// is not part of it: the renderer reads that off its window's movement.
    pub fn set_state(&self, emotion: &str, talking: bool) {
        {
            let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
            if last.emotion == emotion && last.talking == talking {
                return;
            }
            last.emotion = emotion.to_string();
            last.talking = talking;
        }
        self.send(Command::SetState {
            emotion: emotion.to_string(),
            talking,
        });
    }

    /// On the ground, in the air or held. Only sent on change.
    pub fn set_body(&self, mode: BodyMode) {
        if !self.changed(|last| last.body.replace(mode) != Some(mode)) {
            return;
        }
        self.send(Command::SetBody { mode });
    }

    /// The persona, which styles her posture and energy. Only sent on change.
    pub fn set_mood(&self, persona: &str) {
        if !self.changed(|last| std::mem::replace(&mut last.mood, persona.to_string()) != persona) {
            return;
        }
        self.send(Command::SetMood {
            persona: persona.to_string(),
        });
    }

    pub fn set_mouse_look(&self, x: f32, y: f32) {
        if !self.changed(|last| std::mem::replace(&mut last.look, (q(x), q(y))) != (q(x), q(y))) {
            return;
        }
        self.send(Command::SetLook { x, y });
    }

    /// Lip-sync envelope. Re-sent at least every few frames while non-zero so the renderer, which
    /// expires stale envelopes after 300 ms, keeps using it.
    pub fn set_mouth(&self, aa: f32, oh: f32) {
        let key = (q(aa), q(oh));
        let silent = key == (0, 0);
        if !self.changed(|last| std::mem::replace(&mut last.mouth, key) != key || !silent) {
            return;
        }
        self.send(Command::SetMouth { aa, oh });
    }

    /// Runs `f` against the last-sent state; returns its verdict on whether to send.
    fn changed(&self, f: impl FnOnce(&mut LastState) -> bool) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut last)
    }

    pub fn set_peeking(&self, side: &str) {
        let peek = PeekSide::parse(side);
        {
            let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
            if last.peeking == peek {
                return;
            }
            last.peeking = peek;
        }
        self.send(Command::SetPeeking { side: peek });
    }

    /// Music she hears: loudness, tempo (0 = unknown) and the beat phase right now.
    pub fn set_music_vibe(&self, active: bool, energy: f32, bpm: f32, beat: f32) {
        self.send(Command::SetMusicVibe { active, energy, bpm, beat });
    }

    pub fn trigger_headpat(&self) {
        self.send(Command::TriggerHeadpat);
    }

    pub fn set_props(&self, props: Vec<aria_protocol::PropSpec>) {
        self.send(Command::SetProps { props });
    }

    pub fn set_lighting(&self, preset: &str) {
        self.send(Command::SetLighting {
            preset: preset.to_string(),
        });
    }

    pub fn set_camera_framing(&self, framing: &str, zoom: f32, y_offset: f32) {
        self.send(Command::SetCameraFraming {
            framing: framing.to_string(),
            zoom,
            y_offset,
        });
    }

    /// 30 FPS ceiling (15 at rest), 10 in game mode, 0 to pause. Only sent on change.
    pub fn set_fps(&self, fps: u32) {
        if !self.changed(|last| last.fps.replace(fps) != Some(fps)) {
            return;
        }
        self.send(Command::SetFps { fps });
    }

    /// Forwards one cursor sample, relative to the avatar window, in physical pixels. The avatar
    /// window is layered and receives no input itself. Unchanged samples are not re-sent.
    pub fn set_pointer(&self, x: f32, y: f32, left: bool, right: bool, middle: bool) {
        let key = (x.round() as i32, y.round() as i32, left, right, middle);
        if !self.changed(|last| last.pointer.replace(key) != Some(key)) {
            return;
        }
        self.send(Command::SetPointer {
            x,
            y,
            pressed: left,
            right,
            middle,
        });
    }


    /// Forgets what was last sent, so a freshly (re)started renderer receives the full state.
    pub fn reset_sent_state(&self) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = LastState::default();
    }

    /// Enables or disables click-through, so empty areas of the avatar window pass clicks on.
    pub fn set_click_through(&self, enabled: bool) {
        let hwnd = self.hwnd();
        if hwnd == 0 {
            return;
        }
        crate::desktop::win32::set_click_through(hwnd as isize, enabled);
        self.send(Command::SetClickThrough { enabled });
    }

    pub fn load_model(&self, path: &str) {
        self.send(Command::LoadModel {
            path: path.to_string(),
        });
    }

    /// Makes the avatar window an owned window of the mascot window, so the two share one
    /// z-order band and hide together. Sent on every `Ready`: a restarted renderer has a new
    /// window that knows nothing.
    pub fn set_owner(&self, owner_hwnd: isize) {
        if !self.is_ready() || owner_hwnd == 0 {
            return;
        }
        self.send(Command::SetOwner {
            hwnd: owner_hwnd as u64,
        });
    }

    /// Tells the renderer the avatar window's rectangle. The size is what it renders at; the
    /// position is applied by the renderer only until the window is owned (see `set_owner`),
    /// after which `body::set_window_pos` moves it directly.
    pub fn set_window_rect(&self, x: i32, y: i32, width: i32, height: i32) {
        if !self.is_ready() || width <= 0 || height <= 0 {
            return;
        }
        let rect = (x, y, width as u32, height as u32);
        if !self.changed(|last| last.rect.replace(rect) != Some(rect)) {
            return;
        }
        self.send(Command::SetWindowRect {
            x,
            y,
            width: width as u32,
            height: height as u32,
        });
    }

    /// Stops the renderer and its supervisor. Called on app shutdown.
    pub fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        self.inner.alive.store(false, Ordering::SeqCst);
        self.send(Command::Shutdown);
        // Closing the socket is the renderer's own shutdown signal as well.
        if let Some(stream) = self
            .inner
            .writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Accepts the renderer's connection and keeps it running.
fn supervise(
    listener: TcpListener,
    shared: Arc<Shared>,
    on_event: Arc<Mutex<impl Fn(Event) + Send + 'static>>,
) {
    let mut restarts = 0u32;
    loop {
        if shared.shutting_down.load(Ordering::SeqCst) {
            return;
        }

        if !spawn_renderer(&shared) {
            log::error!("avatar renderer binary not found; avatar disabled");
            return;
        }

        // The renderer connects back with a token as its first line.
        let (stream, mut reader) = match accept_with_token(&listener, &shared.token) {
            Some(accepted) => accepted,
            None => {
                log::warn!("avatar renderer did not connect properly");
                kill_renderer(&shared);
                restarts += 1;
                if restarts > MAX_RESTARTS {
                    log::error!("avatar renderer failed to connect {MAX_RESTARTS} times; giving up");
                    return;
                }
                std::thread::sleep(backoff(restarts));
                continue;
            }
        };

        log::info!("avatar renderer connected");
        let _ = stream.set_nodelay(true);
        *shared.writer.lock().unwrap_or_else(|e| e.into_inner()) = Some(stream);

        // Read events until the renderer dies or the app shuts down. Same reader as the
        // handshake, so nothing it already buffered is lost.
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let Some(event) = Event::from_line(line.trim()) else {
                        log::warn!("ignoring unparseable avatar event");
                        continue;
                    };
                    if let Event::Ready { hwnd, .. } = &event {
                        shared.hwnd.store(*hwnd, Ordering::SeqCst);
                        // Telgraph the first successful handshake in a place that is easy to find.
                        eprintln!("[aria] avatar renderer ready, hwnd={hwnd}");
                        log::info!("avatar ready, hwnd={hwnd}");
                    }
                    if let Ok(handler) = on_event.lock() {
                        handler(event);
                    }
                }
                Err(_) => break,
            }
        }

        *shared.writer.lock().unwrap_or_else(|e| e.into_inner()) = None;
        shared.hwnd.store(0, Ordering::SeqCst);

        if shared.shutting_down.load(Ordering::SeqCst) {
            return;
        }

        // An unexpected exit is restarted with backoff.
        restarts += 1;
        if restarts > MAX_RESTARTS {
            log::error!("avatar renderer exited {MAX_RESTARTS} times; giving up");
            kill_renderer(&shared);
            return;
        }
        log::warn!("avatar renderer exited unexpectedly; restarting ({restarts}/{MAX_RESTARTS})");
        kill_renderer(&shared);
        std::thread::sleep(backoff(restarts));
    }
}

/// Exponential-ish backoff: 1 s, 2 s, 5 s, then 8 s.
fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(match attempt {
        0 | 1 => 1000,
        2 => 2000,
        3 => 5000,
        _ => 8000,
    })
}

fn kill_renderer(shared: &Shared) {
    if let Some(mut child) = shared
        .child
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Locates `aria-avatar.exe`, preferring the folder next to this executable.
fn renderer_path() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("aria-avatar.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // Dev layout: workspace `target/<profile>/`.
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        for _ in 0..3 {
            let Some(current) = dir else { break };
            let candidate = current.join("aria-avatar.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
            dir = current.parent().map(|p| p.to_path_buf());
        }
    }
    None
}

fn spawn_renderer(shared: &Shared) -> bool {
    let Some(path) = renderer_path() else {
        log::error!("aria-avatar.exe not found");
        return false;
    };
    // The renderer needs a port; the supervisor owns the listener, so the port travels via env.
    let Some(port) = listener_port(shared) else {
        return false;
    };

    let model = shared.model.clone();
    let mut cmd = ProcessCommand::new(&path);
    cmd.arg("--port")
        .arg(port.to_string())
        .arg("--token")
        .arg(&shared.token)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !model.trim().is_empty() {
        cmd.arg("--model").arg(&model);
    }
    #[cfg(windows)]
    {
        // Its output goes nowhere anyway; without this a console-subsystem renderer started from
        // the windowless release app would flash up a console window.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    match cmd.spawn() {
        Ok(mut child) => {
            log::info!("spawned avatar renderer pid={}", child.id());
            // The renderer must die with the app, even if this process is killed.
            crate::desktop::win32::put_in_kill_on_close_job(&mut child);
            *shared.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
            true
        }
        Err(e) => {
            log::error!("could not spawn avatar renderer: {e}");
            false
        }
    }
}

/// The port the listener is bound to, stored on the shared state so `spawn_renderer` does not
/// need the listener itself.
fn listener_port(shared: &Shared) -> Option<u16> {
    Some(shared.port)
}

/// Accepts one connection and validates its `Hello` token.
///
/// Returns the reader used for `Hello` along with the stream: the renderer often writes `Ready`
/// right behind `Hello`, so that line may already sit in this reader's buffer. Dropping it lost
/// `Ready` and left the avatar window unplaced.
fn accept_with_token(
    listener: &TcpListener,
    expected: &str,
) -> Option<(TcpStream, BufReader<TcpStream>)> {
    // Bound the wait: a renderer that never connects must not hang the supervisor forever.
    let deadline = Instant::now() + Duration::from_secs(30);
    listener.set_nonblocking(true).ok()?;
    loop {
        match listener.accept() {
            Ok((stream, _addr)) => {
                stream.set_nonblocking(false).ok()?;
                let mut reader = BufReader::new(stream.try_clone().ok()?);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return None;
                }
                match Event::from_line(line.trim()) {
                    Some(Event::Hello { token }) if token == expected => {
                        return Some((stream, reader))
                    }
                    _ => {
                        log::warn!("dropping avatar connection with a bad token");
                        return None;
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    log::error!("avatar renderer did not connect within 30 s");
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                log::error!("avatar accept failed: {e}");
                return None;
            }
        }
    }
}

fn random_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    format!("{:x}{:x}{:x}", nanos, pid, nanos.rotate_left(17))
}
