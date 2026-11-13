//! Application shell: owns the Slint windows, the avatar link, timers and global input.
//!
//! Threading rule: Slint components and this `App` are `!Send`, so worker threads capture only
//! plain data and post closures back with `slint::invoke_from_event_loop`. Those closures look
//! the handle up through [`with_app`], which is only valid on the UI thread.
//!
//! The behaviour lives in child modules, all operating on the shared [`AppHandle`]:
//!
//! | Module | What |
//! |---|---|
//! | `chat` | speech, the bubble, the microphone modes, prompts, the last-chat recall |
//! | `body` | geometry, walking, dragging, edge snap, peeking, the avatar slot and hit region |
//! | `physics` | gravity, walking and jumping, as pure logic |
//! | `ticks` | every timer: motion, proactive events, idle events, music, game dodge |
//! | `menus` | mascot buttons, global hotkeys, the context menu and the quick dial |
//! | `avatar` | the renderer process, its events, and touch reactions |
//! | `studio` | the Control Center |

use slint::winit_030::{winit, WinitWindowAccessor};
use slint::{ComponentHandle, PhysicalPosition};

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::config;

pub use aria_ui::*;

mod avatar;
mod body;
mod chat;
mod menus;
mod physics;
mod studio;
mod ticks;
mod valorant;

pub(crate) use avatar::*;
pub(crate) use body::*;
pub(crate) use chat::*;
pub(crate) use menus::*;
pub use studio::{sync_studio, wire_studio};
pub(crate) use ticks::*;

pub const STAGE_BASE_W: f32 = 260.0;
pub const STAGE_BASE_H: f32 = 370.0;
/// Mutable UI-thread state. Never leaves the UI thread.
pub struct App {
    pub ui: MascotWindow,
    pub studio: Option<ControlCenter>,
    pub menu: Option<ContextMenu>,
    pub dial: Option<QuickDial>,
    pub stage: crate::avatar_link::AvatarLink,
    pub pos: (i32, i32),
    /// Where she is walking to, as a window position; only the x matters, gravity decides y.
    pub target: Option<(i32, i32)>,
    /// The window she is heading for, to stand on top of it.
    pub perch_target: Option<isize>,
    /// Her body under gravity, at the soles of her feet.
    pub body: physics::Body,
    /// From the renderer: pixels from the bottom of the avatar window to her soles, and pixels
    /// per metre. `None` until it first reports, and physics waits until then.
    pub stage_geometry: Option<(f32, f32)>,
    pub last_motion: Instant,
    pub post_walk_emotion: String,
    pub dragging: bool,
    pub drag_origin: Option<(i32, i32, i32, i32)>,
    pub peeking: String,
    pub user_placed: bool,
    pub stationary_since: Instant,
    pub clipboard_watch: bool,
    pub last_clipboard: String,
    pub game_compacted: bool,
    pub foreground_game: Option<crate::desktop::presence::Game>,
    pub pre_game_pos: Option<(i32, i32)>,
    pub last_copilot_vision: Instant,
    pub busy: bool,
    pub reaction_pending: bool,
    pub speech_generation: u64,
    /// The last real line she said and its emotion; a click on her shows it again.
    pub last_chat: Option<(String, String)>,
    /// When the next idle event may happen, and the last few she did (so the AI varies them).
    pub next_idle_event: Instant,
    pub recent_idle: Vec<String>,
    pub full_bubble_text: String,
    pub typed: usize,
    pub bubble_hide_at: Option<Instant>,
    pub last_emotion: String,
    pub speaking_until: Option<Instant>,
    pub duplex_pending: bool,
    pub persona: String,
    pub accent: (String, String, String),
    pub music_vibing: bool,
    pub last_discord: Instant,
    /// Last time the chat bar was shown or used; it auto-hides after `BAR_IDLE_HIDE` without use.
    pub bar_activity: Instant,
    /// True while the cursor is over the avatar model (drives click-through).
    pub over_model: bool,
    /// Window size, stage hole and bar visibility the mascot's hit region was last cut for.
    pub region_key: Option<(i32, i32, Vec<crate::desktop::win32::Rect>)>,
    /// Bubble slot height the window was last laid out for (logical px).
    pub bubble_slot: f32,
    /// Owned so they are dropped (and stopped) with the app instead of leaked.
    pub timers: Vec<slint::Timer>,
}

pub type AppHandle = Rc<RefCell<App>>;

/// UI work queued while another mutation was running.
type Deferred = Box<dyn FnOnce(&AppHandle)>;

thread_local! {
    static CURRENT: RefCell<Option<AppHandle>> = const { RefCell::new(None) };
}

pub fn set_current(handle: &AppHandle) {
    CURRENT.with(|slot| *slot.borrow_mut() = Some(handle.clone()));
}
/// Runs `f` against the live app handle. UI thread only; a no-op elsewhere.
pub fn with_app<R>(f: impl FnOnce(&AppHandle) -> R) -> Option<R> {
    CURRENT.with(|slot| {
        let borrowed = slot.borrow();
        borrowed.as_ref().map(f)
    })
}

