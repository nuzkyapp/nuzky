//! Share: a phone or another computer on the same network works on the open project through a web page.
//!
//! Off until the user turns it on. It listens only on this computer's address in the local network, on a random port,
//! and every request needs the secret from the QR code, which the first visit trades for a cookie. The page adds media,
//! talks to the AI panel, seeks, exports and downloads that one export. The editor's window does each of those as if
//! the user had clicked there, so the AI lock, undo and the checks of an import stay the same.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use nuzky_engine::thumbnail::{ImageKind, RenderedThumbnail};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::preview_server::{HEADER_LEN, PreviewServer};
use crate::{AppState, CmdResult, err, store, thumbs};

const PAGE: &str = include_str!("../remote/index.html");
const SCRIPT: &str = include_str!("../remote/app.js");
const STYLE: &str = include_str!("../remote/app.css");
const EXPIRED: &str = "<!doctype html><meta name=viewport content=\"width=device-width\"><body style=\"background:#060607;\
color:#f2f2f5;font:15px -apple-system,system-ui,sans-serif;padding:32px\"><p>This link no longer works. Scan the code \
in Nuzky again.</p>";

const COOKIE: &str = "nuzky_share";
const MAX_CONNECTIONS: usize = 64;
const MAX_HEAD: usize = 16 * 1024;
const MAX_JSON: usize = 64 * 1024;
const MAX_UPLOAD: u64 = 32 << 30;
const MAX_TEXT: usize = 8000;
/// Sharing turns itself off after this long with no page open and no request.
const IDLE: Duration = Duration::from_secs(30 * 60);
const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "mkv", "webm", "avi", "mts", "mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", "png", "jpg",
    "jpeg", "heic", "heif", "webp", "gif", "bmp",
];

#[derive(Default)]
pub struct Remote(Mutex<Option<Arc<Share>>>);

struct Share {
    app: AppHandle,
    preview: Arc<PreviewServer>,
    token: String,
    host: String,
    stopped: AtomicBool,
    /// The page's state as the editor last published it, with its version.
    published: Mutex<(u64, String)>,
    /// The one file the page may download: the last export it started.
    export: Mutex<Option<PathBuf>>,
    /// The preview frame as JPEG, with the frame's number.
    frame: Mutex<(u64, Option<Arc<Vec<u8>>>)>,
    /// Open pages, by stream, as the device they run on.
    pages: Mutex<HashMap<u64, &'static str>>,
    next_page: AtomicU64,
    connections: AtomicUsize,
    last_request: Mutex<Instant>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sharing {
    url: String,
    /// The URL as a QR code, an SVG document.
    qr: String,
}

/// What the page asks the editor to do.
#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
enum Action {
    /// Only an upload adds a file, never a request naming a path.
    #[serde(skip_deserializing)]
    Add {
        path: String,
    },
    Send {
        text: String,
    },
    Choose {
        id: String,
        label: String,
    },
    Seek {
        us: i64,
    },
    Stop,
    Export,
}

#[tauri::command]
pub fn remote_start(app: AppHandle, state: State<'_, AppState>, remote: State<'_, Remote>) -> CmdResult<Sharing> {
    let mut current = remote.0.lock().unwrap();
    if let Some(share) = current.as_ref() {
        return sharing(share);
    }
    let ip = local_address().ok_or("Connect this computer to Wi-Fi or a network first")?;
    let listener = TcpListener::bind((ip, 0)).map_err(err)?;
    listener.set_nonblocking(true).map_err(err)?;
    let port = listener.local_addr().map_err(err)?.port();
    let share = Arc::new(Share {
        app: app.clone(),
        preview: state.preview.clone(),
        token: format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple()),
        host: format!("{ip}:{port}"),
        stopped: AtomicBool::new(false),
        published: Mutex::new((0, "null".into())),
        export: Mutex::new(None),
        frame: Mutex::new((u64::MAX, None)),
        pages: Mutex::default(),
        next_page: AtomicU64::new(0),
        connections: AtomicUsize::new(0),
        last_request: Mutex::new(Instant::now()),
    });
    let accepting = share.clone();
    thread::Builder::new().name("share-accept".into()).spawn(move || accept(listener, accepting)).map_err(err)?;
    let idle = share.clone();
    thread::Builder::new().name("share-idle".into()).spawn(move || watch_idle(idle)).map_err(err)?;
    let shown = sharing(&share);
    *current = Some(share);
    shown
}

