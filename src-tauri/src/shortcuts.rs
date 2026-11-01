use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use serde::Serialize;

use crate::{CmdResult, err};

const NO_KEYS: &str = "This file has no keyboard shortcuts from Premiere Pro, Final Cut Pro or DaVinci Resolve.";
const LIMIT: u64 = 4 * 1024 * 1024;

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedKeys {
    pub program: KeysProgram,
    pub keys: BTreeMap<String, Vec<String>>,
}

#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum KeysProgram {
    Premiere,
    FinalCut,
    Resolve,
}

#[tauri::command]
pub fn import_shortcuts(path: String) -> CmdResult<ImportedKeys> {
    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(err)?.take(LIMIT + 1).read_to_end(&mut bytes).map_err(err)?;
    if bytes.len() as u64 > LIMIT {
        return Err("This shortcuts file is larger than 4 MB.".into());
    }
    parse(&bytes)
}

const PREMIERE: &[(&str, &str)] = &[
    ("cmd.transport.toggleplay", "playPause"),
    ("cmd.transport.step.back", "framePrev"),
    ("cmd.transport.step.forward", "frameNext"),
    ("cmd.transport.step.back.five", "secondBack"),
    ("cmd.transport.step.forward.five", "secondForward"),
    ("cmd.tlnav.prev.edit", "cutPrev"),
    ("cmd.tlnav.next.edit", "cutNext"),
    ("cmd.transport.sequence.start", "goStart"),
    ("cmd.transport.sequence.end", "goEnd"),
    ("cmd.toggle.fullscreen.monitor", "fullScreen"),
    ("cmd.sequence.razorateditline", "split"),
    ("cmd.sequence.rippletrimpreviousedittoplayhead", "deleteLeft"),
    ("cmd.sequence.rippletrimnextedittoplayhead", "deleteRight"),
    ("cmd.clip.videooptions.addframehold", "freeze"),
    ("cmd.edit.clear", "delete"),
    ("cmd.timeline.ripple.delete", "delete"),
    ("cmd.edit.duplicate", "duplicate"),
    ("cmd.edit.copy", "copy"),
    ("cmd.edit.cut", "cut"),
    ("cmd.edit.paste", "paste"),
    ("cmd.edit.pasteattributes", "pasteAttributes"),
    ("cmd.edit.undo", "undo"),
    ("cmd.edit.redo", "redo"),
    ("cmd.zoom.in", "zoomIn"),
    ("cmd.zoom.out", "zoomOut"),
    ("cmd.file.import", "import"),
    ("cmd.file.export.movie", "export"),
];
const FINAL_CUT: &[(&str, &str)] = &[
    ("PlayPause", "playPause"),
    ("JumpToPreviousFrame", "framePrev"),
    ("JumpToNextFrame", "frameNext"),
    ("JumpBackward10Frames", "secondBack"),
    ("JumpForward10Frames", "secondForward"),
    ("PreviousEdit", "cutPrev"),
    ("NextEdit", "cutNext"),
    ("JumpToStart", "goStart"),
    ("JumpToEnd", "goEnd"),
    ("PlayFullscreen", "fullScreen"),
    ("BladeAtPlayhead", "split"),
    ("TrimStart", "deleteLeft"),
    ("TrimEnd", "deleteRight"),
    ("FreezeFrame", "freeze"),
    ("Delete", "delete"),
    ("Duplicate", "duplicate"),
    ("Copy", "copy"),
    ("Cut", "cut"),
    ("Paste", "paste"),
    ("PasteSomeAttributes", "pasteAttributes"),
    ("UndoChanges", "undo"),
    ("RedoChanges", "redo"),
    ("ZoomIn", "zoomIn"),
    ("ZoomOut", "zoomOut"),
    ("Import", "import"),
    ("ShareDefaultDestination", "export"),
];
const RESOLVE: &[(&str, &str)] = &[
    ("controlPlayToggle", "playPause"),
    ("controlStepReverse", "framePrev"),
    ("controlStepForward", "frameNext"),
    ("controlLargeStepReverse", "secondBack"),
    ("controlLargeStepForward", "secondForward"),
    ("controlClipPrev", "cutPrev"),
    ("controlClipNext", "cutNext"),
    ("controlTimelineStart", "goStart"),
    ("controlTimelineEnd", "goEnd"),
    ("workspaceViewerModeCinemaViewer", "fullScreen"),
    ("editM2SplitClip", "split"),
    ("editBladeRazor", "split"),
    ("trimRippleStartToPlayhead", "deleteLeft"),
    ("trimRippleEndToPlayhead", "deleteRight"),
    ("clipFreezeFrame", "freeze"),
    ("editDelete", "delete"),
    ("editBackspace", "delete"),
    ("editCopy", "copy"),
    ("editCut", "cut"),
    ("editPaste", "paste"),
    ("editPasteAttributes", "pasteAttributes"),
    ("editUndo", "undo"),
    ("editRedo", "redo"),
    ("viewZoomSubZoomIn", "zoomIn"),
    ("viewZoomSubZoomOut", "zoomOut"),
    ("fileImportMedia", "import"),
];

