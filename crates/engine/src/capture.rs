//! When, where and on what a file was recorded, as the file itself says: a phone video's QuickTime
//! tags, or a photo's EXIF. Nothing is guessed from the file's dates on disk, which say when it was
//! copied.
use std::io::Read;
use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::media::open_input;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Capture {
    /// RFC 3339: with the camera's own offset when the file has it, `Z` for UTC, or no offset when
    /// only the local time is known.
    pub recorded_at: Option<String>,
    /// How to read `recorded_at`: `local` (the camera's clock and offset), `local_unknown_zone`, or `utc`.
    pub time_basis: Option<&'static str>,
    pub location: Option<Location>,
    /// Make and model, as one line.
    pub device: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altitude_m: Option<f64>,
    /// How far off the phone said the position may be.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accuracy_m: Option<f64>,
}

/// Photos past this are not searched for EXIF; a phone photo is a few megabytes.
const MAX_EXIF_SEARCH: u64 = 64 << 20;
/// Dates before this are a camera's unset clock or an empty field, not a recording.
const FIRST_YEAR: i32 = 1980;

pub fn capture(path: &Path) -> Result<Capture> {
    let input = open_input(path)?;
    // The container's tags first, then each stream's, as ffprobe lists them.
    let mut all: Vec<Vec<(String, String)>> = vec![tag_list(input.metadata())];
    all.extend(input.streams().map(|s| tag_list(s.metadata())));
    let image = crate::media::is_image_format(&input);
    drop(input);
    let tags = |key: &str| -> Option<String> {
        all.iter().find_map(|list| {
            list.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim().to_owned()).filter(|v| !v.is_empty())
        })
    };
    let mut capture = Capture::default();
    if let Some((at, basis)) = tags("com.apple.quicktime.creationdate").and_then(|v| local_time(&v)) {
        capture.recorded_at = Some(at);
        capture.time_basis = Some(basis);
    }
    capture.location = ["com.apple.quicktime.location.ISO6709", "location", "location-eng"]
        .iter()
        .find_map(|key| tags(key).and_then(|v| iso6709(&v)))
        .map(|mut location| {
            location.accuracy_m = tags("com.apple.quicktime.location.accuracy.horizontal")
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0);
            location
        });
    capture.device = device(
        &[
            ("com.apple.quicktime.make", "com.apple.quicktime.model"),
            ("com.android.manufacturer", "com.android.model"),
            ("make", "model"),
        ]
        .map(|(make, model)| (tags(make), tags(model))),
    );
    if image && let Some(exif) = find_exif(path) {
        let photo = exif::read(&exif);
        if capture.recorded_at.is_none() {
            capture.recorded_at = photo.recorded_at;
            capture.time_basis = photo.time_basis;
        }
        capture.location = capture.location.or(photo.location);
        capture.device = capture.device.or(photo.device);
    }
    if capture.recorded_at.is_none()
        && let Some(at) = tags("creation_time").and_then(|v| utc_time(&v))
    {
        capture.recorded_at = Some(at);
        capture.time_basis = Some("utc");
    }
    Ok(capture)
}

fn tag_list(metadata: ffmpeg_next::DictionaryRef<'_>) -> Vec<(String, String)> {
    metadata.iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect()
}

fn device(pairs: &[(Option<String>, Option<String>)]) -> Option<String> {
    let clean = |v: &Option<String>| v.as_deref().map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "));
    pairs.iter().find_map(|(make, model)| match (clean(make), clean(model)) {
        (Some(make), Some(model)) if model.starts_with(&make) => Some(model),
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (None, Some(model)) => Some(model),
        (Some(make), None) => Some(make),
        (None, None) => None,
    })
}

