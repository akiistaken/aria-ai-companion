//! Per-pixel-alpha presentation through a Win32 layered window.
//!
//! wgpu only offers `CompositeAlphaMode::Opaque` for a plain Win32 HWND on both the D3D12 and
//! Vulkan backends (`wgpu-hal::dx12::adapter`), so a swap chain can never composite the 3D scene
//! over the desktop. The documented fallback is therefore in use: the scene renders offscreen, is
//! read back to the CPU, and is pushed to the desktop with `UpdateLayeredWindow`, which supports
//! per-pixel alpha on every Windows version.
//!
//! The window lives on its **own thread** with a real message loop. A Win32 window whose queue is
//! never pumped cannot service the synchronous messages `UpdateLayeredWindow` sends, and pumping it
//! from the render loop is not possible because the call happens inside that same loop.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, AC_SRC_ALPHA,
    AC_SRC_OVER,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetWindowRect, PeekMessageW,
    RegisterClassW, SetWindowPos, ShowWindow, TranslateMessage, UpdateLayeredWindow, CW_USEDEFAULT,
    HWND_TOPMOST, MSG, PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE,
    SW_SHOWNA, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_POPUP,
};

/// The HWND the main app moves with `SetWindowPos`; 0 until the window exists.
pub static HWND_OUT: AtomicIsize = AtomicIsize::new(0);

/// Messages the renderer sends to the window thread.
enum Present {
    Frame(Vec<u8>),
    Show,
    Hide,
    Stop,
}

/// Handle used by the renderer to publish frames.
pub struct LayeredSurface {
    tx: Sender<Present>,
    width: u32,
    height: u32,
}

fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl LayeredSurface {
    /// Starts the window thread. The HWND appears in [`HWND_OUT`] shortly after.
    pub fn spawn(width: u32, height: u32, title: &str) -> Option<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        let title = title.to_string();
        std::thread::Builder::new()
            .name("avatar-window".into())
            .spawn(move || window_thread(width as i32, height as i32, &title, rx))
            .ok()?;
        Some(Self { tx, width, height })
    }

    /// Publishes one straight-alpha RGBA8 frame.
    pub fn present_rgba(&mut self, rgba: &[u8]) {
        let expected = (self.width * self.height * 4) as usize;
        if rgba.len() < expected {
            return;
        }
        // Dropping a frame is better than stalling the renderer behind the window thread.
        let _ = self.tx.send(Present::Frame(rgba[..expected].to_vec()));
    }

    pub fn show(&mut self) {
        let _ = self.tx.send(Present::Show);
    }

    pub fn hide(&mut self) {
        let _ = self.tx.send(Present::Hide);
    }

    pub fn hwnd_value() -> u64 {
        HWND_OUT.load(Ordering::SeqCst) as u64
    }
}

impl Drop for LayeredSurface {
    fn drop(&mut self) {
        let _ = self.tx.send(Present::Stop);
        HWND_OUT.store(0, Ordering::SeqCst);
    }
}

fn window_thread(width: i32, height: i32, title: &str, rx: Receiver<Present>) {
    let Ok(module) = (unsafe { GetModuleHandleW(None) }) else {
        log::error!("layered: no module handle");
        return;
    };
    let class_name = w!("AriaAvatarWindow");
    let title_wide = to_wide(title);

    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: module.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    // A second registration in one session fails harmlessly ("class already exists").
    unsafe {
        RegisterClassW(&wc);
    }

    let hwnd = match unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            class_name,
            PCWSTR(title_wide.as_ptr()),
            WS_POPUP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            width,
            height,
            None,
            None,
            Some(module.into()),
            None,
        )
    } {
        Ok(h) => h,
        Err(e) => {
            log::error!("layered: CreateWindowExW failed: {e}");
            return;
        }
    };

    let screen_dc = unsafe { CreateCompatibleDC(None) };
    let mem_dc = unsafe { CreateCompatibleDC(None) };
    if screen_dc.is_invalid() || mem_dc.is_invalid() {
        log::error!("layered: could not create device contexts");
        return;
    }

    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative height gives top-down rows, matching image memory order.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let bitmap = unsafe { CreateDIBSection(Some(mem_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) };
    let bitmap = match bitmap {
        Ok(b) if !b.is_invalid() && !bits.is_null() => b,
        _ => {
            log::error!("layered: CreateDIBSection failed");
            return;
        }
    };
    let old_bitmap = unsafe { SelectObject(mem_dc, HGDIOBJ(bitmap.0)) };
    HWND_OUT.store(hwnd.0 as isize, Ordering::SeqCst);
    log::info!("layered window ready (hwnd={})", hwnd.0 as isize);

    let dst = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, (width * height * 4) as usize) };
    let mut msg = MSG::default();
    let mut visible = false;

    loop {
        // Drain the window's messages first: this queue must stay serviced for
        // `UpdateLayeredWindow` to return.
        loop {
            let has = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) };
            if !has.as_bool() {
                break;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // Then take at most one frame, so messages are never starved for long.
        match rx.try_recv() {
            Ok(Present::Frame(rgba)) => {
                premultiply_bgra(&rgba, dst);
                blit(hwnd, screen_dc, mem_dc, width, height);
                if !visible {
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_SHOWNA);
                        let _ = SetWindowPos(
                            hwnd,
                            Some(HWND_TOPMOST),
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        );
                    }
                    visible = true;
                }
            }
            Ok(Present::Show) => unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNA);
            },
            Ok(Present::Hide) => unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            },
            Ok(Present::Stop) | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }

    unsafe {
        let _ = SelectObject(mem_dc, old_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem_dc);
        let _ = DeleteDC(screen_dc);
    }
    HWND_OUT.store(0, Ordering::SeqCst);
}

/// Straight RGBA8 -> premultiplied BGRA, in place.
///
/// Without premultiplication, semi-transparent edges bloom against the desktop.
fn premultiply_bgra(src: &[u8], dst: &mut [u8]) {
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        let a = s[3] as u32;
        d[0] = ((s[2] as u32 * a) / 255) as u8;
        d[1] = ((s[1] as u32 * a) / 255) as u8;
        d[2] = ((s[0] as u32 * a) / 255) as u8;
        d[3] = s[3];
    }
}

fn blit(hwnd: HWND, screen_dc: HDC, mem_dc: HDC, width: i32, height: i32) {
    let size = SIZE {
        cx: width,
        cy: height,
    };
    let src = POINT { x: 0, y: 0 };
    let mut rect = windows::Win32::Foundation::RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    }
    let window_pos = POINT {
        x: rect.left,
        y: rect.top,
    };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };

    let result = unsafe {
        UpdateLayeredWindow(
            hwnd,
            Some(screen_dc),
            Some(&window_pos),
            Some(&size),
            Some(mem_dc),
            Some(&src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    };
    if result.is_err() {
        log::warn!("layered: UpdateLayeredWindow failed");
    }
}

#[allow(dead_code)]
fn _keep(_: BOOL) {}