#[tauri::command]
pub fn remote_stop(remote: State<'_, Remote>) {
    if let Some(share) = remote.0.lock().unwrap().take() {
        share.stop();
    }
}

/// The page's state: the editor sends it whenever it changes, and every open page gets it.
#[tauri::command]
pub fn remote_publish(remote: State<'_, Remote>, state: String) -> CmdResult<()> {
    let Some(share) = remote.0.lock().unwrap().clone() else { return Ok(()) };
    let mut value: serde_json::Value = serde_json::from_str(&state).map_err(err)?;
    if let Some(exported) = value.get_mut("exported").filter(|e| e.is_object())
        && let Some(path) = share.export.lock().unwrap().as_ref()
    {
        exported["bytes"] = std::fs::metadata(path).map(|m| m.len()).ok().into();
    }
    let mut published = share.published.lock().unwrap();
    *published = (published.0 + 1, value.to_string());
    Ok(())
}

/// A new file in the Videos folder for an export the page asked for, which only the page may then download.
#[tauri::command]
pub fn remote_export_path(remote: State<'_, Remote>, name: String) -> CmdResult<String> {
    let share = remote.0.lock().unwrap().clone().ok_or("Sharing is off")?;
    let dir = dirs::video_dir().unwrap_or_else(store::data_dir);
    std::fs::create_dir_all(&dir).map_err(err)?;
    let stem = clean_name(&name).unwrap_or_else(|| "Video".into());
    let path = (1..)
        .map(|n| dir.join(if n == 1 { format!("{stem}.mp4") } else { format!("{stem} {n}.mp4") }))
        .find(|p| !p.exists())
        .unwrap();
    *share.export.lock().unwrap() = Some(path.clone());
    Ok(path.to_string_lossy().into_owned())
}

fn sharing(share: &Share) -> CmdResult<Sharing> {
    let url = format!("http://{}/?k={}", share.host, share.token);
    let qr = qrcode::QrCode::new(url.as_bytes()).map_err(err)?;
    let qr = qr
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(232, 232)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    Ok(Sharing { url, qr })
}

/// This computer's address in the network its default route goes through, only when that network is private: on a
/// public address the page would be on the internet. Connecting a UDP socket sends nothing.
fn local_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if ip.is_private() || ip.is_link_local() => Some(IpAddr::V4(ip)),
        _ => None,
    }
}

impl Share {
    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.app.emit("remote-pages", Vec::<&str>::new()).ok();
    }

    fn pages_changed(&self) {
        let pages: Vec<&str> = self.pages.lock().unwrap().values().copied().collect();
        self.app.emit("remote-pages", pages).ok();
    }

    fn act(&self, action: &Action) {
        self.app.emit("remote-action", action).ok();
    }

    fn authorized(&self, req: &Request) -> bool {
        req.header("cookie").is_some_and(|cookies| {
            cookies
                .split(';')
                .filter_map(|c| c.trim().split_once('='))
                .any(|(k, v)| k == COOKIE && same(v, &self.token))
        })
    }
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn accept(listener: TcpListener, share: Arc<Share>) {
    while !share.stopped.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                // macOS and Windows hand out accepted sockets in the listener's non-blocking mode.
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                if share.connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    share.connections.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                let share = share.clone();
                let spawned = thread::Builder::new().name("share-request".into()).spawn({
                    let share = share.clone();
                    move || {
                        serve(&share, stream);
                        share.connections.fetch_sub(1, Ordering::SeqCst);
                    }
                });
                if spawned.is_err() {
                    share.connections.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                log::warn!("Sharing stopped accepting: {e}");
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

fn watch_idle(share: Arc<Share>) {
    while !share.stopped.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_secs(10));
        let quiet = share.pages.lock().unwrap().is_empty() && share.last_request.lock().unwrap().elapsed() > IDLE;
        if quiet {
            let remote = share.app.state::<Remote>();
            let mut current = remote.0.lock().unwrap();
            if current.as_ref().is_some_and(|s| Arc::ptr_eq(s, &share)) {
                *current = None;
                share.stop();
                share.app.emit("remote-stopped", "idle").ok();
            }
        }
    }
}

