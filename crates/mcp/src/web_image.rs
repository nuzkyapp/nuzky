//! A picture from the web for an agent, such as the logo of a brand the user talks about, found with the agent's
//! own web search. Only over https, never through a proxy, and only from addresses on the public internet, checked
//! again on every redirect. What arrives is judged by its bytes, never by its name or headers. Its size is read from
//! its header, then FFmpeg decodes it under the same limit and refuses a larger frame before making room for it, so
//! a header that lies cannot fill memory. Pictures stay in Nuzky's own folder, named by the hash of what was
//! downloaded.
use std::io::Read;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use anyhow::{Result, anyhow, bail, ensure};
use nuzky_engine::edit::new_id;
use nuzky_engine::model::{Asset, AssetKind};
use nuzky_session::jobs::check_cancel;
use sha2::{Digest, Sha256};
use ureq::ResponseExt;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use url::Url;

const MAX_URL_CHARS: usize = 2048;
const MAX_BYTES: usize = 20 << 20;
pub(crate) const MAX_SIDE: usize = 8192;
pub(crate) const MAX_PIXELS: usize = 25_000_000;
/// FFmpeg counts a frame's pixels with its width rounded up to a multiple of as much as 64, so a picture within the
/// limits can count a little more.
pub(crate) const DECODE_PIXELS: usize = MAX_PIXELS + 64 * MAX_SIDE;
/// The long side an SVG is drawn at.
const SVG_PX: u32 = 2048;
const MAX_PICTURES: usize = 12;
const MAX_PICTURE_CHARS: usize = 512;
/// CDNs send AVIF, which Nuzky cannot read, to a client that does not say what it takes.
const ACCEPT: &str = "image/png,image/jpeg,image/webp,image/svg+xml,text/html;q=0.8";
/// Tests serve pages and pictures from `NUZKY_WEB_TEST_URL` on this host. Plain http and this computer are allowed
/// for exactly that address and port, so tests can show that every other one is refused.
const TEST_SERVER: &str = "http://127.0.0.1:";
const FIND_THE_FILE: &str = "Find the address of the picture file itself; for a Wikimedia \"File:\" page that is …/wiki/Special:FilePath/<file name>.";
const UNREADABLE: &str = "NOT_A_PICTURE: Nuzky cannot read the picture at this address. Find a PNG of it.";
const SVG_NS: &str = "http://www.w3.org/2000/svg";

pub enum Found {
    /// A picture in Nuzky's folder as a new asset, and the address it came from after redirects.
    Picture(Box<Asset>, String),
    /// A web page, with the addresses of the pictures it shows.
    Page(Vec<String>),
}

/// Downloads `url` on its own thread, so `cancelled` stops this within 100 ms even while a server keeps silent.
pub fn fetch(url: &str, cancelled: impl Fn() -> bool) -> Result<Found> {
    fetch_into(url, &crate::sounds::data_dir().join("web-images"), cancelled)
}

fn fetch_into(url: &str, dir: &Path, cancelled: impl Fn() -> bool) -> Result<Found> {
    let url = address(url)?;
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let (dir, worker_stop) = (dir.to_owned(), stop.clone());
    std::thread::Builder::new().name("web-image-download".into()).spawn(move || {
        tx.send(fetch_here(&url, &dir, &worker_stop)).ok();
    })?;
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => return result,
            Err(RecvTimeoutError::Timeout) if cancelled() => {
                stop.store(true, Ordering::Relaxed);
                bail!("CANCELLED: the download stopped");
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => bail!("SOURCE_FAILED: the download stopped unexpectedly"),
        }
    }
}

