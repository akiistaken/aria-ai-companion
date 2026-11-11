//! PC tool belt exposed to the model, plus the OpenAI-compatible tool declarations.

use serde_json::{json, Value};

pub const TOOL_DECLARATIONS: &[(&str, &str, &str, &str)] = &[
    (
        "get_system_stats",
        "Gets current PC hardware and system status including CPU usage, RAM usage, disk space, battery status, and top running windows.",
        "",
        "",
    ),
    (
        "move_mascot",
        "Moves Aria's desktop avatar to a location on the user's screen, perches her on the active window, or starts roaming.",
        "location",
        "Target location on screen: 'left', 'right', 'center', 'top_left', 'top_right', 'bottom_left', 'bottom_right', 'perch', or 'roam'.",
    ),
    (
        "remember_user_fact",
        "Saves a personal fact, preference, name, habit, or note about the user into Aria's persistent long-term memory.",
        "fact",
        "The fact or preference to remember about the user.",
    ),
    (
        "manage_todo_list",
        "Manages the user's persistent desktop To-Do list (add a task, complete a task, list tasks, or clear tasks).",
        "action",
        "Action to perform: 'add', 'complete', 'list', or 'clear'.",
    ),
    (
        "open_app_or_website",
        "Force-opens any installed Windows app, a website or full URL, or a web search, then brings its window to the front.",
        "target",
        "Application name (e.g. 'notepad', 'calc'), full URL, or search query (e.g. 'search: lofi hip hop').",
    ),
    (
        "find_and_click_on_screen",
        "Visually inspects the live desktop to find a UI element matching a description, moves the mouse there, clicks it, and optionally types text.",
        "target_description",
        "Description of the visual UI element on screen to locate and click (e.g. 'the search bar').",
    ),
    (
        "focus_or_manage_window",
        "Brings an open desktop window to the foreground, or minimizes, maximizes, restores, or closes it by title match.",
        "window_title",
        "Partial or full title of the window (e.g. 'Notepad', 'Chrome').",
    ),
    (
        "control_mouse_or_keyboard",
        "Performs a keyboard or mouse action: pressing media/volume keys, hotkeys, typing text, scrolling, or clicking coordinates.",
        "action",
        "Action type: 'press_key', 'hotkey', 'type_text', 'move_cursor', 'click', 'double_click', 'right_click', or 'scroll'.",
    ),
    (
        "get_now_playing_media",
        "Detects any currently playing song, music track, or media video on the user's PC.",
        "",
        "",
    ),
    (
        "control_spotify",
        "Controls Spotify and verifies by listening; the result says whether playback was actually confirmed (verified).",
        "action",
        "Action: 'play_song', 'play', 'pause', 'play_pause', 'next', 'previous', 'open', or 'now_playing'.",
    ),
    (
        "control_media_playback",
        "Controls system-wide media playback (play_pause, next, previous) for Spotify, YouTube, VLC and browser audio.",
        "action",
        "Media action: 'play_pause', 'next', or 'previous'.",
    ),
    (
        "list_installed_apps",
        "Scans and lists installed applications, games, and software found on the user's PC.",
        "filter_query",
        "Optional keyword to filter installed apps (e.g. 'game', 'music', 'browser', 'code').",
    ),
    (
        "listen_to_desktop_audio",
        "Listens to the audio actually playing out of the PC's speakers and describes it: song, artist, mood, or speech.",
        "seconds",
        "How many seconds to listen (3-15, default 8).",
    ),
    (
        "get_audio_activity",
        "Instantly checks which apps are currently producing sound and how loud.",
        "",
        "",
    ),
    (
        "run_pc_command",
        "Runs a quick PowerShell command to inspect files, directories, date/time, or system info.",
        "command",
        "The PowerShell command to run.",
    ),
];

