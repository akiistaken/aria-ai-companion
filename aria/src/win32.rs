//! Win32 helpers: enumerate windows, map them to processes, force foreground, idle time,
//! screen capture and working-set trimming.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    SRCCOPY,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    SetProcessWorkingSetSize, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_MENU};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowLongW, GWL_EXSTYLE,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    PostMessageW, SetForegroundWindow, ShowWindow, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOW,
    WM_APPCOMMAND, WM_CLOSE,
};

const _GWL_EXSTYLE_VALUE: i32 = -20;
const WS_EX_TOOLWINDOW: i32 = 0x0000_0080;

#[derive(Debug, Clone, Serialize)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub pid: u32,
    pub title: String,
    pub class: String,
    pub exe: String,
    pub visible: bool,
}

static PROC_CACHE: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();

fn proc_cache() -> &'static Mutex<HashMap<u32, String>> {
    PROC_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Lower-cased executable name for a pid, or "" when inaccessible.
pub fn process_name(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    if let Some(name) = proc_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&pid)
    {
        return name.clone();
    }
    let name = unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .ok()
            .and_then(|handle| {
                let mut buf = [0u16; 1024];
                let mut size = buf.len() as u32;
                let ok = QueryFullProcessImageNameW(
                    handle,
                    PROCESS_NAME_WIN32,
                    PWSTR(buf.as_mut_ptr()),
                    &mut size,
                )
                .is_ok();
                let _ = CloseHandle(handle);
                if !ok {
                    return None;
                }
                let full = String::from_utf16_lossy(&buf[..size as usize]);
                full.rsplit(['\\', '/'])
                    .next()
                    .map(|s| s.to_lowercase())
            })
            .unwrap_or_default()
    };
    let mut cache = proc_cache().lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() > 512 {
        cache.clear();
    }
    cache.insert(pid, name.clone());
    name
}

pub fn enum_windows(include_hidden: bool) -> Vec<WindowInfo> {
    let mut out: Vec<WindowInfo> = Vec::new();
    let collector = &mut out as *mut Vec<WindowInfo>;
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(collector as isize));
    }
    if include_hidden {
        out
    } else {
        out.into_iter().filter(|w| w.visible).collect()
    }
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let collector = &mut *(lparam.0 as *mut Vec<WindowInfo>);
    let visible = IsWindowVisible(hwnd).as_bool();

    let len = GetWindowTextLengthW(hwnd);
    let mut title = String::new();
    if len > 0 {
        let mut buf = vec![0u16; len as usize + 1];
        let written = GetWindowTextW(hwnd, &mut buf);
        if written > 0 {
            title = String::from_utf16_lossy(&buf[..written as usize]);
        }
    }

    let mut class_buf = [0u16; 256];
    let class_len = GetClassNameW(hwnd, &mut class_buf);
    let class = if class_len > 0 {
        String::from_utf16_lossy(&class_buf[..class_len as usize])
    } else {
        String::new()
    };

    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));

    collector.push(WindowInfo {
        hwnd: hwnd.0 as isize,
        pid,
        title,
        class,
        exe: process_name(pid),
        visible,
    });
    BOOL(1)
}

pub fn find_windows(
    exe_names: Option<&[&str]>,
    title_contains: Option<&str>,
    include_hidden: bool,
    require_title: bool,
) -> Vec<WindowInfo> {
    let needle = title_contains.map(|s| s.to_lowercase());
    enum_windows(include_hidden)
        .into_iter()
        .filter(|w| {
            if let Some(exes) = exe_names {
                if !exes.iter().any(|e| w.exe == *e) {
                    return false;
                }
            }
            if let Some(n) = &needle {
                if !w.title.to_lowercase().contains(n) {
                    return false;
                }
            }
            if require_title && w.title.trim().is_empty() {
                return false;
            }
            // Skip tool/helper popups unless a title was explicitly requested.
            if needle.is_none() && w.visible {
                let style = unsafe { GetWindowLongW(HWND(w.hwnd as *mut _), GWL_EXSTYLE) };
                if style & WS_EX_TOOLWINDOW != 0 {
                    return false;
                }
            }
            true
        })
        .collect()
}

pub fn window_title(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let written = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if written <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..written as usize])
}

pub fn active_window() -> Option<WindowInfo> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let title = window_title(hwnd);
    Some(WindowInfo {
        hwnd: hwnd.0 as isize,
        pid,
        title,
        class: String::new(),
        exe: process_name(pid),
        visible: true,
    })
}

