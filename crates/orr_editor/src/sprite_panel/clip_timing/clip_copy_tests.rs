use super::*;

fn custom_source() -> SpriteSource {
    let mut source: SpriteSource = serde_json::from_str(include_str!(
        "../../../../../assets/sprite_demo/sprites.json"
    ))
    .unwrap();
    source.clips = vec![ClipSource {
        id: "sequence".into(),
        mode: PlaybackMode::Once,
        frames: vec![
            Frame {
                region: 21,
                duration_ms: 7,
            },
            Frame {
                region: 10,
                duration_ms: 13,
            },
            Frame {
                region: 21,
                duration_ms: 29,
            },
        ],
    }];
    source
}

#[test]
fn clip_copy_preserves_literal_pairs_mode_and_unique_id_then_exports_new_clip() {
    let source = custom_source();
    let (dir, _, mut panel, editor) = panel_fixture_document(true, Some(source.clone()));
    let checksum = editor.checksum();
    let history = editor.history().entries.len();
    let bindings = panel.bindings.as_ref().unwrap().document().clone();
    panel.begin_clip_sequence(&editor).unwrap();
    let draft = panel.timing.draft.as_ref().unwrap();
    assert_eq!(
        draft.kind,
        DraftKind::Sequence {
            regions: vec![21, 10, 21],
            selected: 0
        }
    );
    assert_eq!(draft.durations, ["7", "13", "29"]);
    assert_eq!(draft.mode, PlaybackMode::Once);
    assert_eq!(draft.id, "sequence-2");
    assert_eq!(panel.timing.sequence_inputs.id, "sequence-2");
    assert_eq!(panel.timing.sequence_inputs.duration, "7");
    assert_eq!(panel.timing.sequence_inputs.region, 21);
    panel.edit_sequence(&editor, SequenceEdit::Reverse).unwrap();
    panel.preview_timing(&editor, 0.0).unwrap();
    let draft = panel.timing.draft.as_mut().unwrap();
    let candidate = draft.preview.as_ref().unwrap();
    let clip = candidate.clip("sequence-2").unwrap();
    for (time, index, finished) in [
        (0, 0, false),
        (28, 0, false),
        (29, 1, false),
        (41, 1, false),
        (42, 2, false),
        (48, 2, false),
        (49, 2, true),
    ] {
        let sample = clip.sample(time);
        assert_eq!((sample.frame_index, sample.finished), (index, finished));
    }
    let output = dir.path().join("copied.json");
    draft.output = output.to_string_lossy().into_owned();
    panel.export_timing(&editor).unwrap();
    let bytes = std::fs::read(&output).unwrap();
    let mut exported: SpriteSource = serde_json::from_slice(&bytes).unwrap();
    let copied = exported.clips.pop().unwrap();
    assert_eq!(
        copied.frames,
        vec![
            Frame {
                region: 21,
                duration_ms: 29
            },
            Frame {
                region: 10,
                duration_ms: 13
            },
            Frame {
                region: 21,
                duration_ms: 7
            }
        ]
    );
    assert_eq!(exported, source);
    assert!(panel.export_timing(&editor).is_err());
    assert_eq!(std::fs::read(output).unwrap(), bytes);
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &bindings);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), history);
}

#[test]
fn clip_copy_budget_refusal_keeps_previous_draft_and_inputs() {
    for clip_limit in [true, false] {
        let mut source = custom_source();
        if clip_limit {
            for i in 1..MAX_CLIPS {
                source.clips.push(ClipSource {
                    id: format!("c{i}"),
                    mode: PlaybackMode::Loop,
                    frames: vec![Frame {
                        region: 10,
                        duration_ms: 1,
                    }],
                });
            }
        } else {
            source.clips[0].frames = vec![
                Frame {
                    region: 10,
                    duration_ms: 1
                };
                MAX_FRAMES_PER_CLIP
            ];
            for i in 1..4 {
                source.clips.push(ClipSource {
                    id: format!("c{i}"),
                    mode: PlaybackMode::Loop,
                    frames: source.clips[0].frames.clone(),
                });
            }
        }
        let (_dir, _, mut panel, editor) = panel_fixture_document(true, Some(source));
        panel.begin_timing(&editor).unwrap();
        panel.timing.sequence_inputs.id = "pending".into();
        let original = panel.timing.draft.as_ref().unwrap().original_json.clone();
        assert!(panel.begin_clip_sequence(&editor).is_err());
        assert!(matches!(
            panel.timing.draft.as_ref().unwrap().kind,
            DraftKind::Timing { .. }
        ));
        assert_eq!(panel.timing.draft.as_ref().unwrap().original_json, original);
        assert_eq!(panel.timing.sequence_inputs.id, "pending");
    }
}