/// OpenAI-compatible `tools` array built from the declarations above.
pub fn openai_tools() -> Value {
    let mut list = Vec::new();
    for (name, description, required, required_desc) in TOOL_DECLARATIONS {
        let mut properties = serde_json::Map::new();
        let mut required_list = Vec::new();

        if !required.is_empty() {
            properties.insert(
                (*required).to_string(),
                json!({ "type": "string", "description": required_desc }),
            );
            required_list.push((*required).to_string());
        }

        // Extra optional parameters each tool understands.
        match *name {
            "move_mascot" => {
                properties.insert(
                    "emotion".into(),
                    json!({ "type": "string", "description": "Optional emotion: 'idle', 'talking', 'annoyed', or 'walking'." }),
                );
            }
            "manage_todo_list" => {
                properties.insert(
                    "task".into(),
                    json!({ "type": "string", "description": "Task description when action is 'add' or 'complete'." }),
                );
            }
            "find_and_click_on_screen" => {
                properties.insert("click_type".into(), json!({ "type": "string", "description": "'single' (default), 'double', 'right', or 'hover'." }));
                properties.insert("text_to_type".into(), json!({ "type": "string", "description": "Optional text to type after clicking." }));
                properties.insert("press_enter".into(), json!({ "type": "boolean", "description": "Press Enter after typing." }));
            }
            "focus_or_manage_window" => {
                properties.insert("action".into(), json!({ "type": "string", "description": "'focus', 'minimize', 'maximize', 'restore', or 'close'." }));
            }
            "control_mouse_or_keyboard" => {
                properties.insert("key".into(), json!({ "type": "string", "description": "Key or '+'-separated hotkey (e.g. 'volumeup', 'win+d', 'ctrl+shift+esc')." }));
                properties.insert("text".into(), json!({ "type": "string", "description": "Text to type when action is 'type_text'." }));
                properties.insert("x".into(), json!({ "type": "integer", "description": "Screen X coordinate." }));
                properties.insert("y".into(), json!({ "type": "integer", "description": "Screen Y coordinate." }));
                properties.insert("scroll_amount".into(), json!({ "type": "integer", "description": "Scroll wheel amount (positive up, negative down)." }));
            }
            "control_spotify" => {
                properties.insert(
                    "query".into(),
                    json!({ "type": "string", "description": "Song title, artist, or playlist when action is 'play_song'." }),
                );
            }
            "listen_to_desktop_audio" => {
                properties.insert(
                    "question".into(),
                    json!({ "type": "string", "description": "Optional specific question about the audio." }),
                );
            }
            _ => {}
        }

        list.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required_list,
                }
            }
        }));
    }
    Value::Array(list)
}

fn arg_str(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Null => None,
            other => Some(other.to_string()),
        })
        .unwrap_or_default()
}

fn arg_i32(args: &Value, key: &str) -> Option<i32> {
    args.get(key).and_then(|v| {
        v.as_i64()
            .map(|n| n as i32)
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    })
}

