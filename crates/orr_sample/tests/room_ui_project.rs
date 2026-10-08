//! Bounded Room UI package/admission tests; real lifecycle proof is separate.
#![cfg(all(
    feature = "room-ui",
    feature = "project-create",
    target_os = "linux",
    target_arch = "x86_64"
))]
use orr_sample::{
    authored_ui::{Document, Profile},
    project_create::{self, CreateOptions},
    room_project::PreparedProject,
};
#[test]
fn room_ui_template_admission_closed_profiles_and_owned_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("room ui");
    let report = project_create::create(&CreateOptions {
        output: root.clone(),
        template: project_create::ROOM_UI_TEMPLATE.into(),
        seed: "room-ui-test".into(),
    })
    .unwrap();
    assert!(PreparedProject::open(&root).is_err()); // default consumer remains unsupported
    let project = PreparedProject::open_with_ui(&root, true).unwrap();
    let initial = project.scene().frame().checksum();
    assert_eq!(initial, report.initial_checksum);
    let ui = project.ui().unwrap();
    ui.document.validate_for(Profile::Room).unwrap();
    assert!(ui.document.validate().is_err());
    assert!(!ui.font.is_empty());
    let camera = project.camera().unwrap().document.clone();
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(
        project.scene().simulation().unwrap().frame().checksum(),
        initial
    );
    assert_eq!(project.camera().unwrap().document, camera);
}
#[test]
fn room_document_wrong_profile_and_reserved_alias_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("room ui");
    project_create::create(&CreateOptions {
        output: root.clone(),
        template: project_create::ROOM_UI_TEMPLATE.into(),
        seed: "room-ui-negative".into(),
    })
    .unwrap();
    std::fs::write(
        root.join("room.ui.json"),
        Document::default_collect().to_bytes().unwrap(),
    )
    .unwrap();
    assert!(PreparedProject::open_with_ui(&root, true).is_err());
    std::fs::write(
        root.join("room.ui.json"),
        Document::default_room()
            .to_bytes_for(Profile::Room)
            .unwrap(),
    )
    .unwrap();
    let path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["entry"]["ui"]["document"] = "orr.packages.lock.json".into();
    std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(PreparedProject::open_with_ui(&root, true).is_err());
}

#[test]
fn unsupported_consumer_rejects_before_reading_package_or_scene_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("room ui");
    project_create::create(&CreateOptions {
        output: root.clone(),
        template: project_create::ROOM_UI_TEMPLATE.into(),
        seed: "unsupported-ui".into(),
    })
    .unwrap();
    std::fs::remove_dir_all(root.join(".orr")).unwrap();
    std::fs::remove_file(root.join("room.scene.yaml")).unwrap();
    let error = PreparedProject::open(&root).err().expect("unsupported");
    assert!(error.contains("explicit UI consumer support"), "{error}");
}
