use super::*;
fn draft() -> Draft {
    let original = SpriteSource {
        format: orr_sprite::FORMAT.into(),
        version: orr_sprite::VERSION,
        atlas: orr_sprite::Atlas {
            image: "atlas.png".into(),
            width: 4,
            height: 2,
        },
        regions: vec![
            orr_sprite::Region {
                id: 7,
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
            orr_sprite::Region {
                id: 9,
                x: 2,
                y: 0,
                width: 2,
                height: 2,
            },
        ],
        clips: vec![orr_sprite::ClipSource {
            id: "source".into(),
            mode: PlaybackMode::Once,
            frames: vec![
                orr_sprite::Frame {
                    region: 7,
                    duration_ms: 10,
                },
                orr_sprite::Frame {
                    region: 9,
                    duration_ms: 20,
                },
                orr_sprite::Frame {
                    region: 7,
                    duration_ms: 30,
                },
            ],
        }],
    };
    let original_json = SpriteDocument::new(original.clone())
        .unwrap()
        .to_json()
        .unwrap();
    Draft {
        pin: Pin {
            epoch: 0,
            scene: None,
            key: ("p".into(), "d".into()),
            source: Source::Clip("source".into()),
            binding_path: "v.json".into(),
            project: ".".into(),
            texture: egui::TextureId::Managed(0),
        },
        original,
        original_json,
        kind: DraftKind::Timing { clip_index: 0 },
        id: "variant".into(),
        durations: vec!["5".into(), "7".into(), "11".into()],
        mode: PlaybackMode::Loop,
        output: String::new(),
        preview: None,
        playing: false,
        started: 0.0,
        elapsed: 0,
    }
}
#[test]
fn clone_preserves_complete_original_and_exact_frame_order() {
    let draft = draft();
    let original = draft.original.clone();
    let candidate = draft.candidate().unwrap();
    let mut source: SpriteSource = serde_json::from_str(&candidate.to_json().unwrap()).unwrap();
    let variant = source.clips.pop().unwrap();
    assert_eq!(source, original);
    assert_eq!(draft.original, original);
    assert_eq!(
        variant.frames.iter().map(|f| f.region).collect::<Vec<_>>(),
        vec![7, 9, 7]
    );
    assert_eq!(
        variant
            .frames
            .iter()
            .map(|f| f.duration_ms)
            .collect::<Vec<_>>(),
        vec![5, 7, 11]
    );
}
#[test]
fn exact_half_open_boundaries_repeat_regions_and_modes() {
    let mut draft = draft();
    let d = draft.candidate().unwrap();
    let clip = d.clip("variant").unwrap();
    for (t, i) in [
        (0, 0),
        (4, 0),
        (5, 1),
        (11, 1),
        (12, 2),
        (22, 2),
        (23, 0),
        (28, 1),
    ] {
        assert_eq!(clip.sample(t).frame_index, i);
        assert!(!clip.sample(t).finished);
    }
    draft.mode = PlaybackMode::Once;
    let d = draft.candidate().unwrap();
    let clip = d.clip("variant").unwrap();
    assert!(!clip.sample(22).finished);
    for t in [23, 24, u64::MAX] {
        assert_eq!(clip.sample(t).frame_index, 2);
        assert!(clip.sample(t).finished);
    }
}
#[test]
fn rejects_collisions_id_byte_limits_zero_decimal_and_overflow_atomically() {
    let mut draft = draft();
    let before = draft.original_json.clone();
    for id in [
        "".into(),
        " ".into(),
        "source".into(),
        "a".repeat(129),
        "é".repeat(65),
    ] {
        draft.id = id;
        assert!(draft.candidate().is_err());
    }
    draft.id = "a".repeat(128);
    assert!(draft.candidate().is_ok());
    for value in [
        "0",
        "-1",
        "+1",
        " 1",
        "1.0",
        "",
        "18446744073709551616",
        "000000000000000000001",
        "18446744073709551615",
    ] {
        draft.durations[0] = value.into();
        assert!(draft.candidate().is_err(), "{value}");
    }
    assert_eq!(draft.original_json, before);
    assert_eq!(
        SpriteDocument::new(draft.original.clone())
            .unwrap()
            .to_json()
            .unwrap(),
        before
    );
}
#[test]
fn full_document_limits_apply_to_added_clip() {
    let mut draft = draft();
    let template = draft.original.clips[0].clone();
    for i in 1..orr_sprite::MAX_CLIPS {
        let mut c = template.clone();
        c.id = format!("c{i}");
        draft.original.clips.push(c);
    }
    assert!(draft.candidate().is_err());
    draft.original.clips.truncate(1);
    draft.durations.pop();
    assert!(draft.candidate().is_err());
}

fn panel_fixture() -> (tempfile::TempDir, egui::Context, SpritePanel, Editor) {
    panel_fixture_with_clips(true)
}
fn panel_fixture_with_clips(
    clips: bool,
) -> (tempfile::TempDir, egui::Context, SpritePanel, Editor) {
    panel_fixture_document(clips, None)
}
fn panel_fixture_document(
    clips: bool,
    replacement: Option<SpriteSource>,
) -> (tempfile::TempDir, egui::Context, SpritePanel, Editor) {
    let dir = tempfile::tempdir().unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/sprite_demo")
        .canonicalize()
        .unwrap();
    let source = if clips && replacement.is_none() {
        source
    } else {
        let copy = dir.path().join("clipless-source");
        std::fs::create_dir(&copy).unwrap();
        for name in [
            "orr.package.json",
            "sprites.json",
            "lantern_keeper.png",
            "lantern_keeper.rgba",
            "LICENSE.txt",
        ] {
            std::fs::copy(source.join(name), copy.join(name)).unwrap();
        }
        let mut document: SpriteSource =
            serde_json::from_slice(&std::fs::read(copy.join("sprites.json")).unwrap()).unwrap();
        if let Some(replacement) = replacement {
            document = replacement;
        } else {
            document.clips.clear();
        }
        std::fs::write(
            copy.join("sprites.json"),
            SpriteDocument::new(document).unwrap().to_json().unwrap(),
        )
        .unwrap();
        copy
    };
    orr_package::Project::open_for_install(
        dir.path(),
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[source])
    .unwrap();
    let scene = dir.path().join("arena.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../../../scenes/arena_blank.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Arena).unwrap();
    editor.sync();
    assert!(editor.spawn_arena_player(0, [0.0, 0.0]));
    editor.sync();
    let guid = editor.selected_guid().unwrap();
    assert!(editor.save());
    editor.sync();
    let mut panel = SpritePanel {
        bindings: Some(
            Bindings::create(
                dir.path().join("arena.sprites.json"),
                "arena.yaml".into(),
                ".".into(),
            )
            .unwrap(),
        ),
        package: "sample-sprites".into(),
        document: "sprites.json".into(),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    panel.load(&ctx, "sample-sprites", "sprites.json").unwrap();
    panel.source = if clips {
        Source::Clip(
            panel.loaded.values().next().unwrap().asset.document.clips()[0]
                .id()
                .into(),
        )
    } else {
        Source::Region(10)
    };
    panel
        .bindings
        .as_mut()
        .unwrap()
        .assign(
            std::slice::from_ref(&guid),
            Some(Binding {
                package: panel.package.clone(),
                document: panel.document.clone(),
                source: panel.source.clone(),
                units_per_pixel: 0.05,
                orientation: None,
            }),
        )
        .unwrap();
    panel.bindings.as_mut().unwrap().save().unwrap();
    (dir, ctx, panel, editor)
}
#[test]
fn real_panel_begin_guards_invalid_export_and_no_state_mutation() {
    let (dir, _ctx, mut panel, editor) = panel_fixture();
    let original_bindings = panel.bindings.as_ref().unwrap().document().clone();
    let dirty = panel.bindings.as_ref().unwrap().dirty();
    let checksum = editor.checksum();
    let history = editor.history().entries.len();
    panel.begin_timing(&editor).unwrap();
    let draft = panel.timing.draft.as_mut().unwrap();
    draft.id = "private-variant".into();
    draft.output = dir
        .path()
        .join("variant.json")
        .to_string_lossy()
        .into_owned();
    draft.durations[0] = "0".into();
    assert!(panel.export_timing(&editor).is_err());
    assert!(!dir.path().join("variant.json").exists());
    assert!(panel.timing.draft.is_some());
    assert_eq!(
        panel.bindings.as_ref().unwrap().document(),
        &original_bindings
    );
    assert_eq!(panel.bindings.as_ref().unwrap().dirty(), dirty);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), history);
    panel.source = Source::Region(10);
    assert!(panel.begin_timing(&editor).is_err());
    panel.update_timing(&editor);
    assert!(panel.timing.draft.is_none());
    panel.source = Source::Locomotion {
        idle: "idle".into(),
        walk: "walk".into(),
    };
    assert!(panel.begin_timing(&editor).is_err());
    panel.tilemap_pinned = true;
    assert!(panel.begin_timing(&editor).is_err());
}
#[test]
fn reload_same_path_texture_and_project_context_revoke_private_draft() {
    let (_dir, ctx, mut panel, mut editor) = panel_fixture();
    panel.begin_timing(&editor).unwrap();
    panel.reload(&ctx).unwrap();
    panel.update_timing(&editor);
    assert!(panel.timing.draft.is_none());
    panel.begin_timing(&editor).unwrap();
    let scene = PathBuf::from(editor.sim().scene_path.as_ref().unwrap());
    assert!(editor.open_path(&scene));
    editor.sync();
    panel.update_timing(&editor);
    assert!(panel.timing.draft.is_none());
    panel.begin_timing(&editor).unwrap();
    panel.bindings.as_mut().unwrap().path = scene.with_extension("different.json");
    panel.update_timing(&editor);
    assert!(panel.timing.draft.is_none());
}
#[test]
fn play_pauses_preview_without_binding_or_draft_writes() {
    let (_dir, _ctx, mut panel, mut editor) = panel_fixture();
    panel.begin_timing(&editor).unwrap();
    panel.timing.draft.as_mut().unwrap().playing = true;
    let bindings = panel.bindings.as_ref().unwrap().document().clone();
    editor.play();
    editor.sync();
    panel.update_timing(&editor);
    assert!(!panel.timing.draft.as_ref().unwrap().playing);
    assert!(panel.export_timing(&editor).is_err());
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &bindings);
}
#[test]
fn installed_source_tamper_rejects_export_without_consuming_draft_or_cache() {
    let (dir, _ctx, mut panel, editor) = panel_fixture();
    panel.begin_timing(&editor).unwrap();
    let key = (panel.package.clone(), panel.document.clone());
    let texture = panel.loaded[&key].texture.id();
    let cached_json = panel.loaded[&key].asset.document.to_json().unwrap();
    let cached_pixels = panel.loaded[&key].asset.rgba.clone();
    let bindings = panel.bindings.as_ref().unwrap().document().clone();
    let dirty = panel.bindings.as_ref().unwrap().dirty();
    let checksum = editor.checksum();
    let history = editor.history().entries.len();
    let draft = panel.timing.draft.as_mut().unwrap();
    draft.id = "private-variant".into();
    draft.output = dir
        .path()
        .join("variant.json")
        .to_string_lossy()
        .into_owned();
    let original_durations = draft.durations.clone();
    let manager = sprite_bindings::open_project(dir.path()).unwrap();
    let lock = manager.verify().unwrap();
    let image = dir
        .path()
        .join(".orr/packages/objects")
        .join(&lock.packages["sample-sprites"].digest)
        .join("lantern_keeper.png");
    std::fs::write(image, b"corrupt installed atlas").unwrap();
    assert!(panel.export_timing(&editor).is_err());
    assert!(!dir.path().join("variant.json").exists());
    assert_eq!(
        panel.timing.draft.as_ref().unwrap().durations,
        original_durations
    );
    assert_eq!(panel.timing.draft.as_ref().unwrap().id, "private-variant");
    assert_eq!(panel.loaded[&key].texture.id(), texture);
    assert_eq!(
        panel.loaded[&key].asset.document.to_json().unwrap(),
        cached_json
    );
    assert_eq!(panel.loaded[&key].asset.rgba, cached_pixels);
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &bindings);
    assert_eq!(panel.bindings.as_ref().unwrap().dirty(), dirty);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), history);
}

