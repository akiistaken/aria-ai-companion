//! Local asset server for the avatar stage, so WebView2 can load .vrm / Live2D / mp3 files
//! without CORS problems.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tiny_http::{Header, Response, Server};

pub struct AssetServer {
    pub base_url: String,
}

static SERVER: OnceLock<AssetServer> = OnceLock::new();

/// Starts the server once and returns its base URL.
pub fn start() -> &'static str {
    &SERVER
        .get_or_init(|| {
            let server = match Server::http("127.0.0.1:0") {
                Ok(s) => s,
                Err(e) => {
                    log::error!("asset server could not bind: {e}");
                    return AssetServer {
                        base_url: String::new(),
                    };
                }
            };
            let port = server
                .server_addr()
                .to_ip()
                .map(|a| a.port())
                .unwrap_or(0);
            let base_url = format!("http://127.0.0.1:{port}");

            std::thread::spawn(move || {
                for request in server.incoming_requests() {
                    let path = request.url().split('?').next().unwrap_or("/").to_string();
                    let decoded = percent_decode(&path);
                    let file = if let Some(rest) = decoded.strip_prefix("/custom/") {
                        custom_root()
                            .map(|root| root.join(rest.trim_start_matches('/')))
                            .filter(|p| p.is_file())
                    } else {
                        let root = crate::config::base_dir();
                        safe_join(&root, decoded.trim_start_matches('/'))
                    };

                    let response = match file {
                        Some(path) if path.is_file() => serve_file(&path),
                        _ => {
                            log::debug!("asset 404: {decoded}");
                            Response::from_string("not found").with_status_code(404)
                        }
                    };
                    let _ = request.respond(response);
                }
            });

            log::info!("asset server on {base_url}");
            AssetServer { base_url }
        })
        .base_url
        .as_str()
}

fn safe_join(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut out = root.to_path_buf();
    for part in relative.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        // Refuse traversal outright; the stage only ever asks for files under the project.
        if part == ".." {
            return None;
        }
        out.push(part);
    }
    Some(out)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "model3" | "exp3" | "physics3" | "cdi3" => "application/json",
        "vrm" | "glb" => "model/gltf-binary",
        "moc3" | "moc" => "application/octet-stream",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "css" => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn serve_file(path: &Path) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut data = Vec::new();
    let ok = std::fs::File::open(path)
        .and_then(|mut f| f.read_to_end(&mut data))
        .is_ok();
    if !ok {
        return Response::from_string("read error").with_status_code(500);
    }
    let mut response = Response::from_data(data);
    if let Ok(header) = Header::from_bytes(&b"Content-Type"[..], mime_for(path).as_bytes()) {
        response.add_header(header);
    }
    if let Ok(header) = Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]) {
        response.add_header(header);
    }
    if let Ok(header) = Header::from_bytes(
        &b"Cache-Control"[..],
        if path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("mp3"))
            .unwrap_or(false)
        {
            &b"no-cache"[..]
        } else {
            &b"no-cache"[..]
        },
    ) {
        response.add_header(header);
    }
    response
}

/// Registers a file outside the project tree so the stage can load it.
pub fn register_custom(path: &Path) -> String {
    let base = start();
    if base.is_empty() {
        return String::new();
    }
    // Custom files are exposed under /custom/<name>; the server resolves the parent via env.
    let parent = path.parent().unwrap_or(Path::new("."));
    CUSTOM_ROOT
        .set(parent.to_path_buf())
        .ok();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    format!("{base}/custom/{name}")
}

static CUSTOM_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn custom_root() -> Option<&'static PathBuf> {
    CUSTOM_ROOT.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_is_refused() {
        let root = Path::new("C:\\root");
        assert!(safe_join(root, "../secret").is_none());
        assert!(safe_join(root, "assets/models/a.vrm").is_some());
    }

    #[test]
    fn percent_decoding_handles_unicode() {
        assert_eq!(percent_decode("%E9%AD%94%E5%A5%B3.model3.json"), "魔女.model3.json");
    }
}
