//! The notification-area (tray) icon: Aria lives here instead of on the taskbar.
//!
//! A hidden window on the UI thread receives the icon's mouse messages; winit's message pump
//! dispatches them, so no extra thread or loop is needed. Left click brings up the chat bar,
//! right click opens a small native menu. Actions are handed to the app with [`crate::app::post`]
//! so they never run inside a borrow of the app state.

use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyMenu, GetCursorPos, PostMessageW, RegisterClassW, RegisterWindowMessageW,
    SetForegroundWindow, TrackPopupMenu, HICON, ICONINFO, MF_SEPARATOR, MF_STRING,
    TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WM_APP, WM_CONTEXTMENU,
    WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};

const CALLBACK_MSG: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;

const CMD_SHOW: usize = 1;
const CMD_STUDIO: usize = 2;
const CMD_RESET: usize = 3;
const CMD_LOG: usize = 4;
const CMD_QUIT: usize = 5;

static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static TRAY_ICON: AtomicIsize = AtomicIsize::new(0);
/// `TaskbarCreated`: broadcast when Explorer restarts, after which the icon must be re-added.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// Creates the tray icon. Must run on the UI thread, once the event loop exists.
pub fn install() {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            log::warn!("tray: no module handle");
            return;
        };
        let class = w!("AriaTrayWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);

        // A hidden top-level window (not message-only, which would miss `TaskbarCreated`).
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("Aria Tray"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module.into()),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                log::warn!("tray: could not create its window: {e}");
                return;
            }
        };
        TRAY_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        if let Some(icon) = make_icon() {
            TRAY_ICON.store(icon.0 as isize, Ordering::Relaxed);
        }
        add_icon(hwnd);
    }
}

/// Removes the icon so no ghost is left in the tray after exit.
pub fn remove() {
    let hwnd = TRAY_HWND.swap(0, Ordering::Relaxed);
    if hwnd == 0 {
        return;
    }
    let data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: HWND(hwnd as *mut _),
        uID: ICON_ID,
        ..Default::default()
    };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

fn add_icon(hwnd: HWND) {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_MESSAGE | NIF_TIP | NIF_ICON,
        uCallbackMessage: CALLBACK_MSG,
        hIcon: HICON(TRAY_ICON.load(Ordering::Relaxed) as *mut _),
        ..Default::default()
    };
    let tip: Vec<u16> = "Aria".encode_utf16().collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);
    // Explorer may not be ready yet right after login; retry briefly. If it never is,
    // `TaskbarCreated` re-adds the icon once the taskbar appears.
    for attempt in 0..5 {
        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
            log::info!("tray icon installed");
            return;
        }
        if attempt < 4 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
    log::warn!(
        "tray: Shell_NotifyIcon(NIM_ADD) failed: {}",
        windows::core::Error::from_thread()
    );
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == CALLBACK_MSG {
        match lparam.0 as u32 {
            WM_LBUTTONUP => run(CMD_SHOW),
            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
            _ => {}
        }
        return LRESULT(0);
    }
    let taskbar_created = TASKBAR_CREATED.load(Ordering::Relaxed);
    if taskbar_created != 0 && msg == taskbar_created {
        add_icon(hwnd);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn show_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let items: [(usize, PCWSTR); 4] = [
        (CMD_SHOW, w!("Talk to Aria")),
        (CMD_STUDIO, w!("Control Center")),
        (CMD_RESET, w!("Reset pose")),
        (CMD_LOG, w!("Open log")),
    ];
    for (id, label) in items {
        let _ = AppendMenuW(menu, MF_STRING, id, label);
    }
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit"));

    let mut cursor = POINT::default();
    let _ = GetCursorPos(&mut cursor);
    // Without the foreground switch the menu would not close when clicking elsewhere.
    let _ = SetForegroundWindow(hwnd);
    let chosen = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
        cursor.x,
        cursor.y,
        None,
        hwnd,
        None,
    );
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);
    if chosen.0 != 0 {
        run(chosen.0 as usize);
    }
}

fn run(command: usize) {
    match command {
        CMD_SHOW => crate::app::post(|app| {
            use slint::ComponentHandle;
            let _ = app.borrow().ui.window().show();
            crate::app::show_bar(app);
        }),
        CMD_STUDIO => crate::app::post(crate::app::open_studio),
        CMD_RESET => crate::app::post(crate::app::reset_pose),
        CMD_LOG => {
            crate::logger::open_log_file();
        }
        // Same exit path as the context menu's Quit.
        CMD_QUIT => {
            remove();
            let _ = slint::quit_event_loop();
            std::process::exit(0);
        }
        _ => {}
    }
}

/// Draws the tray icon: a pink disc with a white heart, 32x32 with antialiased alpha.
fn make_icon() -> Option<HICON> {
    const SIZE: i32 = 32;
    const SAMPLES: i32 = 4;
    let mut pixels = vec![0u32; (SIZE * SIZE) as usize];
    for py in 0..SIZE {
        for px in 0..SIZE {
            let (mut disc, mut heart) = (0.0f32, 0.0f32);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    // Centre-origin coordinates in [-1, 1], y up.
                    let x = ((px * SAMPLES + sx) as f32 + 0.5) / (SIZE * SAMPLES) as f32 * 2.0 - 1.0;
                    let y = 1.0 - ((py * SAMPLES + sy) as f32 + 0.5) / (SIZE * SAMPLES) as f32 * 2.0;
                    if x * x + y * y <= 0.96 {
                        disc += 1.0;
                        // Heart curve (x²+y²-1)³ - x²y³ ≤ 0, scaled into the disc.
                        let (hx, hy) = (x * 1.75, y * 1.75 + 0.25);
                        let r = hx * hx + hy * hy - 1.0;
                        if r * r * r - hx * hx * hy * hy * hy <= 0.0 {
                            heart += 1.0;
                        }
                    }
                }
            }
            let n = (SAMPLES * SAMPLES) as f32;
            let alpha = disc / n;
            if alpha <= 0.0 {
                continue;
            }
            let white = if disc > 0.0 { heart / disc } else { 0.0 };
            let mix = |pink: f32| (pink + (255.0 - pink) * white) * alpha;
            let (r, g, b) = (mix(236.0), mix(92.0), mix(150.0));
            // Premultiplied BGRA.
            pixels[(py * SIZE + px) as usize] = ((alpha * 255.0) as u32) << 24
                | (r as u32) << 16
                | (g as u32) << 8
                | b as u32;
        }
    }

    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE,
                biHeight: -SIZE,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u32, pixels.len());
        let mask = CreateBitmap(SIZE, SIZE, 1, 1, None);
        let icon_info = ICONINFO {
            fIcon: true.into(),
            hbmMask: mask,
            hbmColor: color,
            ..Default::default()
        };
        let icon = CreateIconIndirect(&icon_info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}
