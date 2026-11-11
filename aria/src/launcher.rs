//! App / URL / protocol launching with real window foregrounding.

use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

static MULTI_INSTANCE: &[&str] = &[
    "notepad.exe",
    "calc.exe",
    "calculatorapp.exe",
    "mspaint.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "explorer.exe",
    "wt.exe",
    "taskmgr.exe",
    "snippingtool.exe",
];

const CONSOLE_EXES: &[&str] = &["cmd.exe", "powershell.exe", "pwsh.exe"];

const KNOWN_SITES: &[(&str, &str)] = &[
    ("youtube", "https://www.youtube.com"),
    ("youtube music", "https://music.youtube.com"),
    ("gmail", "https://mail.google.com"),
    ("google", "https://www.google.com"),
    ("github", "https://github.com"),
    ("reddit", "https://www.reddit.com"),
    ("twitter", "https://x.com"),
    ("x", "https://x.com"),
    ("netflix", "https://www.netflix.com"),
    ("twitch", "https://www.twitch.tv"),
    ("facebook", "https://www.facebook.com"),
    ("instagram", "https://www.instagram.com"),
    ("whatsapp web", "https://web.whatsapp.com"),
    ("chatgpt", "https://chatgpt.com"),
    ("gemini", "https://gemini.google.com"),
    ("google maps", "https://maps.google.com"),
    ("maps", "https://maps.google.com"),
    ("google drive", "https://drive.google.com"),
    ("drive", "https://drive.google.com"),
    ("amazon", "https://www.amazon.com"),
    ("wikipedia", "https://www.wikipedia.org"),
    ("linkedin", "https://www.linkedin.com"),
    ("discord", "https://discord.com/app"),
    ("spotify web", "https://open.spotify.com"),
    ("pinterest", "https://www.pinterest.com"),
    ("crunchyroll", "https://www.crunchyroll.com"),
    ("tiktok", "https://www.tiktok.com"),
    ("google docs", "https://docs.google.com"),
    ("google calendar", "https://calendar.google.com"),
    ("stack overflow", "https://stackoverflow.com"),
    ("pixiv", "https://www.pixiv.net"),
];

const FAST_MAP: &[(&str, &str)] = &[
    ("spotify", "spotify:"),
    ("discord", "discord:"),
    ("steam", "steam:"),
    ("chrome", "chrome.exe"),
    ("google chrome", "chrome.exe"),
    ("edge", "msedge.exe"),
    ("microsoft edge", "msedge.exe"),
    ("firefox", "firefox.exe"),
    ("calculator", "calc.exe"),
    ("calc", "calc.exe"),
    ("notepad", "notepad.exe"),
    ("paint", "mspaint.exe"),
    ("mspaint", "mspaint.exe"),
    ("explorer", "explorer.exe"),
    ("file explorer", "explorer.exe"),
    ("terminal", "wt.exe"),
    ("cmd", "cmd.exe"),
    ("command prompt", "cmd.exe"),
    ("powershell", "powershell.exe"),
    ("task manager", "taskmgr.exe"),
    ("taskmgr", "taskmgr.exe"),
    ("settings", "ms-settings:"),
    ("vscode", "code.cmd"),
    ("code", "code.cmd"),
    ("visual studio code", "code.cmd"),
    ("obs", "obs64.exe"),
    ("obs studio", "obs64.exe"),
    ("whatsapp", "whatsapp:"),
    ("telegram", "tg:"),
    ("apple music", "applemusic:"),
];

struct ShortcutCache {
    at: f64,
    paths: Vec<PathBuf>,
}

static SHORTCUTS: OnceLock<Mutex<ShortcutCache>> = OnceLock::new();

fn shortcuts_cache() -> &'static Mutex<ShortcutCache> {
    SHORTCUTS.get_or_init(|| {
        Mutex::new(ShortcutCache {
            at: 0.0,
            paths: Vec::new(),
        })
    })
}

fn start_menu_shortcuts() -> Vec<PathBuf> {
    let now = crate::util::unix_now();
    {
        let cache = shortcuts_cache().lock().unwrap_or_else(|e| e.into_inner());
        if now - cache.at < 60.0 && !cache.paths.is_empty() {
            return cache.paths.clone();
        }
    }
    let mut found = Vec::new();
    for var in ["APPDATA", "PROGRAMDATA"] {
        let Ok(base) = std::env::var(var) else {
            continue;
        };
        if base.is_empty() {
            continue;
        }
        let root = Path::new(&base)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        if root.is_dir() {
            collect_lnk(&root, &mut found);
        }
    }
    let mut cache = shortcuts_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.at = now;
    cache.paths = found.clone();
    found
}