/// Raises a window above everything, bypassing Windows' foreground lock.
pub fn force_foreground(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    let h = HWND(hwnd as *mut _);
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        } else {
            let _ = ShowWindow(h, SW_SHOW);
        }

        let current_thread = GetCurrentThreadId();
        let foreground = GetForegroundWindow();
        let foreground_thread = if foreground.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(foreground, None)
        };
        let target_thread = GetWindowThreadProcessId(h, None);

        keybd_event(VK_MENU.0 as u8, 0, Default::default(), 0);
        #[link(name = "user32")]
        extern "system" {
            fn AttachThreadInput(idattach: u32, idattach_to: u32, fattach: i32) -> i32;
        }
        let mut attached = Vec::new();
        for tid in [foreground_thread, target_thread] {
            if tid != 0 && tid != current_thread && AttachThreadInput(current_thread, tid, 1) != 0 {
                attached.push(tid);
            }
        }
        let _ = BringWindowToTop(h);
        let _ = SetForegroundWindow(h);
        for tid in attached {
            AttachThreadInput(current_thread, tid, 0);
        }
        keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);

        for _ in 0..6 {
            if GetForegroundWindow() == h {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    false
}

/// Sends WM_APPCOMMAND straight to a window (media transport) without needing focus.
pub fn post_app_command(hwnd: isize, command: i32) -> bool {
    if hwnd == 0 {
        return false;
    }
    unsafe {
        PostMessageW(
            Some(HWND(hwnd as *mut _)),
            WM_APPCOMMAND,
            WPARAM(hwnd as usize),
            LPARAM((command as isize) << 16),
        )
        .is_ok()
    }
}

/// Minimize / maximize / restore / close / focus a window by handle.
pub fn show_window(hwnd: isize, action: &str) -> bool {
    let h = HWND(hwnd as *mut _);
    unsafe {
        match action {
            "minimize" => {
                let _ = ShowWindow(h, SW_MINIMIZE);
                true
            }
            "maximize" => {
                let _ = ShowWindow(h, SW_MAXIMIZE);
                true
            }
            "close" => PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok(),
            _ => {
                if IsIconic(h).as_bool() {
                    let _ = ShowWindow(h, SW_RESTORE);
                }
                force_foreground(hwnd)
            }
        }
    }
}

pub fn window_rect(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
    }
    let mut rect = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        None
    } else {
        Some((
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        ))
    }
}

#[repr(C)]
struct LastInputInfo {
    cb_size: u32,
    dw_time: u32,
}

/// Seconds since the last mouse move or key press.
pub fn idle_seconds() -> f64 {
    #[link(name = "user32")]
    extern "system" {
        fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount() -> u32;
    }
    let mut info = LastInputInfo {
        cb_size: std::mem::size_of::<LastInputInfo>() as u32,
        dw_time: 0,
    };
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return 0.0;
    }
    let millis = unsafe { GetTickCount() }.wrapping_sub(info.dw_time);
    f64::from(millis) / 1000.0
}

pub fn screen_size() -> (i32, i32, i32, i32) {
    #[link(name = "user32")]
    extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
    }
    const SM_CXSCREEN: i32 = 0;
    const SM_CYSCREEN: i32 = 1;
    unsafe { (0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

pub fn work_area() -> (i32, i32, i32, i32) {
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, pv: *mut Rect, winini: u32) -> i32;
    }
    const SPI_GETWORKAREA: u32 = 0x0030;
    let mut rect = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut rect, 0) } == 0 {
        return screen_size();
    }
    (
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    )
}

pub struct Capture {
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Captures the primary monitor as RGB pixels, downscaled to `max_width`.
pub fn capture_screen(max_width: u32) -> Option<Capture> {
    unsafe {
        let (_, _, mw, mh) = screen_size();
        if mw <= 0 || mh <= 0 {
            return None;
        }
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return None;
        }
        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        let bitmap = CreateCompatibleBitmap(screen_dc, mw, mh);
        if bitmap.is_invalid() {
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
            return None;
        }
        let old = SelectObject(mem_dc, HGDIOBJ(bitmap.0));

        let blit_ok = BitBlt(mem_dc, 0, 0, mw, mh, Some(screen_dc), 0, 0, SRCCOPY).is_ok();

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: mw,
                biHeight: -mh, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (mw * mh * 4) as usize];
        let lines = if blit_ok {
            GetDIBits(
                mem_dc,
                bitmap,
                0,
                mh as u32,
                Some(buf.as_mut_ptr() as *mut _),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        SelectObject(mem_dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);

        if lines == 0 {
            return None;
        }

        // BGRA -> RGB.
        let mut rgb = Vec::with_capacity((mw * mh * 3) as usize);
        for px in buf.chunks_exact(4) {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }

        let (out_w, out_h, out) = if mw as u32 > max_width {
            let ratio = max_width as f32 / mw as f32;
            let target_h = ((mh as f32) * ratio).round().max(1.0) as u32;
            let img = image::RgbImage::from_raw(mw as u32, mh as u32, rgb)
                .unwrap_or_else(|| image::RgbImage::new(mw as u32, mh as u32));
            let resized = image::imageops::resize(
                &img,
                max_width,
                target_h,
                image::imageops::FilterType::Triangle,
            );
            (
                resized.width(),
                resized.height(),
                resized.into_raw(),
            )
        } else {
            (mw as u32, mh as u32, rgb)
        };

        Some(Capture {
            rgb: out,
            width: out_w,
            height: out_h,
        })
    }
}

pub fn capture_screen_jpeg_b64(max_width: u32) -> Option<String> {
    let cap = capture_screen(max_width)?;
    let img = image::RgbImage::from_raw(cap.width, cap.height, cap.rgb)?;
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80);
    encoder.encode_image(&img).ok()?;
    Some(base64_encode(&buf.into_inner()))
}

