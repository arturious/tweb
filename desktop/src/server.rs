// Serves the built Telegram Web (tweb `dist/`) on a fixed loopback port.
// The port must stay the same between runs: the session lives in IndexedDB,
// which belongs to the origin, so a new port would mean signing in again.
// A real http://127.0.0.1 origin (not tauri://) keeps the Service Worker working.

use std::{
  fs,
  path::{Component, Path, PathBuf},
  thread,
};
use tiny_http::{Header, Response, Server};

pub const PORT: u16 = 47800;

// Roots are tried in order, like tweb's own server.js: the build (`dist`) first,
// then `public`, which holds the assets the build does not copy (copyPublicDir: false)
pub fn start(roots: Vec<PathBuf>) -> Result<String, String> {
  let addr = format!("127.0.0.1:{PORT}");
  let server = Server::http(&addr).map_err(|e| format!("cannot listen on {addr}: {e}"))?;

  thread::spawn(move || {
    for request in server.incoming_requests() {
      let found = roots.iter().find_map(|root| resolve(root, request.url()));
      let response = match found {
        Some(path) => match fs::read(&path) {
          Ok(body) => Response::from_data(body)
            .with_header(header("Content-Type", mime(&path)))
            .with_header(header("Cache-Control", "no-cache")),
          Err(_) => not_found(),
        },
        None => not_found(),
      };
      let _ = request.respond(response);
    }
  });

  Ok(format!("http://{addr}/"))
}

// Maps a request URL to a file under root, refusing anything that climbs out of it
fn resolve(root: &Path, url: &str) -> Option<PathBuf> {
  let path = url.split(['?', '#']).next().unwrap_or("/");
  let relative = Path::new(path.trim_start_matches('/'));
  if relative.components().any(|c| !matches!(c, Component::Normal(_))) {
    return None;
  }

  let mut file = root.join(relative);
  if file.is_dir() {
    file.push("index.html");
  }
  file.is_file().then_some(file)
}

fn mime(path: &Path) -> &'static str {
  match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
    "html" => "text/html; charset=utf-8",
    "js" | "mjs" => "text/javascript; charset=utf-8",
    "css" => "text/css; charset=utf-8",
    "json" | "map" => "application/json",
    "webmanifest" => "application/manifest+json",
    "wasm" => "application/wasm",
    "svg" => "image/svg+xml",
    "png" => "image/png",
    "jpg" | "jpeg" => "image/jpeg",
    "gif" => "image/gif",
    "webp" => "image/webp",
    "ico" => "image/x-icon",
    "woff" => "font/woff",
    "woff2" => "font/woff2",
    "ttf" => "font/ttf",
    "mp3" => "audio/mpeg",
    "ogg" => "audio/ogg",
    "mp4" => "video/mp4",
    "webm" => "video/webm",
    "txt" | "strings" => "text/plain; charset=utf-8",
    _ => "application/octet-stream",
  }
}

fn header(name: &str, value: &str) -> Header {
  Header::from_bytes(name, value).expect("static header is valid")
}

fn not_found() -> Response<std::io::Cursor<Vec<u8>>> {
  Response::from_string("Not found").with_status_code(404)
}
