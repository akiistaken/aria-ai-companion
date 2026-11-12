//! Cua Driver: background desktop control over MCP.
//!
//! [Cua Driver](https://cua.ai/docs/cua-driver) clicks, types and reads apps through UI
//! Automation without moving the user's cursor or stealing focus. When it is installed (its
//! daemon runs at sign-in), this module starts `cua-driver mcp` as a child process and speaks
//! MCP's JSON-RPC over its stdio, one request at a time.
//!
//! It is used two ways:
//! - a curated set of its tools is offered to the model (see [`model_tools`]), replacing the
//!   pixel-guessing mouse and keyboard tools;
//! - [`windows`] feeds presence and game detection with every on-screen window, its owning exe,
//!   bounds and z-order.
//!
//! Everything degrades: no driver, a dead daemon or a timeout all read as "unavailable", and
//! callers fall back to plain Win32.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2025-06-18";
const CALL_TIMEOUT: Duration = Duration::from_secs(25);
/// After a failed start, wait this long before trying again.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// The driver tools the model gets: find the window, read its accessibility tree, act on it.
/// Every declaration is sent with every request, so this is deliberately short; opening apps
/// and sites stays with Aria's own `open_app_or_website`.
const MODEL_TOOLS: &[&str] =
    &["list_windows", "get_window_state", "click", "type_text", "press_key", "hotkey", "scroll", "bring_to_front"];

/// How to use them, for the system prompt.
pub const USAGE: &str = "\
DESKTOP DRIVER (the `desktop_*` tools act in the background without moving the user's cursor):
1. `desktop_list_windows` with an `app` filter to get a window's `pid` and `window_id`.
2. `desktop_get_window_state` with a `query` to read only the matching part of its UI tree.
3. Act on an element by its `element_token` from that read (`desktop_click`, `desktop_type_text`),
   or with keys (`desktop_press_key`, `desktop_hotkey`). Use pixels only for things missing from the tree.
4. Re-read the window state to check the action worked before telling the user it did.";

/// Names the model sees are prefixed, so they never collide with Aria's own tools.
pub const PREFIX: &str = "desktop_";

#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
    tools: Vec<ToolInfo>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

enum State {
    Idle,
    Running(Session),
    Failed(Instant),
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::Idle))
}

/// Where the driver lives: on `PATH`, or where its installer puts it.
fn driver_path() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let exe = dir.join("cua-driver.exe");
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    let local = std::env::var_os("LOCALAPPDATA")?;
    let exe = std::path::Path::new(&local).join(r"Programs\Cua\cua-driver\bin\cua-driver.exe");
    exe.is_file().then_some(exe)
}

impl Session {
    fn start() -> Result<Session, String> {
        let exe = driver_path().ok_or("cua-driver is not installed")?;
        let mut command = Command::new(&exe);
        command.arg("mcp").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().map_err(|e| format!("could not start {}: {e}", exe.display()))?;
        // Dies with the app even if the app is killed.
        crate::desktop::win32::put_in_kill_on_close_job(&mut child);
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;

        // A reader thread turns stdout into lines, so a request can time out instead of hanging.
        let (tx, lines) = mpsc::channel();
        std::thread::Builder::new()
            .name("cua-mcp".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        let mut session = Session { child, stdin, lines, next_id: 1, tools: Vec::new() };
        session.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "aria", "version": env!("CARGO_PKG_VERSION") }
            }),
        )?;
        session.notify("notifications/initialized")?;
        session.tools = session.list_tools()?;
        log::info!("cua-driver connected: {} tools", session.tools.len());
        Ok(session)
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).and_then(|_| self.stdin.flush()).map_err(|e| e.to_string())
    }

    fn notify(&mut self, method: &str) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method }))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        let deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = match self.lines.recv_timeout(left) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => return Err(format!("{method} timed out")),
                Err(RecvTimeoutError::Disconnected) => return Err("cua-driver exited".into()),
            };
            let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
            // Notifications and replies to older (timed-out) requests are skipped.
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(error.get("message").and_then(Value::as_str).unwrap_or("error").to_string());
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn list_tools(&mut self) -> Result<Vec<ToolInfo>, String> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let page = self.request("tools/list", params)?;
            for tool in page.get("tools").and_then(Value::as_array).into_iter().flatten() {
                let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
                if name.is_empty() {
                    continue;
                }
                tools.push(ToolInfo {
                    name: name.to_string(),
                    description: tool.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
                    schema: tool.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
                });
            }
            cursor = page.get("nextCursor").and_then(Value::as_str).map(str::to_string);
            if cursor.is_none() {
                return Ok(tools);
            }
        }
    }
}

/// Runs `f` against a live session, starting one if needed. `None` when the driver is disabled,
/// missing, or failed to start recently.
fn with_session<R>(f: impl FnOnce(&mut Session) -> Result<R, String>) -> Option<Result<R, String>> {
    if !crate::config::get().cua_enabled {
        return None;
    }
    let mut guard = state().lock().unwrap_or_else(|e| e.into_inner());
    if let State::Failed(at) = &*guard {
        if at.elapsed() < RETRY_AFTER {
            return None;
        }
    }
    if !matches!(&*guard, State::Running(_)) {
        match Session::start() {
            Ok(session) => *guard = State::Running(session),
            Err(e) => {
                log::info!("cua-driver unavailable: {e}");
                *guard = State::Failed(Instant::now());
                return None;
            }
        }
    }
    let State::Running(session) = &mut *guard else { return None };
    let result = f(session);
    if let Err(e) = &result {
        // A dead pipe means the next call should start over.
        if e.contains("exited") || e.contains("pipe") {
            *guard = State::Failed(Instant::now());
        }
    }
    Some(result)
}