/// Runs one model tool call and returns its JSON result.
pub fn execute_tool(name: &str, args: &Value, move_callback: &dyn Fn(&str, Option<&str>)) -> Value {
    log::info!("tool '{name}' args: {args}");
    match name {
        "get_system_stats" => serde_json::to_value(crate::sysinfo::system_stats()).unwrap_or_default(),
        "get_now_playing_media" => {
            serde_json::to_value(crate::sysinfo::now_playing()).unwrap_or_default()
        }
        "move_mascot" => {
            let loc = arg_str(args, "location");
            let loc = if loc.is_empty() { "roam".to_string() } else { loc };
            let emotion = args.get("emotion").and_then(|v| v.as_str());
            move_callback(&loc, emotion);
            json!({ "status": "success", "moved_to": loc, "emotion": emotion.unwrap_or("walking") })
        }
        "remember_user_fact" => {
            let (saved, total) = crate::memory::add_fact(&arg_str(args, "fact"), "general");
            if saved {
                json!({ "status": "saved", "total_facts": total })
            } else {
                json!({ "status": "exists", "total_facts": total })
            }
        }
        "manage_todo_list" => {
            let action = match arg_str(args, "action").to_lowercase().as_str() {
                "add" => crate::memory::TodoAction::Add,
                "complete" => crate::memory::TodoAction::Complete,
                "clear" => crate::memory::TodoAction::Clear,
                _ => crate::memory::TodoAction::List,
            };
            let (status, matched, list) =
                crate::memory::manage_todos(action, &arg_str(args, "task"));
            json!({ "status": status, "matched": matched, "todos": list })
        }
        "open_app_or_website" => open_app_or_website(&arg_str(args, "target")),
        "control_spotify" => crate::sysinfo::control_spotify(
            &arg_str(args, "action"),
            &arg_str(args, "query"),
        ),
        "control_media_playback" => crate::sysinfo::control_media(&arg_str(args, "action")),
        "listen_to_desktop_audio" => {
            let secs = args
                .get("seconds")
                .and_then(|v| v.as_f64())
                .unwrap_or(8.0);
            crate::brain::listen_to_desktop_audio(secs, &arg_str(args, "question"))
        }
        "get_audio_activity" => {
            serde_json::to_value(crate::audio::audio_activity(240)).unwrap_or_default()
        }
        "list_installed_apps" => json!({
            "status": "success",
            "apps": crate::launcher::list_installed_apps(&arg_str(args, "filter_query"))
        }),
        "find_and_click_on_screen" => find_and_click_on_screen(
            &arg_str(args, "target_description"),
            &arg_str(args, "click_type"),
            {
                let t = arg_str(args, "text_to_type");
                if t.is_empty() {
                    None
                } else {
                    Some(t)
                }
            }
            .as_deref(),
            args.get("press_enter").and_then(|v| v.as_bool()).unwrap_or(false),
        ),
        "focus_or_manage_window" => focus_or_manage_window(
            &arg_str(args, "window_title"),
            &arg_str(args, "action"),
        ),
        "control_mouse_or_keyboard" => control_mouse_or_keyboard(
            &arg_str(args, "action"),
            args.get("key").and_then(|v| v.as_str()),
            args.get("text").and_then(|v| v.as_str()),
            arg_i32(args, "x"),
            arg_i32(args, "y"),
            arg_i32(args, "scroll_amount"),
        ),
        "run_pc_command" => run_pc_command(&arg_str(args, "command")),
        other => json!({ "status": "error", "error": format!("Unknown tool: {other}") }),
    }
}

/// Opens an app, or handles Spotify playback requests specially.
pub fn open_app_or_website(target: &str) -> Value {
    let lower = target.trim().to_lowercase();
    if lower.contains("spotify")
        && ["play ", "song ", "track ", "music ", "listen "]
            .iter()
            .any(|w| lower.contains(w))
    {
        let mut q = lower
            .replace("on spotify", "")
            .replace("in spotify", "")
            .replace("spotify", "");
        for pfx in ["play", "listen to", "put on"] {
            if let Some(stripped) = q.trim().strip_prefix(pfx) {
                q = stripped.trim().to_string();
            }
        }
        return crate::sysinfo::control_spotify("play_song", q.trim());
    }
    if matches!(lower.as_str(), "spotify" | "open spotify" | "launch spotify") {
        return crate::sysinfo::control_spotify("launch", "");
    }
    crate::launcher::open_target(target)
}