fn sequence_draft() -> Draft {
    let mut draft = draft();
    draft.kind = DraftKind::Sequence {
        regions: vec![9],
        selected: 0,
    };
    draft.pin.source = Source::Region(9);
    draft.durations = vec!["100".into()];
    draft
}
fn draft_snapshot(
    draft: &Draft,
) -> (
    DraftKind,
    String,
    Vec<String>,
    PlaybackMode,
    Option<String>,
    bool,
    u64,
    u64,
) {
    (
        draft.kind.clone(),
        draft.id.clone(),
        draft.durations.clone(),
        draft.mode,
        draft.preview.as_ref().map(|p| p.to_json().unwrap()),
        draft.playing,
        draft.started.to_bits(),
        draft.elapsed,
    )
}
fn prime_sequence_preview(draft: &mut Draft) {
    draft.preview = Some(draft.candidate().unwrap());
    draft.playing = true;
    draft.started = 12.5;
    draft.elapsed = 42;
}
fn sequence_frames(draft: &Draft) -> Vec<Frame> {
    let source: SpriteSource =
        serde_json::from_str(&draft.candidate().unwrap().to_json().unwrap()).unwrap();
    source.clips.last().unwrap().frames.clone()
}
#[test]
fn sequence_builds_noncontiguous_repeated_regions_with_independent_half_open_samples() {
    let mut draft = sequence_draft();
    let original = draft.original.clone();
    draft
        .edit_sequence(SequenceEdit::Duration("5".into()))
        .unwrap();
    for (region, duration) in [(7, "7"), (9, "11"), (7, "13")] {
        draft.edit_sequence(SequenceEdit::Append(region)).unwrap();
        draft
            .edit_sequence(SequenceEdit::Duration(duration.into()))
            .unwrap();
    }
    let document = draft.candidate().unwrap();
    let clip = document.clip("variant").unwrap();
    for (time, index, region) in [
        (0, 0, 9),
        (4, 0, 9),
        (5, 1, 7),
        (11, 1, 7),
        (12, 2, 9),
        (22, 2, 9),
        (23, 3, 7),
        (35, 3, 7),
        (36, 0, 9),
        (41, 1, 7),
        (72, 0, 9),
    ] {
        let sample = clip.sample(time);
        assert_eq!(
            (sample.frame_index, sample.region, sample.finished),
            (index, region, false)
        );
    }
    draft
        .edit_sequence(SequenceEdit::Mode(PlaybackMode::Once))
        .unwrap();
    let document = draft.candidate().unwrap();
    let clip = document.clip("variant").unwrap();
    assert!(!clip.sample(35).finished);
    for time in [36, 37, u64::MAX] {
        let sample = clip.sample(time);
        assert_eq!(
            (sample.frame_index, sample.region, sample.finished),
            (3, 7, true)
        );
    }
    let mut source: SpriteSource = serde_json::from_str(&document.to_json().unwrap()).unwrap();
    source.clips.pop();
    assert_eq!(source, original);
    assert_eq!(draft.original, original);
}
#[test]
fn sequence_remove_reorder_keep_region_duration_pairs_and_invalidate_only_real_edits() {
    let mut draft = sequence_draft();
    prime_sequence_preview(&mut draft);
    let before = draft_snapshot(&draft);
    for edit in [
        SequenceEdit::MoveUp,
        SequenceEdit::MoveDown,
        SequenceEdit::Reverse,
        SequenceEdit::PingPong,
        SequenceEdit::Id("variant".into()),
        SequenceEdit::Duration("100".into()),
        SequenceEdit::Mode(PlaybackMode::Loop),
    ] {
        assert!(!draft.edit_sequence(edit).unwrap());
        assert_eq!(draft_snapshot(&draft), before);
    }
    assert!(draft.edit_sequence(SequenceEdit::Append(7)).unwrap());
    assert!(draft.preview.is_none());
    assert!(!draft.playing);
    assert_eq!((draft.started, draft.elapsed), (0.0, 0));
    draft
        .edit_sequence(SequenceEdit::Duration("17".into()))
        .unwrap();
    prime_sequence_preview(&mut draft);
    assert!(draft.edit_sequence(SequenceEdit::MoveUp).unwrap());
    assert_eq!(
        sequence_frames(&draft),
        vec![
            Frame {
                region: 7,
                duration_ms: 17
            },
            Frame {
                region: 9,
                duration_ms: 100
            }
        ]
    );
    assert!(draft.preview.is_none());
    draft.edit_sequence(SequenceEdit::MoveDown).unwrap();
    assert_eq!(
        sequence_frames(&draft),
        vec![
            Frame {
                region: 9,
                duration_ms: 100
            },
            Frame {
                region: 7,
                duration_ms: 17
            }
        ]
    );
    prime_sequence_preview(&mut draft);
    let before = draft_snapshot(&draft);
    assert!(!draft.edit_sequence(SequenceEdit::MoveDown).unwrap());
    assert_eq!(draft_snapshot(&draft), before);
    assert!(draft.edit_sequence(SequenceEdit::Remove).unwrap());
    assert_eq!(
        sequence_frames(&draft),
        vec![Frame {
            region: 9,
            duration_ms: 100
        }]
    );
    assert_eq!(
        draft.kind,
        DraftKind::Sequence {
            regions: vec![9],
            selected: 0
        }
    );
}
#[test]
fn sequence_invalid_id_duration_region_and_last_removal_are_failure_atomic() {
    let mut draft = sequence_draft();
    prime_sequence_preview(&mut draft);
    let before = draft_snapshot(&draft);
    let original = draft.original_json.clone();
    for id in [
        "".into(),
        " ".into(),
        "source".into(),
        "a".repeat(129),
        "é".repeat(65),
    ] {
        assert!(draft.edit_sequence(SequenceEdit::Id(id)).is_err());
        assert_eq!(draft_snapshot(&draft), before);
    }
    for duration in [
        "",
        "0",
        "-1",
        "+1",
        " 1",
        "1.0",
        "18446744073709551616",
        "000000000000000000001",
    ] {
        assert!(
            draft
                .edit_sequence(SequenceEdit::Duration(duration.into()))
                .is_err(),
            "{duration}"
        );
        assert_eq!(draft_snapshot(&draft), before);
    }
    for edit in [SequenceEdit::Append(8), SequenceEdit::Remove] {
        assert!(draft.edit_sequence(edit).is_err());
        assert_eq!(draft_snapshot(&draft), before);
    }
    assert_eq!(draft.original_json, original);
    draft
        .edit_sequence(SequenceEdit::Id("é".repeat(64)))
        .unwrap();
    draft
        .edit_sequence(SequenceEdit::Duration(u64::MAX.to_string()))
        .unwrap();
    prime_sequence_preview(&mut draft);
    let before = draft_snapshot(&draft);
    assert!(draft.edit_sequence(SequenceEdit::Append(7)).is_err());
    assert_eq!(draft_snapshot(&draft), before);
    draft
        .edit_sequence(SequenceEdit::Duration("1".into()))
        .unwrap();
    draft.edit_sequence(SequenceEdit::Append(7)).unwrap();
    prime_sequence_preview(&mut draft);
    let before = draft_snapshot(&draft);
    assert!(draft
        .edit_sequence(SequenceEdit::Duration(u64::MAX.to_string()))
        .is_err());
    assert_eq!(draft_snapshot(&draft), before);
}
#[test]
fn sequence_enforces_clip_frame_and_aggregate_bounds_before_commit() {
    let mut draft = sequence_draft();
    draft.kind = DraftKind::Sequence {
        regions: vec![9; MAX_FRAMES_PER_CLIP],
        selected: 0,
    };
    draft.durations = vec!["1".into(); MAX_FRAMES_PER_CLIP];
    prime_sequence_preview(&mut draft);
    let before = draft_snapshot(&draft);
    assert!(draft
        .edit_sequence(SequenceEdit::Append(7))
        .unwrap_err()
        .contains("4096"));
    assert_eq!(draft_snapshot(&draft), before);
    let mut draft = sequence_draft();
    let template = draft.original.clips[0].clone();
    draft.original.clips = (0..MAX_CLIPS - 1)
        .map(|i| ClipSource {
            id: format!("clip{i}"),
            ..template.clone()
        })
        .collect();
    assert_eq!(draft.candidate().unwrap().clips().len(), MAX_CLIPS);
    draft.original.clips.push(template);
    let before = draft_snapshot(&draft);
    assert!(draft
        .edit_sequence(SequenceEdit::Append(7))
        .unwrap_err()
        .contains("256"));
    assert_eq!(draft_snapshot(&draft), before);
    let mut draft = sequence_draft();
    draft.original.clips = (0..4)
        .map(|i| ClipSource {
            id: format!("clip{i}"),
            mode: PlaybackMode::Loop,
            frames: vec![
                Frame {
                    region: 7,
                    duration_ms: 1
                };
                MAX_FRAMES_PER_CLIP - usize::from(i == 3)
            ],
        })
        .collect();
    prime_sequence_preview(&mut draft);
    let before = draft_snapshot(&draft);
    assert!(draft
        .edit_sequence(SequenceEdit::Append(7))
        .unwrap_err()
        .contains("16384"));
    assert_eq!(draft_snapshot(&draft), before);
}
#[test]
fn real_sequence_begins_from_zero_clip_atlas_and_keeps_all_original_state() {
    let (dir, _ctx, mut panel, editor) = panel_fixture_with_clips(false);
    let key = (panel.package.clone(), panel.document.clone());
    let original_json = panel.loaded[&key].asset.document.to_json().unwrap();
    let pixels = panel.loaded[&key].asset.rgba.clone();
    let texture = panel.loaded[&key].texture.id();
    let bindings = panel.bindings.as_ref().unwrap().document().clone();
    let dirty = panel.bindings.as_ref().unwrap().dirty();
    let checksum = editor.checksum();
    let history = editor.history().entries.len();
    panel.begin_sequence(&editor).unwrap();
    let draft = panel.timing.draft.as_ref().unwrap();
    assert!(draft.original.clips.is_empty());
    assert_eq!(draft.id, "sequence");
    assert_eq!(draft.mode, PlaybackMode::Loop);
    assert_eq!(
        draft.kind,
        DraftKind::Sequence {
            regions: vec![10],
            selected: 0
        }
    );
    assert_eq!(draft.durations, vec!["100"]);
    panel
        .edit_sequence(&editor, SequenceEdit::Append(20))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Append(10))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Id("first-clip".into()))
        .unwrap();
    let output = dir.path().join("first-clip.json");
    panel.timing.draft.as_mut().unwrap().output = output.to_string_lossy().into_owned();
    panel.export_timing(&editor).unwrap();
    let mut exported: SpriteSource =
        serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(exported.clips.pop().unwrap().id, "first-clip");
    assert_eq!(
        SpriteDocument::new(exported).unwrap().to_json().unwrap(),
        original_json
    );
    assert_eq!(
        panel.loaded[&key].asset.document.to_json().unwrap(),
        original_json
    );
    assert_eq!(panel.loaded[&key].asset.rgba, pixels);
    assert_eq!(panel.loaded[&key].texture.id(), texture);
    assert_eq!(panel.source, Source::Region(10));
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &bindings);
    assert_eq!(panel.bindings.as_ref().unwrap().dirty(), dirty);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), history);
}
#[test]
fn sequence_pending_fields_block_publication_and_failed_apply_keeps_committed_draft() {
    let (dir, _ctx, mut panel, editor) = panel_fixture_with_clips(false);
    panel.begin_sequence(&editor).unwrap();
    let output = dir.path().join("sequence.json");
    let draft = panel.timing.draft.as_mut().unwrap();
    draft.output = output.to_string_lossy().into_owned();
    prime_sequence_preview(draft);
    // This is the same immediate invalidation performed by the text widgets.
    draft.invalidate_preview();
    let before = draft_snapshot(draft);
    panel.timing.sequence_inputs.duration = "0".into();
    assert!(!panel.timing.sequence_inputs_match());
    assert!(panel
        .export_timing(&editor)
        .unwrap_err()
        .contains("Apply valid"));
    assert!(panel
        .edit_sequence(&editor, SequenceEdit::Duration("0".into()))
        .is_err());
    assert_eq!(draft_snapshot(panel.timing.draft.as_ref().unwrap()), before);
    assert_eq!(panel.timing.sequence_inputs.duration, "0");
    assert!(panel
        .edit_sequence(&editor, SequenceEdit::Append(20))
        .is_err());
    assert!(panel
        .edit_sequence(&editor, SequenceEdit::MoveDown)
        .is_err());
    assert!(panel.edit_sequence(&editor, SequenceEdit::Reverse).is_err());
    assert!(panel
        .edit_sequence(&editor, SequenceEdit::PingPong)
        .is_err());
    assert_eq!(panel.timing.sequence_inputs.duration, "0");
    assert_eq!(draft_snapshot(panel.timing.draft.as_ref().unwrap()), before);
    assert!(!output.exists());
    panel.timing.sequence_inputs.duration = "100".into();
    panel.timing.sequence_inputs.id = "".into();
    assert!(panel.export_timing(&editor).is_err());
    assert!(panel
        .edit_sequence(&editor, SequenceEdit::Id("".into()))
        .is_err());
    assert_eq!(draft_snapshot(panel.timing.draft.as_ref().unwrap()), before);
    panel
        .edit_sequence(&editor, SequenceEdit::Id("new-sequence".into()))
        .unwrap();
    assert!(panel.timing.sequence_inputs_match());
    assert!(panel.timing.draft.as_ref().unwrap().preview.is_none());
    assert_eq!(panel.timing.draft.as_ref().unwrap().elapsed, 0);
}
#[test]
fn sequence_source_epoch_texture_project_and_tilemap_changes_revoke_even_when_hidden() {
    let (_dir, ctx, mut panel, mut editor) = panel_fixture_with_clips(false);
    panel.begin_sequence(&editor).unwrap();
    panel.source = Source::Region(20);
    panel.update(&mut editor);
    assert!(panel.timing.draft.is_none());
    panel.begin_sequence(&editor).unwrap();
    let scene = PathBuf::from(editor.sim().scene_path.as_ref().unwrap());
    assert!(editor.open_path(&scene));
    editor.sync();
    panel.update(&mut editor);
    assert!(panel.timing.draft.is_none());
    panel.begin_sequence(&editor).unwrap();
    panel.reload(&ctx).unwrap();
    panel.update(&mut editor);
    assert!(panel.timing.draft.is_none());
    panel.begin_sequence(&editor).unwrap();
    panel.bindings.as_mut().unwrap().path = scene.with_extension("different.json");
    panel.update(&mut editor);
    assert!(panel.timing.draft.is_none());
    panel.begin_sequence(&editor).unwrap();
    panel.tilemap_pinned = true;
    assert!(panel
        .edit_sequence(&editor, SequenceEdit::Append(10))
        .is_err());
    assert!(panel.begin_sequence(&editor).is_err());
    panel.update(&mut editor);
    assert!(panel.timing.draft.is_none());
}
#[test]
fn sequence_rejects_wrong_source_play_and_installed_tamper_without_draft_changes() {
    let (dir, _ctx, mut panel, mut editor) = panel_fixture();
    assert!(panel.begin_sequence(&editor).is_err());
    panel.source = Source::Locomotion {
        idle: "idle".into(),
        walk: "walk".into(),
    };
    assert!(panel.begin_sequence(&editor).is_err());
    panel.source = Source::Region(9999);
    assert!(panel.begin_sequence(&editor).is_err());
    panel.source = Source::Region(10);
    panel.begin_sequence(&editor).unwrap();
    prime_sequence_preview(panel.timing.draft.as_mut().unwrap());
    let before = draft_snapshot(panel.timing.draft.as_ref().unwrap());
    editor.play();
    editor.sync();
    assert!(panel
        .edit_sequence(&editor, SequenceEdit::Append(20))
        .is_err());
    assert!(panel.export_timing(&editor).is_err());
    assert_eq!(draft_snapshot(panel.timing.draft.as_ref().unwrap()), before);
    panel.update(&mut editor);
    assert!(!panel.timing.draft.as_ref().unwrap().playing);
    assert!(editor.stop().is_some());
    editor.sync();
    // Stop acknowledges Edit before its matching view snapshot necessarily arrives.
    // In particular, sync can see the old Play frame with the same checksum.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !panel.editable(&editor) {
        assert!(editor.down().is_none(), "host disconnected while stopping");
        assert!(
            std::time::Instant::now() < deadline,
            "stopped sprite editor did not reach coherent Edit state: {:?}",
            editor.mode()
        );
        editor.pump();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panel.begin_sequence(&editor).unwrap();
    prime_sequence_preview(panel.timing.draft.as_mut().unwrap());
    let before = draft_snapshot(panel.timing.draft.as_ref().unwrap());
    let lock = sprite_bindings::open_project(dir.path())
        .unwrap()
        .verify()
        .unwrap();
    let image = dir
        .path()
        .join(".orr/packages/objects")
        .join(&lock.packages["sample-sprites"].digest)
        .join("lantern_keeper.png");
    std::fs::write(image, b"corrupt installed atlas").unwrap();
    assert!(panel
        .edit_sequence(&editor, SequenceEdit::Append(20))
        .is_err());
    assert!(panel.export_timing(&editor).is_err());
    assert_eq!(draft_snapshot(panel.timing.draft.as_ref().unwrap()), before);
}
#[test]
fn timing_begin_defaults_and_fixed_region_layout_remain_unchanged() {
    let (_dir, _ctx, mut panel, editor) = panel_fixture();
    panel.begin_timing(&editor).unwrap();
    let draft = panel.timing.draft.as_mut().unwrap();
    assert_eq!(draft.kind, DraftKind::Timing { clip_index: 0 });
    assert!(draft.id.is_empty());
    assert!(draft.output.is_empty());
    assert_eq!(draft.mode, draft.original.clips[0].mode);
    assert_eq!(
        draft.durations,
        draft.original.clips[0]
            .frames
            .iter()
            .map(|f| f.duration_ms.to_string())
            .collect::<Vec<_>>()
    );
    assert!(draft.edit_sequence(SequenceEdit::Append(10)).is_err());
    draft.id = "retimed".into();
    assert_eq!(sequence_frames(draft), draft.original.clips[0].frames);
}