fn fetch_here(url: &Url, dir: &Path, stop: &AtomicBool) -> Result<Found> {
    let (bytes, from, page) = download(url, stop)?;
    let (path, picture) = match sniff(&bytes, page)? {
        Sniffed::Page => {
            let pictures = pictures(&String::from_utf8_lossy(&bytes), &from);
            ensure!(
                !pictures.is_empty(),
                "NO_PICTURE: the page at this address shows no picture Nuzky can find. Search for the address of the picture itself."
            );
            return Ok(Found::Page(pictures));
        }
        Sniffed::Raster(ext) => (dir.join(format!("{}.{ext}", hash(&bytes))), bytes),
        Sniffed::Svg => {
            let path = dir.join(format!("{}.png", hash(&bytes)));
            // One drawn before passed the checks.
            let png = if path.is_file() { Vec::new() } else { rasterize(&bytes)? };
            (path, png)
        }
    };
    check_cancel(stop)?;
    let fresh = !path.is_file();
    if fresh {
        crate::sounds::publish(&path, |file| Ok(std::io::Write::write_all(file, &picture)?))?;
    }
    // Measured before `probe`, which decodes without a limit.
    let checked = nuzky_engine::media::picture_size(&path, DECODE_PIXELS, || stop.load(Ordering::Relaxed))
        .map_err(|_| anyhow!(UNREADABLE))
        .and_then(|(width, height)| fits(width as usize, height as usize))
        .and_then(|()| match nuzky_engine::media::probe(&path, new_id()) {
            Ok(asset) if asset.kind == AssetKind::Image => Ok(asset),
            _ => Err(anyhow!(UNREADABLE)),
        });
    let mut asset = match checked {
        Ok(asset) => asset,
        Err(error) => {
            if fresh {
                std::fs::remove_file(&path).ok();
            }
            return Err(error);
        }
    };
    asset.name = name(url);
    Ok(Found::Picture(Box::new(asset), from.to_string()))
}

fn address(url: &str) -> Result<Url> {
    let invalid = || {
        anyhow!(
            "INVALID_URL: give the full https:// address of a picture or a page, up to {MAX_URL_CHARS} characters, without a user name or password."
        )
    };
    ensure!(url.chars().count() <= MAX_URL_CHARS, invalid());
    let url = Url::parse(url).map_err(|_| invalid())?;
    let test = url.scheme() == "http" && test_server(url.host_str().unwrap_or_default(), url.port());
    ensure!(
        (url.scheme() == "https" || test)
            && url.host_str().is_some_and(|host| !host.is_empty())
            && url.username().is_empty()
            && url.password().is_none(),
        invalid()
    );
    Ok(url)
}

fn test_port() -> Option<u16> {
    let url = std::env::var("NUZKY_WEB_TEST_URL").ok().filter(|url| url.starts_with(TEST_SERVER))?;
    Url::parse(&url).ok()?.port()
}

fn test_server(host: &str, port: Option<u16>) -> bool {
    host == "127.0.0.1" && port.is_some() && port == test_port()
}

/// An address refused before connecting, and the host that resolved to it.
#[derive(Debug)]
struct Blocked(String);

impl std::fmt::Display for Blocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not on the public internet", self.0)
    }
}

impl std::error::Error for Blocked {}

/// Looks a host up as the system does and refuses it unless every address is on the public internet. ureq asks
/// it for every redirect and connects only to the addresses it returns.
#[derive(Debug)]
struct PublicOnly;

impl Resolver for PublicOnly {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let host = uri.host().unwrap_or_default();
        let addresses = DefaultResolver::default().resolve(uri, config, timeout)?;
        if test_server(host, uri.port_u16()) {
            return Ok(addresses);
        }
        if !addresses.iter().all(|address| public(address.ip())) {
            return Err(ureq::Error::Other(Box::new(Blocked(host.trim_matches(['[', ']']).to_owned()))));
        }
        // Tests turn https_only off for their server; nothing else gets plain http.
        if uri.scheme_str() != Some("https") {
            return Err(ureq::Error::RequireHttpsOnly(uri.to_string()));
        }
        Ok(addresses)
    }
}

fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_multicast()
                || a == 0
                || a >= 240
                || (a == 100 && b & 0xc0 == 64)
                || (a, b, c) == (192, 0, 0)
                || (a == 198 && b & 0xfe == 18))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            let embedded = |high: u16, low: u16| public(IpAddr::V4(((u32::from(high) << 16) | u32::from(low)).into()));
            if let Some(v4) = ip.to_ipv4_mapped() {
                public(IpAddr::V4(v4))
            } else if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                embedded(s[6], s[7])
            } else if s[0] == 0x2002 {
                embedded(s[1], s[2])
            } else {
                s[0] & 0xe000 == 0x2000 && !(s[0] == 0x2001 && (s[1] == 0xdb8 || s[1] == 0))
            }
        }
    }
}

fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .https_only(test_port().is_none())
        // A proxy looks the host up itself, past PublicOnly.
        .proxy(None)
        .max_redirects(5)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent(crate::sounds::AGENT)
        .build();
    ureq::Agent::with_parts(config, DefaultConnector::default(), PublicOnly)
}

/// The body, the address it came from after redirects, and whether the server calls it a web page.
fn download(url: &Url, stop: &AtomicBool) -> Result<(Vec<u8>, Url, bool)> {
    let host = url.host_str().unwrap_or_default();
    let too_large = || anyhow!("TOO_LARGE: the file at this address is bigger than 20 MB. Find a smaller picture.");
    let offline = || anyhow!("OFFLINE: Can't reach {host}. Check the internet connection.");
    let response = agent().get(url.as_str()).header("Accept", ACCEPT).call().map_err(|error| match error {
        ureq::Error::Other(other) if other.is::<Blocked>() => anyhow!(
            "BLOCKED_ADDRESS: {} is this computer or a private network, which Nuzky never downloads from.",
            other.downcast_ref::<Blocked>().map_or(host, |blocked| &blocked.0)
        ),
        ureq::Error::StatusCode(404 | 410) => {
            anyhow!("NOT_FOUND: {host} has nothing at this address. Search again for the picture's address.")
        }
        ureq::Error::StatusCode(code) => {
            anyhow!("SOURCE_FAILED: {host} answered with an error ({code}). Try another address.")
        }
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed | ureq::Error::RequireHttpsOnly(_) => {
            anyhow!("SOURCE_FAILED: {host} redirects too often or away from https. Try another address.")
        }
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) | ureq::Error::Timeout(_) => {
            offline()
        }
        error => anyhow!("SOURCE_FAILED: {host}: {error}. Try another address."),
    })?;
    let length = response.headers().get("content-length").and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    ensure!(length.is_none_or(|length| length <= MAX_BYTES as u64), too_large());
    let page = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|kind| kind.trim_start().to_ascii_lowercase().starts_with("text/html"));
    let from = Url::parse(&response.get_uri().to_string()).unwrap_or_else(|_| url.clone());
    let mut reader = response.into_body().into_reader();
    let (mut bytes, mut buffer) = (Vec::new(), vec![0; 1 << 16]);
    loop {
        check_cancel(stop)?;
        let read = reader.read(&mut buffer).map_err(|_| offline())?;
        if read == 0 {
            return Ok((bytes, from, page));
        }
        ensure!(bytes.len() + read <= MAX_BYTES, too_large());
        bytes.extend_from_slice(&buffer[..read]);
    }
}

