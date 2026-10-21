//! Credits for the library sounds and the credited footage, such as the tour's sample video, that a video
//! uses, as its description should carry them. CC BY asks for the title, the author, the source, the licence
//! and what was changed; syncing a sound to a video is always a change, footage changes when it is cut or
//! played otherwise. CC0 ones need no credit and are listed after them as a courtesy; the sounds built into
//! Nuzky are left out.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::export::heard_clips;
use crate::model::{Asset, AssetKind, Clip, ClipContent, Credit, License, Project, TrackKind};

/// Under this a trim is the file's own rounding, not a cut.
const TRIM_TOLERANCE_US: i64 = 50_000;

/// What the video does to one credited file.
#[derive(Default)]
struct Changes {
    trimmed: bool,
    faded: bool,
    volume: bool,
    speed: bool,
}

impl Changes {
    fn describe(&self) -> String {
        let done: Vec<&str> = [
            (self.trimmed, "trimmed"),
            (self.faded, "faded"),
            (self.volume, "volume changed"),
            (self.speed, "speed changed"),
        ]
        .into_iter()
        .filter_map(|(on, what)| on.then_some(what))
        .collect();
        if done.is_empty() { "Synced to video.".into() } else { format!("Synced to video; {}.", done.join(", ")) }
    }

    /// For footage; nothing when it plays whole and as it is.
    fn describe_footage(&self) -> Option<String> {
        let done: Vec<&str> = [
            (self.trimmed, "cut"),
            (self.faded, "faded"),
            (self.volume, "volume changed"),
            (self.speed, "speed changed"),
        ]
        .into_iter()
        .filter_map(|(on, what)| on.then_some(what))
        .collect();
        let text = done.join(", ");
        let mut chars = text.chars();
        Some(chars.next()?.to_uppercase().chain(chars).collect::<String>() + ".")
    }
}

fn line(credit: &Credit, changes: &Changes, sound: bool) -> String {
    let mut line = format!("\"{}\"", credit.title);
    if !credit.author.is_empty() {
        line += &format!(" by {}", credit.author);
    }
    if !credit.url.is_empty() {
        line += &format!(" ({})", credit.url);
    }
    line += &format!(", licensed under {} ({}).", credit.license_name(), credit.license_url);
    if credit.license == License::CcBy {
        match sound {
            true => line += &format!(" {}", changes.describe()),
            false => line.extend(changes.describe_footage().map(|changes| format!(" {changes}"))),
        }
    }
    line
}

/// The credits of every credited file heard or seen in the exported video, footage first, each in the order
/// it first plays. None when none of them needs credit.
pub fn credits(project: &Project) -> Option<String> {
    let mut order: Vec<&Asset> = Vec::new();
    let mut changes: HashMap<&str, Changes> = HashMap::new();
    let duration = project.duration_us();
    let heard: HashSet<&str> = heard_clips(project).map(|clip| clip.id.as_str()).collect();
    // Footage on a muted track or without volume is still seen.
    let mut clips: Vec<(&Clip, bool)> = Vec::new();
    for track in &project.tracks {
        let pictured = !track.hidden && track.kind == TrackKind::Video;
        for clip in &track.clips {
            let heard = heard.contains(clip.id.as_str());
            if heard || (pictured && clip.end_us() > 0 && clip.start_us < duration) {
                clips.push((clip, heard));
            }
        }
    }
    clips.sort_by_key(|(clip, _)| clip.start_us);
    for (clip, heard) in clips {
        let ClipContent::Media {
            asset_id,
            source_in_us,
            speed,
            volume,
            fade_in_us,
            fade_out_us,
            duck_db,
            normalize,
            leveler,
            ..
        } = &clip.content
        else {
            continue;
        };
        let Some(asset) = project.asset(asset_id) else { continue };
        if asset.credit.as_ref().is_none_or(|credit| credit.source == "nuzky")
            || (!heard && asset.kind == AssetKind::Audio)
        {
            continue;
        }
        if !changes.contains_key(asset_id.as_str()) {
            order.push(asset);
        }
        let entry = changes.entry(asset_id).or_default();
        let source_end = *source_in_us as f64 + clip.duration_us as f64 * f64::from(*speed);
        entry.trimmed |=
            *source_in_us > TRIM_TOLERANCE_US || source_end < (asset.duration_us - TRIM_TOLERANCE_US) as f64;
        entry.speed |= *speed != 1.0;
        // Detached sound plays from another clip, so a silent picture says nothing about the sound.
        if heard {
            entry.faded |= *fade_in_us > 0 || *fade_out_us > 0;
            entry.volume |= *volume != 1.0 || *duck_db != 0.0 || *normalize || *leveler > 0.0;
        }
    }
    let lines = |license: License, kinds: &[bool]| -> String {
        let assets =
            kinds.iter().flat_map(|&sound| order.iter().filter(move |a| (a.kind == AssetKind::Audio) == sound));
        assets
            .filter_map(|asset| Some((asset, asset.credit.as_ref()?, &changes[asset.id.as_str()])))
            .filter(|(_, credit, _)| credit.license == license)
            .map(|(asset, credit, changes)| line(credit, changes, asset.kind == AssetKind::Audio) + "\n")
            .collect()
    };
    let sections: Vec<String> =
        [("Video", lines(License::CcBy, &[false])), ("Music and sound effects", lines(License::CcBy, &[true]))]
            .into_iter()
            .filter(|(_, lines)| !lines.is_empty())
            .map(|(heading, lines)| format!("{heading}:\n{lines}"))
            .collect();
    if sections.is_empty() {
        return None;
    }
    let mut text = sections.join("\n");
    let courtesy = lines(License::Cc0, &[false, true]);
    if !courtesy.is_empty() {
        text += &format!("\nAlso used, no credit required:\n{courtesy}");
    }
    Some(text)
}

