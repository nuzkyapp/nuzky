mod qa_support;
use nuzky_engine::{
    edit::{CaptionSegment, EditCmd, Editor, TimeRange},
    model::*,
};
use qa_support::*;
fn asset(id: &str, duration: i64) -> Asset {
    Asset {
        id: id.into(),
        name: id.into(),
        path: String::new(),
        kind: AssetKind::Video,
        duration_us: duration,
        width: 64,
        height: 64,
        fps: 30.0,
        has_audio: true,
        rotation: 0,
        mirror: false,
        credit: None,
        part: None,
    }
}
fn editor() -> Editor {
    let mut p = Project::new("properties");
    p.assets = vec![asset("a", 5_000_000), asset("b", 3_000_000)];
    Editor::new(p)
}

fn keyword_caption(group: usize) -> (Editor, String) {
    let mut e = editor();
    let words: Vec<_> = ["Dnes", "je", "Zdarma,", "vše"]
        .into_iter()
        .enumerate()
        .map(|(i, text)| CaptionWord {
            text: text.into(),
            start_us: 1_000_000 + i as i64 * 500_000,
            end_us: 1_400_000 + i as i64 * 500_000,
            key: i == 2,
            marked: None,
        })
        .collect();
    let segments = words
        .chunks(group)
        .map(|words| CaptionSegment {
            start_us: words[0].start_us,
            end_us: words.last().unwrap().end_us + 100_000,
            text: words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
            words: words.to_vec(),
        })
        .collect();
    let out = e
        .apply(
            EditCmd::AddCaptions {
                segments,
                style: serde_json::from_value(serde_json::json!({"fontSize":64.0,"color":"#ffffff"})).unwrap(),
                anim_in: None,
                anim_out: None,
                transform: None,
            },
            None,
        )
        .unwrap();
    (e, out.select[0].clone())
}

#[test]
fn keyword_marks_override_the_pick_and_undo_as_one_step() {
    let (mut e, id) = keyword_caption(4);
    let before = e.project.clone();
    e.apply(EditCmd::MarkKeyWord { clip_id: id.clone(), word: 2, marked: Some(true) }, None).unwrap();
    let picked = |p: &Project, pick| {
        let ClipContent::Text { text, words, .. } = &p.tracks[1].clips[0].content else { panic!() };
        key_words(text, words, pick).into_iter().map(|r| text[r].to_owned()).collect::<Vec<_>>()
    };
    assert_eq!(picked(&e.project, None), ["Zdarma,"]);
    assert!(e.undo());
    assert_eq!(e.project, before, "one undo removes only the mark");
    assert!(e.redo());
    for (marked, expected) in [(Some(false), vec![]), (None, vec!["Zdarma,"])] {
        e.apply(EditCmd::MarkKeyWord { clip_id: id.clone(), word: 2, marked }, None).unwrap();
        assert_eq!(picked(&e.project, Some(KeywordPick::Emphasis)), expected);
        assert!(picked(&e.project, None).is_empty());
    }
}

#[test]
fn keyword_marks_refuse_invalid_targets_without_changing_the_project() {
    let (mut e, id) = keyword_caption(4);
    let media =
        e.apply(EditCmd::AddClip { asset_id: "a".into(), start_us: None, track_id: None }, None).unwrap().select[0]
            .clone();
    for (clip_id, word) in [("missing".to_owned(), 0), (media, 0), (id.clone(), 4)] {
        let before = e.project.clone();
        assert!(e.apply(EditCmd::MarkKeyWord { clip_id, word, marked: Some(true) }, None).is_err());
        assert_eq!(e.project, before);
    }
    e.apply(
        serde_json::from_value(serde_json::json!({"type":"updateClip","clipId":id,"text":"Dnes je vše"})).unwrap(),
        None,
    )
    .unwrap();
    let before = e.project.clone();
    assert!(e.apply(EditCmd::MarkKeyWord { clip_id: id, word: 0, marked: Some(true) }, None).is_err());
    assert_eq!(e.project, before);
}

