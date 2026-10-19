use super::*;
use crate::{backend::HostSpec, game::EditorGame};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_sample::{
    room_game::{KEY, PLAYER, RoomActor},
    room_project::{PreparedProject, PreparedScene},
};
use std::{fs, path::Path};

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    scene: PathBuf,
    editor: Editor,
}

fn create_admitted_fixture(root: &Path) {
    fs::create_dir(root).unwrap();
    let text = include_str!("../tests/fixtures/room_initial.scene.yaml").to_owned();
    fs::write(root.join("room.scene.yaml"), &text).unwrap();
    fs::write(
        root.join("room.camera.json"),
        orr_sample::room_camera::Document::readable_default()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    orr_package::Project::open_for_install(
        root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[source])
    .unwrap();
    let loaded =
        model_bindings::load_asset(root, "sample-imported-scene", "foreground.glb").unwrap();
    let binding = Binding::from_asset(
        "sample-imported-scene".into(),
        "foreground.glb".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let scene = PreparedScene::parse(&text).unwrap();
    let bindings = scene
        .frame()
        .entities()
        .filter(|entity| {
            scene
                .frame()
                .get::<RoomActor>(*entity)
                .is_some_and(|actor| [PLAYER, KEY].contains(&actor.kind))
        })
        .map(|entity| {
            (
                scene.index().guid(entity).unwrap().to_string(),
                binding.clone(),
            )
        })
        .collect();
    fs::write(
        root.join("room.models.json"),
        serde_json::to_vec(&Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings,
        })
        .unwrap(),
    )
    .unwrap();
}

fn fixture() -> Fixture {
    let temporary = tempfile::tempdir().expect("temporary Room project parent");
    let root = temporary.path().join("wall stamp project");
    create_admitted_fixture(&root);
    let (root, scene, prepared_scene, _) = PreparedProject::open(&root)
        .expect("admit Room test project")
        .into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: scene.clone(),
        text: prepared_scene.text().to_owned(),
        listen: None,
        debug_hooks: false,
    })
    .expect("start Room test host");
    editor.sync();
    Fixture {
        _temporary: temporary,
        root,
        scene,
        editor,
    }
}

fn add_source_wall(editor: &mut Editor) -> String {
    let guid = editor
        .spawn_room_wall(
            "stamp source",
            [FP::from_int(10), FP::from_int(-10)],
            [FP::HALF, FP::HALF],
        )
        .expect("create source wall");
    editor.sync();
    guid
}

#[test]
fn capture_and_instantiate_preserve_components_and_commit_as_one_undo_entry() {
    let mut fixture = fixture();
    let source_guid = add_source_wall(&mut fixture.editor);
    let source_row = fixture
        .editor
        .room_wall_row(&source_guid)
        .expect("read source wall");
    let source_components = source_row.components.clone();
    let source_checksum = fixture.editor.checksum();
    let source_history = fixture.editor.history().entries.len();
    let selected = vec![source_guid.clone()];

    let template = fixture
        .editor
        .capture_room_wall_stamp(&selected)
        .expect("capture selected Room wall");
    let encoded = template.encode().expect("encode captured stamp");
    let decoded =
        crate::room_wall_stamp::Template::decode(&encoded).expect("decode captured stamp");
    assert_eq!(decoded, template);
    assert_eq!(template.walls().len(), 1);
    assert_eq!(template.walls()[0].components, source_components);
    assert_eq!(fixture.editor.checksum(), source_checksum);
    assert_eq!(fixture.editor.history().entries.len(), source_history);

    let before_instantiate = fixture.editor.history().entries.len();
    let created = fixture
        .editor
        .instantiate_room_wall_stamp(&template, [FP::from_int(2), FP::ZERO])
        .expect("instantiate wall stamp");
    assert_eq!(created.len(), 1);
    let created_guid = &created[0];
    assert_ne!(created_guid, &source_guid);
    fixture.editor.sync();
    assert_eq!(
        fixture.editor.history().entries.len(),
        before_instantiate + 1
    );

    let created_row = fixture
        .editor
        .room_wall_row(created_guid)
        .expect("read stamped wall");
    assert_eq!(created_row.wall.name, "stamp source");
    assert_eq!(
        created_row.wall.center,
        [FP::from_int(12), FP::from_int(-10)]
    );
    assert_eq!(created_row.wall.half_extents, source_row.wall.half_extents);
    let mut expected_components = source_components;
    let source_position = source_row.position;
    let expected_position = FPVec3::new(
        source_position.x + FP::from_int(2),
        source_position.y,
        source_position.z,
    );
    expected_components[BODY]["pos"] = value_to_json(&Value::Vec3(expected_position));
    assert_eq!(created_row.components, expected_components);
    let instantiated_checksum = fixture.editor.checksum();
    assert_ne!(instantiated_checksum, source_checksum);

    assert!(fixture.editor.undo());
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), source_checksum);
    assert!(fixture
        .editor
        .room_wall_rows()
        .unwrap()
        .iter()
        .all(|row| row.wall.guid.as_str() != created_guid.as_str()));
    assert!(fixture.editor.redo());
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), instantiated_checksum);

    assert!(fixture.editor.save());
    fixture.editor.sync();
    let (reopen_root, reopen_scene, prepared_scene, _) = PreparedProject::open(&fixture.root)
        .expect("reopen saved Room project")
        .into_parts();
    assert_eq!(
        reopen_root.canonicalize().unwrap(),
        fixture.root.canonicalize().unwrap()
    );
    assert_eq!(
        reopen_scene.canonicalize().unwrap(),
        fixture.scene.canonicalize().unwrap()
    );
    let mut reopened = Editor::start(&HostSpec::PreparedRoom {
        scene: reopen_scene,
        text: prepared_scene.text().to_owned(),
        listen: None,
        debug_hooks: false,
    })
    .expect("start reopened Room test host");
    reopened.sync();
    assert!(reopened
        .read_room_walls()
        .unwrap()
        .iter()
        .any(|wall| wall.guid.as_str() == created_guid.as_str()
            && wall.center == [FP::from_int(12), FP::from_int(-10)]));
}