fn bind(
    keys: &mut BTreeMap<String, Vec<String>>,
    table: &[(&str, &str)],
    command: &str,
    chords: impl IntoIterator<Item = String>,
) {
    if let Some((_, action)) = table.iter().find(|(id, _)| *id == command) {
        let values = keys.entry((*action).into()).or_default();
        for chord in chords {
            if !values.contains(&chord) {
                values.push(chord);
            }
        }
    }
}

fn character(key: &str) -> Option<String> {
    let mut chars = key.chars();
    let c = chars.next()?;
    (chars.next().is_none() && c.is_ascii_graphic())
        .then(|| c.to_ascii_uppercase().to_string())
        .or_else(|| (key == " ").then(|| "Space".into()))
}

fn function(key: &str) -> Option<String> {
    (1..=12).any(|n| key == format!("F{n}")).then(|| key.into())
}

fn chord(key: String, ctrl: bool, alt: bool, shift: bool) -> String {
    let ctrl = if ctrl { "Ctrl+" } else { "" };
    let alt = if alt { "Alt+" } else { "" };
    let shift = if shift { "Shift+" } else { "" };
    format!("{ctrl}{alt}{shift}{key}")
}

fn premiere_key(item: roxmltree::Node<'_, '_>) -> Option<String> {
    let field = |tag| item.children().find(|n| n.has_tag_name(tag)).and_then(|n| n.text()).unwrap_or("").trim();
    let text = field("virtualkey");
    let v = if let Some(hex) = text.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        text.parse::<u32>().ok()?
    };
    let key = if v & 0xC0000000 == 0xC0000000 {
        return None;
    } else if v & 0x80000000 != 0 {
        character(&char::from_u32(v & 0x7FFFFFFF)?.to_string())?
    } else {
        match v {
            1 => "Space".into(),
            2 => "Backspace".into(),
            3 => "Tab".into(),
            4 => "Enter".into(),
            7..=18 => format!("F{}", v - 6),
            35 => "Delete".into(),
            36 => "Home".into(),
            37 => "End".into(),
            38 => "PgUp".into(),
            39 => "PgDn".into(),
            42 => "←".into(),
            43 => "→".into(),
            44 => "↑".into(),
            45 => "↓".into(),
            _ => return None,
        }
    };
    let enabled = |tag| matches!(field(tag), "1" | "true");
    Some(chord(
        key,
        enabled("modifier.ctrl") || enabled("modifier.command"),
        enabled("modifier.alt") || enabled("modifier.opt"),
        enabled("modifier.shift"),
    ))
}