struct Request {
    method: String,
    path: String,
    query: String,
    headers: Vec<(String, String)>,
    /// What came after the head in the first read: the start of the body.
    body_start: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn param(&self, name: &str) -> Option<String> {
        self.query
            .split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == name)
            .map(|(_, v)| percent_encoding::percent_decode_str(&v.replace('+', " ")).decode_utf8_lossy().into_owned())
    }

    fn content_length(&self) -> Option<u64> {
        self.header("content-length")?.trim().parse().ok()
    }
}

fn read_head(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
    };
    let mut headers = [httparse::EMPTY_HEADER; 48];
    let mut parsed = httparse::Request::new(&mut headers);
    parsed.parse(&buf[..end]).ok()?;
    let (path, query) = parsed.path?.split_once('?').unwrap_or((parsed.path?, ""));
    Some(Request {
        method: parsed.method?.to_string(),
        path: path.to_string(),
        query: query.to_string(),
        headers: parsed
            .headers
            .iter()
            .map(|h| (h.name.to_string(), String::from_utf8_lossy(h.value).into_owned()))
            .collect(),
        body_start: buf[end..].to_vec(),
    })
}

struct Response {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Response {
    fn new(status: u16, kind: &str, body: impl Into<Vec<u8>>) -> Self {
        Self { status, headers: vec![("Content-Type", kind.into())], body: body.into() }
    }

    fn empty(status: u16) -> Self {
        Self::new(status, "text/plain; charset=utf-8", "")
    }

    fn with(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        303 => "See Other",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        411 => "Length Required",
        413 => "Content Too Large",
        416 => "Range Not Satisfiable",
        _ => "Error",
    }
}

fn write_head(
    stream: &mut TcpStream,
    status: u16,
    headers: &[(&str, String)],
    length: Option<u64>,
) -> std::io::Result<()> {
    let mut head =
        format!("HTTP/1.1 {status} {}\r\nConnection: close\r\nCache-Control: no-store\r\n", status_text(status));
    head.push_str("X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n");
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(length) = length {
        head.push_str(&format!("Content-Length: {length}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())
}

fn send(stream: &mut TcpStream, res: Response) {
    if write_head(stream, res.status, &res.headers, Some(res.body.len() as u64)).is_ok() {
        stream.write_all(&res.body).ok();
    }
}

fn serve(share: &Arc<Share>, mut stream: TcpStream) {
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(30))).ok();
    let Some(req) = read_head(&mut stream) else { return };
    if share.stopped.load(Ordering::SeqCst) {
        return;
    }
    // The page's own address only, so a site the browser visits cannot reach it under a name of its own.
    if req.header("host") != Some(share.host.as_str()) {
        return send(&mut stream, Response::empty(403));
    }
    *share.last_request.lock().unwrap() = Instant::now();
    if req.method == "GET"
        && req.path == "/"
        && let Some(key) = req.param("k")
    {
        if !same(&key, &share.token) {
            return send(&mut stream, Response::new(403, "text/html; charset=utf-8", EXPIRED));
        }
        // Lax, as a link clicked in a web mail or chat is another site's navigation, which a Strict cookie would miss.
        let cookie = format!("{COOKIE}={}; Path=/; HttpOnly; SameSite=Lax", share.token);
        return send(&mut stream, Response::empty(303).with("Location", "/").with("Set-Cookie", cookie));
    }
    if !share.authorized(&req) {
        return send(&mut stream, Response::new(403, "text/html; charset=utf-8", EXPIRED));
    }
    if req.method == "POST" {
        // A form or script on another site can neither add this header nor send the cookie with a POST (SameSite).
        let from_page = req.header("x-nuzky") == Some("1")
            && req.header("origin").is_none_or(|o| o == format!("http://{}", share.host));
        if !from_page {
            return send(&mut stream, Response::empty(403));
        }
    }
    let res = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => Response::new(200, "text/html; charset=utf-8", PAGE).with(
            "Content-Security-Policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; media-src 'self'; \
             connect-src 'self'; form-action 'none'; frame-ancestors 'none'; base-uri 'none'",
        ),
        ("GET", "/app.js") => Response::new(200, "text/javascript; charset=utf-8", SCRIPT),
        ("GET", "/app.css") => Response::new(200, "text/css; charset=utf-8", STYLE),
        ("GET", "/events") => return events(share, stream, &req),
        ("GET", "/frame.jpg") => frame(share),
        ("GET", "/video") => return video(share, stream, &req),
        ("GET", path) if path.starts_with("/thumb/") => thumb(share, &path["/thumb/".len()..]),
        ("POST", "/upload") => refused_unread(&mut stream, &req).unwrap_or_else(|| upload(share, &mut stream, req)),
        ("POST", "/action") => action(share, &mut stream, req),
        _ => Response::empty(404),
    };
    send(&mut stream, res);
}

fn device(agent: &str) -> &'static str {
    if agent.contains("iPhone") {
        "iPhone"
    } else if agent.contains("iPad") {
        "iPad"
    } else if agent.contains("Android") {
        if agent.contains("Mobile") { "Android phone" } else { "Android tablet" }
    } else if agent.contains("Macintosh") {
        "Mac"
    } else if agent.contains("Windows") {
        "Windows PC"
    } else if agent.contains("Linux") || agent.contains("CrOS") {
        "Computer"
    } else {
        "Browser"
    }
}

