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
//!
//! The window never takes focus (`WS_EX_NOACTIVATE`, `MA_NOACTIVATE`): clicking Aria must not pull
//! the keyboard away from whatever the user is typing into.

use std::sync::atomic::{AtomicI64, AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP,
    HDC, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetWindowRect, PeekMessageW, RegisterClassW,
    SWP_NOZORDER,
    SetWindowPos, ShowWindow, TranslateMessage, UpdateLayeredWindow, CW_USEDEFAULT, HWND_TOPMOST,
    MA_NOACTIVATE, MSG, PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SW_HIDE, SW_SHOWNA, ULW_ALPHA, WM_MOUSEACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_POPUP,
};

/// The window's HWND, reported to the app; 0 until the window exists.
pub static HWND_OUT: AtomicIsize = AtomicIsize::new(0);

/// Where the app wants the window, packed as `(x << 32) | (y as u32)`. Written by the socket
/// thread the moment a `SetWindowRect` arrives, applied by the window thread within ~15 ms.
static WANTED_POS: AtomicI64 = AtomicI64::new(i64::MIN);

/// Requests a window position. The window thread is the only mover, so `UpdateLayeredWindow` and
/// the move can never race each other.
pub fn place(x: i32, y: i32) {
    WANTED_POS.store(((x as i64) << 32) | (y as u32 as i64), Ordering::SeqCst);
}

fn wanted_pos() -> Option<(i32, i32)> {
    let packed = WANTED_POS.load(Ordering::SeqCst);
    (packed != i64::MIN).then(|| ((packed >> 32) as i32, packed as i32))
}

/// Messages the renderer sends to the window thread.
enum Present {
    Frame { rgba: Vec<u8>, width: u32, height: u32 },
    Show,
    Hide,
    Stop,
}

