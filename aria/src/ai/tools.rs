//! The tools the model can call: their declarations and what each one does.
//!
//! Declarations are data ([`TOOLS`]), turned into JSON schemas once. When Cua Driver is running,
//! its curated tools are added (prefixed `desktop_`), and the three legacy tools that guess at
//! pixels and steal focus (`find_and_click_on_screen`, `control_mouse_or_keyboard`,
//! `focus_or_manage_window`) are withheld, since the driver does the same jobs in the background
//! and through the accessibility tree. The set is fixed for the session, so the tool block of the
//! prompt stays identical and cacheable.

use std::sync::OnceLock;

use async_openai::types::chat::{ChatCompletionTool, ChatCompletionTools, FunctionObject};
use serde_json::{json, Value};

use crate::desktop::{cua, input, launcher, media, spotify, system, win32};

#[derive(Clone, Copy)]
enum Kind {
    Str,
    Int,
    Num,
    Bool,
}

struct Param {
    name: &'static str,
    kind: Kind,
    desc: &'static str,
    required: bool,
}

const fn req(name: &'static str, desc: &'static str) -> Param {
    Param { name, kind: Kind::Str, desc, required: true }
}

const fn opt(name: &'static str, kind: Kind, desc: &'static str) -> Param {
    Param { name, kind, desc, required: false }
}

