//! Discord Rich Presence over the local IPC socket.
//!
//! The Python version used pypresence; this speaks the same wire protocol directly, which
//! removes a dependency and keeps updates off the UI thread.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CLIENT_ID: &str = "1338274647188701255";
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;

static CONNECTED: AtomicBool = AtomicBool::new(false);
static LAST_UPDATE: Mutex<f64> = Mutex::new(0.0);
static START_TIME: Mutex<i64> = Mutex::new(0);

fn pipe_path(index: u32) -> String {
    format!("\\\\?\\pipe\\discord-ipc-{}", index)
}

struct Frame {
    _opcode: u32,
    json: serde_json::Value,
}

fn write_frame(stream: &mut std::fs::File, opcode: u32, payload: &str) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(8 + payload.len());
    buf.extend_from_slice(&opcode.to_le_bytes());
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload.as_bytes());
    stream.write_all(&buf)
}

fn read_frame(stream: &mut std::fs::File) -> std::io::Result<Frame> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header)?;
    let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if len > 1 << 20 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Discord frame too large",
        ));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    Ok(Frame { _opcode: opcode, json })
}

fn client_id() -> String {
    let settings = crate::config::get();
    let _ = settings;
    DEFAULT_CLIENT_ID.to_string()
}

fn unix_now_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Connects to the first available Discord IPC pipe.
fn connect() -> Option<std::fs::File> {
    let id = client_id();
    (0..10).find_map(|i| {
        let mut stream = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(pipe_path(i))
            .ok()?;
        let handshake = serde_json::json!({ "v": 1, "client_id": id }).to_string();
        write_frame(&mut stream, OP_HANDSHAKE, &handshake).ok()?;
        let frame = read_frame(&mut stream).ok()?;
        if frame.json.get("cmd").and_then(|c| c.as_str()) == Some("DISPATCH")
            && frame.json.get("evt").and_then(|e| e.as_str()) == Some("READY")
        {
            CONNECTED.store(true, Ordering::SeqCst);
            log::info!("connected to Discord Rich Presence");
            Some(stream)
        } else {
            None
        }
    })
}

pub fn is_enabled() -> bool {
    crate::config::get().discord_presence_enabled
}

pub fn set_enabled(enabled: bool) {
    crate::config::update(|s| s.discord_presence_enabled = enabled);
    if !enabled {
        close();
    }
}

/// Fire-and-forget presence update; throttled to 4 s like the original.
pub fn update(details: &str, state: &str, persona: &str) {
    if !is_enabled() {
        return;
    }
    let now = crate::util::unix_now();
    {
        let mut last = LAST_UPDATE.lock().unwrap_or_else(|e| e.into_inner());
        if now - *last < 4.0 {
            return;
        }
        *last = now;
    }
    {
        let mut start = START_TIME.lock().unwrap_or_else(|e| e.into_inner());
        if *start == 0 {
            *start = unix_now_i64();
        }
    }

    let details = details.to_string();
    let state = state.to_string();
    let persona = persona.to_string();
    let details = if details.trim().is_empty() {
        "Pair programming with Aki".to_string()
    } else {
        details.chars().take(120).collect()
    };
    let persona = if persona.trim().is_empty() {
        crate::persona::current_key()
    } else {
        persona.to_string()
    };

    std::thread::spawn(move || {
        let Some(mut stream) = connect() else {
            CONNECTED.store(false, Ordering::SeqCst);
            return;
        };
        let affiliation = crate::memory::affection();
        let state_line: String = format!(
            "{} · {affiliation}% Affection",
            if state.trim().is_empty() {
                format!("Aria v6.0 ({})", capitalize(&persona))
            } else {
                state.to_string()
            }
        )
        .chars()
        .take(120)
        .collect();

        let start = *START_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let payload = serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "args": {
                "pid": std::process::id(),
                "activity": {
                    "details": details,
                    "state": state_line,
                    "timestamps": { "start": start },
                    "assets": {
                        "large_image": "aria_avatar",
                        "large_text": format!("Aria v6.0 ({})", capitalize(&persona)),
                        "small_image": "heart",
                        "small_text": format!("Affection: {affiliation}%")
                    }
                }
            },
            "nonce": format!("{}", crate::util::unix_now())
        })
        .to_string();

        if write_frame(&mut stream, OP_FRAME, &payload).is_err() {
            CONNECTED.store(false, Ordering::SeqCst);
            return;
        }
        let _ = read_frame(&mut stream);
        if write_frame(&mut stream, OP_CLOSE, "{}").is_err() {
            CONNECTED.store(false, Ordering::SeqCst);
        }
    });
}

pub fn close() {
    CONNECTED.store(false, Ordering::SeqCst);
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pipe_paths_are_wellformed() {
        let p = super::pipe_path(0);
        assert!(p.contains("discord-ipc-0"));
    }
}
