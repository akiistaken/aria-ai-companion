//! Aria — autonomous desktop companion.
//!
//! Rust + Slint port of the original Python assistant. Slint draws every surface except the
//! avatar, which is the separate `aria-avatar` process (Bevy + VRM) driven over a local socket
//! by `avatar_link`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod avatar_link;
mod audio;
mod brain;
mod config;
mod discord;
mod input;
mod launcher;
mod logger;
mod memory;
mod mood;
mod offline;
mod proactive;
mod reactions;
mod sysinfo;
mod tools;
mod util;
mod voice;
mod wardrobe;
mod win32;

use slint::ComponentHandle;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    logger::init();
    config::init();

    // Two TLS backends arrive transitively (ring and aws-lc-rs); pick one explicitly so
    // rustls never tries to infer a default at first use.
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        log::debug!("a rustls crypto provider was already installed");
    }

    let pid_file = config::data_dir().join("aria.pid");
    ensure_single_instance(&pid_file);
    let _ = std::fs::write(&pid_file, std::process::id().to_string());

    let ui = app::init()?;
    ui.borrow().ui.show()?;

    // Styling the native window must happen inside the running event loop: touching the frame
    // before it starts leaves winit waiting on its own window creation. The timer is kept alive
    // by the leaked Box only because the event loop outlives this scope by design.
    let style_timer = Box::new(slint::Timer::default());
    style_timer.start(
        slint::TimerMode::SingleShot,
        std::time::Duration::from_millis(250),
        || setup_native_window("Aria"),
    );
    let _style_timer = style_timer;

    slint::run_event_loop_until_quit()?;

    app::cleanup(&ui);
    let _ = std::fs::remove_file(&pid_file);
    Ok(())
}

/// Makes the mascot window a frameless, truly transparent floating surface.
///
/// `winit`'s transparent flag alone leaves an opaque backdrop on Windows because the window is
/// still painted with the system brush; extending the DWM frame across the whole client area is
/// what actually lets the Slint chrome and the avatar composite over the desktop.
fn setup_native_window(window_title: &str) {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, SetWindowLongW,
        SetWindowPos, GWL_STYLE, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, WS_CAPTION, WS_SYSMENU, WS_THICKFRAME,
    };

    struct Search {
        pid: u32,
        title: Vec<u16>,
        found: HWND,
    }

    unsafe extern "system" fn find(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != search.pid {
            return BOOL(1);
        }
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len <= 0 {
            return BOOL(1);
        }
        let title = String::from_utf16_lossy(&buf[..len as usize]);
        if title == String::from_utf16_lossy(&search.title[..search.title.len() - 1]) {
            search.found = hwnd;
            return BOOL(0);
        }
        BOOL(1)
    }

    let mut search = Search {
        pid: std::process::id(),
        title: window_title.encode_utf16().chain(std::iter::once(0)).collect(),
        found: HWND::default(),
    };
    unsafe {
        let _ = EnumWindows(Some(find), LPARAM(&mut search as *mut Search as isize));
        let hwnd = search.found;
        if hwnd.0.is_null() {
            log::warn!("could not find the mascot window to style");
            return;
        }

        // Frame: no caption, no resize border, no system menu.
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        let stripped = style
            & !(WS_CAPTION.0 as i32 | WS_THICKFRAME.0 as i32 | WS_SYSMENU.0 as i32);
        SetWindowLongW(hwnd, GWL_STYLE, stripped);

        // Transparency: a zero-margin frame with the composition attribute is what lets Windows
        // composite this window's client area over the desktop. Extending a -1 margin frame would
        // instead make DWM own the client area, which paints over WebView2's transparent child.
        // Note: WS_EX_LAYERED is deliberately NOT set. WebView2 composites its own transparent
        // child surface, and a layered parent suppresses that path entirely.

        // Topmost here too: winit's always-on-top request in `setup_window` runs before the
        // window exists and never takes effect.
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        log::info!("native window styled: frameless + transparent");
    }
}

/// Terminates a previous instance recorded in the PID file.
///
/// This must never delay startup, so it terminates the handle directly instead of shelling out
/// to `taskkill`, which can block while the old process tears down its WebView children.
fn ensure_single_instance(pid_file: &std::path::Path) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    let Ok(existing) = std::fs::read_to_string(pid_file) else {
        return;
    };
    let Ok(old_pid) = existing.trim().parse::<u32>() else {
        return;
    };
    if old_pid == 0 || old_pid == std::process::id() {
        return;
    }

    unsafe {
        match OpenProcess(PROCESS_TERMINATE, false, old_pid) {
            Ok(handle) => {
                log::info!("terminating previous instance (pid={old_pid})");
                let _ = TerminateProcess(handle, 0);
                let _ = CloseHandle(handle);
            }
            // No handle means the old process is already gone; nothing to do.
            Err(_) => log::debug!("previous instance pid={old_pid} is not running"),
        }
    }
}