struct Tool {
    name: &'static str,
    desc: &'static str,
    params: &'static [Param],
    /// Replaced by Cua Driver's tools when the driver is available.
    legacy_desktop: bool,
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "get_valorant_stats",
        desc: "Gets the automatically detected signed-in Riot account, visible own-player Valorant stats, their timestamps/staleness, recorded completed matches, calculated metrics and post-match recap. Missing fields are unknown. This reads cached data, never scouts opponents or supplies live tactical advice. Use for the user's Valorant statistics and reflective match analysis; current frames are approximate and may be stale.",
        params: &[],
        legacy_desktop: false,
    },
    Tool {
        name: "get_pc_activity",
        desc: "Gets locally detected foreground app, game detection evidence, running games, user presence, idle time, lock state and media activity. Use this to check what the user is doing; detection alone does not reveal gameplay outcomes.",
        params: &[],
        legacy_desktop: false,
    },
    Tool {
        name: "get_system_stats",
        desc: "Gets current PC hardware and system status: CPU, RAM, disk, battery and the open windows.",
        params: &[],
        legacy_desktop: false,
    },
    Tool {
        name: "move_mascot",
        desc: "Moves Aria's desktop avatar to a spot on the screen, perches her on the active window, or starts roaming.",
        params: &[
            req("location", "'left', 'right', 'center', 'top_left', 'top_right', 'bottom_left', 'bottom_right', 'perch', or 'roam'."),
            opt("emotion", Kind::Str, "Optional emotion to walk with, e.g. 'happy' or 'annoyed'."),
        ],
        legacy_desktop: false,
    },
    Tool {
        name: "remember_user_fact",
        desc: "Saves a personal fact, preference, name, habit, or note about the user into Aria's long-term memory.",
        params: &[req("fact", "The fact or preference to remember about the user.")],
        legacy_desktop: false,
    },
    Tool {
        name: "manage_todo_list",
        desc: "Manages the user's persistent desktop to-do list: add, complete, list or clear tasks.",
        params: &[
            req("action", "'add', 'complete', 'list', or 'clear'."),
            opt("task", Kind::Str, "The task, when adding or completing."),
        ],
        legacy_desktop: false,
    },
    Tool {
        name: "open_app_or_website",
        desc: "Opens any installed Windows app, a website or URL, or a web search, and brings its window to the front.",
        params: &[req("target", "App name (e.g. 'notepad'), full URL, or a search (e.g. 'search: lofi hip hop').")],
        legacy_desktop: false,
    },
    Tool {
        name: "list_installed_apps",
        desc: "Lists installed applications, games and software on the PC.",
        params: &[opt("filter_query", Kind::Str, "Optional keyword filter (e.g. 'game', 'music', 'code').")],
        legacy_desktop: false,
    },
    Tool {
        name: "get_now_playing_media",
        desc: "Detects the song or video playing right now (Spotify, YouTube, SoundCloud, Wynk and any media app).",
        params: &[],
        legacy_desktop: false,
    },
    Tool {
        name: "control_spotify",
        desc: "Controls Spotify and verifies by listening; the result says whether playback was confirmed (verified).",
        params: &[
            req("action", "'play_song', 'play', 'pause', 'play_pause', 'next', 'previous', 'open', or 'now_playing'."),
            opt("query", Kind::Str, "Song, artist or playlist when action is 'play_song'."),
        ],
        legacy_desktop: false,
    },
    Tool {
        name: "control_media_playback",
        desc: "System-wide media control (play_pause, next, previous) for whatever app is playing.",
        params: &[req("action", "'play_pause', 'next', or 'previous'.")],
        legacy_desktop: false,
    },
    Tool {
        name: "listen_to_desktop_audio",
        desc: "Listens to the audio actually playing from the PC's speakers and describes it: song, artist, mood, or speech.",
        params: &[
            opt("seconds", Kind::Num, "How many seconds to listen (3-15, default 8)."),
            opt("question", Kind::Str, "Optional specific question about the audio."),
        ],
        legacy_desktop: false,
    },
    Tool {
        name: "get_audio_activity",
        desc: "Instantly checks which apps are producing sound and how loud.",
        params: &[],
        legacy_desktop: false,
    },
    Tool {
        name: "run_pc_command",
        desc: "Runs a quick read-only PowerShell command to inspect files, directories, date/time or system info.",
        params: &[req("command", "The PowerShell command to run.")],
        legacy_desktop: false,
    },
    Tool {
        name: "find_and_click_on_screen",
        desc: "Finds a UI element on the live screen from a description, clicks it, and optionally types text.",
        params: &[
            req("target_description", "The element to click (e.g. 'the search bar')."),
            opt("click_type", Kind::Str, "'single' (default), 'double', 'right', or 'hover'."),
            opt("text_to_type", Kind::Str, "Optional text to type after clicking."),
            opt("press_enter", Kind::Bool, "Press Enter after typing."),
        ],
        legacy_desktop: true,
    },
    Tool {
        name: "focus_or_manage_window",
        desc: "Brings a window to the front, or minimizes, maximizes, restores or closes it, by title.",
        params: &[
            req("window_title", "Part of the window's title (e.g. 'Notepad')."),
            opt("action", Kind::Str, "'focus', 'minimize', 'maximize', 'restore', or 'close'."),
        ],
        legacy_desktop: true,
    },
    Tool {
        name: "control_mouse_or_keyboard",
        desc: "A keyboard or mouse action: media/volume keys, hotkeys, typing, scrolling, or clicking coordinates.",
        params: &[
            req("action", "'press_key', 'hotkey', 'type_text', 'move_cursor', 'click', 'double_click', 'right_click', or 'scroll'."),
            opt("key", Kind::Str, "Key or '+'-separated hotkey (e.g. 'volumeup', 'win+d')."),
            opt("text", Kind::Str, "Text to type for 'type_text'."),
            opt("x", Kind::Int, "Screen X coordinate."),
            opt("y", Kind::Int, "Screen Y coordinate."),
            opt("scroll_amount", Kind::Int, "Wheel amount (positive up, negative down)."),
        ],
        legacy_desktop: true,
    },
];

fn schema(params: &[Param]) -> Value {
    let mut properties = serde_json::Map::new();
    for p in params {
        let kind = match p.kind {
            Kind::Str => "string",
            Kind::Int => "integer",
            Kind::Num => "number",
            Kind::Bool => "boolean",
        };
        properties.insert(p.name.into(), json!({ "type": kind, "description": p.desc }));
    }
    let required: Vec<&str> = params.iter().filter(|p| p.required).map(|p| p.name).collect();
    json!({ "type": "object", "properties": properties, "required": required })
}

fn function(name: String, description: String, parameters: Value) -> ChatCompletionTools {
    ChatCompletionTools::Function(ChatCompletionTool {
        function: FunctionObject { name, description: Some(description), parameters: Some(parameters), strict: None },
    })
}

/// Whether the tool set includes Cua Driver: `None` until the first conversation builds it.
static WITH_DRIVER: OnceLock<bool> = OnceLock::new();