/// Server-sent events: the state whenever the editor publishes it, and the number of each new preview frame.
fn events(share: &Arc<Share>, mut stream: TcpStream, req: &Request) {
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();
    let headers = [("Content-Type", "text/event-stream".to_string())];
    if write_head(&mut stream, 200, &headers, None).is_err() {
        return;
    }
    let id = share.next_page.fetch_add(1, Ordering::SeqCst);
    share.pages.lock().unwrap().insert(id, device(req.header("user-agent").unwrap_or("")));
    share.pages_changed();
    let (mut sent_state, mut sent_frame) = (0, 0);
    let mut quiet_since = Instant::now();
    while !share.stopped.load(Ordering::SeqCst) {
        let mut out = String::new();
        {
            let published = share.published.lock().unwrap();
            if published.0 != sent_state {
                sent_state = published.0;
                out.push_str(&format!("event: state\ndata: {}\n\n", published.1));
            }
        }
        let seq = share.preview.latest().0;
        if seq != sent_frame {
            sent_frame = seq;
            out.push_str(&format!("event: frame\ndata: {seq}\n\n"));
        }
        if out.is_empty() && quiet_since.elapsed() > Duration::from_secs(15) {
            out.push_str(": ping\n\n");
        }
        if !out.is_empty() {
            if stream.write_all(out.as_bytes()).is_err() {
                break;
            }
            quiet_since = Instant::now();
            *share.last_request.lock().unwrap() = Instant::now();
        }
        thread::sleep(Duration::from_millis(250));
    }
    if share.stopped.load(Ordering::SeqCst) {
        stream.write_all(b"event: end\ndata: {}\n\n").ok();
    }
    share.pages.lock().unwrap().remove(&id);
    if !share.stopped.load(Ordering::SeqCst) {
        share.pages_changed();
    }
}

fn frame(share: &Share) -> Response {
    let (seq, latest) = share.preview.latest();
    let mut cached = share.frame.lock().unwrap();
    if cached.0 != seq {
        let jpeg = latest.and_then(|f| {
            let at = |i: usize| u32::from_le_bytes(f[i..i + 4].try_into().unwrap());
            let (width, height) = (at(4), at(8));
            let rendered = RenderedThumbnail {
                width,
                height,
                rgba: f[HEADER_LEN..].to_vec(),
                hidden: Vec::new(),
                bounds: Vec::new(),
                frame_bounds: [[[0.0; 2]; 4]; 2],
                overlay_bounds: Vec::new(),
            };
            nuzky_engine::thumbnail::encode(&rendered, ImageKind::Jpeg)
                .inspect_err(|e| log::warn!("Could not encode the shared preview frame: {e:#}"))
                .ok()
        });
        *cached = (seq, jpeg.map(Arc::new));
    }
    match &cached.1 {
        Some(jpeg) => Response::new(200, "image/jpeg", jpeg.as_slice()),
        None => Response::empty(404),
    }
}

