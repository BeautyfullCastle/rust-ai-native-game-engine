//! Source-declared Room lighting admission cases for explicit consumer capabilities.
//! The same target covers feature-off consumers and explicitly enabled consumers.
#![cfg(all(
    feature = "room-project",
    feature = "project-create",
    target_os = "linux"
))]

use orr_sample::{
    project_create::{create, CreateOptions},
    room_project::{self, CheckpointSupport, PreparedProject},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

const OFF: &[u8] = br#"{"version":1,"point_light":null}"#;

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("lighting room");
    create(&CreateOptions {
        output: root.clone(),
        template: "room-escape-3d-v1".into(),
        seed: "lighting-acceptance".into(),
    })
    .unwrap();
    (temp, root)
}

fn declare(root: &Path, lighting: serde_json::Value) {
    let path = root.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["entry"]["lighting"] = lighting;
    fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}

fn open(root: &Path, supported: bool) -> Result<PreparedProject, String> {
    PreparedProject::open_with_presentation(
        root,
        false,
        CheckpointSupport::Disabled,
        false,
        supported,
    )
}

fn error(result: Result<PreparedProject, String>) -> String {
    match result {
        Ok(_) => panic!("invalid lighting project was admitted"),
        Err(error) => error,
    }
}

fn manifest_admitted(root: &Path) -> bool {
    // This is the package manifest boundary itself. A disabled runtime consumer
    // must not make malformed-path cases pass merely by rejecting every light.
    orr_package::Project::open(root, room_project::compiled_runtime()).is_ok()
}

#[test]
fn legacy_absent_lighting_stays_off_and_unsupported_consumers_refuse_declaration() {
    let (_temp, root) = fixture();
    let original = PreparedProject::open(&root).unwrap();
    let initial = original.scene().frame().to_bytes();
    assert_eq!(original.point_light_settings(), Default::default());
    assert_eq!(
        open(&root, true).unwrap().point_light_settings(),
        Default::default()
    );

    // An undeclared file cannot silently turn on presentation support.
    fs::write(root.join("room.lighting.json"), b"{unreferenced lighting").unwrap();
    assert_eq!(
        PreparedProject::open(&root)
            .unwrap()
            .scene()
            .frame()
            .to_bytes(),
        initial
    );
    assert_eq!(
        PreparedProject::open(&root).unwrap().point_light_settings(),
        Default::default()
    );
    declare(&root, serde_json::json!("room.lighting.json"));
    fs::write(root.join("room.lighting.json"), OFF).unwrap();
    assert!(
        manifest_admitted(&root),
        "fixture must have a valid lighting manifest"
    );

    for rejected in [
        PreparedProject::open(&root),
        PreparedProject::open_with_ui(&root, false),
        PreparedProject::open_with_options(&root, false, CheckpointSupport::Disabled),
        PreparedProject::open_with_capabilities(&root, false, CheckpointSupport::Disabled, false),
        open(&root, false),
    ] {
        let error = error(rejected);
        assert!(
            error.contains("lighting consumer"),
            "wrong capability failure: {error}"
        );
    }
    assert_eq!(fs::read(root.join("room.lighting.json")).unwrap(), OFF);
}

#[cfg(not(feature = "room-lighting"))]
#[test]
fn declared_lighting_is_rejected_even_when_uncompiled_consumer_claims_support() {
    let (_temp, root) = fixture();
    declare(&root, serde_json::json!("room.lighting.json"));
    for bytes in [
        OFF,
        br#"{"version":1,"point_light":{"position":[0,4,0],"color":[1,1,1],"intensity":3,"range":12}}"#,
    ] {
        fs::write(root.join("room.lighting.json"), bytes).unwrap();
        assert!(manifest_admitted(&root));
        let error = error(open(&root, true));
        assert!(error.contains("lighting consumer"), "compiled feature must still be required: {error}");
        assert_eq!(fs::read(root.join("room.lighting.json")).unwrap(), bytes);
    }
}

