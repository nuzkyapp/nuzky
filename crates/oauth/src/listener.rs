//! The address the browser comes back to: `http://127.0.0.1:<free port>/auth/callback`, listening
//! before the browser opens and only for as long as one sign-in waits.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

pub const CALLBACK_PATH: &str = "/auth/callback";
/// A request line and headers; anything longer is not the browser coming back.
const MAX_REQUEST: usize = 16 * 1024;

pub struct Listener {
    socket: TcpListener,
}

/// The browser's request for the callback, waiting for its answer.
pub struct Request {
    stream: TcpStream,
    pub query: String,
}

impl Listener {
    /// On 127.0.0.1, never `localhost`, which can resolve elsewhere; the system picks the port.
    pub fn bind() -> Result<Self> {
        let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket })
    }

    pub fn redirect_uri(&self) -> Result<String> {
        Ok(format!("http://127.0.0.1:{}{CALLBACK_PATH}", self.socket.local_addr()?.port()))
    }

    /// The next request for the callback; other requests, such as a favicon, get 404.
    pub fn next(&self, deadline: Instant, cancel: &AtomicBool) -> Result<Request> {
        loop {
            if cancel.load(Ordering::Relaxed) {
                bail!("CANCELLED: Signing in was cancelled.");
            }
            if Instant::now() >= deadline {
                bail!("SIGN_IN_TIMEOUT: Nuzky waited 5 minutes for the browser.");
            }
            match self.socket.accept() {
                Ok((stream, _)) => match read_target(&stream) {
                    Some(target) => match target.split_once('?').unwrap_or((&target, "")) {
                        (CALLBACK_PATH, query) => return Ok(Request { query: query.to_owned(), stream }),
                        _ => reply(stream, 404, "Not found."),
                    },
                    None => continue,
                },
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Request {
    pub fn reply(self, status: u16, message: &str) {
        reply(self.stream, status, message);
    }
}

/// The target of a GET, such as `/auth/callback?code=…`. A connection that has not sent its whole
/// request within two seconds, such as a browser's spare one or a slow local process, is dropped.
fn read_target(mut stream: &TcpStream) -> Option<String> {
    stream.set_nonblocking(false).ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut request = Vec::new();
    let mut chunk = [0u8; 2048];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < MAX_REQUEST {
        stream.set_read_timeout(Some(deadline.checked_duration_since(Instant::now())?)).ok()?;
        let read = stream.read(&mut chunk).ok().filter(|&read| read > 0)?;
        request.extend_from_slice(&chunk[..read]);
    }
    let line = std::str::from_utf8(&request).ok()?.lines().next()?;
    let mut parts = line.split(' ');
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) => Some(target.to_owned()),
        _ => None,
    }
}

fn reply(mut stream: TcpStream, status: u16, message: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\">\
         <title>Nuzky</title><body style=\"margin:0;min-height:100vh;display:grid;place-items:center;background:#0c0c0e;\
         color:#f2f2f5;font:15px/1.5 system-ui,sans-serif\"><p style=\"max-width:28em;padding:0 24px\">{message}</p></body></html>"
    );
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body.as_bytes()))
        .and_then(|()| stream.flush());
}
