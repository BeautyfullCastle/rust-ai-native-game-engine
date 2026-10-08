//! Explicit feature-unification negative: sample support is deliberately present,
//! but this consuming editor was built without its room-character capability.
#![cfg(all(
    feature = "room-project",
    feature = "project-create",
    not(feature = "room-character"),
    target_os = "linux"
))]
#[test]
#[ignore = "explicit feature-unification negative; build sample/room-character and not editor/room-character"]
fn editor_rejects_character_even_when_sample_dependency_support_is_unified() {
    assert!(
        orr_sample::room_project::compiled_runtime_with_character(true)
            .capabilities
            .contains("animation"),
        "the negative must really unify the sample capability"
    );
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("room");
    orr_sample::project_create::create(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: orr_sample::project_create::ROOM_CHARACTER_TEMPLATE.into(),
        seed: "unified".into(),
    })
    .unwrap();
    assert!(orr_sample::room_project::PreparedProject::open(&root).is_err());
    let project = orr_sample::room_project::PreparedProject::open_with_capabilities(
        &root,
        false,
        orr_sample::room_project::CheckpointSupport::Disabled,
        true,
    )
    .unwrap();
    let (_, _, _, models) = project.into_parts();
    let mut panel = orr_editor::model_panel::ModelPanel::default();
    let error = panel.install_room(models).unwrap_err();
    assert!(
        error.contains("explicit editor room-character support"),
        "{error}"
    );
    assert!(panel.bindings.is_none());
}