pub fn cua_connected() -> Option<bool> {
    WITH_DRIVER.get().copied()
}

/// The tool set for this session, built once (Cua Driver is probed the first time).
pub fn definitions() -> &'static [ChatCompletionTools] {
    static DEFS: OnceLock<Vec<ChatCompletionTools>> = OnceLock::new();
    DEFS.get_or_init(|| {
        let driver = cua::model_tools();
        let with_driver = !driver.is_empty();
        let _ = WITH_DRIVER.set(with_driver);
        let mut defs: Vec<ChatCompletionTools> = TOOLS
            .iter()
            .filter(|t| !(with_driver && t.legacy_desktop))
            .map(|t| function(t.name.into(), t.desc.into(), schema(t.params)))
            .collect();
        defs.extend(driver.into_iter().map(|t| function(t.name, t.description, t.schema)));
        log::info!("{} tools for the model ({})", defs.len(), if with_driver { "with Cua Driver" } else { "Win32 only" });
        defs
    })
}

/// How to use the driver's tools, for the system prompt (empty without the driver).
pub fn desktop_notes() -> &'static str {
    definitions();
    if cua_connected() == Some(true) {
        cua::USAGE
    } else {
        ""
    }
}

// ---------------------------------------------------------------------------------- running

/// What a tool needs from the conversation that called it.
pub struct ToolCtx<'a> {
    /// Moves the mascot: `(location, emotion)`. Runs on the UI thread.
    pub move_mascot: &'a dyn Fn(&str, Option<&str>),
}