/// Visual grounding: screenshot -> model box -> click.
pub fn find_and_click_on_screen(
    target_description: &str,
    click_type: &str,
    text_to_type: Option<&str>,
    press_enter: bool,
) -> Value {
    let api_key = crate::config::api_key();
    if api_key.is_empty() {
        return json!({
            "status": "error",
            "error": "An API key is required for visual screen element detection. Set it in the Control Center."
        });
    }

    let Some(b64) = crate::win32::capture_screen_jpeg_b64(1366) else {
        return json!({ "status": "error", "error": "Could not capture the screen." });
    };
    let (mx, my, mw, mh) = crate::win32::screen_size();

    let prompt = format!(
        "Locate the UI element on this desktop screenshot matching: '{target_description}'. \
         Return JSON: {{\"found\": true, \"label\": \"name of element\", \"box_2d\": [ymin, xmin, ymax, xmax]}} \
         where the box values are integers normalized to 0-1000. \
         Ignore any desktop companion mascot overlay if visible. \
         If the element is not visible, return {{\"found\": false, \"label\": \"\", \"box_2d\": [0,0,0,0]}}."
    );

    let body = json!({
        "model": crate::config::model(),
        "messages": [
            { "role": "system", "content": "You are a precise desktop visual grounding assistant. Return only valid JSON." },
            { "role": "user", "content": [
                { "type": "text", "text": prompt },
                { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{b64}") } }
            ]}
        ],
        "response_format": { "type": "json_object" }
    });

    let Ok(raw) = crate::brain::post_chat(&body) else {
        return json!({ "status": "error", "error": "Vision request failed." });
    };
    let content = crate::brain::message_content(&raw).unwrap_or_else(|| "{}".into());
    let data: Value = serde_json::from_str(&content).unwrap_or_else(|_| json!({}));
    let found = data.get("found").and_then(|v| v.as_bool()).unwrap_or(false);
    let Some(box_vals) = data.get("box_2d").and_then(|v| v.as_array()) else {
        return json!({ "status": "not_found", "target": target_description });
    };
    if !found || box_vals.len() != 4 {
        return json!({
            "status": "not_found", "target": target_description,
            "message": format!("Could not visually find '{target_description}' on the current screen.")
        });
    }

    let coords: Vec<f64> = box_vals
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0))
        .collect();
    let cx = (coords[1] + coords[3]) / 2.0 / 1000.0;
    let cy = (coords[0] + coords[2]) / 2.0 / 1000.0;
    let x = (mx as f64 + cx * mw as f64) as i32;
    let y = (my as f64 + cy * mh as f64) as i32;

    crate::input::move_cursor(x, y);
    std::thread::sleep(std::time::Duration::from_millis(450));
    let ctype = click_type.trim().to_lowercase();
    match ctype.as_str() {
        "double" => crate::input::click(x, y, false, true),
        "right" => crate::input::click(x, y, true, false),
        "hover" => {}
        _ => crate::input::click(x, y, false, false),
    }

    if let Some(text) = text_to_type {
        std::thread::sleep(std::time::Duration::from_millis(150));
        crate::input::type_text(text);
        if press_enter {
            crate::input::press_key("enter");
        }
    }

    json!({
        "status": "success",
        "clicked_label": data.get("label").and_then(|v| v.as_str()).unwrap_or(target_description),
        "screen_x": x, "screen_y": y, "click_type": ctype,
        "typed_text": text_to_type.unwrap_or("")
    })
}

pub fn focus_or_manage_window(window_title: &str, action: &str) -> Value {
    let target = window_title.trim().to_lowercase();
    let act = action.trim().to_lowercase();
    if target.is_empty() {
        return json!({ "status": "error", "error": "window_title is required" });
    }

    let matches: Vec<_> = crate::win32::enum_windows(false)
        .into_iter()
        .filter(|w| {
            w.title.to_lowercase().contains(&target) && !w.title.to_lowercase().contains("aria")
        })
        .collect();
    let Some(win) = matches.first() else {
        return json!({ "status": "not_found", "error": format!("No open window matching '{window_title}'") });
    };

    let hwnd = win.hwnd;
    let title = win.title.clone();
    let ok = crate::win32::show_window(hwnd, &act);
    if ok {
        json!({ "status": "success", "window": title, "action": act })
    } else {
        json!({ "status": "error", "error": format!("Could not apply '{act}' to '{title}'") })
    }
}