/// The curated tools, with the model-facing (prefixed) names and short descriptions.
pub fn model_tools() -> Vec<ToolInfo> {
    let Some(Ok(all)) = with_session(|s| Ok(s.tools.clone())) else { return Vec::new() };
    MODEL_TOOLS
        .iter()
        .filter_map(|wanted| all.iter().find(|t| t.name == *wanted))
        .map(|t| ToolInfo {
            name: format!("{PREFIX}{}", t.name),
            description: short_description(&t.description),
            schema: compact_schema(&t.schema),
        })
        .collect()
}

/// The driver's input schemas run to 2-6 KB each (long prose per field, nested examples), and
/// they are sent with every request. This keeps the structure and the top-level field notes
/// (capped), drops nested prose, examples and titles, and drops `session`, which Aria never uses.
fn compact_schema(schema: &Value) -> Value {
    fn walk(value: &Value, depth: usize) -> Value {
        match value {
            Value::Object(map) => {
                let mut out = serde_json::Map::new();
                for (key, v) in map {
                    match key.as_str() {
                        "examples" | "example" | "title" | "$comment" | "$schema" => {}
                        "description" if depth > 2 => {}
                        "description" => {
                            out.insert(key.clone(), json!(crate::util::truncate(v.as_str().unwrap_or_default(), 140)));
                        }
                        _ => {
                            out.insert(key.clone(), walk(v, depth + 1));
                        }
                    }
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(|v| walk(v, depth)).collect()),
            other => other.clone(),
        }
    }
    let mut compact = walk(schema, 0);
    if let Some(props) = compact.get_mut("properties").and_then(Value::as_object_mut) {
        props.remove("session");
    }
    compact
}

/// The first paragraph, capped: the full descriptions run to thousands of characters each and
/// would be sent with every request.
fn short_description(full: &str) -> String {
    let first = full.split("\n\n").next().unwrap_or(full).trim();
    crate::util::truncate(first, 360)
}

/// The result of one tool call, reduced to what a model can read.
#[derive(Debug, Clone)]
pub struct CallResult {
    pub ok: bool,
    pub text: String,
    pub structured: Option<Value>,
}

/// Calls a driver tool by its own (unprefixed) name.
pub fn call(name: &str, arguments: Value) -> Result<CallResult, String> {
    let name = name.strip_prefix(PREFIX).unwrap_or(name).to_string();
    let result = with_session(|s| s.request("tools/call", json!({ "name": name, "arguments": arguments })))
        .unwrap_or_else(|| Err("cua-driver is not available".into()))?;
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| match part.get("type").and_then(Value::as_str) {
            Some("text") => part.get("text").and_then(Value::as_str).map(str::to_string),
            Some("image") => Some("[an image was returned; it is not shown here]".into()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(CallResult {
        ok: !result.get("isError").and_then(Value::as_bool).unwrap_or(false),
        text,
        structured: result.get("structuredContent").cloned(),
    })
}

// ---------------------------------------------------------------------------------- windows

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Window {
    pub window_id: i64,
    pub pid: u32,
    pub app_name: String,
    pub title: String,
    pub bounds: Bounds,
    pub z_index: Option<i64>,
    pub is_on_screen: bool,
    pub minimized: bool,
}

#[derive(Deserialize)]
struct WindowList {
    #[serde(default)]
    windows: Vec<Window>,
}

/// Every on-screen window, front-most first. `None` when the driver is not available.
pub fn windows() -> Option<Vec<Window>> {
    let result = call("list_windows", json!({ "on_screen_only": true })).ok()?;
    let mut list = serde_json::from_value::<WindowList>(result.structured?).ok()?.windows;
    list.retain(|w| w.is_on_screen && !w.minimized);
    list.sort_by_key(|w| std::cmp::Reverse(w.z_index.unwrap_or(i64::MIN)));
    Some(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_lists_decode_from_structured_content() {
        let raw = json!({ "windows": [
            { "app_name": "Spotify.exe", "pid": 7, "title": "Coolio - Gangsta's Paradise", "window_id": 1,
              "bounds": { "x": 0, "y": 0, "width": 800, "height": 600 }, "z_index": 3,
              "is_on_screen": true, "minimized": false, "layer": 0 },
            { "app_name": "x.exe", "pid": 8, "title": "", "window_id": 2, "z_index": null }
        ]});
        let list: WindowList = serde_json::from_value(raw).unwrap();
        assert_eq!(list.windows.len(), 2);
        assert_eq!(list.windows[0].bounds.width, 800);
        assert_eq!(list.windows[1].z_index, None);
    }

    #[test]
    fn schemas_keep_structure_but_lose_prose() {
        let schema = json!({
            "type": "object", "title": "x",
            "properties": {
                "pid": { "type": "integer", "description": "p".repeat(400) },
                "session": { "type": "string" },
                "steps": { "type": "array", "items": { "type": "object", "description": "deep prose",
                    "properties": { "tool": { "type": "string", "description": "deeper", "examples": ["click"] } } } }
            },
            "required": ["pid"]
        });
        let compact = compact_schema(&schema);
        assert_eq!(compact["required"], json!(["pid"]));
        assert!(compact.get("title").is_none() && compact["properties"].get("session").is_none());
        assert!(compact["properties"]["pid"]["description"].as_str().unwrap().chars().count() <= 140);
        let tool = &compact["properties"]["steps"]["items"]["properties"]["tool"];
        assert_eq!(tool["type"], "string");
        assert!(tool.get("description").is_none() && tool.get("examples").is_none());
    }

    #[test]
    fn descriptions_are_kept_short() {
        let long = format!("First paragraph.\n\n{}", "detail ".repeat(500));
        assert_eq!(short_description(&long), "First paragraph.");
        assert!(short_description(&"x".repeat(1000)).chars().count() <= 360);
    }
}