#[test]
fn sequence_text_widgets_immediately_reset_preview_and_block_pending_publication() {
    use egui::accesskit::Role;
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };
    let (_dir, _ctx, mut panel, editor) = panel_fixture_with_clips(false);
    panel.begin_sequence(&editor).unwrap();
    let mut harness = Harness::builder()
        .with_size([1000.0, 1600.0])
        .build_ui_state(
            |ui, state: &mut (SpritePanel, Editor)| {
                state.0.show_timing(ui, &state.1);
            },
            (panel, editor),
        );
    harness.run_steps(2);
    for (label, value, button) in [
        ("Sequence clip ID", "", "Apply sequence ID"),
        (
            "Selected sequence frame duration ms",
            "0",
            "Apply sequence frame duration",
        ),
    ] {
        let state = harness.state_mut();
        state.0.timing.sequence_inputs.id = "sequence".into();
        state.0.timing.sequence_inputs.duration = "100".into();
        harness.run_steps(2);
        harness
            .get_by_role_and_label(Role::Button, "Preview clip sequence")
            .click();
        harness.run_steps(2);
        assert!(harness
            .state()
            .0
            .timing
            .draft
            .as_ref()
            .unwrap()
            .preview
            .is_some());
        harness
            .get_by_role_and_label(Role::TextInput, label)
            .focus();
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run_steps(2);
        if value.is_empty() {
            harness.key_press(egui::Key::Backspace);
        } else {
            harness
                .get_by_role_and_label(Role::TextInput, label)
                .type_text(value);
        }
        harness.run_steps(2);
        let draft = harness.state().0.timing.draft.as_ref().unwrap();
        assert!(draft.preview.is_none());
        assert!(!draft.playing);
        assert_eq!((draft.started, draft.elapsed), (0.0, 0));
        assert_eq!(draft.id, "sequence");
        assert_eq!(draft.durations, vec!["100"]);
        let before = draft_snapshot(draft);
        for action in [
            "Preview clip sequence",
            "Export clip sequence JSON",
            "Append sequence frame",
            "Move sequence frame up",
            "Move sequence frame down",
            "Remove sequence frame",
            "Sequence frame 0 (region 10)",
        ] {
            assert!(harness
                .get_by_role_and_label(Role::Button, action)
                .accesskit_node()
                .is_disabled());
        }
        let pending = harness.state().0.timing.sequence_inputs.duration.clone();
        harness
            .get_by_role_and_label(Role::Button, "Sequence frame 0 (region 10)")
            .click_accesskit();
        harness
            .get_by_role_and_label(Role::Button, "Append sequence frame")
            .click_accesskit();
        harness.run_steps(2);
        assert_eq!(harness.state().0.timing.sequence_inputs.duration, pending);
        harness.get_by_role_and_label(Role::Button, button).click();
        harness.run_steps(2);
        assert!(harness.state().0.error.is_some());
        assert_eq!(
            draft_snapshot(harness.state().0.timing.draft.as_ref().unwrap()),
            before
        );
    }
}