pub fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18 & 63) as usize] as char);
        out.push(TABLE[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Asks Windows to trim unused physical RAM from the working set.
pub fn trim_working_set() -> bool {
    unsafe {
        let process = GetCurrentProcess();
        SetProcessWorkingSetSize(process, usize::MAX, usize::MAX).is_ok()
    }
}

// TODO(wire-up): kept as a small Win32 predicate for future HWND checks.
#[allow(dead_code)]
pub fn handle_is_null(h: HANDLE) -> bool {
    h.0.is_null()
}

/// True while the workstation is locked (the lock screen owns the foreground).
///
/// Cheap enough for a once-a-second policy check: no session notifications to register for.
pub fn session_locked() -> bool {
    let Some(win) = active_window() else {
        // No foreground window at all: the secure desktop is up.
        return true;
    };
    win.exe.eq_ignore_ascii_case("LockApp.exe")
}

/// Gives a window a hit/paint region of its full rect minus one hole `(x, y, w, h)`, in window
/// pixels. Clicks inside the hole fall through to whatever is underneath.
pub fn set_region_with_hole(hwnd: isize, width: i32, height: i32, hole: (i32, i32, i32, i32)) {
    use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_DIFF};
    if hwnd == 0 || width <= 0 || height <= 0 {
        return;
    }
    unsafe {
        let full = CreateRectRgn(0, 0, width, height);
        let cut = CreateRectRgn(hole.0, hole.1, hole.0 + hole.2, hole.1 + hole.3);
        CombineRgn(Some(full), Some(full), Some(cut), RGN_DIFF);
        let _ = DeleteObject(cut.into());
        // On success the system owns `full`; it must not be deleted here.
        if SetWindowRgn(HWND(hwnd as *mut _), Some(full), true) == 0 {
            let _ = DeleteObject(full.into());
        }
    }
}

/// Enables or disables click-through on the avatar window.
///
/// `WS_EX_TRANSPARENT` makes the window let mouse input fall through to whatever is beneath it,
/// which is what turns the empty space around the model into normal desktop.
pub fn set_click_through(hwnd: isize, enabled: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetWindowLongW, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_TRANSPARENT,
    };
    if hwnd == 0 {
        return;
    }
    unsafe {
        let handle = HWND(hwnd as *mut _);
        let style = GetWindowLongW(handle, GWL_EXSTYLE);
        let transparent = WS_EX_TRANSPARENT.0 as i32;
        let layered = WS_EX_LAYERED.0 as i32;
        let updated = if enabled {
            style | transparent | layered
        } else {
            style & !transparent
        };
        if updated != style {
            SetWindowLongW(handle, GWL_EXSTYLE, updated);
        }
    }
}

/// Puts a child process in a job object that kills it when this process dies.
///
/// The handle is deliberately leaked: the job must stay open for the lifetime of the app, and
/// Windows closes it (killing the child) when the process exits for any reason.
pub fn put_in_kill_on_close_job(child: &mut std::process::Child) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
        JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    unsafe {
        let Ok(job) = CreateJobObjectW(None, None) else {
            log::warn!("could not create the avatar job object");
            return;
        };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .is_err()
        {
            log::warn!("could not set KILL_ON_JOB_CLOSE on the avatar job");
            let _ = CloseHandle(job);
            return;
        }

        let Ok(process) = OpenProcess(
            PROCESS_SET_QUOTA | PROCESS_TERMINATE,
            false,
            child.id(),
        ) else {
            log::warn!("could not open the avatar process for job assignment");
            let _ = CloseHandle(job);
            return;
        };
        if AssignProcessToJobObject(job, process).is_err() {
            log::warn!("could not assign the avatar to its job object");
        }
        let _ = CloseHandle(process);
        // `job` is intentionally not closed; see the doc comment.
        std::mem::forget(job);
        log::info!("avatar renderer assigned to a kill-on-close job object");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vector() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn enumeration_returns_window_handles() {
        let windows = enum_windows(false);
        assert!(windows.iter().all(|w| w.hwnd != 0));
    }

    #[test]
    fn screen_capture_produces_pixels() {
        if let Some(cap) = capture_screen(320) {
            assert!(cap.width <= 320);
            assert_eq!(cap.rgb.len(), (cap.width * cap.height * 3) as usize);
        }
    }

    #[test]
    fn work_area_is_sane() {
        let (_, _, w, h) = work_area();
        assert!(w > 0 && h > 0);
    }
}