#[test]
fn clip_copy_source_guards_and_failed_begin_keep_existing_sequence() {
    let (dir, _, mut panel, mut editor) = panel_fixture();
    panel.begin_clip_sequence(&editor).unwrap();
    let before = draft_snapshot(panel.timing.draft.as_ref().unwrap());
    panel.source = Source::Clip("missing".into());
    assert!(panel.begin_clip_sequence(&editor).is_err());
    panel.source = Source::Region(10);
    assert!(panel.begin_clip_sequence(&editor).is_err());
    panel.source = Source::Clip("idle".into());
    panel.tilemap_pinned = true;
    assert!(panel.begin_clip_sequence(&editor).is_err());
    panel.tilemap_pinned = false;
    editor.play();
    editor.sync();
    assert!(panel.begin_clip_sequence(&editor).is_err());
    assert_eq!(draft_snapshot(panel.timing.draft.as_ref().unwrap()), before);
    editor.stop();
    editor.sync();
    let manager = sprite_bindings::open_project(dir.path()).unwrap();
    let lock = manager.verify().unwrap();
    let image = dir
        .path()
        .join(".orr/packages/objects")
        .join(&lock.packages["sample-sprites"].digest)
        .join("lantern_keeper.png");
    std::fs::write(image, b"changed atlas").unwrap();
    assert!(panel.begin_clip_sequence(&editor).is_err());
    assert_eq!(draft_snapshot(panel.timing.draft.as_ref().unwrap()), before);
}

#[test]
fn clip_copy_actual_widget_edits_and_exports_runtime_consumable_sequence() {
    use egui::accesskit::Role;
    use egui_kittest::{kittest::Queryable, Harness};
    let (dir, _, panel, editor) = panel_fixture();
    let checksum = editor.checksum();
    let history = editor.history().entries.len();
    let mut harness = Harness::builder()
        .with_size([640.0, 1000.0])
        .build_ui_state(
            |ui, state: &mut (SpritePanel, Editor)| state.0.show_timing(ui, &state.1),
            (panel, editor),
        );
    harness.run_steps(3);
    harness
        .get_by_role_and_label(Role::Button, "Copy clip to sequence")
        .click();
    harness.run_steps(3);
    assert!(harness.state().0.error.is_none());
    assert_eq!(
        harness.state().0.timing.draft.as_ref().unwrap().durations,
        ["600", "400"]
    );
    let output = dir.path().join("widget-copy.json");
    harness.state_mut().0.timing.draft.as_mut().unwrap().output =
        output.to_string_lossy().into_owned();
    for label in [
        "Reverse clip sequence",
        "Preview clip sequence",
        "Export clip sequence JSON",
    ] {
        harness
            .get_by_role_and_label(Role::Button, label)
            .scroll_to_me();
        harness.run_steps(3);
        harness.get_by_role_and_label(Role::Button, label).click();
        harness.run_steps(3);
        assert!(
            harness.state().0.error.is_none(),
            "{:?}",
            harness.state().0.error
        );
    }
    let document = SpriteDocument::from_json(&std::fs::read_to_string(output).unwrap()).unwrap();
    let clip = document.clip("sequence").unwrap();
    for (time, region) in [(0, 11), (399, 11), (400, 10), (999, 10), (1000, 11)] {
        assert_eq!(clip.sample(time).region, region);
    }
    assert_eq!(harness.state().1.checksum(), checksum);
    assert_eq!(harness.state().1.history().entries.len(), history);
}
