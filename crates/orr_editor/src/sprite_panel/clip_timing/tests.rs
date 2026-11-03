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
        clip_index: 0,
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
    let dir = tempfile::tempdir().unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/sprite_demo")
        .canonicalize()
        .unwrap();
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
    panel.source = Source::Clip(
        panel.loaded.values().next().unwrap().asset.document.clips()[0]
            .id()
            .into(),
    );
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