/// The credits file written beside an export: `reel.mp4` gets `reel.credits.txt`.
pub fn credits_path(video: &Path) -> PathBuf {
    video.with_extension("credits.txt")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::EditCmd;
    use crate::model::AssetKind;

    fn sound(id: &str, license: License, source: &str) -> Asset {
        Asset {
            id: id.into(),
            name: format!("{id}.mp3"),
            path: format!("/sounds/{id}.mp3"),
            kind: AssetKind::Audio,
            duration_us: 10_000_000,
            width: 0,
            height: 0,
            fps: 0.0,
            has_audio: true,
            rotation: 0,
            mirror: false,
            credit: Some(Credit {
                source: source.into(),
                id: id.into(),
                title: format!("Song {id}"),
                author: "Ann Author".into(),
                license,
                license_version: if license == License::Cc0 { "1.0".into() } else { "4.0".into() },
                license_url: if license == License::Cc0 {
                    "https://creativecommons.org/publicdomain/zero/1.0/".into()
                } else {
                    "https://creativecommons.org/licenses/by/4.0/".into()
                },
                url: format!("https://example.org/{id}"),
            }),
            part: None,
        }
    }

    fn project(assets: Vec<Asset>) -> Project {
        let mut project = Project::new("Credits");
        let ids: Vec<String> = assets.iter().map(|a| a.id.clone()).collect();
        project.apply(EditCmd::AddAssets { assets }).unwrap();
        for (i, id) in ids.iter().enumerate() {
            project
                .apply(EditCmd::AddClip { asset_id: id.clone(), start_us: Some(i as i64 * 20_000_000), track_id: None })
                .unwrap();
        }
        project
    }

    #[test]
    fn cc_by_sounds_are_credited_with_their_changes_and_cc0_ones_as_a_courtesy() {
        let mut project = project(vec![
            sound("by", License::CcBy, "openverse"),
            sound("zero", License::Cc0, "openverse"),
            sound("builtin", License::Cc0, "nuzky"),
        ]);
        let text = credits(&project).unwrap();
        assert_eq!(
            text,
            "Music and sound effects:\n\
             \"Song by\" by Ann Author (https://example.org/by), licensed under CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/). Synced to video.\n\
             \nAlso used, no credit required:\n\
             \"Song zero\" by Ann Author (https://example.org/zero), licensed under CC0 1.0 (https://creativecommons.org/publicdomain/zero/1.0/).\n"
        );
        for (match_loudness, even_volume) in [(true, 0.0), (false, 0.5)] {
            let mut changed = project.clone();
            let clip = changed
                .tracks
                .iter_mut()
                .flat_map(|t| &mut t.clips)
                .find(|c| matches!(&c.content, ClipContent::Media { asset_id, .. } if asset_id == "by"))
                .unwrap();
            let ClipContent::Media { normalize, leveler, .. } = &mut clip.content else { panic!() };
            *normalize = match_loudness;
            *leveler = even_volume;
            assert!(credits(&changed).unwrap().contains("Synced to video; volume changed."));
        }
        // Trimmed and faded in the video, so the credit says so.
        for track in &mut project.tracks {
            for clip in &mut track.clips {
                if let ClipContent::Media { asset_id, fade_in_us, .. } = &mut clip.content
                    && asset_id == "by"
                {
                    clip.duration_us = 4_000_000;
                    *fade_in_us = 500_000;
                }
            }
        }
        assert!(credits(&project).unwrap().contains("Synced to video; trimmed, faded."));
    }

    #[test]
    fn only_heard_sounds_need_credit() {
        let mut project =
            project(vec![sound("by", License::CcBy, "openverse"), sound("zero", License::Cc0, "openverse")]);
        // CC0 alone needs nothing.
        project.tracks.iter_mut().for_each(|t| {
            t.clips.retain(|c| !matches!(&c.content, ClipContent::Media { asset_id, .. } if asset_id == "by"))
        });
        assert_eq!(credits(&project), None);
        // A CC BY sound in the project but muted is not in the video.
        let mut muted = self::project(vec![sound("by", License::CcBy, "openverse")]);
        muted.tracks.iter_mut().filter(|t| !t.clips.is_empty()).for_each(|t| t.muted = true);
        assert_eq!(credits(&muted), None);
    }

    #[test]
    fn credited_footage_has_its_own_section_and_says_how_it_was_cut() {
        let mut talk = sound("talk", License::CcBy, "youtube");
        talk.kind = AssetKind::Video;
        let mut footage = project(vec![talk.clone()]);
        let uncut = "Video:\n\"Song talk\" by Ann Author (https://example.org/talk), licensed under CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/).\n";
        assert_eq!(credits(&footage).unwrap(), uncut);
        let clip = footage.tracks[0].clips[0].id.clone();
        footage.apply(EditCmd::SplitClip { clip_id: clip.clone(), at_us: 4_000_000 }).unwrap();
        footage.apply(EditCmd::DeleteClips { clip_ids: vec![clip] }).unwrap();
        assert_eq!(credits(&footage).unwrap(), uncut.replace(").\n", "). Cut.\n"));

        // With a song under it, each has its section, the footage first.
        let mut both = project(vec![talk]);
        both.apply(EditCmd::AddAssets { assets: vec![sound("by", License::CcBy, "openverse")] }).unwrap();
        both.apply(EditCmd::AddClip { asset_id: "by".into(), start_us: Some(0), track_id: None }).unwrap();
        let text = credits(&both).unwrap();
        assert!(text.starts_with(uncut) && text.ends_with("\nMusic and sound effects:\n\"Song by\" by Ann Author (https://example.org/by), licensed under CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/). Synced to video.\n"), "{text}");
        let ClipContent::Media { speed, .. } = &mut both.tracks[0].clips[0].content else { panic!() };
        *speed = 2.0;
        assert!(credits(&both).unwrap().contains("4.0/). Speed changed.\n"));
    }

    #[test]
    fn footage_is_credited_while_its_picture_is_in_the_video() {
        let mut talk = sound("talk", License::CcBy, "youtube");
        talk.kind = AssetKind::Video;
        let mut both = project(vec![talk]);
        both.apply(EditCmd::AddAssets { assets: vec![sound("by", License::CcBy, "openverse")] }).unwrap();
        both.apply(EditCmd::AddClip { asset_id: "by".into(), start_us: Some(0), track_id: None }).unwrap();
        let full = credits(&both).unwrap();
        assert!(full.starts_with("Video:\n\"Song talk\""), "{full}");
        // Muted under music, or silent by its own volume, its picture still plays.
        both.tracks[0].muted = true;
        assert_eq!(credits(&both).unwrap(), full);
        both.tracks[0].muted = false;
        let ClipContent::Media { volume, .. } = &mut both.tracks[0].clips[0].content else { panic!() };
        *volume = 0.0;
        assert_eq!(credits(&both).unwrap(), full);
        // Hidden and silent, it is not in the video.
        both.tracks[0].hidden = true;
        assert!(credits(&both).unwrap().starts_with("Music and sound effects:\n"));
    }

    #[test]
    fn credits_file_sits_beside_the_video() {
        assert_eq!(credits_path(Path::new("/v/reel.mp4")), Path::new("/v/reel.credits.txt"));
        assert_eq!(credits_path(Path::new("/v/my.trip.mp4")), Path::new("/v/my.trip.credits.txt"));
    }
}