fn collect_lnk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lnk(&path, out);
        } else if path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("lnk"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

fn clean_name(app_name: &str) -> String {
    let mut n = app_name.trim().to_lowercase();
    for pfx in ["the ", "my "] {
        if let Some(stripped) = n.strip_prefix(pfx) {
            n = stripped.trim().to_string();
        }
    }
    for sfx in [" app", " application", " program", " desktop"] {
        if let Some(stripped) = n.strip_suffix(sfx) {
            n = stripped.trim().to_string();
        }
    }
    n
}

pub fn find_installed_app(app_name: &str) -> Option<String> {
    let name = clean_name(app_name);
    if name.is_empty() {
        return None;
    }

    let words: Vec<&str> = name.split_whitespace().collect();
    let mut best: Option<(usize, PathBuf)> = None;
    for lnk in start_menu_shortcuts() {
        let stem = lnk
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ["uninstall", "readme", "release notes", "documentation", "license"]
            .iter()
            .any(|bad| stem.contains(bad))
        {
            continue;
        }
        let score = if stem == name {
            0
        } else if stem.starts_with(&name) {
            10 + stem.len()
        } else if name.len() >= 3 && stem.contains(&name) {
            100 + stem.len()
        } else if words.len() > 1 && words.iter().all(|w| stem.contains(w)) {
            200 + stem.len()
        } else {
            continue;
        };
        if best.as_ref().map(|(b, _)| score < *b).unwrap_or(true) {
            best = Some((score, lnk));
        }
    }
    if let Some((_, path)) = best {
        return Some(path.to_string_lossy().to_string());
    }

    if let Some((_, mapped)) = FAST_MAP.iter().find(|(k, _)| *k == name) {
        return Some((*mapped).to_string());
    }

    if let Some(found) = registry_app_path(&name) {
        return Some(found);
    }

    which(&name).or_else(|| which(&format!("{name}.exe")))
}

fn registry_app_path(name: &str) -> Option<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
        HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ,
    };

    let subkey: Vec<u16> = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\0"
        .encode_utf16()
        .collect();

    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(root, windows::core::PCWSTR(subkey.as_ptr()), Some(0), KEY_READ, &mut key)
                != ERROR_SUCCESS
            {
                continue;
            }
            let mut index = 0u32;
            loop {
                let mut buf = [0u16; 256];
                if RegEnumKeyW(key, index, Some(&mut buf)) != ERROR_SUCCESS {
                    break;
                }
                index += 1;
                // The name is NUL-terminated inside the buffer; without trimming, every key name
                // carried the rest of the buffer as NULs.
                let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                let sub = String::from_utf16_lossy(&buf[..len]);
                if !sub.to_lowercase().contains(name) {
                    continue;
                }
                let sub_wide: Vec<u16> = format!("{sub}\0").encode_utf16().collect();
                let mut subkey_handle = HKEY::default();
                if RegOpenKeyExW(
                    key,
                    windows::core::PCWSTR(sub_wide.as_ptr()),
                    Some(0),
                    KEY_READ,
                    &mut subkey_handle,
                ) != ERROR_SUCCESS
                {
                    continue;
                }
                let mut data = [0u8; 1024];
                let mut data_len = data.len() as u32;
                let mut ty = REG_SZ;
                let status = RegQueryValueExW(
                    subkey_handle,
                    windows::core::PCWSTR::null(),
                    None,
                    Some(&mut ty),
                    Some(data.as_mut_ptr()),
                    Some(&mut data_len),
                );
                let _ = RegCloseKey(subkey_handle);
                if status == ERROR_SUCCESS && data_len > 2 {
                    let wide: Vec<u16> = data[..data_len as usize]
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    let value = OsString::from_wide(&wide);
                    let value = value.to_string_lossy().trim_matches('"').to_string();
                    if Path::new(&value).exists() {
                        let _ = RegCloseKey(key);
                        return Some(value);
                    }
                }
            }
            let _ = RegCloseKey(key);
        }
    }
    None
}