#[test]
fn sequence_preview_widget_rejects_changed_source_and_resets_stale_private_playback() {
    use egui::accesskit::Role;
    use egui_kittest::{kittest::Queryable, Harness};
    let (dir, _ctx, mut panel, editor) = panel_fixture_with_clips(false);
    let lock = sprite_bindings::open_project(dir.path())
        .unwrap()
        .verify()
        .unwrap();
    let image = dir
        .path()
        .join(".orr/packages/objects")
        .join(&lock.packages["sample-sprites"].digest)
        .join("lantern_keeper.png");
    panel.begin_sequence(&editor).unwrap();
    let mut harness = Harness::builder()
        .with_size([1000.0, 1600.0])
        .build_ui_state(
            |ui, state: &mut (SpritePanel, Editor)| {
                state.0.show_timing(ui, &state.1);
            },
            (panel, editor),
        );
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::Button, "Preview clip sequence")
        .click();
    harness.run_steps(2);
    let draft = harness.state().0.timing.draft.as_ref().unwrap();
    assert!(draft.preview.is_some());
    assert!(draft.playing);
    let committed = (
        draft.kind.clone(),
        draft.id.clone(),
        draft.durations.clone(),
        draft.mode,
        draft.original_json.clone(),
    );
    std::fs::write(image, b"corrupt installed atlas").unwrap();
    harness
        .get_by_role_and_label(Role::Button, "Preview clip sequence")
        .click();
    harness.run_steps(2);
    assert!(harness.state().0.error.is_some());
    let draft = harness.state().0.timing.draft.as_ref().unwrap();
    assert!(draft.preview.is_none());
    assert!(!draft.playing);
    assert_eq!((draft.started, draft.elapsed), (0.0, 0));
    assert_eq!(
        (
            draft.kind.clone(),
            draft.id.clone(),
            draft.durations.clone(),
            draft.mode,
            draft.original_json.clone()
        ),
        committed
    );
}

