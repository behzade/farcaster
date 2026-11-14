use std::{
    collections::HashMap,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use url::Url;

type Roots = Arc<Mutex<HashMap<String, (PathBuf, Instant)>>>;
struct Server {
    address: std::net::SocketAddr,
    roots: Roots,
}
static SERVER: OnceLock<Result<Server, String>> = OnceLock::new();

// A loopback origin lets modules and fetch work while retaining relative assets.
// Each unguessable route grants access only to the selected file's directory.
pub(super) fn url(path: &Path) -> Result<Url, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("Cannot preview {}: {e}", path.display()))?;
    if !path.is_file() {
        return Err("HTML preview requires a file.".into());
    }
    let server = SERVER.get_or_init(start).as_ref().map_err(Clone::clone)?;
    let root = path
        .parent()
        .ok_or("HTML file has no parent directory")?
        .to_path_buf();
    let mut roots = server.roots.lock().map_err(|e| e.to_string())?;
    let token = grant_root(&mut roots, root);
    let mut url =
        Url::parse(&format!("http://{}/{token}/", server.address)).map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|()| "Invalid preview URL")?
        .pop_if_empty()
        .push(&path.file_name().unwrap_or_default().to_string_lossy());
    Ok(url)
}

fn grant_root(roots: &mut HashMap<String, (PathBuf, Instant)>, root: PathBuf) -> String {
    roots.retain(|_, (_, created)| created.elapsed() < Duration::from_secs(3600));
    if let Some((token, (_, created))) = roots.iter_mut().find(|(_, (path, _))| *path == root) {
        *created = Instant::now();
        return token.clone();
    }
    if roots.len() >= 128
        && let Some(oldest) = roots
            .iter()
            .min_by_key(|(_, (_, created))| *created)
            .map(|(token, _)| token.clone())
    {
        roots.remove(&oldest);
    }
    let token = uuid::Uuid::new_v4().to_string();
    roots.insert(token.clone(), (root, Instant::now()));
    token
}

fn start() -> Result<Server, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let roots: Roots = Arc::default();
    let request_roots = roots.clone();
    std::thread::Builder::new()
        .name("html-preview".into())
        .spawn(move || {
            for stream in listener.incoming() {
                if let Ok(stream) = stream {
                    let roots = request_roots.clone();
                    let _ = std::thread::Builder::new()
                        .name("html-preview-request".into())
                        .spawn(move || {
                            let _ = serve(stream, &roots);
                        });
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Server { address, roots })
}

fn asset_path(root: &Path, target: &str) -> Option<PathBuf> {
    let base = Url::from_directory_path(root).ok()?;
    let path = base
        .join(target)
        .ok()?
        .to_file_path()
        .ok()?
        .canonicalize()
        .ok()?;
    let visible = path
        .strip_prefix(root)
        .ok()?
        .components()
        .all(|part| !part.as_os_str().to_string_lossy().starts_with('.'));
    (path.starts_with(root) && path.is_file() && visible).then_some(path)
}

fn serve(mut stream: TcpStream, roots: &Roots) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut request = Vec::new();
    let mut byte = [0];
    while request.len() < 8192 && stream.read(&mut byte)? != 0 {
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&request);
    let mut words = text.lines().next().unwrap_or_default().split_whitespace();
    let method = words.next().unwrap_or_default();
    let target = words.next().unwrap_or_default();
    let path = target
        .strip_prefix('/')
        .and_then(|s| s.split_once('/'))
        .and_then(|(token, target)| {
            let roots = roots.lock().ok()?;
            let (root, created) = roots.get(token)?;
            if created.elapsed() >= Duration::from_secs(3600) {
                return None;
            }
            asset_path(root, target)
        });
    if !matches!(method, "GET" | "HEAD") || path.is_none() {
        stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
        return Ok(());
    }
    let path = path.expect("checked preview path");
    let file = std::fs::File::open(&path)?;
    let length = file.metadata()?.len();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        mime(&path),
        length
    )?;
    if method == "GET" {
        std::io::copy(&mut file.take(length), &mut stream)?;
    }
    Ok(())
}

fn mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "mp4" => "video/mp4",
        "pdf" => "application/pdf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
#[path = "html_preview_tests.rs"]
mod tests;