fn which(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

pub fn list_installed_apps(filter: &str) -> Vec<String> {
    let mut names: Vec<String> = start_menu_shortcuts()
        .into_iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .filter(|n| {
            let low = n.to_lowercase();
            !low.starts_with("uninstall") && !low.contains("install")
        })
        .collect();
    names.sort();
    names.dedup();
    if !filter.trim().is_empty() {
        let q = filter.trim().to_lowercase();
        names.retain(|n| n.to_lowercase().contains(&q));
    }
    names.truncate(35);
    names
}

/// Registered handler command line for a URI scheme, if any.
pub fn protocol_command(scheme: &str) -> Option<String> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ, REG_SZ,
    };
    let path: Vec<u16> = format!("{scheme}\\shell\\open\\command\0")
        .encode_utf16()
        .collect();
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CLASSES_ROOT,
            windows::core::PCWSTR(path.as_ptr()),
            Some(0),
            KEY_READ,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut data = [0u8; 2048];
        let mut len = data.len() as u32;
        let mut ty = REG_SZ;
        let status = RegQueryValueExW(
            key,
            windows::core::PCWSTR::null(),
            None,
            Some(&mut ty),
            Some(data.as_mut_ptr()),
            Some(&mut len),
        );
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS || len < 2 {
            return None;
        }
        let wide: Vec<u16> = data[..len as usize]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let text = String::from_utf16_lossy(&wide).trim_matches('\0').to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

fn exe_from_command(cmd: &str) -> String {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return rest[..end].to_string();
        }
        return rest.trim_matches('"').to_string();
    }
    cmd.split_whitespace().next().unwrap_or("").to_string()
}

