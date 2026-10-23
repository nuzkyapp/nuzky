use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, prelude::BASE64_STANDARD};
use nuzky_engine::{
    edit::{EditCmd, new_id},
    model::{Asset, Transform},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PACK: &[u8] = include_bytes!("../../../assets/stickers/art.pack");
const MAX_FRAME: u64 = 4 * 1024 * 1024;

#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[derive(Deserialize, Serialize)]
pub struct StickerCatalog {
    pub groups: Vec<StickerGroup>,
    #[serde(skip_serializing)]
    #[cfg_attr(feature = "ts", ts(skip))]
    frames: Vec<[usize; 2]>,
    #[cfg(test)]
    #[serde(rename = "packSha256", skip_serializing)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pack_sha256: String,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[derive(Deserialize, Serialize)]
pub struct StickerGroup {
    pub name: String,
    pub items: Vec<StickerItem>,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[derive(Deserialize, Serialize)]
pub struct StickerItem {
    pub key: String,
    pub emoji: Option<String>,
    pub name: String,
    pub tones: Option<Vec<String>>,
    #[serde(default, skip_serializing)]
    #[cfg_attr(feature = "ts", ts(skip))]
    keywords: Vec<String>,
    #[serde(default, skip_serializing)]
    #[cfg_attr(feature = "ts", ts(skip))]
    shortcodes: Vec<String>,
    #[serde(skip_serializing)]
    #[cfg_attr(feature = "ts", ts(skip))]
    art: [usize; 3],
    #[serde(default, skip_serializing)]
    #[cfg_attr(feature = "ts", ts(skip))]
    alternates: Vec<StickerItem>,
}

pub type Pick = &'static StickerItem;

pub fn catalog() -> &'static StickerCatalog {
    static CATALOG: LazyLock<StickerCatalog> = LazyLock::new(|| {
        serde_json::from_str(include_str!("../../../assets/stickers/catalog.json")).expect("valid sticker catalog")
    });
    &CATALOG
}

fn items() -> impl Iterator<Item = Pick> {
    catalog().groups.iter().flat_map(|g| &g.items)
}

fn by_key(key: &str) -> Option<Pick> {
    items().flat_map(|i| std::iter::once(i).chain(&i.alternates)).find(|i| i.key == key)
}

fn key_of(text: &str) -> Option<String> {
    if text.is_ascii() {
        return None;
    }
    Some(
        text.chars()
            .filter(|&c| c != '\u{fe0f}' && c != '\u{fe0e}')
            .map(|c| format!("{:04x}", c as u32))
            .collect::<Vec<_>>()
            .join("_"),
    )
}

pub fn search(query: &str, limit: usize) -> Vec<&'static str> {
    let query = query.trim().trim_matches(':').to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<_> = items()
        .filter_map(|i| {
            let name = i.name.to_lowercase();
            let pasted = key_of(&query);
            let rank = if i.key == query
                || name == query
                || pasted.as_ref().is_some_and(|k| *k == i.key || i.alternates.iter().any(|a| a.key == *k))
            {
                0
            } else if i.shortcodes.iter().any(|s| s.eq_ignore_ascii_case(&query))
                || name.split([' ', ':', ',']).any(|word| word == query)
            {
                1
            } else if name.contains(&query) {
                2
            } else if i.keywords.iter().chain(&i.shortcodes).any(|s| s.to_lowercase().contains(&query)) {
                3
            } else {
                return None;
            };
            Some((rank, name.len(), i.key.as_str()))
        })
        .collect();
    // Among equal matches the shortest name is the plainest emoji: "red heart" before "heart with arrow".
    found.sort_by_key(|&(rank, len, _)| (rank, len));
    found.into_iter().take(limit).map(|(_, _, key)| key).collect()
}

pub fn find(text: &str) -> Result<Pick> {
    let text = text.trim();
    if let Some(pick) = by_key(text).or_else(|| key_of(text).and_then(|key| by_key(&key))) {
        return Ok(pick);
    }
    let name = text.trim_matches(':');
    if let Some(pick) = items()
        .flat_map(|i| std::iter::once(i).chain(&i.alternates))
        .find(|i| i.name.eq_ignore_ascii_case(name) || i.shortcodes.iter().any(|s| s.eq_ignore_ascii_case(name)))
    {
        return Ok(pick);
    }
    if key_of(text).is_some_and(|key| {
        items().flat_map(|i| std::iter::once(i).chain(&i.alternates)).any(|i| {
            key.strip_prefix(&i.key)
                .is_some_and(|rest| rest.starts_with('_') && !rest.starts_with("_200d") && !rest.starts_with("_1f3f"))
        })
    }) {
        bail!("STICKER_UNKNOWN: pass one emoji at a time");
    }
    let query = name.to_lowercase();
    let mut close: Vec<_> = search(text, 5).into_iter().filter_map(by_key).collect();
    if close.is_empty() {
        let mut distances: Vec<_> = items().map(|i| (distance(&query, &i.name.to_lowercase()), i)).collect();
        distances.sort_by_key(|&(score, _)| score);
        close = distances.into_iter().take(5).map(|(_, i)| i).collect();
    }
    let suggestions = close
        .into_iter()
        .map(|i| format!("{} {}", i.name, i.emoji.as_deref().unwrap_or(&i.key)))
        .collect::<Vec<_>>()
        .join(", ");
    bail!("STICKER_UNKNOWN: {text:?} is not an emoji or a sticker Nuzky has. Close: {suggestions}");
}

fn distance(a: &str, b: &str) -> usize {
    let mut row: Vec<_> = (0..=b.chars().count()).collect();
    for (i, ac) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, bc) in b.chars().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ac != bc)).min(row[j] + 1).min(old + 1);
            diagonal = old;
        }
    }
    *row.last().unwrap()
}