thread_local! {
    /// Guards against re-entrant UI mutation: Slint may evaluate a binding that calls back into
    /// `say` or `sync_chrome` while one is already running, and a second `borrow_mut` would panic.
    static IN_MUTATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };

    /// Work that arrived while a mutation was in flight. Draining happens once the outer
    /// mutation finishes, so nested callbacks are deferred rather than silently dropped.
    static DEFERRED: RefCell<Vec<Deferred>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` with exclusive access to the app.
///
/// If a mutation is already in progress the work is queued and run immediately after the
/// current one finishes instead of being dropped.
pub fn try_with_app_mut<R>(f: impl FnOnce(&AppHandle) -> R) -> Option<R> {
    if IN_MUTATION.with(|flag| flag.get()) {
        return None;
    }
    IN_MUTATION.with(|flag| flag.set(true));
    let result = with_app(f);
    IN_MUTATION.with(|flag| flag.set(false));
    drain_deferred();
    result
}

/// Sends `f` to the UI thread from a worker. UI-thread callers should use [`defer`].
pub fn post(f: impl FnOnce(&AppHandle) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        defer(f);
    });
}

/// Same contract as [`post`], without requiring `Send` (UI-thread callers).
pub fn defer(f: impl FnOnce(&AppHandle) + 'static) {
    if IN_MUTATION.with(|flag| flag.get()) {
        DEFERRED.with(|slot| slot.borrow_mut().push(Box::new(f)));
        return;
    }
    IN_MUTATION.with(|flag| flag.set(true));
    let _ = with_app(f);
    IN_MUTATION.with(|flag| flag.set(false));
    drain_deferred();
}

fn drain_deferred() {
    loop {
        // Take one item at a time so a callback that queues more work is handled in order.
        let next = DEFERRED.with(|slot| {
            let mut queue = slot.borrow_mut();
            if queue.is_empty() {
                None
            } else {
                Some(queue.remove(0))
            }
        });
        let Some(next) = next else { return };
        if DEFERRED.with(|slot| slot.borrow().len()) > 64 {
            log::warn!("deferred UI queue is backing up; dropping the rest");
            DEFERRED.with(|slot| slot.borrow_mut().clear());
            return;
        }
        IN_MUTATION.with(|flag| flag.set(true));
        let _ = with_app(next);
        IN_MUTATION.with(|flag| flag.set(false));
    }
}
pub fn hex_to_color(hex: &str) -> slint::Color {
    let clean = hex.trim_start_matches('#');
    let parse = |range: std::ops::Range<usize>| u8::from_str_radix(&clean[range], 16).unwrap_or(0);
    if clean.len() >= 6 {
        slint::Color::from_rgb_u8(parse(0..2), parse(2..4), parse(4..6))
    } else {
        slint::Color::from_rgb_u8(56, 189, 248)
    }
}
/// Derives the wash/edge tints from the accent so chrome always agrees with her mood.
pub fn accent_tints(hex: &str) -> (String, String) {
    let clean = hex.trim_start_matches('#');
    if clean.len() < 6 {
        return ("#12232c".into(), "#1f5f7a".into());
    }
    let parse =
        |r: std::ops::Range<usize>| u8::from_str_radix(&clean[r], 16).unwrap_or(80) as f32 / 255.0;
    let (r, g, b) = (parse(0..2), parse(2..4), parse(4..6));
    let to_hex = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
    let wash = format!(
        "#{:02x}{:02x}{:02x}",
        to_hex(r * 0.13 + 0.05),
        to_hex(g * 0.13 + 0.055),
        to_hex(b * 0.13 + 0.065)
    );
    let edge = format!(
        "#{:02x}{:02x}{:02x}",
        to_hex(r * 0.52 + 0.05),
        to_hex(g * 0.52 + 0.06),
        to_hex(b * 0.52 + 0.07)
    );
    (wash, edge)
}
pub fn init() -> Result<AppHandle, slint::PlatformError> {
    let ui = MascotWindow::new()?;
    let scale = config::get().avatar_scale;
    ui.set_stage_w(STAGE_BASE_W * scale);
    ui.set_stage_h(STAGE_BASE_H * scale);
    let stage = crate::avatar_link::AvatarLink::placeholder();
    let app: AppHandle = Rc::new(RefCell::new(App {
        ui,
        studio: None,
        menu: None,
        dial: None,
        stage,
        pos: (0, 0),
        target: None,
        perch_target: None,
        body: physics::Body::default(),
        stage_geometry: None,
        last_motion: Instant::now(),
        post_walk_emotion: "idle".into(),
        dragging: false,
        drag_origin: None,
        peeking: "none".into(),
        user_placed: false,
        stationary_since: Instant::now(),
        clipboard_watch: true,
        last_clipboard: String::new(),
        game_compacted: false,
        foreground_game: None,
        pre_game_pos: None,
        last_copilot_vision: Instant::now(),
        busy: false,
        last_chat: None,
        next_idle_event: Instant::now() + Duration::from_secs(90),
        recent_idle: Vec::new(),
        reaction_pending: false,
        speech_generation: 0,
        full_bubble_text: String::new(),
        typed: 0,
        bubble_hide_at: None,
        last_emotion: "idle".into(),
        speaking_until: None,
        duplex_pending: false,
        persona: String::new(),
        accent: (String::new(), String::new(), String::new()),
        music_vibing: false,
        last_discord: Instant::now(),
        bar_activity: Instant::now(),
        over_model: false,
        region_key: None,
        bubble_slot: 0.0,
        timers: Vec::new(),
    }));
    set_current(&app);
    wire_callbacks(&app);
    apply_persona(&app);
    setup_window(&app);
    install_hotkeys();
    apply_mic_settings();
    attach_stage(&app);
    start_timers(&app);
    greeting(&app);
    sync_chrome(&app);
    Ok(app)
}

pub fn cleanup(app: &AppHandle) {
    crate::voice::shutdown();
    crate::discord::close();
    app.borrow_mut().stage.shutdown();
    config::save();
    log::info!("Aria exited");
}