fn spawn(cmdline: &str, cwd: Option<&str>, console: bool) -> std::io::Result<std::process::Child> {
    let flags = if console {
        CREATE_NEW_CONSOLE
    } else {
        DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    };
    let expanded_cwd = cwd.map(expand_envs);
    std::process::Command::new("cmd.exe")
        .arg("/c")
        .arg(cmdline)
        .current_dir(
            expanded_cwd
                .filter(|c| Path::new(c).is_dir())
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default()),
        )
        .creation_flags(flags | CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

/// Expands `%VAR%` placeholders, matching the Python launcher's cwd handling.
fn expand_envs(text: &str) -> String {
    let mut out = text.to_string();
    for (key, value) in std::env::vars() {
        if out.contains('%') {
            out = out.replace(&format!("%{key}%"), &value);
        } else {
            break;
        }
    }
    out
}

fn cmdline_for(exe: &str, args: &str) -> String {
    let low = exe.to_lowercase();
    let base = if low.ends_with(".cmd") || low.ends_with(".bat") {
        format!("\"{exe}\" {args}")
    } else if args.is_empty() {
        format!("\"{exe}\"")
    } else {
        format!("\"{exe}\" {args}")
    };
    base.trim().to_string()
}

fn visible_hwnds() -> std::collections::HashSet<isize> {
    crate::win32::enum_windows(false)
        .into_iter()
        .map(|w| w.hwnd)
        .collect()
}

fn expected_exes(exe: &str, args: &str, label: &str) -> Vec<String> {
    let mut names = Vec::new();
    if !exe.is_empty() {
        if let Some(file) = Path::new(exe).file_name() {
            names.push(file.to_string_lossy().to_lowercase());
        }
    }
    let lower_args = args.to_lowercase();
    if let Some(pos) = lower_args.find("--processstart") {
        let rest = &args[pos + "--processstart".len()..];
        let token = rest.trim().trim_start_matches('"').split_whitespace().next();
        if let Some(t) = token {
            if t.to_lowercase().ends_with(".exe") {
                names.push(t.trim_matches('"').to_lowercase());
            }
        }
    }
    let lab: String = label
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if !lab.is_empty() {
        names.push(format!("{lab}.exe"));
    }
    names
}

fn wait_and_raise(
    exes: &[String],
    before: &std::collections::HashSet<isize>,
    pid: Option<u32>,
    title_hint: &str,
    timeout: f64,
) -> (bool, Option<crate::win32::WindowInfo>) {
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    let hint = title_hint.trim().to_lowercase();
    while Instant::now() < deadline {
        let exe_refs: Vec<&str> = exes.iter().map(String::as_str).collect();
        let mut wins = if exe_refs.is_empty() {
            Vec::new()
        } else {
            crate::win32::find_windows(Some(&exe_refs), None, false, true)
        };
        if wins.is_empty() && !hint.is_empty() {
            wins = crate::win32::find_windows(None, Some(&hint), false, true);
        }
        let fresh: Vec<_> = wins.iter().filter(|w| !before.contains(&w.hwnd)).cloned().collect();
        let candidate = if !fresh.is_empty() {
            fresh
        } else {
            wins.into_iter()
                .filter(|w| pid.map(|p| w.pid == p).unwrap_or(false))
                .collect()
        };
        if let Some(win) = candidate.into_iter().next() {
            std::thread::sleep(Duration::from_millis(200));
            let raised = crate::win32::force_foreground(win.hwnd);
            return (raised, Some(win));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    (false, None)
}

fn focus_existing(exes: &[String], hint: &str) -> Option<crate::win32::WindowInfo> {
    let exe_refs: Vec<&str> = exes.iter().map(String::as_str).collect();
    let mut wins = crate::win32::find_windows(Some(&exe_refs), None, false, true);
    if wins.is_empty() && hint.len() >= 4 {
        wins = crate::win32::find_windows(None, Some(&hint.to_lowercase()), false, true);
    }
    wins.retain(|w| w.title.to_lowercase() != "aria");
    let win = wins.into_iter().next()?;
    crate::win32::force_foreground(win.hwnd);
    Some(win)
}

enum Spec {
    Protocol { uri: String, label: String },
    Exe { exe: String, args: String, cwd: Option<String>, label: String },
}

fn launch_spec(name: &str) -> Option<Spec> {
    let path = find_installed_app(name)?;
    let label = if path.to_lowercase().ends_with(".lnk") {
        Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| name.to_string())
    } else {
        clean_name(name)
    };

    if path.ends_with(':') {
        return Some(Spec::Protocol {
            uri: path,
            label,
        });
    }

    if path.to_lowercase().ends_with(".lnk") {
        if let Some(info) = read_lnk(&path) {
            let target = info.get("target").and_then(|v| v.as_str()).unwrap_or("");
            if !target.is_empty() && Path::new(target).exists() && !target.to_lowercase().ends_with("explorer.exe") {
                return Some(Spec::Exe {
                    exe: target.to_string(),
                    args: info.get("args").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    cwd: info.get("cwd").and_then(|v| v.as_str()).map(str::to_string),
                    label,
                });
            }
        }
        // Unresolvable shortcut: let explorer run it.
        return Some(Spec::Exe {
            exe: path,
            args: String::new(),
            cwd: None,
            label,
        });
    }

    let exe = if Path::new(&path).is_absolute() && Path::new(&path).exists() {
        path
    } else {
        which(&path).unwrap_or(path)
    };
    Some(Spec::Exe {
        exe,
        args: String::new(),
        cwd: None,
        label,
    })
}

static LNK_MEMORY: OnceLock<Mutex<std::collections::HashMap<String, Value>>> = OnceLock::new();

fn lnk_memory() -> &'static Mutex<std::collections::HashMap<String, Value>> {
    LNK_MEMORY.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn cache_path() -> PathBuf {
    crate::config::data_dir().join("app_launch_cache.json")
}

fn read_lnk(path: &str) -> Option<Value> {
    let key = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| format!("{path}|{}", t.elapsed().map(|e| e.as_secs()).unwrap_or(0)))
        .unwrap_or_else(|| path.to_string());

    if let Some(v) = lnk_memory()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
    {
        return Some(v.clone());
    }

    let disk: Value = std::fs::read_to_string(cache_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(Value::Object(Default::default()));
    if let Some(v) = disk.get(&key) {
        lnk_memory()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.clone(), v.clone());
        return Some(v.clone());
    }

    let safe = path.replace('\'', "''");
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{safe}');\
         ConvertTo-Json -Compress @{{target=$s.TargetPath;args=$s.Arguments;cwd=$s.WorkingDirectory}}"
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let info: Value = serde_json::from_str(&text).ok()?;

    lnk_memory()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.clone(), info.clone());
    let mut merged = disk;
    if let Value::Object(map) = &mut merged {
        map.insert(key, info.clone());
        let _ = std::fs::write(
            cache_path(),
            serde_json::to_string_pretty(&merged).unwrap_or_default(),
        );
    }
    Some(info)
}

