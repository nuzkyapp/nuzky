use nuzky_engine::{
    Renderer, Wait,
    edit::{CaptionSegment, EditCmd},
    model::*,
};

#[test]
fn keyword_mark_without_a_pick_draws_only_that_word_in_default_yellow() {
    let mut project = Project::new("Marked word");
    project.canvas.width = 640;
    project.canvas.height = 360;
    let words = ["Kůň", "úpěl", "ódy"]
        .into_iter()
        .enumerate()
        .map(|(i, text)| CaptionWord {
            text: text.into(),
            start_us: i as i64 * 400_000,
            end_us: (i + 1) as i64 * 400_000,
            key: false,
            marked: None,
        })
        .collect();
    let style =
        serde_json::from_value(serde_json::json!({"fontSize":64.0,"color":"#ffffff","strokeWidth":0.0})).unwrap();
    let id = project
        .apply(EditCmd::AddCaptions {
            segments: vec![CaptionSegment { start_us: 0, end_us: 1_500_000, text: "Kůň úpěl ódy".into(), words }],
            style,
            anim_in: None,
            anim_out: None,
            transform: None,
        })
        .unwrap()
        .select[0]
        .clone();
    let mut renderer = Renderer::new().unwrap();
    let plain = renderer.render(&project, 500_000, 640, 360, Wait::Exact, false).unwrap();
    project.apply(EditCmd::MarkKeyWord { clip_id: id, word: 1, marked: Some(true) }).unwrap();
    let marked = renderer.render(&project, 500_000, 640, 360, Wait::Exact, false).unwrap();
    let yellow: Vec<_> =
        marked.chunks_exact(4).enumerate().filter(|(_, p)| p[..3] == [255, 212, 0]).map(|(i, _)| i).collect();
    assert!(yellow.len() > 50, "no #ffd400 fill: {} pixels", yellow.len());
    let (left, right) = (yellow.iter().map(|i| i % 640).min().unwrap(), yellow.iter().map(|i| i % 640).max().unwrap());
    let mut white = [0; 3];
    for (i, (before, after)) in plain.chunks_exact(4).zip(marked.chunks_exact(4)).enumerate() {
        if before[..3] != [255, 255, 255] {
            continue;
        }
        let x = i % 640;
        let part = if x < left {
            0
        } else if x > right {
            2
        } else {
            1
        };
        white[part] += 1;
        assert_eq!(&after[..3], if part == 1 { &[255, 212, 0] } else { &[255, 255, 255] }, "pixel {i}, word {part}");
    }
    assert!(white.iter().all(|&n| n > 50), "each word has visible fill: {white:?}");
}
