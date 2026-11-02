//! Room progress metadata admission is explicit and does not open persistence.
use super::*;
use std::fs;
fn fixture(template: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    crate::project_create::create(&crate::project_create::CreateOptions {
        output: root.clone(),
        template: template.into(),
        seed: "progress-admission".into(),
    })
    .unwrap();
    (temp, root)
}
fn progress(root: &Path) {
    let path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["schema"] = 3.into();
    manifest["progress"] = serde_json::json!({"schema":1,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"room-escape-clear-v1"});
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}
fn caps() -> RoomProjectCapabilities {
    RoomProjectCapabilities {
        lighting: false,
        character: false,
    }
}
#[test]
fn legacy_and_explicit_progress_support_preserve_scene_and_manifest() {
    let (_temp, root) = fixture(crate::project_create::ROOM_TEMPLATE);
    let legacy = PreparedProject::open(&root).unwrap();
    assert!(legacy.progress().is_none());
    let checksum = legacy.scene().frame().checksum();
    progress(&root);
    let path = root.join("orr.project.json");
    let bytes = fs::read(&path).unwrap();
    assert!(PreparedProject::open(&root).is_err());
    assert!(PreparedProject::open_with_capabilities(&root, caps()).is_err());
    assert!(
        PreparedProject::open_with_progress(&root, caps(), ProgressSupport::Unsupported).is_err()
    );
    let admitted =
        PreparedProject::open_with_progress(&root, caps(), ProgressSupport::MetadataOnly).unwrap();
    assert_eq!(
        admitted.progress().unwrap().game_id,
        "12345678-1234-4234-8234-123456789abc"
    );
    assert_eq!(admitted.scene().frame().checksum(), checksum);
    assert_eq!(fs::read(path).unwrap(), bytes);
}
#[test]
fn room_rejects_collect_profile_and_invalid_identity_without_repair() {
    let (_temp, root) = fixture(crate::project_create::ROOM_TEMPLATE);
    progress(&root);
    let path = root.join("orr.project.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (field, value) in [
        ("profile", "collect-dodge-highscore-v1"),
        ("game_id", "not-a-uuid"),
    ] {
        let mut invalid = original.clone();
        invalid["progress"][field] = value.into();
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(
            PreparedProject::open_with_progress(&root, caps(), ProgressSupport::MetadataOnly)
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}
#[cfg(all(feature = "room-lighting", feature = "room-character"))]
#[test]
fn schema_three_combines_camera_lighting_character_with_independent_capabilities() {
    let (_temp, root) = fixture(crate::project_create::ROOM_CHARACTER_TEMPLATE);
    progress(&root);
    fs::write(
        root.join("room.camera.json"),
        crate::room_camera::Document::readable_default()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("room.lighting.json"),
        crate::room_lighting::Document::enabled_default()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["entry"]["camera"] = "room.camera.json".into();
    manifest["entry"]["lighting"] = "room.lighting.json".into();
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    for capabilities in [
        caps(),
        RoomProjectCapabilities {
            lighting: true,
            character: false,
        },
        RoomProjectCapabilities {
            lighting: false,
            character: true,
        },
    ] {
        assert!(PreparedProject::open_with_progress(
            &root,
            capabilities,
            ProgressSupport::MetadataOnly
        )
        .is_err());
    }
    let admitted = PreparedProject::open_with_progress(
        &root,
        RoomProjectCapabilities {
            lighting: true,
            character: true,
        },
        ProgressSupport::MetadataOnly,
    )
    .unwrap();
    assert!(
        admitted.progress().is_some()
            && admitted.camera().is_some()
            && admitted.lighting().is_some()
            && admitted.character().is_some()
    );
}