pub fn launch_uri(uri: &str, wait_for: Option<&[&str]>, label: &str) -> Value {
    let scheme = uri.split(':').next().unwrap_or(uri);
    let before = visible_hwnds();
    let cmd = protocol_command(scheme);
    let mut method = "direct_protocol_handler";
    let mut exes: Vec<String> = wait_for
        .map(|w| w.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();

    let spawn_result = if let Some(cmd) = &cmd {
        let cmdline = if cmd.contains("%1") {
            cmd.replace("%1", uri).replace("%L", uri)
        } else {
            format!("{cmd} \"{uri}\"")
        };
        let exe_path = exe_from_command(cmd);
        if let Some(name) = Path::new(&exe_path).file_name() {
            exes.push(name.to_string_lossy().to_lowercase());
        }
        spawn(&cmdline, None, false)
    } else {
        method = "direct_explorer";
        spawn(&format!("explorer.exe \"{uri}\""), None, false)
    };

    match spawn_result {
        Ok(child) => {
            let (raised, win) = if !exes.is_empty() || !label.is_empty() {
                wait_and_raise(&exes, &before, Some(child.id()), label, 8.0)
            } else {
                (false, None)
            };
            serde_json::json!({
                "status": "success", "uri": uri, "method": method, "pid": child.id(),
                "foreground": raised, "window": win.map(|w| w.title).unwrap_or_default()
            })
        }
        Err(e) => {
            log::warn!("URI launch failed for {uri}: {e}");
            serde_json::json!({ "status": "error", "error": e.to_string() })
        }
    }
}

pub fn force_launch_app(name: &str) -> Option<Value> {
    let spec = launch_spec(name)?;
    match spec {
        Spec::Protocol { uri, label } => {
            let mut res = launch_uri(&uri, None, &label);
            if let Some(m) = res.as_object_mut() {
                m.insert("opened".into(), serde_json::json!(label));
            }
            Some(res)
        }
        Spec::Exe {
            exe,
            args,
            cwd,
            label,
        } => {
            let exes = expected_exes(&exe, &args, &label);
            let multi = exes
                .iter()
                .any(|e| MULTI_INSTANCE.contains(&e.as_str()));
            if !multi {
                if let Some(existing) = focus_existing(&exes, &label) {
                    return Some(serde_json::json!({
                        "status": "success", "opened": label, "method": "focused_running_instance",
                        "already_running": true, "foreground": true, "window": existing.title
                    }));
                }
            }
            let before = visible_hwnds();
            let is_console = Path::new(&exe)
                .file_name()
                .map(|f| CONSOLE_EXES.contains(&f.to_string_lossy().to_lowercase().as_str()))
                .unwrap_or(false);
            match spawn(&cmdline_for(&exe, &args), cwd.as_deref(), is_console) {
                Ok(child) => {
                    let (raised, win) =
                        wait_and_raise(&exes, &before, Some(child.id()), &label, 9.0);
                    Some(serde_json::json!({
                        "status": "success", "opened": label, "method": "direct_exe", "path": exe,
                        "pid": child.id(), "foreground": raised,
                        "window": win.map(|w| w.title).unwrap_or_default()
                    }))
                }
                Err(e) => Some(serde_json::json!({
                    "status": "error", "error": format!("{e}")
                })),
            }
        }
    }
}

pub fn default_browser_exe() -> Option<String> {
    let pref = crate::config::get().preferred_browser;
    if !pref.trim().is_empty() {
        if let Some(found) = find_installed_app(pref.trim()) {
            if found.to_lowercase().ends_with(".exe") && Path::new(&found).exists() {
                return Some(found);
            }
        }
    }

    if let Some(default) = user_choice_browser() {
        return Some(default);
    }

    for candidate in ["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe"] {
        if let Some(path) = registry_app_path(candidate) {
            if Path::new(&path).exists() {
                return Some(path);
            }
        }
    }
    None
}

fn user_choice_browser() -> Option<String> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ,
    };
    let path: Vec<u16> =
        "Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\https\\UserChoice\0"
            .encode_utf16()
            .collect();
    let prog_id = unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(path.as_ptr()),
            Some(0),
            KEY_READ,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let name: Vec<u16> = "ProgId\0".encode_utf16().collect();
        let mut data = [0u8; 512];
        let mut len = data.len() as u32;
        let mut ty = REG_SZ;
        let status = RegQueryValueExW(
            key,
            windows::core::PCWSTR(name.as_ptr()),
            None,
            Some(&mut ty),
            Some(data.as_mut_ptr()),
            Some(&mut len),
        );
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS {
            return None;
        }
        let wide: Vec<u16> = data[..len as usize]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&wide)
            .trim_matches('\0')
            .to_string()
    };
    if prog_id.is_empty() {
        return None;
    }
    let cmd = protocol_command(&prog_id)?;
    let exe = exe_from_command(&cmd);
    let name = Path::new(&exe).file_name()?.to_string_lossy().to_lowercase();
    if Path::new(&exe).exists() && name != "openwith.exe" && name != "iexplore.exe" {
        Some(exe)
    } else {
        None
    }
}