fn thumb(share: &Share, asset_id: &str) -> Response {
    if asset_id.is_empty()
        || asset_id.len() > 64
        || !asset_id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Response::empty(404);
    }
    let state = share.app.state::<AppState>();
    let url = state.asset_preview(
        asset_id,
        &state.thumbs,
        |locks| &locks.thumb,
        |asset| thumbs::thumbnail(&state.cache_dir, asset),
    );
    let png = url
        .ok()
        .flatten()
        .and_then(|u| base64::engine::general_purpose::STANDARD.decode(u.strip_prefix("data:image/png;base64,")?).ok());
    match png {
        Some(png) => Response::new(200, "image/png", png),
        None => Response::empty(404),
    }
}

/// A file name safe on every system, from what the page or the project calls it.
fn clean_name(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c)).take(120).collect();
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// A file from the page, kept in Nuzky's own folder under its own name; then the editor imports it.
/// Why an upload is refused before its body is read. A small body is read first, as a browser sending it would
/// otherwise see the connection reset instead of the answer.
fn refused_unread(stream: &mut TcpStream, req: &Request) -> Option<Response> {
    let name = req.param("name").as_deref().and_then(clean_name);
    let extension = name.as_deref().and_then(|n| Path::new(n).extension()?.to_str()).map(str::to_ascii_lowercase);
    let refused = if !extension.is_some_and(|e| MEDIA_EXTENSIONS.contains(&e.as_str())) {
        Response::new(400, "text/plain; charset=utf-8", "Nuzky takes videos, photos and sound only")
    } else {
        match req.content_length() {
            None => Response::empty(411),
            Some(length) if length > MAX_UPLOAD => Response::empty(413),
            Some(_) => return None,
        }
    };
    let unread = req.content_length().unwrap_or(0).saturating_sub(req.body_start.len() as u64);
    if unread <= MAX_JSON as u64 {
        std::io::copy(&mut stream.take(unread), &mut std::io::sink()).ok();
    }
    Some(refused)
}

fn upload(share: &Share, stream: &mut TcpStream, req: Request) -> Response {
    let (Some(name), Some(length)) = (req.param("name").as_deref().and_then(clean_name), req.content_length()) else {
        return Response::empty(400);
    };
    let dir = store::data_dir().join("shared").join(uuid::Uuid::new_v4().simple().to_string());
    let path = dir.join(&name);
    let part = dir.join(format!(".nuzky-part-{name}"));
    let received = (|| -> std::io::Result<bool> {
        std::fs::create_dir_all(&dir)?;
        let mut file = File::create(&part)?;
        let mut left = length;
        let start = &req.body_start[..req.body_start.len().min(length as usize)];
        file.write_all(start)?;
        left -= start.len() as u64;
        let mut buf = vec![0u8; 256 * 1024];
        while left > 0 {
            if share.stopped.load(Ordering::SeqCst) {
                return Ok(false);
            }
            let want = buf.len().min(left as usize);
            let n = stream.read(&mut buf[..want])?;
            if n == 0 {
                return Ok(false);
            }
            file.write_all(&buf[..n])?;
            left -= n as u64;
        }
        file.sync_all()?;
        std::fs::rename(&part, &path)?;
        Ok(true)
    })();
    match received {
        Ok(true) => {
            share.act(&Action::Add { path: path.to_string_lossy().into_owned() });
            Response::new(200, "application/json", "{}")
        }
        other => {
            if let Err(e) = other {
                log::warn!("A shared upload failed: {e}");
            }
            std::fs::remove_dir_all(&dir).ok();
            Response::empty(400)
        }
    }
}