fn final_cut_key(value: &plist::Value) -> Option<String> {
    let dict = value.as_dictionary()?;
    let text = dict.get("characterString")?.as_string()?;
    let mask = dict.get("modifierMask").and_then(plist::Value::as_unsigned_integer).unwrap_or(0);
    let key = match text {
        "NSLeftArrowFunctionKey" => "←".into(),
        "NSRightArrowFunctionKey" => "→".into(),
        "NSUpArrowFunctionKey" => "↑".into(),
        "NSDownArrowFunctionKey" => "↓".into(),
        "NSHomeFunctionKey" => "Home".into(),
        "NSEndFunctionKey" => "End".into(),
        "NSPageUpFunctionKey" => "PgUp".into(),
        "NSPageDownFunctionKey" => "PgDn".into(),
        "NSDeleteCharacter" => "Backspace".into(),
        "NSDeleteFunctionKey" => "Delete".into(),
        "NSTabCharacter" => "Tab".into(),
        "NSCarriageReturnCharacter" | "NSEnterCharacter" => "Enter".into(),
        _ => character(text).or_else(|| function(text.strip_prefix("NS")?.strip_suffix("FunctionKey")?))?,
    };
    if mask & (1 << 21) != 0 && !matches!(key.as_str(), "←" | "→" | "↑" | "↓") {
        return None;
    }
    Some(chord(key, mask & ((1 << 18) | (1 << 20)) != 0, mask & (1 << 19) != 0, mask & (1 << 17) != 0))
}

fn resolve_key(text: &str) -> Option<String> {
    let text = text.trim();
    let (mods, key) = if text == "+" {
        ("", "+")
    } else if let Some(mods) = text.strip_suffix("++") {
        (mods, "+")
    } else {
        text.rsplit_once('+').unwrap_or(("", text))
    };
    let (mut ctrl, mut alt, mut shift) = (false, false, false);
    for modifier in mods.split('+').filter(|s| !s.is_empty()) {
        match modifier {
            "Ctrl" | "Meta" => ctrl = true,
            "Alt" => alt = true,
            "Shift" => shift = true,
            _ => return None,
        }
    }
    let key = match key {
        "Del" => "Delete".into(),
        "Backspace" | "Space" | "Esc" | "Tab" | "Home" | "End" | "PgUp" => key.into(),
        "Return" | "Enter" => "Enter".into(),
        "PgDown" => "PgDn".into(),
        "Left" => "←".into(),
        "Right" => "→".into(),
        "Up" => "↑".into(),
        "Down" => "↓".into(),
        _ => character(key).or_else(|| function(key))?,
    };
    Some(chord(key, ctrl, alt, shift))
}