#[test]
fn sequence_bounded_inspector_scroll_reaches_preview_export_status_and_discard_by_pointer() {
    use egui::accesskit::Role;
    use egui_kittest::{kittest::Queryable, Harness};
    for zero_layout_height in [false, true] {
        let (dir, _ctx, mut panel, editor) = panel_fixture_with_clips(false);
        panel.begin_sequence(&editor).unwrap();
        for region in [20, 10, 21, 11, 20, 10, 21, 11, 20, 10, 21, 11] {
            panel
                .edit_sequence(&editor, SequenceEdit::Append(region))
                .unwrap();
        }
        let output = dir.path().join("scrolled-sequence.json");
        panel.timing.draft.as_mut().unwrap().output = output.to_string_lossy().into_owned();
        let mut harness = Harness::builder().with_size([360.0, 700.0]).build_ui_state(
            move |ui, state: &mut (SpritePanel, Editor)| {
                // A zero remaining layout height must not permanently collapse
                // the local scroll area while the viewport itself is visible.
                if zero_layout_height {
                    ui.set_max_height(0.0);
                }
                state.0.show_timing(ui, &state.1);
            },
            (panel, editor),
        );
        harness.run_steps(3);
        for label in ["Preview clip sequence", "Export clip sequence JSON"] {
            harness
                .get_by_role_and_label(Role::Button, label)
                .scroll_to_me();
            harness.run_steps(3);
            let rect = harness.get_by_role_and_label(Role::Button, label).rect();
            let screen = harness.ctx.input(|input| input.viewport_rect());
            assert!(
                screen.expand(0.5).contains_rect(rect),
                "{label}: {rect:?} outside {screen:?}"
            );
            harness.get_by_role_and_label(Role::Button, label).click();
            harness.run_steps(3);
            assert!(
                harness.state().0.error.is_none(),
                "{:?}",
                harness.state().0.error
            );
            assert!(harness
                .state()
                .0
                .timing
                .draft
                .as_ref()
                .unwrap()
                .preview
                .is_some());
        }
        assert!(output.is_file());
        let message = harness.state().0.timing.message.clone().unwrap();
        harness.get_by_label(&message).scroll_to_me();
        harness.run_steps(3);
        let screen = harness.ctx.input(|input| input.viewport_rect());
        assert!(screen
            .expand(0.5)
            .contains_rect(harness.get_by_label(&message).rect()));
        harness
            .get_by_role_and_label(Role::TextInput, "Sequence output path")
            .scroll_to_me();
        harness.run_steps(3);
        assert!(screen.expand(0.5).contains_rect(
            harness
                .get_by_role_and_label(Role::TextInput, "Sequence output path")
                .rect()
        ));
        harness
            .get_by_role_and_label(Role::Button, "Discard clip sequence")
            .scroll_to_me();
        harness.run_steps(3);
        let rect = harness
            .get_by_role_and_label(Role::Button, "Discard clip sequence")
            .rect();
        assert!(screen.expand(0.5).contains_rect(rect));
        harness
            .get_by_role_and_label(Role::Button, "Discard clip sequence")
            .click();
        harness.run_steps(3);
        assert!(harness.state().0.timing.draft.is_none());
    }
}