#[test]
fn invalid_selection_offsets_and_wall_count_refusals_preserve_document_and_history() {
    let mut fixture = fixture();
    let source_guid = add_source_wall(&mut fixture.editor);
    let valid_selection = vec![source_guid.clone()];
    let checksum = fixture.editor.checksum();
    let history_len = fixture.editor.history().entries.len();

    assert!(fixture
        .editor
        .capture_room_wall_stamp(&["not-a-guid".into()])
        .is_err());
    assert!(fixture
        .editor
        .capture_room_wall_stamp(&[source_guid.clone(), source_guid.clone()])
        .is_err());
    let template = fixture
        .editor
        .capture_room_wall_stamp(&valid_selection)
        .expect("capture source wall");

    assert!(fixture
        .editor
        .instantiate_room_wall_stamp(&template, [FP::MAX, FP::ZERO])
        .is_err());
    // ERP notifications may lag; refresh caches before observing rollback.
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), checksum);
    assert_eq!(fixture.editor.history().entries.len(), history_len);
    assert!(!fixture.editor.history().in_tx);
    assert!(!fixture.editor.sim().in_tx);

    assert!(fixture
        .editor
        .instantiate_room_wall_stamp(&template, [FP::from_int(20), FP::ZERO])
        .is_err());
    // ERP notifications may lag; refresh caches before observing rollback.
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), checksum);
    assert_eq!(fixture.editor.history().entries.len(), history_len);
    assert!(!fixture.editor.history().in_tx);
    assert!(!fixture.editor.sim().in_tx);

    let source_components = template.walls()[0].components.clone();
    let oversized = crate::room_wall_stamp::Template::capture(
        (0..orr_sample::room_game::MAX_WALLS)
            .map(|index| (format!("wall-{index}"), source_components.clone()))
            .collect(),
    )
    .expect("construct bounded maximum-sized stamp");
    assert!(fixture
        .editor
        .instantiate_room_wall_stamp(&oversized, [FP::ZERO; 2])
        .is_err());
    // ERP notifications may lag; refresh caches before observing rollback.
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), checksum);
    assert_eq!(fixture.editor.history().entries.len(), history_len);
    assert!(!fixture.editor.history().in_tx);
    assert!(!fixture.editor.sim().in_tx);
    assert_eq!(
        fixture
            .editor
            .room_wall_rows()
            .unwrap()
            .iter()
            .filter(|row| row.wall.guid.as_str() == source_guid.as_str())
            .count(),
        1
    );
}

#[test]
fn capture_rejects_nonwall_guids_and_nonroom_games_without_mutation() {
    let mut room = fixture();
    let source_guid = add_source_wall(&mut room.editor);
    let (player_target, _) = &room
        .editor
        .room_layout_actors()
        .expect("read Room layout actors")[0];
    let Target::Guid(player_guid) = player_target else {
        panic!("Room layout actor has persistent GUID")
    };
    let checksum = room.editor.checksum();
    let history_len = room.editor.history().entries.len();
    assert!(room
        .editor
        .capture_room_wall_stamp(&[player_guid.to_string()])
        .is_err());
    assert_eq!(room.editor.checksum(), checksum);
    assert_eq!(room.editor.history().entries.len(), history_len);

    let physics_scene = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenes/physics_demo.scene.yaml")
        .canonicalize()
        .expect("resolve existing physics fixture");
    let mut physics = Editor::open_game(&physics_scene, EditorGame::PhysGame)
        .expect("start existing PhysGame fixture");
    physics.sync();
    let physics_checksum = physics.checksum();
    let physics_history_len = physics.history().entries.len();
    assert!(physics.capture_room_wall_stamp(&[source_guid]).is_err());
    assert_eq!(physics.checksum(), physics_checksum);
    assert_eq!(physics.history().entries.len(), physics_history_len);
}