/// "2024-05-12T14:03:22+0200" or "+02:00", as QuickTime keeps the phone's local time.
fn local_time(text: &str) -> Option<(String, &'static str)> {
    let (date, time) = text.split_once('T')?;
    let date = valid_date(date)?;
    let clock: String = time.chars().take_while(|c| c.is_ascii_digit() || *c == ':').collect();
    let clock = valid_clock(&clock)?;
    let rest = time[clock.len()..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    match offset(rest) {
        Some(offset) if offset == "Z" => Some((format!("{date}T{clock}Z"), "utc")),
        Some(offset) => Some((format!("{date}T{clock}{offset}"), "local")),
        None if rest.is_empty() => Some((format!("{date}T{clock}"), "local_unknown_zone")),
        None => None,
    }
}

/// "2024-05-12T12:03:22.000000Z", MP4's creation time, which the format defines as UTC.
fn utc_time(text: &str) -> Option<String> {
    let (date, time) = text.split_once(['T', ' '])?;
    let date = valid_date(date)?;
    let clock = valid_clock(time.get(..8)?)?;
    let rest = time[8..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    matches!(rest, "Z" | "" | "+00:00" | "+0000").then(|| format!("{date}T{clock}Z"))
}

fn valid_date(date: &str) -> Option<String> {
    let parts: Vec<&str> = date.split(['-', ':']).collect();
    let [y, m, d] = parts.as_slice() else { return None };
    let (y, m, d) = (y.parse::<i32>().ok()?, m.parse::<u32>().ok()?, d.parse::<u32>().ok()?);
    ((FIRST_YEAR..=9999).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d))
        .then(|| format!("{y:04}-{m:02}-{d:02}"))
}

fn valid_clock(clock: &str) -> Option<String> {
    let parts: Vec<&str> = clock.split(':').collect();
    let [h, m, s] = parts.as_slice() else { return None };
    let (h, m, s) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?, s.parse::<u32>().ok()?);
    (h < 24 && m < 60 && s < 61 && clock.len() == 8).then(|| clock.to_owned())
}

/// "+0200", "+02:00", "-05:30" or "Z" as "+02:00".
fn offset(text: &str) -> Option<String> {
    if text == "Z" {
        return Some("Z".into());
    }
    let sign = text.chars().next().filter(|c| *c == '+' || *c == '-')?;
    let digits: String = text[1..].chars().filter(|c| *c != ':').collect();
    if digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m) = (digits[..2].parse::<u32>().ok()?, digits[2..].parse::<u32>().ok()?);
    (h <= 14 && m < 60).then(|| format!("{sign}{h:02}:{m:02}"))
}

/// ISO 6709 as phones write it, "+50.0755+014.4378+250.000/": latitude and longitude in degrees,
/// then an optional altitude in metres. Degrees and minutes ("+5004.53") are read too.
fn iso6709(text: &str) -> Option<Location> {
    let text = text.trim().trim_end_matches('/');
    let mut numbers = Vec::new();
    let mut start = 0;
    for (i, c) in text.char_indices().skip(1) {
        if c == '+' || c == '-' {
            numbers.push(&text[start..i]);
            start = i;
        }
    }
    numbers.push(&text[start..]);
    if !(2..=3).contains(&numbers.len()) || numbers.iter().any(|n| !n.starts_with(['+', '-'])) {
        return None;
    }
    let angle = |n: &str, degree_digits: usize| -> Option<f64> {
        let value: f64 = n.parse().ok()?;
        let whole = n[1..].split('.').next()?.len();
        let (sign, abs) = (value.signum(), value.abs());
        let degrees = match whole.checked_sub(degree_digits)? {
            0 => abs,
            2 => (abs / 100.0).trunc() + (abs % 100.0) / 60.0,
            4 => (abs / 10_000.0).trunc() + (abs / 100.0).trunc() % 100.0 / 60.0 + (abs % 100.0) / 3600.0,
            _ => return None,
        };
        Some(sign * degrees)
    };
    let latitude = angle(numbers[0], 2)?;
    let longitude = angle(numbers[1], 3)?;
    let altitude_m = numbers.get(2).and_then(|n| n.parse::<f64>().ok()).filter(|a| a.is_finite());
    real(latitude, longitude).then_some(Location { latitude, longitude, altitude_m, accuracy_m: None })
}

/// A position on Earth, and not the 0, 0 an unset receiver writes.
fn real(latitude: f64, longitude: f64) -> bool {
    latitude.is_finite()
        && longitude.is_finite()
        && latitude.abs() <= 90.0
        && longitude.abs() <= 180.0
        && (latitude, longitude) != (0.0, 0.0)
}