#[test]
fn sequence_reverse_pairs_selection_involution_and_runtime_boundaries() {
    let mut draft = sequence_draft();
    draft.kind = DraftKind::Sequence {
        regions: vec![9, 7, 7],
        selected: 0,
    };
    draft.durations = vec!["5".into(), "7".into(), "11".into()];
    let original = draft.original_json.clone();
    let before_kind = draft.kind.clone();
    let before_frames = sequence_frames(&draft);
    prime_sequence_preview(&mut draft);
    assert!(draft.edit_sequence(SequenceEdit::Reverse).unwrap());
    assert_eq!(
        draft.kind,
        DraftKind::Sequence {
            regions: vec![7, 7, 9],
            selected: 2
        }
    );
    assert_eq!(draft.durations, ["11", "7", "5"]);
    assert!(draft.preview.is_none());
    assert!(!draft.playing);
    assert_eq!((draft.started, draft.elapsed), (0.0, 0));
    let document = draft.candidate().unwrap();
    let clip = document.clip("variant").unwrap();
    for (time, index, region) in [
        (0, 0, 7),
        (10, 0, 7),
        (11, 1, 7),
        (17, 1, 7),
        (18, 2, 9),
        (22, 2, 9),
        (23, 0, 7),
    ] {
        let sample = clip.sample(time);
        assert_eq!((sample.frame_index, sample.region), (index, region));
    }
    assert!(draft.edit_sequence(SequenceEdit::Reverse).unwrap());
    assert_eq!(draft.kind, before_kind);
    assert_eq!(sequence_frames(&draft), before_frames);
    assert_eq!(draft.original_json, original);
}

#[test]
fn sequence_reverse_single_frame_is_noop_and_invalid_draft_is_atomic() {
    let mut draft = sequence_draft();
    prime_sequence_preview(&mut draft);
    let before = draft_snapshot(&draft);
    assert!(!draft.edit_sequence(SequenceEdit::Reverse).unwrap());
    assert_eq!(draft_snapshot(&draft), before);
    draft.durations[0] = "0".into();
    let before = draft_snapshot(&draft);
    assert!(draft.edit_sequence(SequenceEdit::Reverse).is_err());
    assert_eq!(draft_snapshot(&draft), before);
}