fn range(bytes: &[u8], start: usize, len: usize) -> Result<&[u8]> {
    bytes.get(start..start.checked_add(len).context("Sticker range overflow")?).context("Sticker range outside frame")
}

fn frame(index: usize) -> Result<Vec<u8>> {
    let &[start, len] = catalog().frames.get(index).context("Unknown sticker frame")?;
    let compressed = range(PACK, start, len)?;
    let decoder = ruzstd::decoding::StreamingDecoder::new_with_max_window_size(compressed, MAX_FRAME)?;
    let mut raw = Vec::new();
    decoder.take(MAX_FRAME + 1).read_to_end(&mut raw)?;
    ensure!(raw.len() as u64 <= MAX_FRAME, "Sticker frame exceeds 4 MiB");
    Ok(raw)
}

fn svg(pick: Pick, raw: &[u8]) -> Result<&[u8]> {
    range(raw, pick.art[1], pick.art[2])
}

pub(crate) fn render(svg: &[u8], px: u32) -> Result<Vec<u8>> {
    // The default resolver reads the local files an `<image href>` names, and SVGs come from the web too.
    let options = resvg::usvg::Options {
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            ..Default::default()
        },
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_data(svg, &options)?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    let (w, h) = ((size.width() * scale).round() as u32, (size.height() * scale).round() as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).context("Sticker dimensions")?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let rgba: Vec<_> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let p = p.demultiply();
            [p.red(), p.green(), p.blue(), p.alpha()]
        })
        .collect();
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&rgba)?;
    }
    Ok(bytes)
}

fn file_name(pick: Pick, svg: &[u8]) -> String {
    let hash = Sha256::digest(svg).iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!("{}-{}-1024.png", pick.key, &hash[..12])
}

fn picture(pick: Pick, dir: &Path) -> Result<PathBuf> {
    let raw = frame(pick.art[0])?;
    let svg = svg(pick, &raw)?;
    let path = dir.join(file_name(pick, svg));
    if !path.exists() {
        let png = render(svg, 1024)?;
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Could not create the sticker library {}", dir.display()))?;
        let tmp = dir.join(format!(".nuzky-part-{}", new_id()));
        let result = (|| -> Result<()> {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&png)?;
            file.sync_all()?;
            if !path.exists() {
                std::fs::rename(&tmp, &path)?;
            }
            Ok(())
        })();
        std::fs::remove_file(&tmp).ok();
        result.with_context(|| format!("Could not save the sticker's picture in {}", dir.display()))?;
    }
    dunce::canonicalize(path).context("Resolving sticker PNG")
}

#[derive(Clone)]
pub struct Prepared {
    pub asset_id: String,
    pub edits: Vec<EditCmd>,
}

pub fn prepare(
    pick: Pick,
    assets: &[Asset],
    start_us: i64,
    duration_us: Option<i64>,
    transform: Option<Transform>,
) -> Result<Prepared> {
    prepare_in(pick, assets, start_us, duration_us, transform, &crate::sounds::data_dir().join("stickers"))
}