pub fn normalize_url(target: &str) -> String {
    let t = target.trim();
    let looks_like_scheme = {
        let mut chars = t.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() => {}
            _ => return format!("https://{t}"),
        }
        let scheme: String = t
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
            .collect();
        scheme.len() < t.len() && t[scheme.len()..].starts_with("://")
    };
    if looks_like_scheme {
        t.to_string()
    } else {
        format!("https://{t}")
    }
}

pub fn force_open_url(url: &str) -> Value {
    let url = normalize_url(url);
    let Some(exe) = default_browser_exe() else {
        let _ = std::process::Command::new("cmd.exe")
            .args(["/c", "start", "", &url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        return serde_json::json!({
            "status": "success", "opened_url": url, "method": "shell_fallback", "foreground": false
        });
    };

    let exes = vec![Path::new(&exe)
        .file_name()
        .map(|f| f.to_string_lossy().to_lowercase())
        .unwrap_or_default()];
    let before = visible_hwnds();
    match spawn(&cmdline_for(&exe, &format!("\"{url}\"")), None, false) {
        Ok(child) => {
            std::thread::sleep(Duration::from_millis(500));
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut raised = false;
            let mut title = String::new();
            while Instant::now() < deadline {
                let refs: Vec<&str> = exes.iter().map(String::as_str).collect();
                let wins = crate::win32::find_windows(Some(&refs), None, false, true);
                if let Some(win) = wins.into_iter().next() {
                    raised = crate::win32::force_foreground(win.hwnd);
                    title = win.title;
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            let _ = before;
            serde_json::json!({
                "status": "success", "opened_url": url, "method": "direct_browser",
                "browser": Path::new(&exe).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
                "pid": child.id(), "foreground": raised, "window": title
            })
        }
        Err(e) => {
            log::warn!("direct browser launch failed: {e}");
            let _ = std::process::Command::new("cmd.exe")
                .args(["/c", "start", "", &url])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            serde_json::json!({
                "status": "success", "opened_url": url, "method": "shell_fallback", "foreground": false
            })
        }
    }
}

pub fn google_search_url(query: &str) -> String {
    let mut encoded = String::new();
    for b in query.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char)
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{b:02X}")),
        }
    }
    format!("https://www.google.com/search?q={encoded}")
}

/// Opens an app, URL, site or search. Order: search -> URL -> installed app -> domain -> known site -> web search.
pub fn open_target(target: &str) -> Value {
    let mut t = target.trim().to_string();
    let mut low = t.to_lowercase();
    for pfx in ["open ", "launch ", "start ", "run "] {
        if let Some(stripped) = low.strip_prefix(pfx) {
            t = t[pfx.len()..].trim().to_string();
            low = stripped.trim().to_string();
            break;
        }
    }

    if let Some(q) = low.strip_prefix("search:") {
        let mut res = force_open_url(&google_search_url(q.trim()));
        if let Some(m) = res.as_object_mut() {
            m.insert("searched".into(), serde_json::json!(q.trim()));
        }
        return res;
    }
    if low.starts_with("http://") || low.starts_with("https://") {
        return force_open_url(&t);
    }

    if let Some(res) = force_launch_app(&t) {
        return res;
    }

    if low.contains('.')
        && !low.contains(' ')
        && !low.ends_with(".exe")
        && !low.ends_with(".lnk")
        && !low.ends_with(".bat")
        && !low.ends_with(".cmd")
        && !low.ends_with(".ps1")
    {
        return force_open_url(&t);
    }
    if let Some((_, url)) = KNOWN_SITES.iter().find(|(k, _)| *k == low) {
        return force_open_url(url);
    }

    let mut res = force_open_url(&google_search_url(&t));
    if let Some(m) = res.as_object_mut() {
        m.insert("searched_web_for".into(), serde_json::json!(t));
        m.insert(
            "note".into(),
            serde_json::json!(format!(
                "'{t}' isn't an installed app I can find, so I searched the web for it."
            )),
        );
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_normalised() {
        assert_eq!(normalize_url("youtube.com"), "https://youtube.com");
        assert_eq!(normalize_url("https://x.com"), "https://x.com");
    }

    #[test]
    fn app_name_cleanup_strips_filler() {
        assert_eq!(clean_name("the Notepad app"), "notepad");
        assert_eq!(clean_name("My Code Program"), "code");
    }

    #[test]
    fn installed_app_listing_is_bounded() {
        assert!(list_installed_apps("").len() <= 35);
    }
}
