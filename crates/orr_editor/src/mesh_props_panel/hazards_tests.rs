use super::*;

fn fixture() -> (tempfile::TempDir, Editor, Panel) {
    let directory = tempfile::tempdir().unwrap();
    let scene = directory.path().join("hazard-props.scene.yaml");
    drop(
        crate::mesh_workspace::mission::prepare(
            &scene,
            &crate::mesh_workspace::mission::Settings::default(),
        )
        .unwrap(),
    );
    let mut editor = Editor::open_game(&scene, EditorGame::MeshHazardsV1).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.sync(&editor);
    assert!(panel.can_edit(&editor));
    (directory, editor, panel)
}

fn art(panel: &Panel) -> Snapshot {
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    Snapshot::from_package(
        &package,
        "foreground.glb",
        "LICENSE.txt",
        panel.graph_revision.unwrap(),
    )
    .unwrap()
}

fn add(panel: &mut Panel) {
    let snapshot = art(panel);
    panel.copy_snapshot_fields(&snapshot);
    panel.candidate = Some(snapshot);
    panel.add_prop();
    assert!(panel.error.is_none(), "{:?}", panel.error);
}

#[test]
fn hazards_props_add_update_undo_redo_save_reopen_and_canonical_bytes() {
    let (_directory, editor, mut panel) = fixture();
    let checksum = editor.checksum();
    let scene_bytes = std::fs::read(editor.path().unwrap()).unwrap();
    assert!(panel.matches_saved_sidecar(&editor, None));
    add(&mut panel);
    assert!(panel.play_blocker(&editor).is_some());
    assert!(!panel.matches_saved_sidecar(&editor, None));
    panel.position[0] = "2.5".into();
    panel.update_selected();
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert_eq!(panel.document.props[0].art.offset()[0], 2.5);
    panel.undo();
    assert_eq!(panel.document.props[0].art.offset()[0], 0.0);
    panel.redo();
    assert_eq!(panel.document.props[0].art.offset()[0], 2.5);
    assert_eq!(panel.placements(&editor).len(), 1);
    panel.save(&editor);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert!(panel.play_blocker(&editor).is_none());
    let digest = panel.sidecar_digest.clone().unwrap();
    assert!(panel.matches_saved_sidecar(&editor, Some(&digest)));
    assert!(!panel.matches_saved_sidecar(&editor, Some("wrong-digest")));
    assert!(!panel.matches_saved_sidecar(&editor, None));
    let bytes = panel.document_bytes(&editor).unwrap();
    let mut reopened =
        Editor::open_game(&editor.path().unwrap(), EditorGame::MeshHazardsV1).unwrap();
    reopened.sync();
    let mut reloaded = Panel::default();
    reloaded.sync(&reopened);
    assert!(reloaded.matches_saved_sidecar(&reopened, Some(&digest)));
    assert_eq!(reloaded.document_bytes(&reopened).unwrap(), bytes);
    assert_eq!(reloaded.placements(&reopened).len(), 1);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), scene_bytes);
}

#[test]
fn hazards_props_identity_guards_reject_wrong_profile_path_epoch_and_graph() {
    let (_directory, editor, mut panel) = fixture();
    add(&mut panel);
    panel.save(&editor);
    let digest = panel.sidecar_digest.clone();
    let epoch = panel.scene_epoch;
    let graph = panel.graph_revision;
    let scene = panel.scene.clone();
    for kind in 0..4 {
        match kind {
            0 => panel.game = EditorGame::MeshNavigation,
            1 => panel.scene = Some(PathBuf::from("different.scene.yaml")),
            2 => panel.scene_epoch = Some(u64::MAX),
            _ => panel.graph_revision = Some([0; 32]),
        }
        assert!(!panel.can_edit(&editor));
        assert!(panel.play_blocker(&editor).is_some());
        assert!(panel.placements(&editor).is_empty());
        assert!(!panel.matches_saved_sidecar(&editor, digest.as_deref()));
        assert!(panel.document_bytes(&editor).is_err());
        panel.save(&editor);
        assert!(panel.error.is_some());
        panel.game = EditorGame::MeshHazardsV1;
        panel.scene = scene.clone();
        panel.scene_epoch = epoch;
        panel.graph_revision = graph;
    }
}

#[test]
fn hazards_props_request_admission_requires_exact_explicit_profile_and_coherent_frame() {
    let path = Path::new("hazard.scene.yaml");
    let spec = HostSpec::local_game(path, EditorGame::MeshHazardsV1);
    assert!(reported_scene_matches_request(
        EditorGame::MeshHazardsV1,
        &spec,
        Some(path)
    ));
    assert!(!reported_scene_matches_request(
        EditorGame::MeshNavigation,
        &spec,
        Some(path)
    ));
    assert!(!reported_scene_matches_request(
        EditorGame::MeshHazardsV1,
        &HostSpec::local_game(path, EditorGame::MeshNavigation),
        Some(path)
    ));
    assert!(!reported_scene_matches_request(
        EditorGame::MeshHazardsV1,
        &HostSpec::local(path),
        Some(path)
    ));
    assert!(!reported_scene_matches_request(
        EditorGame::MeshHazardsV1,
        &HostSpec::remote("ws://localhost:7777", None),
        Some(path)
    ));
    assert!(!reported_scene_matches_request(
        EditorGame::MeshHazardsV1,
        &spec,
        Some(Path::new("other.scene.yaml"))
    ));
    assert!(!document_frame_ready(
        EditorGame::MeshHazardsV1,
        &spec,
        Some(path),
        Mode::Edit,
        false,
        1,
        1
    ));
    assert!(!document_frame_ready(
        EditorGame::MeshHazardsV1,
        &spec,
        Some(path),
        Mode::Edit,
        true,
        1,
        2
    ));
}