pub fn control_mouse_or_keyboard(
    action: &str,
    key: Option<&str>,
    text: Option<&str>,
    x: Option<i32>,
    y: Option<i32>,
    scroll_amount: Option<i32>,
) -> Value {
    let act = action.trim().to_lowercase();
    match act.as_str() {
        "press_key" => match key {
            Some(k) if crate::input::press_key(k) => {
                json!({ "status": "success", "action": format!("Pressed key '{k}'") })
            }
            _ => json!({ "status": "error", "error": "Unknown key" }),
        },
        "hotkey" => match key {
            Some(k) if crate::input::hotkey(k) => {
                json!({ "status": "success", "action": format!("Pressed hotkey '{k}'") })
            }
            _ => json!({ "status": "error", "error": "Unknown hotkey" }),
        },
        "type_text" => match text {
            Some(t) => {
                let n = crate::input::type_text(t);
                json!({ "status": "success", "action": format!("Typed text ({n} chars)") })
            }
            None => json!({ "status": "error", "error": "text is required" }),
        },
        "move_cursor" => match (x, y) {
            (Some(x), Some(y)) => {
                crate::input::move_cursor(x, y);
                json!({ "status": "success", "action": format!("Moved cursor to ({x}, {y})") })
            }
            _ => json!({ "status": "error", "error": "x and y are required" }),
        },
        "click" | "double_click" | "right_click" => {
            let right = act == "right_click";
            let double = act == "double_click";
            match (x, y) {
                (Some(x), Some(y)) => {
                    crate::input::click(x, y, right, double);
                    json!({ "status": "success", "action": format!("Performed {act} at ({x}, {y})") })
                }
                _ => {
                    let cur = crate::input::cursor_position();
                    crate::input::click(cur.0, cur.1, right, double);
                    json!({ "status": "success", "action": format!("Performed {act} at the cursor") })
                }
            }
        }
        "scroll" => {
            let amt = scroll_amount.unwrap_or(-400);
            crate::input::scroll(amt);
            json!({ "status": "success", "action": format!("Scrolled wheel by {amt}") })
        }
        _ => json!({ "status": "error", "error": "Invalid parameters for control_mouse_or_keyboard" }),
    }
}

/// PowerShell execution with a destructive-command guard.
pub fn run_pc_command(command: &str) -> Value {
    let forbidden = [
        "remove-item",
        "del ",
        "rm -rf",
        "format-",
        "stop-computer",
        "restart-computer",
    ];
    let lower = command.to_lowercase();
    if forbidden.iter().any(|f| lower.contains(f)) {
        return json!({
            "status": "blocked",
            "error": "Destructive file/system commands are blocked for safety."
        });
    }
    use std::os::windows::process::CommandExt;
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", command])
        .creation_flags(0x0800_0000)
        .output();
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let code = out.status.code().unwrap_or(-1);
            json!({
                "status": if code == 0 { "success" } else { "warning" },
                "stdout": stdout.chars().take(1500).collect::<String>(),
                "stderr": stderr.chars().take(500).collect::<String>(),
            })
        }
        Err(e) => json!({ "status": "error", "error": e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schema_is_wellformed() {
        let tools = openai_tools();
        let arr = tools.as_array().expect("tools array");
        assert_eq!(arr.len(), TOOL_DECLARATIONS.len());
        for tool in arr {
            assert_eq!(tool["type"], "function");
            assert!(tool["function"]["name"].is_string());
            assert_eq!(tool["function"]["parameters"]["type"], "object");
        }
    }

    #[test]
    fn destructive_commands_are_blocked() {
        let res = run_pc_command("Remove-Item C:\\Windows -Recurse");
        assert_eq!(res["status"], "blocked");
    }

    #[test]
    fn harmless_command_runs() {
        let res = run_pc_command("Write-Output aria-ok");
        assert_eq!(res["status"], "success");
        assert!(res["stdout"].as_str().unwrap_or("").contains("aria-ok"));
    }
}
