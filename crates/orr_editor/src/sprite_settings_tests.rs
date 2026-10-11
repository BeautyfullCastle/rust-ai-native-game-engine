//! Actual panel input tests for the presentation-only settings picker.
use super::*;
use crate::model::Target;
use egui_kittest::{kittest::Queryable, Harness};

fn setup() -> (
    tempfile::TempDir,
    SpritePanel,
    Editor,
    orr_reflect::Guid,
    orr_reflect::Guid,
) {
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
        include_str!("../../../scenes/arena_blank.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Arena).unwrap();
    editor.sync();
    assert!(editor.spawn_arena_player(0, [0.0, 0.0]));
    editor.sync();
    let first = editor.selected_guid().unwrap();
    assert!(editor.spawn_arena_player(1, [3.0, 0.0]));
    editor.sync();
    let second = editor.selected_guid().unwrap();
    editor.select(Some(Target::Guid(first.clone())));
    let mut bindings = Bindings::create(
        dir.path().join("arena.sprites.json"),
        "arena.yaml".into(),
        ".".into(),
    )
    .unwrap();
    bindings
        .assign(
            std::slice::from_ref(&first),
            Some(Binding {
                package: "sample-sprites".into(),
                document: "sprites.json".into(),
                source: Source::Locomotion {
                    idle: "idle".into(),
                    walk: "walk".into(),
                },
                units_per_pixel: 100.0,
            }),
        )
        .unwrap();
    bindings.save().unwrap();
    let mut panel = SpritePanel {
        bindings: Some(bindings),
        package: "draft".into(),
        document: "old.json".into(),
        source: Source::Region(21),
        scale: 0.2,
        ..Default::default()
    };
    panel
        .load(&egui::Context::default(), "sample-sprites", "sprites.json")
        .unwrap();
    (dir, panel, editor, first, second)
}
fn draft(panel: &SpritePanel) -> (String, String, Source, f32, bool) {
    (
        panel.package.clone(),
        panel.document.clone(),
        panel.source.clone(),
        panel.scale,
        panel.preview_playing,
    )
}

#[test]
fn sprite_settings_picker_widgets_copy_assign_history_save_reopen() {
    let (_dir, panel, editor, first, second) = setup();
    let original = panel.bindings.as_ref().unwrap().document().clone();
    let checksum = editor.checksum();
    let history_len = editor.history().entries.len();
    let mut h = Harness::builder()
        .with_size([1200.0, 1600.0])
        .build_ui_state(
            |ui, (panel, editor): &mut (SpritePanel, Editor)| panel.show(ui, editor),
            (panel, editor),
        );
    h.get_by_label("Sprite bindings (view only)").click();
    h.run_steps(3);
    for _ in 0..2 {
        h.get_by_label("Use selected sprite settings").click();
        h.run_steps(3);
        assert_eq!(h.state().0.package, "sample-sprites");
        assert_eq!(h.state().0.document, "sprites.json");
        assert_eq!(
            h.state().0.source,
            original.bindings[&first.to_string()].source
        );
        assert_eq!(h.state().0.scale, 100.0);
        assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &original);
        assert!(!h.state().0.bindings.as_ref().unwrap().dirty());
    }
    h.state_mut().1.select(Some(Target::Guid(second.clone())));
    h.run_steps(3);
    h.get_by_label("Assign sprite to selection").click();
    h.run_steps(3);
    let assigned = h.state().0.bindings.as_ref().unwrap().document().clone();
    assert_eq!(
        assigned.bindings[&first.to_string()],
        assigned.bindings[&second.to_string()]
    );
    h.get_by_label("Undo binding").click();
    h.run_steps(3);
    assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &original);
    h.get_by_label("Redo binding").click();
    h.run_steps(3);
    assert_eq!(h.state().0.bindings.as_ref().unwrap().document(), &assigned);
    h.get_by_label("Save bindings").click();
    h.run_steps(3);
    let bindings = h.state().0.bindings.as_ref().unwrap();
    assert_eq!(
        Bindings::open(bindings.path.clone()).unwrap().document(),
        &assigned
    );
    assert!(!bindings.dirty());
    assert_eq!(h.state().1.checksum(), checksum);
    assert_eq!(h.state().1.history().entries.len(), history_len);
}