fn action(share: &Share, stream: &mut TcpStream, req: Request) -> Response {
    let Some(length) = req.content_length().filter(|l| *l as usize <= MAX_JSON) else { return Response::empty(413) };
    let mut body = req.body_start;
    body.truncate(length as usize);
    let mut rest = vec![0u8; length as usize - body.len()];
    if stream.read_exact(&mut rest).is_err() {
        return Response::empty(400);
    }
    body.extend(rest);
    let Ok(action) = serde_json::from_slice::<Action>(&body) else { return Response::empty(400) };
    let valid = match &action {
        Action::Send { text } => !text.trim().is_empty() && text.len() <= MAX_TEXT,
        Action::Choose { id, label } => id.len() <= 200 && !label.is_empty() && label.len() <= MAX_TEXT,
        Action::Seek { us } => *us >= 0,
        _ => true,
    };
    if !valid {
        return Response::empty(400);
    }
    share.act(&action);
    Response::new(200, "application/json", "{}")
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` of a file `size` bytes long, as the first and last byte.
fn byte_range(header: &str, size: u64) -> Option<(u64, u64)> {
    let (from, to) = header.strip_prefix("bytes=")?.split(',').next()?.trim().split_once('-')?;
    let (first, last) = match (from.trim(), to.trim()) {
        ("", n) => (size.checked_sub(n.parse::<u64>().ok()?.min(size))?, size.checked_sub(1)?),
        (a, "") => (a.parse().ok()?, size.checked_sub(1)?),
        (a, b) => (a.parse().ok()?, b.parse::<u64>().ok()?.min(size.checked_sub(1)?)),
    };
    (first <= last && last < size).then_some((first, last))
}

/// The export the page started, played in place or downloaded, in byte ranges as browsers play video.
fn video(share: &Share, mut stream: TcpStream, req: &Request) {
    let Some(path) = share.export.lock().unwrap().clone() else { return send(&mut stream, Response::empty(404)) };
    let Ok(mut file) = File::open(&path) else { return send(&mut stream, Response::empty(404)) };
    let Ok(size) = file.metadata().map(|m| m.len()) else { return send(&mut stream, Response::empty(404)) };
    let mut headers = vec![("Content-Type", "video/mp4".to_string()), ("Accept-Ranges", "bytes".to_string())];
    if req.param("download").is_some() {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Video.mp4".into());
        let ascii: String =
            name.chars().map(|c| if c.is_ascii_graphic() && c != '"' || c == ' ' { c } else { '_' }).collect();
        let encoded = percent_encoding::utf8_percent_encode(&name, percent_encoding::NON_ALPHANUMERIC);
        headers.push(("Content-Disposition", format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")));
    }
    let (status, first, last) = match req.header("range") {
        Some(range) => match byte_range(range, size) {
            Some((first, last)) => {
                headers.push(("Content-Range", format!("bytes {first}-{last}/{size}")));
                (206, first, last)
            }
            None => {
                return send(&mut stream, Response::empty(416).with("Content-Range", format!("bytes */{size}")));
            }
        },
        None if size == 0 => return send(&mut stream, Response::new(200, "video/mp4", "")),
        None => (200, 0, size - 1),
    };
    let length = last - first + 1;
    if write_head(&mut stream, status, &headers, Some(length)).is_err() {
        return;
    }
    if file.seek(SeekFrom::Start(first)).is_ok() {
        std::io::copy(&mut file.take(length), &mut stream).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_ranges_stay_inside_the_file() {
        assert_eq!(byte_range("bytes=0-1", 10), Some((0, 1)));
        assert_eq!(byte_range("bytes=4-", 10), Some((4, 9)));
        assert_eq!(byte_range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(byte_range("bytes=-30", 10), Some((0, 9)));
        assert_eq!(byte_range("bytes=5-50", 10), Some((5, 9)));
        assert_eq!(byte_range("bytes=10-", 10), None);
        assert_eq!(byte_range("bytes=6-2", 10), None);
        assert_eq!(byte_range("bytes=0-", 0), None);
        assert_eq!(byte_range("items=0-1", 10), None);
    }

    #[test]
    fn names_from_the_page_cannot_leave_the_folder() {
        assert_eq!(clean_name("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(clean_name("C:\\Users\\a\\IMG_1.MOV").as_deref(), Some("IMG_1.MOV"));
        assert_eq!(clean_name("..").as_deref(), None);
        assert_eq!(clean_name("a<b>:c?.mov").as_deref(), Some("abc.mov"));
        assert_eq!(clean_name(" \u{7}IMG.mov ").as_deref(), Some("IMG.mov"));
    }
}