fn prepare_in(
    pick: Pick,
    assets: &[Asset],
    start_us: i64,
    duration_us: Option<i64>,
    transform: Option<Transform>,
    dir: &Path,
) -> Result<Prepared> {
    let path = picture(pick, dir)?;
    let mut edits = Vec::new();
    let asset_id = match assets.iter().find(|a| Path::new(&a.path) == path) {
        Some(asset) => asset.id.clone(),
        None => {
            let mut asset = nuzky_engine::media::probe(&path, new_id())?;
            let mut chars = pick.name.chars();
            asset.name = chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default() + chars.as_str();
            let id = asset.id.clone();
            edits.push(EditCmd::AddAssets { assets: vec![asset] });
            id
        }
    };
    edits.push(EditCmd::AddSticker { asset_id: asset_id.clone(), start_us, duration_us, transform });
    Ok(Prepared { asset_id, edits })
}

pub fn thumbnails(keys: &[String], px: u32) -> Vec<String> {
    let px = px.clamp(16, 256);
    let threads = std::thread::available_parallelism().map_or(1, usize::from).min(4);
    std::thread::scope(|scope| {
        let workers: Vec<_> = keys
            .chunks(keys.len().div_ceil(threads).max(1))
            .map(|keys| {
                scope.spawn(move || {
                    let mut cached = None;
                    keys.iter()
                        .map(|key| {
                            let Some(pick) = by_key(key) else {
                                return String::new();
                            };
                            if cached.as_ref().is_none_or(|(index, _)| *index != pick.art[0]) {
                                cached = Some((pick.art[0], frame(pick.art[0]).expect("valid bundled sticker frame")));
                            }
                            let bytes = render(
                                svg(pick, &cached.as_ref().unwrap().1).expect("valid bundled sticker range"),
                                px,
                            )
                            .expect("valid bundled sticker SVG");
                            format!("data:image/png;base64,{}", BASE64_STANDARD.encode(bytes))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut out = Vec::with_capacity(keys.len());
        for worker in workers {
            out.extend(worker.join().expect("sticker renderer stopped"));
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp-test/stickers-data").join(new_id());
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sticker_keys_and_find_are_literal() {
        for (text, key) in [
            ("👍🏽", "1f44d_1f3fd"),
            ("❤", "2764"),
            ("❤️", "2764"),
            ("❤︎", "2764"),
            ("#️⃣", "0023_20e3"),
            ("🏳️‍🌈", "1f3f3_200d_1f308"),
        ] {
            assert_eq!(key_of(text).as_deref(), Some(key));
            assert_eq!(find(text).unwrap().key, key);
        }
        assert_eq!(key_of("fire"), None);
        for text in ["🔥", "fire", "FiRe", ":fire:", "1f525"] {
            assert_eq!(find(text).unwrap().key, "1f525");
        }
        assert_eq!(find("arrow").unwrap().key, "arrow");
        assert_eq!(find("👍🏽").unwrap().name, "thumbs up: medium skin tone");
        assert!(find("🔥🔥").err().unwrap().to_string().contains("STICKER_UNKNOWN: pass one emoji at a time"));
        let error = find("firee").err().unwrap().to_string();
        assert!(error.starts_with("STICKER_UNKNOWN:") && error.contains("Close: fire 🔥"), "{error}");
        assert!(error.split("Close: ").nth(1).unwrap().split(", ").count() <= 5);
    }

    #[test]
    fn sticker_unknown_suggestions_match_search() {
        let error = find("party").err().unwrap().to_string();
        assert!(error.contains("party popper 🎉"), "{error}");
        let error = find("laughin").err().unwrap().to_string();
        assert!(error.starts_with("STICKER_UNKNOWN:") && error.contains("rolling on the floor laughing 🤣"), "{error}");
    }

    #[test]
    fn sticker_unknown_symbol_is_not_multiple_emoji() {
        for text in ["→", "🐱‍👤"] {
            let error = find(text).err().unwrap().to_string();
            assert!(error.starts_with("STICKER_UNKNOWN:") && !error.contains("one emoji at a time"), "{error}");
        }
    }

    #[test]
    fn sticker_gendered_and_directional_families_are_pickable() {
        for (base, key, name, emoji) in [
            ("1f937", "1f937_200d_2640", "woman shrugging", "🤷‍♀️"),
            ("1f6b6", "1f6b6_200d_27a1", "person walking: facing right", "🚶‍➡️"),
        ] {
            assert_eq!(find(name).unwrap().key, key);
            assert_eq!(find(emoji).unwrap().key, key);
            assert_eq!(search(name, 3)[0], key);
            let group = catalog().groups.iter().find(|g| g.items.iter().any(|i| i.key == base)).unwrap();
            let at = group.items.iter().position(|i| i.key == base).unwrap();
            let pick = &group.items[at + 1];
            assert_eq!(pick.key, key);
            assert!(!pick.keywords.is_empty());
            assert_eq!(pick.tones.as_ref().unwrap().len(), 5);
            assert_eq!(pick.alternates.len(), 5);
        }
    }

    #[test]
    fn sticker_search_ranking_is_literal() {
        assert_eq!(search("heart", 3), ["heart", "2764", "1f499"]);
        assert_eq!(search("👍🏽", 3), ["1f44d"]);
        assert!(search("", 10).is_empty());
        let serialized = serde_json::to_value(catalog()).unwrap();
        assert_eq!(serialized.as_object().unwrap().len(), 1);
        let item = &serialized["groups"][0]["items"][0];
        assert_eq!(item.as_object().unwrap().len(), 4);
        assert!(item["emoji"].is_null() && item["tones"].is_null());
    }

    #[test]
    fn sticker_entries_have_their_own_provenance() {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../../../assets/stickers/catalog.json")).unwrap();
        for group in data["groups"].as_array().unwrap() {
            for item in group["items"].as_array().unwrap() {
                for entry in std::iter::once(item).chain(item["alternates"].as_array().into_iter().flatten()) {
                    assert!(entry["source"].is_string(), "{}", entry["key"]);
                }
            }
        }
    }

    #[test]
    fn sticker_pack_hash_ranges_and_svg_are_valid() {
        assert_eq!(Sha256::digest(PACK).iter().map(|b| format!("{b:02x}")).collect::<String>(), catalog().pack_sha256);
        let mut seen = std::collections::HashSet::new();
        for index in 0..catalog().frames.len() {
            let raw = frame(index).unwrap();
            for pick in items().flat_map(|i| std::iter::once(i).chain(&i.alternates)).filter(|i| i.art[0] == index) {
                assert!(seen.insert(&pick.key), "{}", pick.key);
                resvg::usvg::Tree::from_data(svg(pick, &raw).unwrap(), &resvg::usvg::Options::default()).unwrap();
            }
        }
        assert_eq!(seen.len(), items().map(|i| 1 + i.alternates.len()).sum::<usize>());
        assert!(range(b"abc", usize::MAX, 2).is_err());
        assert!(range(b"abc", 1, 3).is_err());
    }

    #[test]
    fn sticker_cheeks_are_pink_on_both_sides() {
        let pick = find("🥰").unwrap();
        let bytes = render(svg(pick, &frame(pick.art[0]).unwrap()).unwrap(), 128).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut rgba).unwrap();
        for (x, y, expected) in [(24, 60, [243u8, 163, 80, 255]), (100, 64, [240, 139, 98, 255])] {
            let at = (y * 128 + x) * 4;
            assert!(
                rgba[at..at + 4].iter().zip(expected).all(|(&actual, expected)| actual.abs_diff(expected) <= 3),
                "cheek {x},{y}: {:?}",
                &rgba[at..at + 4]
            );
        }
    }

    #[test]
    fn sticker_png_is_never_rewritten_and_existing_asset_is_reused() {
        let dir = dir();
        let pick = find("🔥").unwrap();
        assert_eq!(file_name(pick, b"<svg/>"), "1f525-d4dc56669143-1024.png");
        let prepared = prepare_in(pick, &[], 123_456, None, None, &dir).unwrap();
        let EditCmd::AddAssets { assets } = &prepared.edits[0] else { panic!() };
        let asset = &assets[0];
        assert_eq!(asset.name, "Fire");
        assert!(asset.credit.is_none());
        assert_eq!((asset.width, asset.height), (1024, 1024));
        let existing = prepare_in(pick, assets, 456_789, Some(2_000_000), None, &dir).unwrap();
        assert_eq!(existing.asset_id, asset.id);
        assert!(
            matches!(existing.edits.as_slice(), [EditCmd::AddSticker { asset_id, start_us: 456_789, duration_us: Some(2_000_000), transform: None }] if asset_id == &asset.id)
        );
        std::fs::write(&asset.path, b"existing project pixels").unwrap();
        picture(pick, &dir).unwrap();
        assert_eq!(std::fs::read(&asset.path).unwrap(), b"existing project pixels");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sticker_thumbnails_keep_order_and_clamp_size() {
        for (px, expected) in [(0, 16), (512, 256)] {
            let thumbs = thumbnails(&["arrow".into(), "missing".into(), "1f525".into()], px);
            assert_eq!(thumbs[1], "");
            for t in [&thumbs[0], &thumbs[2]] {
                let bytes = BASE64_STANDARD.decode(t.strip_prefix("data:image/png;base64,").unwrap()).unwrap();
                let reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
                assert_eq!(reader.info().width.max(reader.info().height), expected);
            }
        }
        assert!(thumbnails(&[], 64).is_empty());
    }
}