#[test]
fn sprite_settings_picker_rejects_atomically_and_preserves_admitted_cache() {
    let (dir, mut panel, mut editor, first, second) = setup();
    panel.preview_playing = true;
    let before = draft(&panel);
    let original = panel.bindings.as_ref().unwrap().document().clone();
    let key = ("sample-sprites".to_string(), "sprites.json".to_string());
    let texture = panel.loaded[&key].texture.id();
    for selection in [
        None,
        Some(Target::Guid(second.clone())),
        Some(Target::Guid(orr_reflect::Guid::from_u32(999999))),
    ] {
        editor.select(selection);
        assert!(panel.use_selected_settings(&editor).is_err());
        assert_eq!(draft(&panel), before);
    }
    editor.select(Some(Target::Guid(first.clone())));
    assert!(editor.toggle_selection(Target::Guid(second.clone())));
    assert!(panel.use_selected_settings(&editor).is_err());
    assert_eq!(draft(&panel), before);
    editor.select(Some(Target::Guid(first.clone())));
    let asset = panel.loaded.remove(&key).unwrap();
    assert!(panel.use_selected_settings(&editor).is_err());
    assert_eq!(draft(&panel), before);
    panel.loaded.insert(key.clone(), asset);
    assert_eq!(panel.loaded[&key].texture.id(), texture);
    editor.play();
    editor.sync();
    assert!(panel.use_selected_settings(&editor).is_err());
    assert_eq!(draft(&panel), before);
    editor.stop();
    editor.sync();
    let other = dir.path().join("other.yaml");
    std::fs::write(
        &other,
        include_str!("../../../scenes/arena_blank.scene.yaml"),
    )
    .unwrap();
    assert!(editor.open_path(&other));
    editor.sync();
    assert!(panel.use_selected_settings(&editor).is_err());
    assert_eq!(draft(&panel), before);
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &original);
    assert!(!panel.bindings.as_ref().unwrap().dirty());
    assert_eq!(panel.loaded.len(), 1);
    assert_eq!(panel.loaded[&key].texture.id(), texture);
}

#[test]
fn sprite_settings_picker_rejects_invalid_inactive_clip_without_mutation() {
    let (_dir, mut panel, editor, first, _) = setup();
    let mut invalid =
        panel.bindings.as_ref().unwrap().document().bindings[&first.to_string()].clone();
    for source in [Source::Region(21), Source::Clip("greet".into())] {
        let mut binding = invalid.clone();
        binding.source = source.clone();
        panel
            .bindings
            .as_mut()
            .unwrap()
            .assign(std::slice::from_ref(&first), Some(binding))
            .unwrap();
        panel.use_selected_settings(&editor).unwrap();
        assert_eq!(panel.source, source);
        assert_eq!(panel.scale, 100.0);
    }
    let valid_document = panel.bindings.as_ref().unwrap().document().clone();
    let valid_draft = draft(&panel);
    for scale in [101.0, f32::INFINITY, f32::NAN, 0.0] {
        let mut binding = invalid.clone();
        binding.units_per_pixel = scale;
        assert!(panel
            .bindings
            .as_mut()
            .unwrap()
            .assign(std::slice::from_ref(&first), Some(binding))
            .is_err());
        assert_eq!(panel.bindings.as_ref().unwrap().document(), &valid_document);
        assert_eq!(draft(&panel), valid_draft);
    }
    invalid.source = Source::Locomotion {
        idle: "idle".into(),
        walk: "missing".into(),
    };
    panel
        .bindings
        .as_mut()
        .unwrap()
        .assign(std::slice::from_ref(&first), Some(invalid))
        .unwrap();
    let before = draft(&panel);
    let document = panel.bindings.as_ref().unwrap().document().clone();
    let cache_len = panel.loaded.len();
    assert!(panel
        .use_selected_settings(&editor)
        .unwrap_err()
        .contains("missing walk clip"));
    assert_eq!(draft(&panel), before);
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &document);
    assert_eq!(panel.loaded.len(), cache_len);
}