enum Sniffed {
    Raster(&'static str),
    Svg,
    Page,
}

fn sniff(bytes: &[u8], page: bool) -> Result<Sniffed> {
    use imagesize::ImageType;
    let raster = match imagesize::image_type(bytes) {
        Ok(ImageType::Png) => Some("png"),
        Ok(ImageType::Jpeg) => Some("jpg"),
        Ok(ImageType::Webp) => Some("webp"),
        _ => None,
    };
    if let Some(ext) = raster {
        let size = imagesize::blob_size(bytes).map_err(|_| anyhow!(UNREADABLE))?;
        fits(size.width, size.height)?;
        return Ok(Sniffed::Raster(ext));
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    let start = head.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if start.starts_with("<!doctype html") || start.starts_with("<html") {
        return Ok(Sniffed::Page);
    }
    if head.contains("<svg") {
        return Ok(Sniffed::Svg);
    }
    ensure!(page, "NOT_A_PICTURE: this address gives no PNG, JPEG, WebP or SVG picture. {FIND_THE_FILE}");
    Ok(Sniffed::Page)
}

fn fits(width: usize, height: usize) -> Result<()> {
    ensure!(
        width <= MAX_SIDE && height <= MAX_SIDE && width * height <= MAX_PIXELS,
        "TOO_LARGE: this picture is {width}×{height} px, more than Nuzky takes ({MAX_SIDE} px a side, 25 megapixels). Find a smaller one."
    );
    Ok(())
}

fn rasterize(svg: &[u8]) -> Result<Vec<u8>> {
    let find_png = "Find a PNG of it.";
    let cannot_draw = || anyhow!("NOT_A_PICTURE: Nuzky cannot draw this SVG. {find_png}");
    // As usvg reads it, which also takes an element without a namespace for SVG.
    let options = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let document = std::str::from_utf8(svg)
        .ok()
        .and_then(|text| roxmltree::Document::parse_with_options(text, options).ok())
        .ok_or_else(cannot_draw)?;
    // This resvg build draws no text, so its letters would quietly be missing.
    ensure!(
        !document.descendants().any(|node| node.is_element()
            && node.tag_name().name() == "text"
            && matches!(node.tag_name().namespace(), None | Some(SVG_NS))),
        "NOT_A_PICTURE: this SVG has text, which Nuzky cannot draw. {find_png}"
    );
    let png = crate::stickers::render(svg, SVG_PX).map_err(|_| cannot_draw())?;
    let mut reader = png::Decoder::new(std::io::Cursor::new(&png)).read_info()?;
    let mut rgba = vec![0; reader.output_buffer_size().unwrap_or_default()];
    reader.next_frame(&mut rgba)?;
    // Embedded pictures are not drawn.
    ensure!(
        rgba.as_chunks::<4>().0.iter().any(|[_, _, _, alpha]| *alpha > 0),
        "NOT_A_PICTURE: this SVG draws nothing Nuzky can show, as its picture is an embedded image. {find_png}"
    );
    Ok(png)
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// The address's file name, such as `Spotify_logo.svg`, else its host.
fn name(url: &Url) -> String {
    let last = url.path_segments().and_then(|mut segments| segments.next_back()).unwrap_or_default();
    let decoded = percent_encoding::percent_decode_str(last).decode_utf8_lossy().trim().to_owned();
    if decoded.is_empty() || decoded.chars().count() > 80 || decoded.contains(char::is_control) {
        url.host_str().unwrap_or_default().to_owned()
    } else {
        decoded
    }
}

/// The https pictures a page shows, and those on the test server: `<img>` tags that mention a logo, icons, the page's
/// share images, then other `<img>`, each in document order.
fn pictures(html: &str, base: &Url) -> Vec<String> {
    let mut groups: [Vec<String>; 4] = Default::default();
    for (tag, attributes) in tags(html) {
        let value = |name: &str| attributes.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone());
        let lower = |name: &str| value(name).unwrap_or_default().to_ascii_lowercase();
        let (group, address) = match tag.split_whitespace().next().map(str::to_ascii_lowercase).as_deref() {
            Some("img") => (if tag.to_ascii_lowercase().contains("logo") { 0 } else { 3 }, value("src")),
            Some("link") if lower("rel").contains("icon") => (1, value("href")),
            Some("meta")
                if ["og:image", "og:image:url", "og:image:secure_url", "twitter:image"]
                    .iter()
                    .any(|&key| lower("property") == key || lower("name") == key) =>
            {
                (2, value("content"))
            }
            _ => continue,
        };
        // No group needs more than the answer holds, so a page of countless pictures takes no longer than reading it.
        let kept = &mut groups[group];
        let Some(url) = address.filter(|_| kept.len() < MAX_PICTURES).and_then(|a| base.join(a.trim()).ok()) else {
            continue;
        };
        let test = url.scheme() == "http" && test_server(url.host_str().unwrap_or_default(), url.port());
        let url = url.to_string();
        if (url.starts_with("https://") || test) && url.len() <= MAX_PICTURE_CHARS && !kept.contains(&url) {
            kept.push(url);
        }
    }
    let mut found: Vec<String> = Vec::new();
    for url in groups.into_iter().flatten() {
        if !found.contains(&url) {
            found.push(url);
        }
    }
    found.truncate(MAX_PICTURES);
    found
}

/// Each tag's text and its attributes, names in lowercase, values unquoted with `&amp;` read as `&`.
fn tags(html: &str) -> Vec<(&str, Vec<(String, String)>)> {
    let b = html.as_bytes();
    let skip = |mut i: usize, stop: &dyn Fn(u8) -> bool| {
        while i < b.len() && !stop(b[i]) {
            i += 1;
        }
        i
    };
    let mut tags = Vec::new();
    let mut at = 0;
    while let Some(open) = html[at..].find('<') {
        let start = at + open + 1;
        let mut i = skip(start, &|c| c.is_ascii_whitespace() || c == b'>' || c == b'/');
        let mut attributes = Vec::new();
        loop {
            i = skip(i, &|c| !c.is_ascii_whitespace() && c != b'/');
            if i >= b.len() || b[i] == b'>' {
                break;
            }
            let key = i;
            i = skip(i, &|c| c.is_ascii_whitespace() || matches!(c, b'=' | b'>' | b'/'));
            let name = html[key..i].to_ascii_lowercase();
            i = skip(i, &|c| !c.is_ascii_whitespace());
            let mut value = "";
            if b.get(i) == Some(&b'=') {
                i = skip(i + 1, &|c| !c.is_ascii_whitespace());
                if let Some(&quote) = b.get(i).filter(|&&c| c == b'"' || c == b'\'') {
                    let end = skip(i + 1, &|c| c == quote);
                    value = &html[i + 1..end];
                    i = (end + 1).min(b.len());
                } else {
                    let end = skip(i, &|c| c.is_ascii_whitespace() || c == b'>');
                    value = &html[i..end];
                    i = end;
                }
            }
            attributes.push((name, value.replace("&amp;", "&")));
        }
        tags.push((&html[start..i], attributes));
        at = (i + 1).min(b.len());
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_internet_addresses_pass() {
        for allowed in ["8.8.8.8", "1.1.1.1", "2606:4700::1111", "64:ff9b::808:808", "2002:808:808::"] {
            assert!(public(allowed.parse().unwrap()), "{allowed}");
        }
        for blocked in [
            "0.0.0.0",
            "0.1.2.3",
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "255.255.255.255",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "100.64.0.1",
            "100.127.255.255",
            "192.0.0.8",
            "198.18.0.1",
            "198.19.255.255",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            "64:ff9b::7f00:1",
            "2002:7f00:1::",
            "2002:a9fe:a9fe::",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "ff02::1",
            "2001:db8::1",
            "2001:0:4136:e378::1",
        ] {
            assert!(!public(blocked.parse().unwrap()), "{blocked}");
        }
        // Just outside the shared and benchmarking ranges.
        assert!(public("100.128.0.1".parse().unwrap()) && public("198.20.0.1".parse().unwrap()));
    }

    #[test]
    fn a_page_offers_its_logo_first_then_icons_then_share_images() {
        let base = Url::parse("https://brand.example/about/team.html").unwrap();
        let html = r#"<!DOCTYPE html><html><head>
            <meta property="og:image" content="https://cdn.brand.example/share.jpg">
            <meta name='twitter:image' content='/twitter.png'>
            <link rel="stylesheet" href="/site.css">
            <link REL="shortcut icon" href=/favicon.ico>
            <link rel="apple-touch-icon" href="touch.png?v=1&amp;s=2">
            </head><body>
            <img src="hero.jpg" alt="a > b">
            <img class=site-logo src="/img/brand.svg">
            <img src="data:image/png;base64,AAAA">
            <img src="http://brand.example/plain-logo.png">
            <img src='https://cdn.brand.example/share.jpg'>
            <IMG SRC="/img/Logo-dark.png"/>
            </body></html>"#;
        assert_eq!(
            pictures(html, &base),
            [
                "https://brand.example/img/brand.svg",
                "https://brand.example/img/Logo-dark.png",
                "https://brand.example/favicon.ico",
                "https://brand.example/about/touch.png?v=1&s=2",
                "https://cdn.brand.example/share.jpg",
                "https://brand.example/twitter.png",
                "https://brand.example/about/hero.jpg",
            ]
        );
        let many: String = (0..20).map(|i| format!("<img src=/{i}.png>")).collect();
        assert_eq!(pictures(&many, &base).len(), MAX_PICTURES);
        let long = format!("<img src=/{}.png>", "a".repeat(MAX_PICTURE_CHARS));
        assert!(pictures(&long, &base).is_empty());
        assert!(pictures("<p>No pictures <img", &base).is_empty());
    }

    #[test]
    fn a_page_of_countless_pictures_answers_at_once() {
        let base = Url::parse("https://brand.example/").unwrap();
        let mut html: String = (0..100_000).map(|i| format!("<img src=/{i}.png>")).collect();
        html.push_str("<link rel=icon href=/favicon.ico><img class=logo src=/logo.svg>");
        let started = std::time::Instant::now();
        let found = pictures(&html, &base);
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        let expected =
            ["logo.svg".to_owned(), "favicon.ico".to_owned()].into_iter().chain((0..10).map(|i| format!("{i}.png")));
        assert_eq!(found, expected.map(|file| format!("https://brand.example/{file}")).collect::<Vec<_>>());
    }

    #[test]
    fn a_picture_is_named_after_its_file() {
        let name = |url: &str| name(&Url::parse(url).unwrap());
        assert_eq!(name("https://upload.wikimedia.org/wikipedia/commons/8/84/Spotify_logo.svg"), "Spotify_logo.svg");
        assert_eq!(name("https://cdn.example/Caf%C3%A9%20logo.png?x=1"), "Café logo.png");
        assert_eq!(name("https://cdn.example/100%-sure.png"), "100%-sure.png");
        assert_eq!(name("https://cdn.example/brand/"), "cdn.example");
        assert_eq!(name(&format!("https://cdn.example/{}.png", "a".repeat(80))), "cdn.example");
    }

    /// A server that accepts and then says nothing never holds up a stop, and nothing is left behind.
    #[test]
    fn a_silent_server_never_holds_up_a_stop() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let _held: Vec<_> = listener.incoming().take(1).collect();
            std::thread::sleep(Duration::from_secs(60));
        });
        // Only this module reads the test server's address.
        unsafe { std::env::set_var("NUZKY_WEB_TEST_URL", format!("http://127.0.0.1:{port}")) };
        let dir = std::env::temp_dir().join(format!("nuzky-silent-web-{}", new_id()));
        let started = std::time::Instant::now();
        let stopped = || started.elapsed() > Duration::from_millis(300);
        let error = fetch_into(&format!("http://127.0.0.1:{port}/logo.png"), &dir, stopped).err().unwrap();
        assert!(error.to_string().starts_with("CANCELLED"), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
        std::thread::sleep(Duration::from_millis(200));
        assert!(!dir.exists());
    }
}