fn parse(bytes: &[u8]) -> CmdResult<ImportedKeys> {
    let mut keys = BTreeMap::new();
    let text = std::str::from_utf8(bytes).unwrap_or("");
    let xml = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() },
    );
    let root = xml.as_ref().ok().map(roxmltree::Document::root_element);
    let program = if let Some(root) = root.filter(|n| n.has_tag_name("PremiereData")) {
        for item in root.descendants().filter(|n| n.is_element() && n.tag_name().name().starts_with("item.")) {
            if let Some(command) = item.children().find(|n| n.has_tag_name("commandname")).and_then(|n| n.text()) {
                bind(&mut keys, PREMIERE, command.trim(), premiere_key(item));
            }
        }
        KeysProgram::Premiere
    } else if bytes.starts_with(b"bplist00") || root.is_some_and(|n| n.has_tag_name("plist")) {
        let value = plist::Value::from_reader(Cursor::new(bytes)).map_err(|_| NO_KEYS)?;
        for (command, value) in value.as_dictionary().ok_or(NO_KEYS)? {
            let entries = value.as_array().map(Vec::as_slice).unwrap_or(std::slice::from_ref(value));
            bind(&mut keys, FINAL_CUT, command, entries.iter().filter_map(final_cut_key));
        }
        KeysProgram::FinalCut
    } else {
        for line in text.lines() {
            if let Some((command, value)) = line.split_once(":=") {
                bind(&mut keys, RESOLVE, command.trim(), value.split('|').filter_map(resolve_key));
            }
        }
        KeysProgram::Resolve
    };
    if keys.is_empty() {
        return Err(NO_KEYS.into());
    }
    Ok(ImportedKeys { program, keys })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_premiere() {
        let result = parse(br#"<PremiereData><shortcuts><platform>macintosh</platform><context.global>
            <item.0><virtualkey>1</virtualkey><commandname>cmd.transport.toggleplay</commandname></item.0>
            <item.1><modifier.shift>true</modifier.shift><virtualkey>0x8000007a</virtualkey><modifier.command>1</modifier.command><commandname>cmd.edit.redo</commandname><modifier.opt>1</modifier.opt></item.1>
            <item.2><commandname>cmd.file.import</commandname></item.2>
            <item.3><commandname>cmd.edit.clear</commandname><virtualkey>35</virtualkey></item.3>
            </context.global><context.timeline>
            <item.4><commandname>cmd.timeline.ripple.delete</commandname><virtualkey>0xC0000031</virtualkey></item.4>
            <item.5><commandname>cmd.timeline.ripple.delete</commandname><virtualkey>35</virtualkey></item.5>
            <item.6><commandname>cmd.timeline.ripple.delete</commandname><virtualkey>2</virtualkey></item.6>
            </context.timeline></shortcuts></PremiereData>"#).unwrap();
        assert_eq!(result.program, KeysProgram::Premiere);
        assert_eq!(result.keys["playPause"], ["Space"]);
        assert_eq!(result.keys["redo"], ["Ctrl+Alt+Shift+Z"]);
        assert_eq!(result.keys["delete"], ["Delete", "Backspace"]);
        assert!(result.keys["import"].is_empty());
        assert!(!result.keys.contains_key("export"));
    }

    #[test]
    fn shortcuts_final_cut() {
        let xml = br#"<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict>
            <key>PlayPause</key><dict><key>characterString</key><string> </string></dict>
            <key>RedoChanges</key><array><dict><key>characterString</key><string>z</string><key>modifierMask</key><integer>1966080</integer></dict><dict><key>characterString</key><string>z</string><key>modifierMask</key><integer>1966080</integer></dict></array>
            <key>JumpToPreviousFrame</key><dict><key>characterString</key><string>NSLeftArrowFunctionKey</string><key>modifierMask</key><integer>2359296</integer></dict>
            <key>Import</key><dict><key>characterString</key><string></string></dict>
            <key>Delete</key><dict><key>characterString</key><string>1</string><key>modifierMask</key><integer>2097152</integer></dict>
            </dict></plist>"#;
        let mut binary = Vec::new();
        plist::Value::from_reader_xml(xml.as_slice()).unwrap().to_writer_binary(&mut binary).unwrap();
        for bytes in [xml.as_slice(), binary.as_slice()] {
            let result = parse(bytes).unwrap();
            assert_eq!(result.program, KeysProgram::FinalCut);
            assert_eq!(result.keys["playPause"], ["Space"]);
            assert_eq!(result.keys["redo"], ["Ctrl+Alt+Shift+Z"]);
            assert_eq!(result.keys["framePrev"], ["Ctrl+←"]);
            assert!(result.keys["import"].is_empty());
            assert!(result.keys["delete"].is_empty());
        }
    }

    #[test]
    fn shortcuts_resolve() {
        let result = parse(b"controlPlayToggle := Space\r\neditRedo := Meta+Alt+Shift+z | Ctrl+Alt+Shift+Z\r\nviewZoomSubZoomIn := Ctrl++ | +\r\nfileImportMedia :=\r\neditM2SplitClip := Num+1 | Ctrl+B\r\neditBladeRazor := Ctrl+B | B\r\nEditTimeline.editCopy := Ctrl+C\r\neditDelete := Del\r\neditBackspace := Backspace").unwrap();
        assert_eq!(result.program, KeysProgram::Resolve);
        assert_eq!(result.keys["playPause"], ["Space"]);
        assert_eq!(result.keys["redo"], ["Ctrl+Alt+Shift+Z"]);
        assert_eq!(result.keys["zoomIn"], ["Ctrl++", "+"]);
        assert_eq!(result.keys["split"], ["Ctrl+B", "B"]);
        assert_eq!(result.keys["delete"], ["Delete", "Backspace"]);
        assert!(result.keys["import"].is_empty());
        assert!(!result.keys.contains_key("copy"));
    }

    #[test]
    fn shortcuts_garbage() {
        for bytes in [
            b"garbage".as_slice(),
            b"<PremiereData/>",
            b"<plist><dict/></plist>",
            b"other := Ctrl+A",
            b"bplist00broken",
            b"\xff\xfe",
        ] {
            assert_eq!(parse(bytes).unwrap_err(), NO_KEYS);
        }
    }
}
