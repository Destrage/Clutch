//! The overlay server: Clutch's own overlay pages (Alerts) for browser sources, on the loopback
//! address only. Every request needs the install's token; user files are served by id, never by
//! path. Pages get their settings and live events over a WebSocket.
//!
//! Routes (all with `?t=<token>`): `/alerts` (the page), `/asset/<id>` (a registered file),
//! `/ws?source=<id>` (JSON messages: `config` on connect and when it changes, then `alert`s).

pub mod alerts;

use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

const ALERTS_PAGE: &str = include_str!("alerts.html");
/// Clients are pinged this often so dead ones are noticed.
const PING_EVERY: Duration = Duration::from_secs(15);

/// The running server's base address (`http://127.0.0.1:<port>`) and token, for page URLs.
static BASE: RwLock<Option<(String, String)>> = RwLock::new(None);

/// The address of an overlay page for `source` (empty when the server isn't running).
pub fn page_url(page: &str, source: &str) -> String {
    match BASE.read().ok().and_then(|b| b.clone()) {
        Some((base, token)) => format!("{base}/{page}?t={token}&source={source}"),
        None => String::new(),
    }
}

/// A new random token.
pub fn new_token() -> String {
    estream_clips::integrations::gsi::new_token()
}

struct Client {
    source: String,
    tx: Sender<String>,
}

#[derive(Default)]
struct Shared {
    /// The page settings per source id.
    configs: Mutex<HashMap<String, Value>>,
    /// Registered files by id.
    assets: Mutex<HashMap<String, PathBuf>>,
    clients: Mutex<Vec<Client>>,
}

pub struct OverlayServer {
    port: u16,
    token: String,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
}

impl OverlayServer {
    /// Listens on `127.0.0.1:port`, or a free port when that one is taken.
    pub fn start(port: u16, token: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .or_else(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let shared = Arc::new(Shared::default());
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (shared, stop, token) = (shared.clone(), stop.clone(), token.to_owned());
            std::thread::Builder::new()
                .name("estream-overlay".into())
                .spawn(move || accept(&listener, &shared, &stop, &token))?;
        }
        if let Ok(mut b) = BASE.write() {
            *b = Some((format!("http://127.0.0.1:{port}"), token.to_owned()));
        }
        tracing::info!(port, "overlay server started");
        Ok(Self {
            port,
            token: token.to_owned(),
            shared,
            stop,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Registers a file and returns its address (stable per path).
    pub fn asset_url(&self, path: &Path) -> String {
        let id = asset_id(path);
        if let Ok(mut a) = self.shared.assets.lock() {
            a.insert(id.clone(), path.to_path_buf());
        }
        format!("/asset/{id}?t={}", self.token)
    }

    /// Sets the page settings for `source`, sending them to its open pages when they changed.
    pub fn set_config(&self, source: &str, config: Value) {
        let changed = self
            .shared
            .configs
            .lock()
            .map(|mut c| c.insert(source.to_owned(), config.clone()).as_ref() != Some(&config))
            .unwrap_or(false);
        if changed {
            let msg = serde_json::json!({ "type": "config", "config": config }).to_string();
            self.send(Some(source), &msg);
        }
    }

    /// Forgets sources that are gone.
    pub fn retain_sources(&self, keep: &[String]) {
        if let Ok(mut c) = self.shared.configs.lock() {
            c.retain(|s, _| keep.contains(s));
        }
    }

    /// Sends a message to the pages of `source`, or to every page.
    pub fn send(&self, source: Option<&str>, message: &str) {
        if let Ok(mut clients) = self.shared.clients.lock() {
            clients.retain(|c| {
                if source.is_some_and(|s| s != c.source) {
                    return true;
                }
                c.tx.send(message.to_owned()).is_ok()
            });
        }
    }
}

impl Drop for OverlayServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut b) = BASE.write() {
            *b = None;
        }
    }
}

/// A file's id: a hash of its path (so the same file keeps its address).
fn asset_id(path: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn accept(listener: &TcpListener, shared: &Arc<Shared>, stop: &AtomicBool, token: &str) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let (shared, token) = (shared.clone(), token.to_owned());
                let _ = std::thread::Builder::new()
                    .name("estream-overlay-conn".into())
                    .spawn(move || {
                        if let Err(e) = handle(stream, &shared, &token) {
                            tracing::debug!(error = %e, "overlay request failed");
                        }
                    });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                tracing::warn!(error = %e, "overlay server stopped accepting");
                return;
            }
        }
    }
}

/// A parsed request head.
struct Request {
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
}

fn read_request(stream: &TcpStream) -> std::io::Result<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let target = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let query = query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    Ok(Request {
        path: path.to_owned(),
        query,
        headers,
    })
}

