//! Authored Room follow-camera admission and frame-resolution acceptance.
#![cfg(all(
    feature = "room-project",
    feature = "project-create",
    target_os = "linux"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use orr_bridge::FrameView;
use orr_ecs::{Entity, Frame};
use orr_fp::{FPVec3, FP};
use orr_physics3d::Body;
use orr_reflect::{Guid, Scene};
use orr_sample::{
    project_create::{create, CreateOptions},
    room_camera::{Document, Follow},
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, PLAYER},
    room_project::{self, CheckpointSupport, PreparedProject, PreparedScene, SEED},
};
use orr_sim::Simulation;
use std::{fs, path::PathBuf};

fn player(frame: &Frame) -> Entity {
    let mut players = frame.entities().filter(|&entity| {
        frame
            .get::<RoomActor>(entity)
            .is_some_and(|actor| actor.kind == PLAYER)
    });
    let player = players.next().expect("one PLAYER in a generated Room");
    assert!(
        players.next().is_none(),
        "Room fixture must have one PLAYER"
    );
    player
}

fn sample_follow(player: String) -> Document {
    let mut document = Document::readable_default();
    document.schema = 2;
    document.follow = Some(Follow {
        player,
        offset: [1.25, -0.5, 0.75],
    });
    document
}

fn create_room(template: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("generated Room");
    create(&CreateOptions {
        output: root.clone(),
        template: template.into(),
        seed: "follow-camera-acceptance".into(),
    })
    .unwrap();
    (temp, root)
}

fn try_open_room(root: &std::path::Path) -> Result<PreparedProject, String> {
    PreparedProject::open_with_capabilities(
        root,
        cfg!(feature = "room-ui"),
        CheckpointSupport::Disabled,
        cfg!(feature = "room-character"),
    )
}

fn open_room(root: &std::path::Path) -> PreparedProject {
    try_open_room(root).unwrap()
}

#[test]
fn follow_camera_schema1_wire_bytes_and_schema2_contract() {
    let fixed = Document::readable_default();
    let fixed_bytes = fixed.to_bytes().unwrap();
    assert!(!String::from_utf8_lossy(&fixed_bytes).contains("follow"));
    assert_eq!(Document::parse(&fixed_bytes).unwrap(), fixed);
    let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    assert_eq!(
        fixed
            .camera_for_frame(
                &fixed.orbit(),
                FrameView::of(simulation.frame()),
                &Default::default(),
                (1024, 768),
            )
            .unwrap(),
        fixed.camera(&fixed.orbit(), (1024, 768)).unwrap()
    );

    let follow = sample_follow("e_00000001".into());
    let bytes = follow.to_bytes().unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("\"schema\": 2"));
    assert!(String::from_utf8_lossy(&bytes).contains("\"follow\""));
    assert_eq!(Document::parse(&bytes).unwrap(), follow);

    let mut invalid = follow.clone();
    invalid.follow.as_mut().unwrap().player = "not-a-guid".into();
    assert!(invalid.to_bytes().unwrap_err().contains("GUID"));
    for bad_offset in [-16.01, 16.01, f32::NAN, f32::INFINITY] {
        let mut invalid = follow.clone();
        invalid.follow.as_mut().unwrap().offset[1] = bad_offset;
        assert!(invalid.validate().is_err());
    }
    let mut missing = follow.clone();
    missing.follow = None;
    assert!(missing.validate().is_err(), "schema 2 requires follow");
    let mut forbidden = fixed.clone();
    forbidden.follow = Some(follow.follow.unwrap());
    assert!(forbidden.validate().is_err(), "schema 1 forbids follow");
}