#[test]
fn hazards_props_play_mode_preserves_mutation_restrictions() {
    let (_directory, mut editor, mut panel) = fixture();
    add(&mut panel);
    panel.save(&editor);
    editor.play();
    editor.sync();
    assert_eq!(editor.mode(), Mode::Play);
    assert!(!panel.can_edit(&editor));
    assert!(!panel.can_change_history(&editor));
    assert!(!panel.can_reload(&editor));
    panel.save(&editor);
    assert!(panel.error.is_some());
}

#[cfg(feature = "mesh-prop-shading")]
#[test]
fn hazards_props_reject_shaded_prefabs_add_update_and_saved_sidecars() {
    let (_directory, editor, mut panel) = fixture();
    let snapshot = art(&panel);
    let shading = crate::mesh_prop_shading::Settings::new(
        vec![crate::mesh_prop_shading::Slot {
            material: snapshot.source_model_arc().source().primitives[0].material,
            metallic: 0.5,
            roughness: 0.5,
        }],
        snapshot.source_model_arc(),
    )
    .unwrap();
    let template = crate::mesh_prop_prefab::Template::capture("Shaded", &snapshot)
        .unwrap()
        .with_shading(Some(shading.clone()))
        .unwrap();
    assert!(panel
        .stage_prefab(template.clone())
        .unwrap_err()
        .contains("shading"));
    assert!(panel.document.props.is_empty());
    assert!(!panel.pending_prefab());
    panel.copy_snapshot_fields(&snapshot);
    panel
        .shading
        .load(snapshot.source_model_arc(), Some(&shading));
    panel.candidate = Some(snapshot.clone());
    panel.add_prop();
    assert!(panel.error.as_ref().unwrap().contains("shading"));
    assert!(panel.document.props.is_empty());
    add(&mut panel);
    panel
        .shading
        .load(snapshot.source_model_arc(), Some(&shading));
    panel.update_selected();
    assert!(panel.error.as_ref().unwrap().contains("shading"));
    assert!(panel.document.props[0].shading.is_none());
    panel.copy_selection_fields();
    let mut shaded = panel.document.clone();
    shaded.props[0].shading = Some(shading);
    let scene = editor.path().unwrap();
    mesh_props::save(&scene, Some(&shaded), None, panel.graph_revision.unwrap()).unwrap();
    let mut rejected = Panel::default();
    rejected.sync(&editor);
    assert!(rejected.sidecar_error.as_ref().unwrap().contains("shading"));
    assert!(rejected.play_blocker(&editor).is_some());
    assert!(rejected.placements(&editor).is_empty());
    let mut navigation = Panel {
        graph_revision: panel.graph_revision,
        ..Default::default()
    };
    navigation.stage_prefab(template).unwrap();
    navigation.add_prop();
    assert!(navigation.error.is_none(), "{:?}", navigation.error);
    assert!(navigation.document.props[0].shading.is_some());
}

#[test]
fn hazards_props_open_proposals_keep_save_history_and_viewport_mutation_closed() {
    let (_directory, mut editor, mut panel) = fixture();
    add(&mut panel);
    editor
        .agent_client("hazard-prop-review")
        .unwrap()
        .call(
            "proposal.begin",
            serde_json::json!({"label": "pending prop review"}),
        )
        .unwrap();
    editor.sync();
    assert_eq!(editor.proposals().len(), 1);
    assert!(!panel.can_edit(&editor));
    assert!(!panel.can_change_history(&editor));
    assert!(!panel.can_reload(&editor));
    panel.begin_placement(&editor, false);
    assert!(panel.placement.is_none());
    panel.save(&editor);
    assert!(panel.error.is_some());
    assert!(panel.sidecar_digest.is_none());
}

#[test]
fn hazards_props_explicit_empty_sidecar_retains_saved_digest_and_canonical_bytes() {
    let (_directory, editor, panel) = fixture();
    let scene = editor.path().unwrap();
    let revision = panel.graph_revision.unwrap();
    let filename = scene.file_name().unwrap().to_str().unwrap();
    let bytes = Document::default().encode(filename, revision).unwrap();
    let sidecar = scene.with_file_name(format!("{filename}.mesh-props.json"));
    std::fs::write(sidecar, &bytes).unwrap();
    let mut loaded = Panel::default();
    loaded.sync(&editor);
    assert!(loaded.sidecar_error.is_none(), "{:?}", loaded.sidecar_error);
    let digest = loaded.sidecar_digest.as_deref().unwrap();
    assert!(loaded.matches_saved_sidecar(&editor, Some(digest)));
    assert!(!loaded.matches_saved_sidecar(&editor, None));
    assert_eq!(loaded.document_bytes(&editor).unwrap(), bytes);
}