fn arg_str(args: &Value, key: &str) -> String {
    match args.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn arg_opt(args: &Value, key: &str) -> Option<String> {
    Some(arg_str(args, key)).filter(|s| !s.is_empty())
}

fn arg_i32(args: &Value, key: &str) -> Option<i32> {
    let v = args.get(key)?;
    v.as_i64().map(|n| n as i32).or_else(|| v.as_str()?.trim().parse().ok())
}

/// Runs one tool call and returns its JSON result.
pub fn execute(name: &str, args: &Value, ctx: &ToolCtx) -> Value {
    log::info!("tool {name}({args})");
    if name.starts_with(cua::PREFIX) {
        return match cua::call(name, args.clone()) {
            Ok(result) => json!({ "ok": result.ok, "result": crate::util::truncate(&result.text, 6000) }),
            Err(e) => json!({ "status": "error", "error": e }),
        };
    }
    match name {
        "get_valorant_stats" => {
            if !crate::config::get().valorant_enabled {
                json!({"status":"disabled","message":"Enable Valorant awareness in Presence & gaming."})
            } else {
                let data = crate::valorant::snapshot();
                let metrics = data.recent_matches.iter().map(|record| json!({
                    "match_id":record.id,"source":record.source,"metrics":record.stats.metrics()
                })).collect::<Vec<_>>();
                json!({"status":"success","data":data,"completed_match_metrics":metrics,
                    "limitations":"Visible frames are partial observations, not live telemetry. Review completed matches; do not give immediate tactical instructions or infer stats from missing fields."})
            }
        },
        "get_system_stats" => serde_json::to_value(system::system_stats()).unwrap_or_default(),
        "get_pc_activity" => serde_json::to_value(crate::desktop::presence::latest()).unwrap_or_default(),
        "get_now_playing_media" => serde_json::to_value(media::now_playing()).unwrap_or_default(),
        "move_mascot" => {
            let location = arg_opt(args, "location").unwrap_or_else(|| "roam".into());
            let emotion = arg_opt(args, "emotion");
            (ctx.move_mascot)(&location, emotion.as_deref());
            json!({ "status": "success", "moved_to": location })
        }
        "remember_user_fact" => {
            let (saved, total) = crate::memory::add_fact(&arg_str(args, "fact"), "general");
            json!({ "status": if saved { "saved" } else { "exists" }, "total_facts": total })
        }
        "manage_todo_list" => {
            use crate::memory::TodoAction;
            let action = match arg_str(args, "action").to_lowercase().as_str() {
                "add" => TodoAction::Add,
                "complete" => TodoAction::Complete,
                "clear" => TodoAction::Clear,
                _ => TodoAction::List,
            };
            let (status, matched, list) = crate::memory::manage_todos(action, &arg_str(args, "task"));
            json!({ "status": status, "matched": matched, "todos": list })
        }
        "open_app_or_website" => open_app_or_website(&arg_str(args, "target")),
        "control_spotify" => spotify::control(&arg_str(args, "action"), &arg_str(args, "query")),
        "control_media_playback" => spotify::control_media(&arg_str(args, "action")),
        "listen_to_desktop_audio" => {
            let seconds = args.get("seconds").and_then(Value::as_f64).unwrap_or(8.0).clamp(3.0, 15.0);
            crate::ai::tasks::listen_to_desktop_audio(seconds, &arg_str(args, "question"))
        }
        "get_audio_activity" => serde_json::to_value(crate::desktop::audio::audio_activity(240)).unwrap_or_default(),
        "list_installed_apps" => json!({ "status": "success", "apps": launcher::list_installed_apps(&arg_str(args, "filter_query")) }),
        "find_and_click_on_screen" => find_and_click(
            &arg_str(args, "target_description"),
            &arg_str(args, "click_type"),
            arg_opt(args, "text_to_type").as_deref(),
            args.get("press_enter").and_then(Value::as_bool).unwrap_or(false),
        ),
        "focus_or_manage_window" => focus_or_manage_window(&arg_str(args, "window_title"), &arg_str(args, "action")),
        "control_mouse_or_keyboard" => control_mouse_or_keyboard(args),
        "run_pc_command" => run_pc_command(&arg_str(args, "command")),
        other => json!({ "status": "error", "error": format!("Unknown tool: {other}") }),
    }
}

/// Opens an app, or routes Spotify playback requests to the verified Spotify path.
pub fn open_app_or_website(target: &str) -> Value {
    let lower = target.trim().to_lowercase();
    if lower.contains("spotify") && ["play ", "song ", "track ", "music ", "listen "].iter().any(|w| lower.contains(w)) {
        let mut query = lower.replace("on spotify", "").replace("in spotify", "").replace("spotify", "");
        for prefix in ["play", "listen to", "put on"] {
            if let Some(rest) = query.trim().strip_prefix(prefix) {
                query = rest.trim().to_string();
            }
        }
        return spotify::control("play_song", query.trim());
    }
    if matches!(lower.as_str(), "spotify" | "open spotify" | "launch spotify") {
        return spotify::control("launch", "");
    }
    launcher::open_target(target)
}

/// Visual grounding: screenshot, ask the model where the element is, then click it.
fn find_and_click(target: &str, click_type: &str, text: Option<&str>, press_enter: bool) -> Value {
    let Some((x, y, label)) = crate::ai::tasks::locate_on_screen(target) else {
        return json!({ "status": "not_found", "message": format!("Could not find '{target}' on the screen.") });
    };
    input::move_cursor(x, y);
    std::thread::sleep(std::time::Duration::from_millis(450));
    let click_type = click_type.trim().to_lowercase();
    match click_type.as_str() {
        "double" => input::click(x, y, false, true),
        "right" => input::click(x, y, true, false),
        "hover" => {}
        _ => input::click(x, y, false, false),
    }
    if let Some(text) = text {
        std::thread::sleep(std::time::Duration::from_millis(150));
        input::type_text(text);
        if press_enter {
            input::press_key("enter");
        }
    }
    json!({ "status": "success", "clicked_label": label, "screen_x": x, "screen_y": y, "click_type": click_type })
}

pub fn focus_or_manage_window(window_title: &str, action: &str) -> Value {
    let target = window_title.trim().to_lowercase();
    if target.is_empty() {
        return json!({ "status": "error", "error": "window_title is required" });
    }
    let action = action.trim().to_lowercase();
    let Some(win) = win32::enum_windows(false).into_iter().find(|w| {
        let title = w.title.to_lowercase();
        title.contains(&target) && !title.contains("aria")
    }) else {
        return json!({ "status": "not_found", "error": format!("No open window matching '{window_title}'") });
    };
    if win32::show_window(win.hwnd, &action) {
        json!({ "status": "success", "window": win.title, "action": action })
    } else {
        json!({ "status": "error", "error": format!("Could not apply '{action}' to '{}'", win.title) })
    }
}

fn control_mouse_or_keyboard(args: &Value) -> Value {
    let action = arg_str(args, "action").trim().to_lowercase();
    let key = arg_opt(args, "key");
    let (x, y) = (arg_i32(args, "x"), arg_i32(args, "y"));
    let done = |what: String| json!({ "status": "success", "action": what });
    let fail = |why: &str| json!({ "status": "error", "error": why });
    match action.as_str() {
        "press_key" => match key {
            Some(k) if input::press_key(&k) => done(format!("Pressed key '{k}'")),
            _ => fail("Unknown key"),
        },
        "hotkey" => match key {
            Some(k) if input::hotkey(&k) => done(format!("Pressed hotkey '{k}'")),
            _ => fail("Unknown hotkey"),
        },
        "type_text" => match arg_opt(args, "text") {
            Some(t) => done(format!("Typed text ({} chars)", input::type_text(&t))),
            None => fail("text is required"),
        },
        "move_cursor" => match (x, y) {
            (Some(x), Some(y)) => {
                input::move_cursor(x, y);
                done(format!("Moved cursor to ({x}, {y})"))
            }
            _ => fail("x and y are required"),
        },
        "click" | "double_click" | "right_click" => {
            let (cx, cy) = match (x, y) {
                (Some(x), Some(y)) => (x, y),
                _ => input::cursor_position(),
            };
            input::click(cx, cy, action == "right_click", action == "double_click");
            done(format!("Performed {action} at ({cx}, {cy})"))
        }
        "scroll" => {
            let amount = arg_i32(args, "scroll_amount").unwrap_or(-400);
            input::scroll(amount);
            done(format!("Scrolled wheel by {amount}"))
        }
        _ => fail("Invalid parameters for control_mouse_or_keyboard"),
    }
}

/// PowerShell with a guard against destructive commands.
pub fn run_pc_command(command: &str) -> Value {
    const FORBIDDEN: &[&str] = &["remove-item", "del ", "rm -rf", "format-", "stop-computer", "restart-computer"];
    let lower = command.to_lowercase();
    if FORBIDDEN.iter().any(|f| lower.contains(f)) {
        return json!({ "status": "blocked", "error": "Destructive file/system commands are blocked for safety." });
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    match std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", command])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(out) => json!({
            "status": if out.status.success() { "success" } else { "warning" },
            "stdout": crate::util::truncate(String::from_utf8_lossy(&out.stdout).trim(), 1500),
            "stderr": crate::util::truncate(String::from_utf8_lossy(&out.stderr).trim(), 500),
        }),
        Err(e) => json!({ "status": "error", "error": e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_become_wellformed_schemas() {
        let mut names = std::collections::HashSet::new();
        for tool in TOOLS {
            assert!(names.insert(tool.name), "duplicate tool {}", tool.name);
            let s = schema(tool.params);
            assert_eq!(s["type"], "object");
            for required in s["required"].as_array().unwrap() {
                assert!(s["properties"][required.as_str().unwrap()].is_object());
            }
        }
        let move_schema = schema(TOOLS.iter().find(|t| t.name == "move_mascot").unwrap().params);
        assert_eq!(move_schema["required"], json!(["location"]));
        assert_eq!(move_schema["properties"]["emotion"]["type"], "string");
    }

    #[test]
    fn destructive_commands_are_blocked() {
        assert_eq!(run_pc_command("Remove-Item C:\\Windows -Recurse")["status"], "blocked");
    }

    #[test]
    fn harmless_command_runs() {
        let res = run_pc_command("Write-Output aria-ok");
        assert_eq!(res["status"], "success");
        assert!(res["stdout"].as_str().unwrap_or("").contains("aria-ok"));
    }

    #[test]
    fn arguments_tolerate_loose_types() {
        let args = json!({ "x": "42", "y": 7, "text": null });
        assert_eq!((arg_i32(&args, "x"), arg_i32(&args, "y")), (Some(42), Some(7)));
        assert_eq!(arg_opt(&args, "text"), None);
    }
}
