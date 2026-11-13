//! Installed game names and roots, read locally from Steam/Epic manifests (including custom drives).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub(super) struct InstalledGame {
    pub root: String,
    pub name: String,
}

pub(super) fn normal_path(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

pub(super) fn find<'a>(games: &'a [InstalledGame], path: &str) -> Option<&'a str> {
    let path = normal_path(path);
    games.iter().filter(|g| path.strip_prefix(&g.root).is_some_and(|rest| rest.starts_with('\\')))
        .max_by_key(|g| g.root.len()).map(|g| g.name.as_str())
}

pub(super) fn installed() -> Vec<InstalledGame> {
    static CACHE: Mutex<Option<(Instant, Vec<InstalledGame>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, games)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(120) { return games.clone(); }
    }
    let games = discover();
    *cache = Some((Instant::now(), games.clone()));
    games
}

// VDF values are quoted strings; only unescape quotes/backslashes, preserving Windows paths.
fn vdf_values(text: &str, key: &str) -> Vec<String> {
    static TOKENS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = TOKENS.get_or_init(|| regex::Regex::new(r#""((?:\\.|[^"\\])*)""#).unwrap());
    let tokens: Vec<_> = re.captures_iter(text).map(|c| c[1].replace(r"\\", r"\").replace(r#"\""#, "\"")).collect();
    tokens.windows(2).filter(|pair| pair[0].eq_ignore_ascii_case(key)).map(|pair| pair[1].clone()).collect()
}

fn registry_string(root: windows::Win32::System::Registry::HKEY, key: &str, value: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};
    let key: Vec<_> = key.encode_utf16().chain(Some(0)).collect();
    let value: Vec<_> = value.encode_utf16().chain(Some(0)).collect();
    let mut buf = [0u16; 2048];
    let mut bytes = std::mem::size_of_val(&buf) as u32;
    let status = unsafe { RegGetValueW(root, PCWSTR(key.as_ptr()), PCWSTR(value.as_ptr()), RRF_RT_REG_SZ,
        None, Some(buf.as_mut_ptr().cast()), Some(&mut bytes)) };
    if status != ERROR_SUCCESS { return None; }
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn discover() -> Vec<InstalledGame> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let mut steam_roots = Vec::new();
    for (hive, key, value) in [
        (HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
    ] {
        if let Some(path) = registry_string(hive, key, value) { steam_roots.push(PathBuf::from(path)); }
    }
    if let Some(path) = std::env::var_os("ProgramFiles(x86)") {
        steam_roots.push(PathBuf::from(path).join("Steam"));
    }
    let mut libraries = steam_roots.clone();
    for root in steam_roots {
        for file in [root.join("steamapps/libraryfolders.vdf"), root.join("config/libraryfolders.vdf")] {
            if let Ok(text) = std::fs::read_to_string(file) {
                libraries.extend(vdf_values(&text, "path").into_iter().map(PathBuf::from));
            }
        }
    }
    libraries.sort();
    libraries.dedup();
    let mut games = Vec::new();
    for library in libraries {
        let apps = library.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&apps) else { continue };
        for entry in entries.flatten() {
            let file = entry.path();
            if !entry.file_name().to_string_lossy().starts_with("appmanifest_") || file.extension().is_none_or(|e| e != "acf") { continue; }
            if let Ok(text) = std::fs::read_to_string(file) { add_steam(&mut games, &apps, &text); }
        }
    }
    if let Some(data) = std::env::var_os("ProgramData") {
        let folder = PathBuf::from(data).join("Epic/EpicGamesLauncher/Data/Manifests");
        if let Ok(entries) = std::fs::read_dir(folder) {
            for entry in entries.flatten() {
                if entry.path().extension().is_none_or(|e| e != "item") { continue; }
                if let Ok(text) = std::fs::read_to_string(entry.path()) { add_epic(&mut games, &text); }
            }
        }
    }
    games
}

fn add_steam(games: &mut Vec<InstalledGame>, apps: &Path, text: &str) {
    let name = vdf_values(text, "name").into_iter().next();
    let dir = vdf_values(text, "installdir").into_iter().next();
    if let (Some(name), Some(dir)) = (name, dir) {
        if !name.is_empty() && !dir.is_empty() && !Path::new(&dir).is_absolute() && !dir.contains("..") {
            games.push(InstalledGame { root: normal_path(&apps.join("common").join(dir).to_string_lossy()), name });
        }
    }
}

fn add_epic(games: &mut Vec<InstalledGame>, text: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else { return };
    if let (Some(root), Some(name)) = (value["InstallLocation"].as_str(), value["DisplayName"].as_str()) {
        if !root.is_empty() && !name.is_empty() && value["LaunchExecutable"].as_str().is_some_and(|e| e.to_lowercase().ends_with(".exe")) {
            games.push(InstalledGame { root: normal_path(root), name: name.into() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_libraries_and_manifest_names() {
        assert_eq!(vdf_values(r#""libraryfolders" { "1" { "path" "E:\\Games\\SteamLibrary" } }"#, "path"), vec![r"E:\Games\SteamLibrary"]);
        let mut games = Vec::new();
        add_steam(&mut games, Path::new(r"E:\Games\SteamLibrary\steamapps"), r#""AppState" { "name" "Apex Legends" "installdir" "Apex Legends" }"#);
        assert_eq!(find(&games, r"E:\Games\SteamLibrary\steamapps\common\Apex Legends\r5apex.exe"), Some("Apex Legends"));
        assert_eq!(find(&games, r"E:\Games\SteamLibrary\steamapps\common\Apex Legends Editor\app.exe"), None);
    }
    #[test]
    fn epic_custom_paths_and_bad_manifests() {
        let mut games = Vec::new();
        add_epic(&mut games, r#"{"InstallLocation":"F:/Custom/Fortnite","DisplayName":"Fortnite","LaunchExecutable":"FortniteGame/Binaries/Win64/FortniteClient-Win64-Shipping.exe"}"#);
        add_epic(&mut games, "invalid");
        assert_eq!(find(&games, r"f:\custom\fortnite\FortniteGame\Binaries\Win64\FortniteClient-Win64-Shipping.exe"), Some("Fortnite"));
        assert_eq!(games.len(), 1);
    }
}
