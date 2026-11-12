//! Synthetic mouse and keyboard input, plus window focus/minimize helpers.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_MOVE, MOUSEINPUT,
    MOUSEEVENTF_ABSOLUTE, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
};

fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    unsafe {
        SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn key_input(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_input(flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS, dx: i32, dy: i32, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

pub fn press_virtual_key(vk: u16) {
    send(&[key_input(vk, false), key_input(vk, true)]);
}

/// Named key -> virtual key code, matching the names the LLM tools use.
pub fn vk_for_name(name: &str) -> Option<u16> {
    let n = name.trim().to_lowercase();
    let vk = match n.as_str() {
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "space" => 0x20,
        "backspace" => 0x08,
        "escape" | "esc" => 0x1B,
        "up" => 0x26,
        "down" => 0x28,
        "left" => 0x25,
        "right" => 0x27,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "delete" | "del" => 0x2E,
        "insert" => 0x2D,
        "playpause" | "play_pause" | "media_play_pause" => 0xB3,
        "nexttrack" | "next_track" => 0xB0,
        "prevtrack" | "previous_track" => 0xB1,
        "volumeup" | "volume_up" => 0xAF,
        "volumedown" | "volume_down" => 0xAE,
        "volumemute" | "mute" => 0xAD,
        "win" | "super" | "meta" => 0x5B,
        "shift" => 0x10,
        "ctrl" | "control" => 0x11,
        "alt" => 0x12,
        "capslock" => 0x14,
        "printscreen" => 0x2C,
        "f1" => 0x70,
        "f2" => 0x71,
        "f3" => 0x72,
        "f4" => 0x73,
        "f5" => 0x74,
        "f6" => 0x75,
        "f7" => 0x76,
        "f8" => 0x77,
        "f9" => 0x78,
        "f10" => 0x79,
        "f11" => 0x7A,
        "f12" => 0x7B,
        single if single.chars().count() == 1 => {
            let c = single.chars().next()?.to_ascii_uppercase();
            if c.is_ascii_alphanumeric() {
                if c.is_ascii_digit() {
                    0x30 + (c as u16 - b'0' as u16)
                } else {
                    c as u16
                }
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(vk)
}

pub fn press_key(name: &str) -> bool {
    match vk_for_name(name) {
        Some(vk) => {
            press_virtual_key(vk);
            true
        }
        None => false,
    }
}

pub fn hotkey(combo: &str) -> bool {
    let parts: Vec<&str> = combo
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return false;
    }
    let mut vks = Vec::new();
    for part in parts {
        match vk_for_name(part) {
            Some(vk) => vks.push(vk),
            None => return false,
        }
    }
    let mut events: Vec<INPUT> = vks.iter().map(|vk| key_input(*vk, false)).collect();
    for vk in vks.iter().rev() {
        events.push(key_input(*vk, true));
    }
    send(&events);
    true
}

/// Types text via Unicode key events, so no keymap assumptions are needed.
pub fn type_text(text: &str) -> usize {
    const KEYEVENTF_UNICODE: KEYBD_EVENT_FLAGS = KEYBD_EVENT_FLAGS(0x0004);
    let mut count = 0;
    for ch in text.chars() {
        let mut buf = [0u16; 2];
        let units = ch.encode_utf16(&mut buf);
        for unit in units.iter() {
            let down = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: *unit,
                        dwFlags: KEYEVENTF_UNICODE,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            let up = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: *unit,
                        dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            send(&[down, up]);
            count += 1;
            std::thread::sleep(std::time::Duration::from_millis(18));
        }
    }
    count
}

pub fn move_cursor(x: i32, y: i32) {
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    if w <= 0 || h <= 0 {
        return;
    }
    let nx = (x as f32 / w as f32 * 65535.0) as i32;
    let ny = (y as f32 / h as f32 * 65535.0) as i32;
    send(&[mouse_input(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, nx, ny, 0)]);
}

pub fn click(x: i32, y: i32, right: bool, double: bool) {
    move_cursor(x, y);
    std::thread::sleep(std::time::Duration::from_millis(60));
    let (down, up) = if right {
        (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
    } else {
        (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
    };
    let presses = if double { 2 } else { 1 };
    for _ in 0..presses {
        send(&[mouse_input(down, 0, 0, 0), mouse_input(up, 0, 0, 0)]);
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
}

pub fn scroll(amount: i32) {
    send(&[mouse_input(MOUSEEVENTF_WHEEL, 0, 0, amount)]);
}

/// Current cursor position in screen pixels.
pub fn cursor_position() -> (i32, i32) {
    let mut p = windows::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// `(left, right, middle)`: which mouse buttons are held right now.
///
/// The avatar window is layered and gets no input events, so the app polls the global button state
/// and forwards it. Swapped buttons are honoured (`GetAsyncKeyState` reports physical buttons).
pub fn mouse_buttons() -> (bool, bool, bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
    // The high bit means "currently down".
    let down = |vk: u16| unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 };
    let (l, r, m) = (down(VK_LBUTTON.0), down(VK_RBUTTON.0), down(VK_MBUTTON.0));
    if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 {
        (r, l, m)
    } else {
        (l, r, m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_resolve() {
        assert_eq!(vk_for_name("enter"), Some(0x0D));
        assert_eq!(vk_for_name("a"), Some(0x41));
        assert_eq!(vk_for_name("volumeup"), Some(0xAF));
        assert_eq!(vk_for_name("nonsense-key"), None);
    }
}