#[test]
fn keyword_marks_follow_the_word_through_restyle_split_trim_ripple_and_duplicate() {
    let (mut e, id) = keyword_caption(4);
    e.apply(EditCmd::MarkKeyWord { clip_id: id.clone(), word: 2, marked: Some(true) }, None).unwrap();
    let marked_at = |p: &Project| {
        p.tracks[1]
            .clips
            .iter()
            .flat_map(|c| {
                let ClipContent::Text { text, words, .. } = &c.content else { panic!() };
                let keys = key_words(text, words, None);
                assert_eq!(keys.len(), words.iter().filter(|w| w.marked == Some(true)).count());
                words
                    .iter()
                    .filter(|w| w.marked == Some(true) && c.contains(c.start_us + (w.start_us + w.end_us) / 2))
                    .map(|w| (w.text.clone(), c.start_us + (w.start_us + w.end_us) / 2))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let track = e.project.tracks[1].id.clone();
    let cmds = [
        serde_json::from_value(serde_json::json!({"type":"restyleTrack","trackId":track,"style":{"fontSize":80.0}}))
            .unwrap(),
        EditCmd::SplitClip { clip_id: id.clone(), at_us: 1_950_000 },
        EditCmd::TrimClip { clip_id: id.clone(), start_us: 1_450_000, duration_us: 1_550_000, source_in_us: None },
        EditCmd::RippleDeleteRanges {
            ranges: vec![TimeRange { start_us: 1_450_000, end_us: 1_950_000 }],
            keep_track_ids: None,
        },
        EditCmd::DuplicateClip { clip_id: id },
    ];
    for (i, cmd) in cmds.into_iter().enumerate() {
        let mut p = e.project.clone();
        p.apply(cmd).unwrap();
        let expected = match i {
            3 => vec![("Zdarma,".into(), 1_700_000)],
            4 => vec![("Zdarma,".into(), 2_200_000), ("Zdarma,".into(), 4_200_000)],
            _ => vec![("Zdarma,".into(), 2_200_000)],
        };
        assert_eq!(marked_at(&p), expected, "edit {i}");
    }
}

#[test]
fn keyword_marks_survive_regrouping_only_for_the_same_speech_and_respect_new_marks() {
    let (mut e, _) = keyword_caption(2);
    let id = e.project.tracks[1].clips[1].id.clone();
    e.apply(EditCmd::MarkKeyWord { clip_id: id, word: 0, marked: Some(true) }, None).unwrap();
    let track_id = e.project.tracks[1].id.clone();
    let ClipContent::Text { style, .. } = &e.project.tracks[1].clips[0].content else { panic!() };
    for (text, marked, at, expected) in [
        ("zdarma", None, 2_000_000, Some(true)),
        ("placené", None, 2_000_000, None),
        ("zdarma", Some(false), 2_000_000, Some(false)),
        ("zdarma", None, 2_400_000, None),
    ] {
        let word = |text: &str, start_us, marked| CaptionWord {
            text: text.into(),
            start_us,
            end_us: start_us + 400_000,
            key: false,
            marked,
        };
        let segments = vec![
            CaptionSegment {
                start_us: 1_000_000,
                end_us: 2_900_000,
                text: format!("Dnes je {text}"),
                words: vec![word("Dnes", 1_000_000, None), word("je", 1_500_000, None), word(text, at, marked)],
            },
            CaptionSegment {
                start_us: 2_900_000,
                end_us: 3_500_000,
                text: "vše".into(),
                words: vec![word("vše", 3_000_000, None)],
            },
        ];
        let mut p = e.project.clone();
        p.apply(EditCmd::ReplaceCaptions {
            track_id: track_id.clone(),
            segments,
            style: style.clone(),
            anim_in: None,
            anim_out: None,
            transform: None,
            ranges: None,
        })
        .unwrap();
        let ClipContent::Text { words, .. } = &p.tracks[1].clips[0].content else { panic!() };
        assert_eq!(
            words.iter().map(|w| w.marked).collect::<Vec<_>>(),
            [None, None, expected],
            "{text} at {at}, explicit {marked:?}"
        );
        let ClipContent::Text { words, .. } = &p.tracks[1].clips[1].content else { panic!() };
        assert_eq!(words[0].marked, None);
    }
}

#[test]
fn regenerating_takes_the_mark_of_the_half_a_split_word_is_heard_in() {
    let (mut e, id) = keyword_caption(4);
    e.apply(EditCmd::MarkKeyWord { clip_id: id.clone(), word: 2, marked: Some(true) }, None).unwrap();
    // Both halves keep every word; "Zdarma," is heard in the right one, which makes it plain.
    e.apply(EditCmd::SplitClip { clip_id: id, at_us: 1_950_000 }, None).unwrap();
    let right = e.project.tracks[1].clips[1].id.clone();
    e.apply(EditCmd::MarkKeyWord { clip_id: right, word: 2, marked: Some(false) }, None).unwrap();
    let ClipContent::Text { words, style, .. } = e.project.tracks[1].clips[0].content.clone() else { panic!() };
    let words: Vec<_> = words
        .into_iter()
        .map(|w| CaptionWord { start_us: w.start_us + 1_000_000, end_us: w.end_us + 1_000_000, marked: None, ..w })
        .collect();
    let segment = CaptionSegment { start_us: 1_000_000, end_us: 3_000_000, text: "Dnes je Zdarma, vše".into(), words };
    let track_id = e.project.tracks[1].id.clone();
    e.apply(
        EditCmd::ReplaceCaptions {
            track_id,
            segments: vec![segment],
            style,
            anim_in: None,
            anim_out: None,
            transform: None,
            ranges: None,
        },
        None,
    )
    .unwrap();
    let ClipContent::Text { words, .. } = &e.project.tracks[1].clips[0].content else { panic!() };
    assert_eq!(words.iter().map(|w| w.marked).collect::<Vec<_>>(), [None, None, Some(false), None]);
}

fn update_speed(id: &str, speed: f32) -> EditCmd {
    EditCmd::UpdateClip {
        clip_id: id.into(),
        transform: None,
        volume: None,
        text: None,
        style: None,
        speed: Some(speed),
        keep_pitch: None,
        adjust: None,
        fade_in_us: None,
        fade_out_us: None,
        clean_voice: None,
        dereverb: None,
        normalize: None,
        leveler: None,
        shape: None,
        duck_db: None,
        mono: None,
        background: None,
    }
}
fn invariants(p: &Project) -> Result<(), String> {
    for track in &p.tracks {
        let mut end = 0;
        for c in &track.clips {
            if c.start_us < end || track.id == "main" && c.start_us != end {
                return Err(format!("track {} overlap/gap: expected >= {end}, clip starts {}", track.id, c.start_us));
            }
            if c.duration_us <= 0 {
                return Err("nonpositive duration".into());
            }
            end = c.end_us();
            if let ClipContent::Media { asset_id, source_in_us, speed, .. } = &c.content {
                let a = p.asset(asset_id).unwrap();
                let source_end = *source_in_us as f64 + c.duration_us as f64 * *speed as f64;
                if *source_in_us < 0 || a.kind != AssetKind::Image && source_end > a.duration_us as f64 + 2.0 {
                    return Err(format!(
                        "source range exceeds {}: in={source_in_us}, duration={}, speed={speed}, end={source_end}, limit={}",
                        a.id, c.duration_us, a.duration_us
                    ));
                }
            }
        }
    }
    let json = serde_json::to_string(p).unwrap();
    let back: Project = serde_json::from_str(&json).unwrap();
    assert_eq!(p, &back);
    Ok(())
}
#[test]
fn seeded_edits_without_speed_preserve_invariants_and_history() {
    seeded_sequences(false);
}
#[test]
fn seeded_edit_sequences_preserve_invariants_and_history() {
    seeded_sequences(true);
}
fn seeded_sequences(with_speed: bool) {
    let mut failures = Vec::new();
    for seed in [0x12345678, 0xCAFEBABE, 0xF00DFACE, 0xBADC0DE] {
        let mut rng = Rng(seed);
        let mut e = editor();
        let mut history = Vec::new();
        for step in 0..120 {
            let clips: Vec<_> =
                e.project.tracks.iter().flat_map(|t| t.clips.iter().map(|c| (t.id.clone(), c.clone()))).collect();
            let cmd = if clips.is_empty() || rng.next().is_multiple_of(7) {
                EditCmd::AddClip {
                    asset_id: if rng.next().is_multiple_of(2) { "a" } else { "b" }.into(),
                    start_us: None,
                    track_id: None,
                }
            } else {
                let (track, c) = &clips[rng.next() as usize % clips.len()];
                match rng.next() % if with_speed { 6 } else { 5 } {
                    0 => EditCmd::MoveClip {
                        clip_id: c.id.clone(),
                        track_id: if rng.next().is_multiple_of(2) { Some("main".into()) } else { None },
                        start_us: (rng.next() % 10_000_000) as i64,
                        new_track: None,
                    },
                    1 => EditCmd::TrimClip {
                        clip_id: c.id.clone(),
                        start_us: c.start_us,
                        duration_us: 100_000 + (rng.next() % 6_000_000) as i64,
                        source_in_us: Some((rng.next() % 5_000_000) as i64),
                    },
                    2 => EditCmd::SplitClip { clip_id: c.id.clone(), at_us: c.start_us + c.duration_us / 2 },
                    3 => EditCmd::DuplicateClip { clip_id: c.id.clone() },
                    4 => EditCmd::DeleteClips { clip_ids: vec![c.id.clone()] },
                    _ => {
                        let _ = track;
                        update_speed(&c.id, [0.5, 1.0, 2.0, 4.0][rng.next() as usize % 4])
                    }
                }
            };
            let before = e.project.clone();
            let desc = format!("{cmd:?}");
            let result = e.apply(cmd, None);
            history.push(desc);
            if result.is_err() {
                assert_eq!(e.project, before, "rejected command mutated project");
                continue;
            }
            let after = e.project.clone();
            if after != before {
                assert!(e.undo());
                assert_eq!(e.project, before);
                assert!(e.redo());
                assert_eq!(e.project, after);
            }
            if let Err(error) = invariants(&e.project) {
                std::fs::write(dir("edit-traces").join(format!("seed-{seed:x}-{with_speed}.txt")), history.join("\n"))
                    .unwrap();
                failures.push(format!("seed={seed:#x} step={step}: {error}"));
                break;
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn slowing_overlay_must_not_overlap_next_clip() {
    let mut e = editor();
    e.apply(EditCmd::AddClip { asset_id: "a".into(), start_us: None, track_id: None }, None).unwrap();
    let id = e.project.tracks[0].clips[0].id.clone();
    e.apply(EditCmd::MoveClip { clip_id: id.clone(), track_id: None, start_us: 0, new_track: None }, None).unwrap();
    let track = e.project.tracks[1].id.clone();
    e.apply(EditCmd::AddClip { asset_id: "b".into(), start_us: Some(5_000_000), track_id: Some(track) }, None).unwrap();
    let _ = e.apply(update_speed(&id, 0.5), None);
    invariants(&e.project).unwrap();
}
#[test]
fn fast_trim_stays_inside_source() {
    let mut e = editor();
    e.apply(EditCmd::AddClip { asset_id: "a".into(), start_us: None, track_id: None }, None).unwrap();
    let id = e.project.tracks[0].clips[0].id.clone();
    e.apply(update_speed(&id, 10.0), None).unwrap();
    let _ = e.apply(
        EditCmd::TrimClip { clip_id: id, start_us: 0, duration_us: 33_334, source_in_us: Some(4_966_666) },
        None,
    );
    invariants(&e.project).unwrap();
}
#[test]
fn a_whole_clip_changes_from_one_speed_to_another() {
    let mut p = Project::new("speeds");
    p.assets = vec![asset("a", 17_812_000)];
    let mut e = Editor::new(p);
    e.apply(EditCmd::AddClip { asset_id: "a".into(), start_us: None, track_id: None }, None).unwrap();
    let id = e.project.tracks[0].clips[0].id.clone();
    e.apply(update_speed(&id, 1.25), None).unwrap();
    e.apply(update_speed(&id, 1.5), None).unwrap();
    invariants(&e.project).unwrap();
    // Half an hour at 1.97x lands a float error above the end of the file.
    let mut p = Project::new("speeds");
    p.assets = vec![asset("a", 1_829_517_000)];
    let mut e = Editor::new(p);
    e.apply(EditCmd::AddClip { asset_id: "a".into(), start_us: None, track_id: None }, None).unwrap();
    let id = e.project.tracks[0].clips[0].id.clone();
    e.apply(update_speed(&id, 1.97), None).unwrap();
    invariants(&e.project).unwrap();
}
#[test]
fn equal_start_captions_do_not_overlap() {
    let mut e = editor();
    let style = TextStyle {
        font_family: None,
        font_size: 40.0,
        color: "#fff".into(),
        bold: false,
        font_weight: None,
        stroke_width: 0.0,
        stroke_color: "#000".into(),
        background: None,
        max_width: None,
        highlight: None,
        keywords: None,
        shadow: None,
        letter_case: Default::default(),
        letter_spacing: 0.0,
        line_spacing: 0.0,
        align: TextAlign::Center,
        box_radius: BOX_RADIUS,
    };
    let _ = e.apply(
        EditCmd::AddCaptions {
            segments: vec![
                CaptionSegment { start_us: 0, end_us: 1_000_000, text: "first".into(), words: Vec::new() },
                CaptionSegment { start_us: 0, end_us: 2_000_000, text: "second".into(), words: Vec::new() },
            ],
            style,
            anim_in: None,
            anim_out: None,
            transform: None,
        },
        None,
    );
    invariants(&e.project).unwrap();
}
#[test]
fn replacing_media_keeps_everything_but_the_source() {
    let mut e = editor();
    let mut image = asset("still", 0);
    image.kind = AssetKind::Image;
    image.has_audio = false;
    let mut silent = asset("silent", 9_000_000);
    silent.has_audio = false;
    let mut music = asset("music", 9_000_000);
    music.kind = AssetKind::Audio;
    let mut chosen = asset("chosen", 20_000_000);
    chosen.part = Some(Part { start_us: 12_000_000, end_us: 16_000_000 });
    e.project.assets.extend([image, silent, music, chosen]);
    e.apply(EditCmd::AddClip { asset_id: "a".into(), start_us: None, track_id: None }, None).unwrap();
    let id = e.project.tracks[0].clips[0].id.clone();
    e.apply(
        EditCmd::TrimClip { clip_id: id.clone(), start_us: 0, duration_us: 4_000_000, source_in_us: Some(1_000_000) },
        None,
    )
    .unwrap();
    e.apply(update_speed(&id, 2.0), None).unwrap();
    let mut styled = update_speed(&id, 2.0);
    if let EditCmd::UpdateClip { speed, transform, clean_voice, normalize, .. } = &mut styled {
        *speed = None;
        *transform = Some(Transform { x: 0.2, y: -0.1, scale: 0.6, ..Transform::default() });
        *clean_voice = Some(true);
        *normalize = Some(true);
    }
    e.apply(styled, None).unwrap();
    let before = e.project.tracks[0].clips[0].clone();
    assert_eq!(before.duration_us, 2_000_000, "4 s at 2x play in 2 s");

    let replace = |e: &mut Editor, asset: &str| {
        e.apply(EditCmd::ReplaceClip { clip_id: id.clone(), asset_id: asset.into() }, None)
    };
    let project = e.project.clone();
    let err = replace(&mut e, "b").map(|_| ()).unwrap_err().to_string();
    assert!(err.starts_with("This media is 3.0 s and the clip needs 4.0 s at 2x."), "{err}");
    assert_eq!(e.project, project, "too short media changes nothing");
    let err = replace(&mut e, "music").map(|_| ()).unwrap_err().to_string();
    assert!(err.starts_with("Sound can't go in a video clip"), "{err}");

    replace(&mut e, "silent").unwrap();
    let after = e.project.tracks[0].clips[0].clone();
    let ClipContent::Media { asset_id, source_in_us, clean_voice, normalize, .. } = &after.content else { panic!() };
    assert_eq!((asset_id.as_str(), *source_in_us, *clean_voice, *normalize), ("silent", 0, false, false));
    let mut expected = before.clone();
    if let ClipContent::Media { asset_id, source_in_us, clean_voice, normalize, .. } = &mut expected.content {
        *asset_id = "silent".into();
        *source_in_us = 0;
        *clean_voice = false;
        *normalize = false;
    }
    assert_eq!(after, expected, "only the media and its sound settings change");
    assert!(e.undo());
    assert_eq!(e.project.tracks[0].clips[0], before);

    replace(&mut e, "chosen").unwrap();
    let ClipContent::Media { source_in_us, .. } = e.project.tracks[0].clips[0].content else { panic!() };
    assert_eq!(source_in_us, 12_000_000, "starts at the part the user chose");
    replace(&mut e, "still").unwrap();
    assert_eq!(e.project.tracks[0].clips[0].duration_us, 2_000_000);
    invariants(&e.project).unwrap();
}