/// Compares without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn handle(mut stream: TcpStream, shared: &Shared, token: &str) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let req = read_request(&stream)?;
    if !req.query.get("t").is_some_and(|t| same(t, token)) {
        return respond(&mut stream, "403 Forbidden", "text/plain", b"forbidden");
    }
    match req.path.as_str() {
        "/alerts" => respond(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            ALERTS_PAGE.as_bytes(),
        ),
        "/ws" => websocket(stream, &req, shared),
        p => match p.strip_prefix("/asset/") {
            Some(id) => {
                let path = shared.assets.lock().ok().and_then(|a| a.get(id).cloned());
                match path {
                    Some(path) => serve_file(&mut stream, &path, req.headers.get("range")),
                    None => respond(&mut stream, "404 Not Found", "text/plain", b"not found"),
                }
            }
            None => respond(&mut stream, "404 Not Found", "text/plain", b"not found"),
        },
    }
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("webm") => "video/webm",
        Some("mp4") => "video/mp4",
        Some("webp") => "image/webp",
        Some("png" | "apng") => "image/png",
        Some("gif") => "image/gif",
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        Some("mp3") => "audio/mpeg",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Serves a file, honouring a `bytes=start-[end]` range (video elements ask for ranges).
fn serve_file(stream: &mut TcpStream, path: &Path, range: Option<&String>) -> std::io::Result<()> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return respond(stream, "404 Not Found", "text/plain", b"not found");
    };
    let len = file.metadata()?.len();
    let kind = content_type(path);
    let span = range
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.split_once('-'))
        .and_then(|(a, b)| {
            let start: u64 = a.parse().ok()?;
            let end: u64 = if b.is_empty() {
                len.checked_sub(1)?
            } else {
                b.parse().ok()?
            };
            (start <= end && end < len).then_some((start, end))
        });
    match span {
        Some((start, end)) => {
            let n = end - start + 1;
            write!(
                stream,
                "HTTP/1.1 206 Partial Content\r\nContent-Type: {kind}\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{end}/{len}\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n"
            )?;
            file.seek(SeekFrom::Start(start))?;
            std::io::copy(&mut file.take(n), stream)?;
        }
        None => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nAccept-Ranges: bytes\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
            )?;
            std::io::copy(&mut file, stream)?;
        }
    }
    Ok(())
}

fn websocket(mut stream: TcpStream, req: &Request, shared: &Shared) -> std::io::Result<()> {
    use tungstenite::protocol::{Message, Role, WebSocket};
    let Some(key) = req.headers.get("sec-websocket-key") else {
        return respond(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"not a websocket",
        );
    };
    let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
    write!(
        stream,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;
    let source = req.query.get("source").cloned().unwrap_or_default();
    let (tx, rx): (Sender<String>, Receiver<String>) = channel();
    let config = shared
        .configs
        .lock()
        .ok()
        .and_then(|c| c.get(&source).cloned());
    if let Some(config) = config {
        let _ = tx.send(serde_json::json!({ "type": "config", "config": config }).to_string());
    }
    if let Ok(mut clients) = shared.clients.lock() {
        clients.push(Client { source, tx });
    }
    stream.set_read_timeout(None)?;
    let mut socket = WebSocket::from_raw_socket(stream, Role::Server, None);
    loop {
        match rx.recv_timeout(PING_EVERY) {
            Ok(text) => {
                if socket.send(Message::text(text)).is_err() {
                    return Ok(());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if socket.send(Message::Ping(Vec::new().into())).is_err() {
                    return Ok(());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(port: u16, target: &str, extra: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {target} HTTP/1.1\r\nHost: x\r\n{extra}\r\n").unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    #[test]
    fn requests_need_the_token_and_assets_are_served_by_id() {
        let server = OverlayServer::start(0, "secret").unwrap();
        let port = server.port();
        assert!(get(port, "/alerts", "").starts_with("HTTP/1.1 403"));
        assert!(get(port, "/alerts?t=wrong", "").starts_with("HTTP/1.1 403"));
        assert!(get(port, "/alerts?t=secret", "").starts_with("HTTP/1.1 200"));

        let file = std::env::temp_dir().join(format!("estream-overlay-{}.wav", std::process::id()));
        std::fs::write(&file, b"0123456789").unwrap();
        let url = server.asset_url(&file);
        let whole = get(port, &url, "");
        assert!(
            whole.starts_with("HTTP/1.1 200") && whole.ends_with("0123456789"),
            "{whole}"
        );
        let part = get(port, &url, "Range: bytes=2-4\r\n");
        assert!(
            part.starts_with("HTTP/1.1 206") && part.ends_with("234"),
            "{part}"
        );
        assert!(get(port, "/asset/0000000000000000?t=secret", "").starts_with("HTTP/1.1 404"));
        // Paths aren't ids.
        let by_path = format!("/asset/{}?t=secret", file.display());
        assert!(get(port, &by_path, "").starts_with("HTTP/1.1 404"));
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn pages_get_their_config_then_events() {
        let server = OverlayServer::start(0, "tok").unwrap();
        server.set_config("src1", serde_json::json!({ "a": 1 }));
        let url = format!("ws://127.0.0.1:{}/ws?t=tok&source=src1", server.port());
        let (mut ws, _) = tungstenite::connect(url).unwrap();
        let first = ws.read().unwrap().into_text().unwrap();
        assert!(
            first.contains("\"config\"") && first.contains("\"a\":1"),
            "{first}"
        );
        // The client registers right after the handshake; give it a moment.
        std::thread::sleep(Duration::from_millis(100));
        server.send(Some("other"), "not for you");
        server.send(Some("src1"), "{\"type\":\"alert\"}");
        let next = ws.read().unwrap().into_text().unwrap();
        assert_eq!(next.as_str(), "{\"type\":\"alert\"}");
    }
}