#[test]
fn follow_camera_resolves_the_current_player_frame_without_touching_simulation() {
    let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let authored = Scene::unbake(&room_project::types(), simulation.frame(), None).unwrap();
    let prepared = PreparedScene::parse(&authored.to_yaml()).unwrap();
    let entity = player(prepared.frame());
    let guid = prepared.index().guid(entity).unwrap().to_string();
    let document = sample_follow(guid);
    let orbit = document.orbit();
    let initial_bytes = prepared.frame().to_bytes();
    let initial_checksum = prepared.frame().checksum();
    let frame = FrameView::of(prepared.frame());
    document.validate_frame(frame, prepared.index()).unwrap();
    let camera = document
        .camera_for_frame(&orbit, frame, prepared.index(), (1200, 800))
        .unwrap();
    let body = prepared.frame().get::<Body>(entity).unwrap();
    let position = orr_view::fp_to_vec3(body.pos).to_array();
    assert_eq!(
        camera.target,
        [position[0] + 1.25, position[1] - 0.5, position[2] + 0.75,]
    );
    let fixed = document.camera(&orbit, (1200, 800)).unwrap();
    assert_ne!(
        camera.target, fixed.target,
        "follow replaces only the target"
    );
    let camera_eye = camera.eye;
    let camera_target = camera.target;
    let fixed_eye = fixed.eye;
    let fixed_target = fixed.target;
    for axis in 0..3 {
        assert!(
            ((camera_eye[axis] - camera_target[axis]) - (fixed_eye[axis] - fixed_target[axis]))
                .abs()
                < 0.00001,
            "follow preserves the existing orbit distance and orientation"
        );
    }
    assert_eq!(camera.projection, fixed.projection);

    let mut moved = prepared.frame().clone();
    moved.get_mut::<Body>(entity).unwrap().pos =
        FPVec3::new(FP::from_int(3), FP::HALF, FP::from_int(-2));
    let moved_camera = document
        .camera_for_frame(&orbit, FrameView::of(&moved), prepared.index(), (1200, 800))
        .unwrap();
    assert_eq!(moved_camera.target, [4.25, 0.0, -1.25]);
    assert_ne!(
        moved_camera.target, camera.target,
        "there is no camera history or smoothing"
    );
    assert_eq!(prepared.frame().to_bytes(), initial_bytes);
    assert_eq!(prepared.frame().checksum(), initial_checksum);

    // A GUID that still parses but maps to another row must fail closed.
    let mut reassigned = prepared.index().clone();
    let other = prepared
        .frame()
        .entities()
        .find(|candidate| *candidate != entity)
        .unwrap();
    reassigned.insert(
        Guid::parse(&document.follow.as_ref().unwrap().player).unwrap(),
        other,
    );
    let error = document
        .camera_for_frame(&orbit, frame, &reassigned, (1200, 800))
        .unwrap_err();
    assert!(
        error.contains("reverse mapping") || error.contains("sole PLAYER"),
        "{error}"
    );

    let mut missing = orr_reflect::SceneIndex::default();
    for (candidate_guid, candidate_entity) in prepared.index().iter() {
        if *candidate_guid != Guid::parse(&document.follow.as_ref().unwrap().player).unwrap() {
            missing.insert(candidate_guid.clone(), candidate_entity);
        }
    }
    assert!(document
        .validate_frame(frame, &missing)
        .unwrap_err()
        .contains("not in the scene index"));
}