/// Handle used by the renderer to publish frames.
pub struct LayeredSurface {
    tx: Sender<Present>,
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
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
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
        Some(Self { tx })
    }

    /// Publishes one straight-alpha RGBA8 frame. Ownership moves to the window thread, so the
    /// renderer never copies a frame twice.
    pub fn present_rgba(&mut self, rgba: Vec<u8>, width: u32, height: u32) {
        if rgba.len() < (width * height * 4) as usize || width == 0 || height == 0 {
            return;
        }
        let _ = self.tx.send(Present::Frame { rgba, width, height });
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

/// A top-down 32-bit DIB selected into a memory DC.
struct Dib {
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
}

impl Dib {
    fn create(mem_dc: HDC, width: i32, height: i32) -> Option<Dib> {
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
        let bitmap =
            unsafe { CreateDIBSection(Some(mem_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) };
        match bitmap {
            Ok(b) if !b.is_invalid() && !bits.is_null() => {
                let old = unsafe { SelectObject(mem_dc, HGDIOBJ(b.0)) };
                Some(Dib {
                    bitmap: b,
                    old,
                    bits: bits as *mut u8,
                    width,
                    height,
                })
            }
            _ => None,
        }
    }

    fn pixels(&mut self) -> &mut [u8] {
        unsafe {
            std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize)
        }
    }

    fn destroy(self, mem_dc: HDC) {
        unsafe {
            let _ = SelectObject(mem_dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
        }
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
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
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
    let mut dib = Dib::create(mem_dc, width, height);
    if dib.is_none() {
        log::error!("layered: CreateDIBSection failed");
        return;
    }
    HWND_OUT.store(hwnd.0 as isize, Ordering::SeqCst);
    log::info!("layered window ready (hwnd={})", hwnd.0 as isize);

    let mut msg = MSG::default();
    let mut visible = false;
    let mut hidden = false;
    // The position this thread last gave the window; the only source of truth for it.
    let mut pos = {
        let mut rect = windows::Win32::Foundation::RECT::default();
        unsafe {
            let _ = GetWindowRect(hwnd, &mut rect);
        }
        (rect.left, rect.top)
    };

    loop {
        // Follow the app's requested position immediately, not only on the next frame, so the
        // avatar keeps up with its chrome while walking or being dragged.
        if let Some(wanted) = wanted_pos().filter(|w| *w != pos) {
            pos = wanted;
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    pos.0,
                    pos.1,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }

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

        // Block briefly for work instead of spinning: a paused avatar costs nothing here.
        match rx.recv_timeout(std::time::Duration::from_millis(15)) {
            Ok(Present::Frame { rgba, width: w, height: h }) => {
                if hidden {
                    continue;
                }
                let stale = dib
                    .as_ref()
                    .is_none_or(|d| d.width != w as i32 || d.height != h as i32);
                if stale {
                    // Release the old bitmap *before* selecting the new one: each `Dib` restores
                    // whatever was selected before it, so the other order deselects the new
                    // bitmap and every later `UpdateLayeredWindow` fails with error 31.
                    if let Some(old) = dib.take() {
                        old.destroy(mem_dc);
                    }
                    dib = Dib::create(mem_dc, w as i32, h as i32);
                }
                let Some(d) = dib.as_mut() else {
                    log::warn!("layered: could not create a {w}x{h} bitmap");
                    continue;
                };
                premultiply_bgra(&rgba, d.pixels());
                blit(hwnd, screen_dc, mem_dc, d.width, d.height, pos);
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
            Ok(Present::Show) => {
                hidden = false;
                if visible {
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_SHOWNA);
                    }
                }
            }
            Ok(Present::Hide) => unsafe {
                hidden = true;
                let _ = ShowWindow(hwnd, SW_HIDE);
            },
            Ok(Present::Stop) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    if let Some(dib) = dib {
        dib.destroy(mem_dc);
    }
    unsafe {
        let _ = DeleteDC(mem_dc);
        let _ = DeleteDC(screen_dc);
    }
    HWND_OUT.store(0, Ordering::SeqCst);
}

/// Straight RGBA8 -> premultiplied BGRA.
///
/// Without premultiplication, semi-transparent edges bloom against the desktop. Fully transparent
/// pixels (most of the frame) take the cheap branch.
fn premultiply_bgra(src: &[u8], dst: &mut [u8]) {
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        let a = s[3] as u32;
        match a {
            0 => d.copy_from_slice(&[0, 0, 0, 0]),
            255 => {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = 255;
            }
            _ => {
                // (x * a + 127) / 255 without a division.
                let mul = |x: u8| {
                    let t = x as u32 * a + 128;
                    ((t + (t >> 8)) >> 8) as u8
                };
                d[0] = mul(s[2]);
                d[1] = mul(s[1]);
                d[2] = mul(s[0]);
                d[3] = s[3];
            }
        }
    }
}

/// Pushes the DIB to the screen at `pos`, which this thread also used for its last `SetWindowPos`,
/// so the two can never disagree.
fn blit(hwnd: HWND, screen_dc: HDC, mem_dc: HDC, width: i32, height: i32, pos: (i32, i32)) {
    let size = SIZE {
        cx: width,
        cy: height,
    };
    let dst = POINT { x: pos.0, y: pos.1 };
    let src = POINT { x: 0, y: 0 };
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
            Some(&dst),
            Some(&size),
            Some(mem_dc),
            Some(&src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    };
    if let Err(e) = result {
        log::warn!("layered: UpdateLayeredWindow failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::premultiply_bgra;

    #[test]
    fn premultiply_matches_exact_division() {
        for a in [0u8, 1, 64, 128, 200, 254, 255] {
            for c in [0u8, 1, 100, 200, 255] {
                let src = [c, c / 2, 255 - c, a];
                let mut dst = [0u8; 4];
                premultiply_bgra(&src, &mut dst);
                let exact = |x: u8| ((x as u32 * a as u32 + 127) / 255) as u8;
                assert!((dst[0] as i32 - exact(src[2]) as i32).abs() <= 1);
                assert!((dst[2] as i32 - exact(src[0]) as i32).abs() <= 1);
                assert_eq!(dst[3], a);
            }
        }
    }
}