/// The TIFF block of a photo's EXIF, wherever its container keeps it: JPEG's APP1 and HEIC's Exif
/// item start with "Exif\0\0"; PNG's eXIf and WebP's EXIF chunks hold it after their name.
fn find_exif(path: &Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(MAX_EXIF_SEARCH).read_to_end(&mut bytes).ok()?;
    let tiff = |at: usize| bytes.get(at..at + 4).is_some_and(|h| h == b"II*\0" || h == b"MM\0*");
    let at = (0..bytes.len().saturating_sub(10)).find_map(|i| match &bytes[i..i + 4] {
        b"Exif" if bytes[i + 4..i + 6] == [0, 0] && tiff(i + 6) => Some(i + 6),
        b"eXIf" if tiff(i + 4) => Some(i + 4),
        b"EXIF" if tiff(i + 8) => Some(i + 8),
        _ => None,
    })?;
    bytes.drain(..at);
    Some(bytes)
}

mod exif {
    //! The few EXIF fields of when and where: a bounded TIFF reader that never trusts an offset.
    use super::{Capture, Location, device, local_time, real};

    const MAX_ENTRIES: usize = 512;
    const ASCII: u16 = 2;
    const RATIONAL: u16 = 5;

    struct Tiff<'a> {
        bytes: &'a [u8],
        big: bool,
    }

    struct Entry {
        tag: u16,
        kind: u16,
        count: usize,
        /// Where the value is: inside the entry when it fits in four bytes.
        at: usize,
    }

    impl Tiff<'_> {
        fn u16(&self, at: usize) -> Option<u16> {
            let b: [u8; 2] = self.bytes.get(at..at + 2)?.try_into().ok()?;
            Some(if self.big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) })
        }

        fn u32(&self, at: usize) -> Option<u32> {
            let b: [u8; 4] = self.bytes.get(at..at + 4)?.try_into().ok()?;
            Some(if self.big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
        }

        fn entries(&self, ifd: usize) -> Vec<Entry> {
            let Some(count) = self.u16(ifd) else { return Vec::new() };
            (0..usize::from(count).min(MAX_ENTRIES))
                .map_while(|i| {
                    let e = ifd + 2 + i * 12;
                    let (tag, kind, count) = (self.u16(e)?, self.u16(e + 2)?, self.u32(e + 4)? as usize);
                    let size: usize = match kind {
                        1 | 2 | 6 | 7 => 1,
                        3 | 8 => 2,
                        4 | 9 | 11 => 4,
                        5 | 10 | 12 => 8,
                        _ => 0,
                    };
                    let at = if size.saturating_mul(count) <= 4 { e + 8 } else { self.u32(e + 8)? as usize };
                    Some(Entry { tag, kind, count, at })
                })
                .collect()
        }

        fn text(&self, entries: &[Entry], tag: u16) -> Option<String> {
            let e = entries.iter().find(|e| e.tag == tag && e.kind == ASCII)?;
            let raw = self.bytes.get(e.at..e.at.checked_add(e.count)?)?;
            let text: String = raw.iter().take_while(|b| **b != 0).map(|&b| b as char).collect();
            Some(text.trim().to_owned()).filter(|t| !t.is_empty())
        }

        fn rationals(&self, entries: &[Entry], tag: u16) -> Option<Vec<f64>> {
            let e = entries.iter().find(|e| e.tag == tag && e.kind == RATIONAL && e.count <= 3)?;
            (0..e.count)
                .map(|i| {
                    let (n, d) = (self.u32(e.at + i * 8)?, self.u32(e.at + i * 8 + 4)?);
                    (d != 0).then(|| f64::from(n) / f64::from(d))
                })
                .collect()
        }

        fn byte(&self, entries: &[Entry], tag: u16) -> Option<u8> {
            let e = entries.iter().find(|e| e.tag == tag && e.count >= 1)?;
            self.bytes.get(e.at).copied()
        }

        fn pointer(&self, entries: &[Entry], tag: u16) -> Option<usize> {
            let e = entries.iter().find(|e| e.tag == tag)?;
            Some(self.u32(e.at)? as usize)
        }
    }

    pub(super) fn read(bytes: &[u8]) -> Capture {
        let big = bytes.starts_with(b"MM");
        let tiff = Tiff { bytes, big };
        let Some(first) = tiff.u32(4) else { return Capture::default() };
        let ifd0 = tiff.entries(first as usize);
        let mut capture =
            Capture { device: device(&[(tiff.text(&ifd0, 0x010F), tiff.text(&ifd0, 0x0110))]), ..Default::default() };
        if let Some(exif) = tiff.pointer(&ifd0, 0x8769).map(|at| tiff.entries(at)) {
            let original = tiff.text(&exif, 0x9003).or_else(|| tiff.text(&exif, 0x9004));
            let offset = tiff.text(&exif, 0x9011).or_else(|| tiff.text(&exif, 0x9012));
            if let Some((date, time)) = original.as_deref().and_then(|t| t.split_once(' ')) {
                let date = date.replace(':', "-");
                let text = format!("{date}T{time}{}", offset.unwrap_or_default());
                if let Some((at, basis)) = local_time(&text) {
                    capture.recorded_at = Some(at);
                    capture.time_basis = Some(basis);
                }
            }
        }
        if let Some(gps) = tiff.pointer(&ifd0, 0x8825).map(|at| tiff.entries(at)) {
            let angle = |value: u16, reference: u16, negative: &str| -> Option<f64> {
                let parts = tiff.rationals(&gps, value)?;
                let degrees = parts.iter().zip([1.0, 60.0, 3600.0]).map(|(v, d)| v / d).sum::<f64>();
                let sign = if tiff.text(&gps, reference)?.eq_ignore_ascii_case(negative) { -1.0 } else { 1.0 };
                Some(sign * degrees)
            };
            if let (Some(latitude), Some(longitude)) = (angle(2, 1, "S"), angle(4, 3, "W"))
                && real(latitude, longitude)
            {
                let altitude_m = tiff
                    .rationals(&gps, 6)
                    .and_then(|a| a.first().copied())
                    .map(|a| if tiff.byte(&gps, 5) == Some(1) { -a } else { a });
                capture.location = Some(Location { latitude, longitude, altitude_m, accuracy_m: None });
            }
        }
        capture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_times_keep_their_offset_and_utc_stays_utc() {
        assert_eq!(local_time("2024-05-12T14:03:22+0200"), Some(("2024-05-12T14:03:22+02:00".into(), "local")));
        assert_eq!(local_time("2023-12-31T23:59:59-05:30"), Some(("2023-12-31T23:59:59-05:30".into(), "local")));
        assert_eq!(local_time("2024-05-12T14:03:22"), Some(("2024-05-12T14:03:22".into(), "local_unknown_zone")));
        assert_eq!(utc_time("2024-05-12T12:03:22.000000Z").as_deref(), Some("2024-05-12T12:03:22Z"));
        // An unset clock is no recording date.
        assert_eq!(utc_time("1970-01-01T00:00:00.000000Z"), None);
        assert_eq!(utc_time("1904-01-01 00:00:00"), None);
        for bad in ["", "2024-13-01T00:00:00Z", "2024-05-12T25:00:00+0200", "2024-05-12T14:03:22+2500", "nonsense"] {
            assert_eq!(local_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn iso6709_reads_degrees_minutes_and_altitude() {
        let at = iso6709("+50.0755+014.4378+250.000/").unwrap();
        assert_eq!((at.latitude, at.longitude, at.altitude_m), (50.0755, 14.4378, Some(250.0)));
        let at = iso6709("-33.8688+151.2093/").unwrap();
        assert_eq!((at.latitude, at.longitude, at.altitude_m), (-33.8688, 151.2093, None));
        // Degrees and minutes: 50° 04.53' N, 14° 26.268' E.
        let at = iso6709("+5004.53+01426.268/").unwrap();
        assert!((at.latitude - 50.0755).abs() < 1e-9 && (at.longitude - 14.4378).abs() < 1e-9, "{at:?}");
        for bad in ["", "+0+0/", "+91.0+014.0/", "50.0+14.0", "+50.0/", "+50.0+014.0+1+2/"] {
            assert_eq!(iso6709(bad), None, "{bad}");
        }
    }

    #[test]
    fn broken_exif_reads_nothing_and_never_panics() {
        for bytes in [
            &b""[..],
            b"MM\0*",
            b"MM\0*\xff\xff\xff\xff",
            b"II*\0\x08\0\0\0\xff\xff",
            b"II*\0\x08\0\0\0\x01\0\x69\x87\x04\0\x01\0\0\0\xff\xff\xff\x7f",
        ] {
            assert_eq!(exif::read(bytes), Capture::default());
        }
    }
}