#[test]
fn generated_project_follow_camera_admission_is_strict() {
    let templates: &[&str] = if cfg!(feature = "room-character") {
        &["room-escape-3d-v1", "room-escape-character-3d-v1"]
    } else {
        &["room-escape-3d-v1"]
    };
    for template in templates {
        let (_temp, root) = create_room(template);
        let legacy = open_room(&root);
        let camera = legacy
            .camera()
            .expect("generated Room template owns a camera");
        assert_eq!(
            camera.document.schema, 1,
            "the generated legacy view stays fixed"
        );
        assert!(camera.document.follow.is_none());
        assert_eq!(camera.bytes, fs::read(&camera.path).unwrap());
        let player_entity = player(legacy.scene().frame());
        let player_guid = legacy
            .scene()
            .index()
            .guid(player_entity)
            .unwrap()
            .to_string();
        let default_frame = legacy.scene().frame().to_bytes();
        let default_checksum = legacy.scene().frame().checksum();
        drop(legacy);

        let follow = sample_follow(player_guid);
        fs::write(root.join("room.camera.json"), follow.to_bytes().unwrap()).unwrap();
        let followed = open_room(&root);
        assert_eq!(followed.camera().unwrap().document, follow);
        assert_eq!(
            followed.camera().unwrap().bytes,
            fs::read(&followed.camera().unwrap().path).unwrap()
        );
        assert_eq!(followed.scene().frame().to_bytes(), default_frame);
        assert_eq!(followed.scene().frame().checksum(), default_checksum);
        follow
            .validate_frame(
                FrameView::of(followed.scene().frame()),
                followed.scene().index(),
            )
            .unwrap();
        let expected = follow
            .camera_for_frame(
                &follow.orbit(),
                FrameView::of(followed.scene().frame()),
                followed.scene().index(),
                (1024, 768),
            )
            .unwrap();
        assert_eq!(
            expected.target,
            followed
                .camera()
                .unwrap()
                .document
                .camera_for_frame(
                    &follow.orbit(),
                    FrameView::of(followed.scene().frame()),
                    followed.scene().index(),
                    (1024, 768),
                )
                .unwrap()
                .target
        );
        drop(followed);

        for bad_follow in ["e_ffffffffffffffffffffffffffffffff".to_string(), {
            let prepared = open_room(&root);
            let key = prepared
                .scene()
                .frame()
                .entities()
                .find(|&entity| {
                    prepared
                        .scene()
                        .frame()
                        .get::<RoomActor>(entity)
                        .unwrap()
                        .kind
                        == orr_sample::room_game::KEY
                })
                .unwrap();
            prepared.scene().index().guid(key).unwrap().to_string()
        }] {
            let invalid = sample_follow(bad_follow);
            fs::write(root.join("room.camera.json"), invalid.to_bytes().unwrap()).unwrap();
            assert!(
                try_open_room(&root).is_err(),
                "invalid follow must not admit"
            );
        }
        fs::write(root.join("room.camera.json"), b"{\"schema\":2}").unwrap();
        assert!(
            try_open_room(&root).is_err(),
            "schema 2 cannot omit its anchor"
        );

        // Projects authored before the optional camera descriptor remain readable.
        let manifest_path = root.join("orr.project.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["entry"].as_object_mut().unwrap().remove("camera");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        fs::remove_file(root.join("room.camera.json")).unwrap();
        assert!(open_room(&root).camera().is_none());
    }
}

#[cfg(all(feature = "room-checkpoint", target_os = "linux"))]
#[test]
fn checkpoint_identity_and_initial_follow_target_ignore_presentation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("checkpoint Room");
    create_room_checkpoint(&root);
    let open = || {
        PreparedProject::open_with_capabilities(
            &root,
            true,
            CheckpointSupport::MetadataOnly,
            cfg!(feature = "room-character"),
        )
        .unwrap()
    };
    let initial = open();
    let initial_digest = orr_sample::room_checkpoint::challenge_digest(&initial);
    let entity = player(initial.scene().frame());
    let guid = initial.scene().index().guid(entity).unwrap().to_string();
    let checksum = initial.scene().frame().checksum();
    drop(initial);

    let follow = sample_follow(guid);
    fs::write(root.join("room.camera.json"), follow.to_bytes().unwrap()).unwrap();
    let followed = open();
    assert_eq!(
        orr_sample::room_checkpoint::challenge_digest(&followed),
        initial_digest
    );
    let base_simulation = followed.scene().simulation().unwrap();
    assert_eq!(base_simulation.frame().checksum(), checksum);
    let camera = follow
        .camera_for_frame(
            &follow.orbit(),
            FrameView::of(base_simulation.frame()),
            followed.scene().index(),
            (1024, 768),
        )
        .unwrap();
    assert_eq!(
        camera.target[0] - follow.follow.as_ref().unwrap().offset[0],
        orr_view::fp_to_vec3(base_simulation.frame().get::<Body>(entity).unwrap().pos).x
    );
}

#[cfg(all(feature = "room-checkpoint", target_os = "linux"))]
fn create_room_checkpoint(root: &std::path::Path) {
    use orr_sample::project_create::create_room_checkpoint;
    create_room_checkpoint(
        &CreateOptions {
            output: root.to_path_buf(),
            template: "room-escape-ui-3d-v1".into(),
            seed: "follow-camera-checkpoint".into(),
        },
        "12345678-1234-4234-8234-123456789abc",
    )
    .unwrap();
}