#[test]
fn sequence_reverse_widget_exports_pairs_without_scene_mutation() {
    use egui::accesskit::Role;
    use egui_kittest::{kittest::Queryable, Harness};
    let (dir, _ctx, mut panel, editor) = panel_fixture_with_clips(false);
    panel.begin_sequence(&editor).unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Duration("5".into()))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Append(20))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Duration("17".into()))
        .unwrap();
    let output = dir.path().join("reversed.json");
    panel.timing.draft.as_mut().unwrap().output = output.to_string_lossy().into_owned();
    let checksum = editor.checksum();
    let history = editor.history().entries.len();
    let bindings = panel.bindings.as_ref().unwrap().document().clone();
    let mut harness = Harness::builder().with_size([600.0, 900.0]).build_ui_state(
        |ui, state: &mut (SpritePanel, Editor)| state.0.show_timing(ui, &state.1),
        (panel, editor),
    );
    harness.run_steps(3);
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
    let source: SpriteSource = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    let frames = &source.clips.last().unwrap().frames;
    assert_eq!(
        frames,
        &[
            Frame {
                region: 20,
                duration_ms: 17
            },
            Frame {
                region: 10,
                duration_ms: 5
            }
        ]
    );
    let (panel, editor) = harness.state();
    assert_eq!(panel.timing.sequence_inputs.duration, "17");
    assert_eq!(
        panel.timing.draft.as_ref().unwrap().kind,
        DraftKind::Sequence {
            regions: vec![20, 10],
            selected: 0
        }
    );
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &bindings);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), history);
}

#[test]
fn sequence_pingpong_pairs_selection_and_runtime_boundaries() {
    let mut draft = sequence_draft();
    draft.kind = DraftKind::Sequence {
        regions: vec![9, 7, 9, 7],
        selected: 2,
    };
    draft.durations = ["5", "7", "11", "13"].map(String::from).into();
    let original = draft.original_json.clone();
    prime_sequence_preview(&mut draft);
    assert!(draft.edit_sequence(SequenceEdit::PingPong).unwrap());
    assert_eq!(
        draft.kind,
        DraftKind::Sequence {
            regions: vec![9, 7, 9, 7, 9, 7],
            selected: 2
        }
    );
    assert_eq!(draft.durations, ["5", "7", "11", "13", "11", "7"]);
    assert!(draft.preview.is_none());
    assert!(!draft.playing);
    assert_eq!((draft.started, draft.elapsed), (0.0, 0));
    let document = draft.candidate().unwrap();
    let reopened = SpriteDocument::from_json(&document.to_json().unwrap()).unwrap();
    let clip = reopened.clip("variant").unwrap();
    for (time, index, region) in [
        (0, 0, 9),
        (4, 0, 9),
        (5, 1, 7),
        (12, 2, 9),
        (23, 3, 7),
        (35, 3, 7),
        (36, 4, 9),
        (46, 4, 9),
        (47, 5, 7),
        (53, 5, 7),
        (54, 0, 9),
    ] {
        let sample = clip.sample(time);
        assert_eq!((sample.frame_index, sample.region), (index, region));
    }
    draft
        .edit_sequence(SequenceEdit::Mode(PlaybackMode::Once))
        .unwrap();
    let document = draft.candidate().unwrap();
    let sample = document.clip("variant").unwrap().sample(54);
    assert_eq!(
        (sample.frame_index, sample.region, sample.finished),
        (5, 7, true)
    );
    assert_eq!(draft.original_json, original);
    assert!(draft.edit_sequence(SequenceEdit::PingPong).unwrap());
    assert_eq!(
        draft.durations,
        ["5", "7", "11", "13", "11", "7", "11", "13", "11", "7"]
    );
}

#[test]
fn sequence_pingpong_noops_and_over_budget_are_atomic() {
    let mut draft = sequence_draft();
    for count in [1, 2] {
        draft.kind = DraftKind::Sequence {
            regions: vec![7; count],
            selected: 0,
        };
        draft.durations = vec!["5".into(); count];
        prime_sequence_preview(&mut draft);
        let before = draft_snapshot(&draft);
        assert!(!draft.edit_sequence(SequenceEdit::PingPong).unwrap());
        assert_eq!(draft_snapshot(&draft), before);
    }
    for (count, duration) in [(2050, "1"), (3, "4611686018427387904")] {
        draft.kind = DraftKind::Sequence {
            regions: vec![7; count],
            selected: 0,
        };
        draft.durations = vec![duration.into(); count];
        prime_sequence_preview(&mut draft);
        let before = draft_snapshot(&draft);
        assert!(draft.edit_sequence(SequenceEdit::PingPong).is_err());
        assert_eq!(draft_snapshot(&draft), before);
    }
    draft.kind = DraftKind::Sequence {
        regions: vec![7; 2049],
        selected: 0,
    };
    draft.durations = vec!["1".into(); 2049];
    assert!(draft.edit_sequence(SequenceEdit::PingPong).unwrap());
    assert_eq!(draft.durations.len(), MAX_FRAMES_PER_CLIP);
}
#[test]
fn sequence_pingpong_widget_exports_pairs_without_scene_mutation() {
    use egui::accesskit::Role;
    use egui_kittest::{kittest::Queryable, Harness};
    let (dir, _ctx, mut panel, editor) = panel_fixture_with_clips(false);
    panel.begin_sequence(&editor).unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Duration("5".into()))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Append(20))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Duration("17".into()))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Append(10))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Duration("23".into()))
        .unwrap();
    let output = dir.path().join("pingpong.json");
    panel.timing.draft.as_mut().unwrap().output = output.to_string_lossy().into_owned();
    let checksum = editor.checksum();
    let history = editor.history().entries.len();
    let bindings = panel.bindings.as_ref().unwrap().document().clone();
    let mut harness = Harness::builder().with_size([600.0, 900.0]).build_ui_state(
        |ui, state: &mut (SpritePanel, Editor)| state.0.show_timing(ui, &state.1),
        (panel, editor),
    );
    harness.run_steps(3);
    for label in [
        "Expand ping-pong sequence",
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
    let source: SpriteSource = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    let frames = &source.clips.last().unwrap().frames;
    assert_eq!(
        frames,
        &[
            Frame {
                region: 10,
                duration_ms: 5
            },
            Frame {
                region: 20,
                duration_ms: 17
            },
            Frame {
                region: 10,
                duration_ms: 23
            },
            Frame {
                region: 20,
                duration_ms: 17
            },
        ]
    );
    let document = SpriteDocument::new(source.clone()).unwrap();
    let clip = document.clip(&source.clips.last().unwrap().id).unwrap();
    for (time, index, region) in [
        (0, 0, 10),
        (5, 1, 20),
        (22, 2, 10),
        (45, 3, 20),
        (61, 3, 20),
        (62, 0, 10),
    ] {
        let sample = clip.sample(time);
        assert_eq!((sample.frame_index, sample.region), (index, region));
    }
    let (panel, editor) = harness.state();
    assert_eq!(panel.timing.sequence_inputs.duration, "23");
    assert_eq!(
        panel.timing.draft.as_ref().unwrap().kind,
        DraftKind::Sequence {
            regions: vec![10, 20, 10, 20],
            selected: 2
        }
    );
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &bindings);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), history);
}