#[test]
fn lighting_manifest_is_room_only_distinct_contained_and_nonnull() {
    for path in [
        "../outside.json",
        "/tmp/outside.json",
        "sub/light.json",
        "sub\\light.json",
        "./light.json",
        ".hidden",
        "orr.project.json",
        "ORR.PACKAGES.LOCK.JSON",
        "room.scene.yaml",
        "ROOM.MODELS.JSON",
        "room.camera.json",
        "",
    ] {
        let (_temp, root) = fixture();
        declare(&root, serde_json::json!(path));
        assert!(
            !manifest_admitted(&root),
            "invalid lighting manifest accepted: {path}"
        );
        assert!(
            open(&root, true).is_err(),
            "consumer accepted invalid lighting path: {path}"
        );
    }
    let (_temp, root) = fixture();
    declare(&root, serde_json::Value::Null);
    assert!(
        !manifest_admitted(&root),
        "declared null is not an absent descriptor"
    );
    assert!(open(&root, true).is_err());

    for game in ["arena", "collect-dodge-v1", "terrain-point-route-3d-v1"] {
        let (_temp, root) = fixture();
        declare(&root, serde_json::json!("room.lighting.json"));
        let path = root.join("orr.project.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["entry"]["game"] = serde_json::json!(game);
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(
            !manifest_admitted(&root),
            "non-Room lighting accepted: {game}"
        );
        assert!(open(&root, true).is_err());
    }
}

#[cfg(feature = "room-lighting")]
#[test]
fn generated_room_lighting_admission_is_strict_and_presentation_only() {
    use orr_sample::{
        room_game::{RoomInput, INTERACT},
        room_lighting::Document,
    };
    use orr_sim::{PlayerSlot, TickInputs};

    let (_temp, root) = fixture();
    let original = PreparedProject::open(&root).unwrap();
    let initial = original.scene().frame().to_bytes();
    let checksum = original.scene().frame().checksum();
    let scene = fs::read(root.join("room.scene.yaml")).unwrap();
    let models = fs::read(root.join("room.models.json")).unwrap();
    let camera = fs::read(root.join("room.camera.json")).unwrap();
    let lock = fs::read(root.join("orr.packages.lock.json")).unwrap();
    declare(&root, serde_json::json!("room.lighting.json"));
    let manifest = fs::read(root.join("orr.project.json")).unwrap();
    let path = root.join("room.lighting.json");
    let mut moved = Document::enabled_default();
    moved.point_light.as_mut().unwrap().position = [-3.0, 2.0, 3.0];

    for document in [Document::default(), Document::enabled_default(), moved] {
        let bytes = document.to_bytes().unwrap();
        assert_eq!(Document::parse(&bytes).unwrap(), document);
        assert_eq!(
            orr_render::PointLightSettings::from_bytes(&bytes).unwrap(),
            document.settings()
        );
        fs::write(&path, &bytes).unwrap();
        let prepared = open(&root, true).unwrap();
        let lighting = prepared.lighting().expect("explicit lighting admission");
        assert_eq!(lighting.bytes, bytes);
        assert_eq!(lighting.document, document);
        assert_eq!(lighting.path, path);
        assert_eq!(prepared.point_light_settings(), document.settings());
        assert_eq!(prepared.scene().frame().to_bytes(), initial);
        assert_eq!(prepared.scene().frame().checksum(), checksum);

        let mut off = original.scene().simulation().unwrap();
        let mut lit = prepared.scene().simulation().unwrap();
        for tick in 0..120 {
            let input = RoomInput {
                move_x: if tick < 60 { 1 } else { -1 },
                move_z: if tick < 30 { 0 } else { -1 },
                buttons: if tick % 7 == 0 { INTERACT } else { 0 },
                ..Default::default()
            };
            let mut inputs = TickInputs::new(off.tick(), 1);
            inputs.set_input(PlayerSlot(0), input);
            off.step(&inputs);
            lit.step(&inputs);
            assert_eq!(
                off.frame().checksum(),
                lit.frame().checksum(),
                "lighting changed tick {tick}"
            );
            assert_eq!(off.frame().to_bytes(), lit.frame().to_bytes());
        }
        assert_ne!(
            off.frame().checksum(),
            checksum,
            "exercise an actual stepped simulation"
        );

        let mut off_session = original.scene().session().unwrap();
        let mut lit_session = prepared.scene().session().unwrap();
        for _ in 0..60 {
            assert!(off_session.step_now().is_some());
            assert!(lit_session.step_now().is_some());
        }
        off_session.seek(17).unwrap();
        lit_session.seek(17).unwrap();
        assert_eq!(
            off_session.frame().to_bytes(),
            lit_session.frame().to_bytes()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read(root.join("orr.project.json")).unwrap(), manifest);
        assert_eq!(fs::read(root.join("room.scene.yaml")).unwrap(), scene);
        assert_eq!(fs::read(root.join("room.models.json")).unwrap(), models);
        assert_eq!(fs::read(root.join("room.camera.json")).unwrap(), camera);
        assert_eq!(fs::read(root.join("orr.packages.lock.json")).unwrap(), lock);
    }
}

#[cfg(feature = "room-lighting")]
#[test]
fn lighting_invalid_documents_and_nonregular_files_fail_closed() {
    let (_temp, root) = fixture();
    declare(&root, serde_json::json!("room.lighting.json"));
    let path = root.join("room.lighting.json");
    let scene = fs::read(root.join("room.scene.yaml")).unwrap();
    for bytes in [
        b"[]".as_slice(),
        br#"[1,null]"#,
        br#"{}"#,
        br#"{"version":1}"#,
        br#"{"version":2,"point_light":null}"#,
        br#"{"version":1,"version":1,"point_light":null}"#,
        br#"{"version":1,"point_light":null,"shadow":true}"#,
        br#"{"version":1,"point_light":[[0,1,0],[1,1,1],2,8]}"#,
        br#"{"version":1,"point_light":{"position":[0,1,0],"color":[1,1,1],"intensity":2,"range":0}}"#,
        br#"{"version":1,"point_light":{"position":[0,1,0],"color":[1,1,1],"intensity":-1,"range":8}}"#,
        br#"{"version":1,"point_light":null} {}"#,
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(open(&root, true).is_err(), "invalid lighting accepted: {bytes:?}");
        assert_eq!(fs::read(&path).unwrap(), bytes, "failed admission must not rewrite the sidecar");
    }
    fs::write(&path, vec![b' '; orr_sample::room_lighting::MAX_BYTES + 1]).unwrap();
    assert!(open(&root, true).is_err());
    fs::remove_file(&path).unwrap();
    assert!(open(&root, true).is_err(), "declared lighting must exist");
    fs::create_dir(&path).unwrap();
    assert!(
        open(&root, true).is_err(),
        "lighting must be a regular file"
    );
    assert_eq!(fs::read(root.join("room.scene.yaml")).unwrap(), scene);
}

#[cfg(all(feature = "room-lighting", unix))]
#[test]
fn lighting_symlinks_remain_rejected() {
    let (temp, root) = fixture();
    declare(&root, serde_json::json!("room.lighting.json"));
    let outside = temp.path().join("outside.json");
    fs::write(&outside, OFF).unwrap();
    let path = root.join("room.lighting.json");
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert!(
        open(&root, true).is_err(),
        "outside symlink must not be followed"
    );
    assert_eq!(fs::read(&outside).unwrap(), OFF);
    fs::remove_file(&path).unwrap();
    let sibling = root.join("sibling-lighting.json");
    fs::write(&sibling, OFF).unwrap();
    std::os::unix::fs::symlink(&sibling, &path).unwrap();
    assert!(
        open(&root, true).is_err(),
        "even an in-project symlink is nonregular"
    );
    assert_eq!(fs::read(&sibling).unwrap(), OFF);
}