#[test]
fn sequence_uniform_duration_preserves_order_and_samples_exact_boundaries() {
    let mut draft = sequence_draft();
    draft.kind = DraftKind::Sequence {
        regions: vec![9, 7, 9],
        selected: 1,
    };
    draft.durations = ["5", "7", "11"].map(String::from).into();
    let original = draft.original_json.clone();
    let kind = draft.kind.clone();
    prime_sequence_preview(&mut draft);
    assert!(draft.edit_sequence(SequenceEdit::UniformDuration).unwrap());
    assert_eq!(draft.kind, kind);
    assert_eq!(draft.durations, ["7", "7", "7"]);
    assert!(draft.preview.is_none());
    assert!(!draft.playing);
    assert_eq!((draft.started, draft.elapsed), (0.0, 0));
    for mode in [PlaybackMode::Loop, PlaybackMode::Once] {
        draft.edit_sequence(SequenceEdit::Mode(mode)).unwrap();
        let document = draft.candidate().unwrap();
        let reopened = SpriteDocument::from_json(&document.to_json().unwrap()).unwrap();
        let clip = reopened.clip("variant").unwrap();
        for (time, index, region) in [
            (0, 0, 9),
            (6, 0, 9),
            (7, 1, 7),
            (13, 1, 7),
            (14, 2, 9),
            (20, 2, 9),
        ] {
            let sample = clip.sample(time);
            assert_eq!(
                (sample.frame_index, sample.region, sample.finished),
                (index, region, false)
            );
        }
        let sample = clip.sample(21);
        assert_eq!(
            (sample.frame_index, sample.finished),
            if mode == PlaybackMode::Loop {
                (0, false)
            } else {
                (2, true)
            }
        );
    }
    assert_eq!(draft.original_json, original);
}

#[test]
fn sequence_uniform_duration_noop_and_overflow_are_atomic() {
    let mut draft = sequence_draft();
    for count in [1, 3, MAX_FRAMES_PER_CLIP] {
        draft.kind = DraftKind::Sequence {
            regions: vec![7; count],
            selected: count - 1,
        };
        draft.durations = vec!["5".into(); count];
        prime_sequence_preview(&mut draft);
        let before = draft_snapshot(&draft);
        assert!(!draft.edit_sequence(SequenceEdit::UniformDuration).unwrap());
        assert_eq!(draft_snapshot(&draft), before);
    }
    // The original total fits; replicating its selected hold does not.
    draft.kind = DraftKind::Sequence {
        regions: vec![7, 9],
        selected: 1,
    };
    draft.durations = vec!["1".into(), (u64::MAX - 1).to_string()];
    prime_sequence_preview(&mut draft);
    let before = draft_snapshot(&draft);
    assert!(draft
        .edit_sequence(SequenceEdit::UniformDuration)
        .unwrap_err()
        .contains("Total clip duration"));
    assert_eq!(draft_snapshot(&draft), before);
    for invalid in ["0", "bad", "18446744073709551616"] {
        draft.durations[1] = invalid.into();
        let before = draft_snapshot(&draft);
        assert!(draft.edit_sequence(SequenceEdit::UniformDuration).is_err());
        assert_eq!(draft_snapshot(&draft), before);
    }
}

#[test]
fn sequence_uniform_duration_pending_fields_and_stale_source_reject() {
    let (_dir, _ctx, mut panel, editor) = panel_fixture_with_clips(false);
    panel.begin_sequence(&editor).unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Append(20))
        .unwrap();
    for (id, duration) in [("pending", "100"), ("sequence", "invalid")] {
        panel.timing.sequence_inputs.id = id.into();
        panel.timing.sequence_inputs.duration = duration.into();
        let before = draft_snapshot(panel.timing.draft.as_ref().unwrap());
        assert!(panel
            .edit_sequence(&editor, SequenceEdit::UniformDuration)
            .is_err());
        assert_eq!(draft_snapshot(panel.timing.draft.as_ref().unwrap()), before);
    }
    panel.timing.sequence_inputs.id = panel.timing.draft.as_ref().unwrap().id.clone();
    panel.timing.sequence_inputs.duration = "100".into();
    panel.timing.draft.as_mut().unwrap().pin.epoch += 1;
    let before = draft_snapshot(panel.timing.draft.as_ref().unwrap());
    assert!(panel
        .edit_sequence(&editor, SequenceEdit::UniformDuration)
        .is_err());
    assert_eq!(draft_snapshot(panel.timing.draft.as_ref().unwrap()), before);
}

#[test]
fn sequence_uniform_duration_widget_exports_pairs_without_scene_mutation() {
    use egui::accesskit::Role;
    use egui_kittest::{kittest::Queryable, Harness};
    let (dir, _ctx, mut panel, editor) = panel_fixture_with_clips(false);
    panel.begin_sequence(&editor).unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Duration("5".into()))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Append(20))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Duration("17".into()))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Append(10))
        .unwrap();
    panel
        .edit_sequence(&editor, SequenceEdit::Duration("23".into()))
        .unwrap();
    let output = dir.path().join("uniform.json");
    panel.timing.draft.as_mut().unwrap().output = output.to_string_lossy().into_owned();
    let checksum = editor.checksum();
    let history = editor.history().entries.len();
    let bindings = panel.bindings.as_ref().unwrap().document().clone();
    let mut harness = Harness::builder().with_size([600.0, 900.0]).build_ui_state(
        |ui, state: &mut (SpritePanel, Editor)| state.0.show_timing(ui, &state.1),
        (panel, editor),
    );
    harness.run_steps(3);
    for label in [
        "Use selected duration for all frames",
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
    let source: SpriteSource = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    let frames = &source.clips.last().unwrap().frames;
    assert_eq!(
        frames,
        &[
            Frame {
                region: 10,
                duration_ms: 23
            },
            Frame {
                region: 20,
                duration_ms: 23
            },
            Frame {
                region: 10,
                duration_ms: 23
            },
        ]
    );
    let document = SpriteDocument::new(source.clone()).unwrap();
    let clip = document.clip(&source.clips.last().unwrap().id).unwrap();
    for (time, index, region) in [
        (0, 0, 10),
        (22, 0, 10),
        (23, 1, 20),
        (45, 1, 20),
        (46, 2, 10),
        (68, 2, 10),
        (69, 0, 10),
    ] {
        let sample = clip.sample(time);
        assert_eq!((sample.frame_index, sample.region), (index, region));
    }
    let (panel, editor) = harness.state();
    assert_eq!(panel.timing.sequence_inputs.duration, "23");
    assert_eq!(
        panel.timing.draft.as_ref().unwrap().kind,
        DraftKind::Sequence {
            regions: vec![10, 20, 10],
            selected: 2
        }
    );
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &bindings);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), history);
}

#[path = "clip_copy_tests.rs"]
mod clip_copy_tests;
