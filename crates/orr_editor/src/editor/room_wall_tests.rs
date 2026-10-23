use super::*;
use orr_sample::room_game::{RoomActor, WALL};
use orr_sample::room_project::PreparedProject;

struct Fixture {
    _temporary: tempfile::TempDir,
    editor: Editor,
}

fn fixture(bind_wall: bool) -> Fixture {
    let temporary = tempfile::tempdir().expect("temporary Room project parent");
    let root = temporary.path().join("room wall project");
    let prepared = orr_sample::windows_project::create_room_project(&root, "room-wall-tests")
        .expect("create admitted Room project");
    if bind_wall {
        let wall_guid = prepared
            .scene()
            .index()
            .iter()
            .find_map(|(guid, entity)| {
                prepared
                    .scene()
                    .frame()
                    .get::<RoomActor>(entity)
                    .is_some_and(|actor| actor.kind == WALL)
                    .then(|| guid.to_string())
            })
            .expect("starter wall GUID");
        let mut document = prepared.models().document.clone();
        let binding = document
            .bindings
            .values()
            .next()
            .expect("starter model binding")
            .clone();
        document.bindings.insert(wall_guid, binding);
        std::fs::write(
            &prepared.models().path,
            serde_json::to_vec(&document).expect("serialize Room model bindings"),
        )
        .expect("write test sidecar");
    }
    let prepared = PreparedProject::open(&root).expect("admit Room test project");
    let (_, scene_path, scene, _) = prepared.into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: scene_path,
        text: scene.text().to_owned(),
        listen: None,
        debug_hooks: false,
    })
    .expect("start Room test host");
    editor.sync();
    Fixture {
        _temporary: temporary,
        editor,
    }
}

fn field(editor: &mut Editor, guid: &str, component: &str, path: &str) -> Value {
    editor
        .field_of(
            &Target::parse(guid).expect("persistent GUID"),
            component,
            path,
        )
        .expect("read Room wall field")
}

#[test]
fn room_wall_update_preserves_y_rotation_and_non_geometry_fields_and_undoes_as_one_entry() {
    let mut fixture = fixture(false);
    let wall = fixture
        .editor
        .read_room_walls()
        .expect("read starter walls")
        .into_iter()
        .next()
        .expect("starter wall");
    let original_checksum = fixture.editor.checksum();
    let original_rotation = field(
        &mut fixture.editor,
        &wall.guid,
        "orr_physics3d::Body",
        "rot",
    );
    let original_velocity = field(
        &mut fixture.editor,
        &wall.guid,
        "orr_physics3d::Body",
        "vel",
    );
    let original_friction = field(
        &mut fixture.editor,
        &wall.guid,
        "orr_physics3d::Collider",
        "friction",
    );
    let original_body_position = field(
        &mut fixture.editor,
        &wall.guid,
        "orr_physics3d::Body",
        "pos",
    );
    let original_half = field(
        &mut fixture.editor,
        &wall.guid,
        "orr_physics3d::Collider",
        "shape.half_extents",
    );

    fixture
        .editor
        .update_room_wall(
            &wall.guid,
            "renamed wall",
            [FP::from_int(12), FP::from_int(12)],
            wall.half_extents,
        )
        .expect("update Room wall");
    fixture.editor.sync();
    let updated = fixture
        .editor
        .read_room_walls()
        .expect("read updated walls")
        .into_iter()
        .find(|candidate| candidate.guid == wall.guid)
        .expect("updated wall remains");
    assert_eq!(updated.name, "renamed wall");
    assert_eq!(updated.center, [FP::from_int(12), FP::from_int(12)]);
    assert_eq!(updated.half_extents, wall.half_extents);
    assert_eq!(
        field(
            &mut fixture.editor,
            &wall.guid,
            "orr_physics3d::Body",
            "rot"
        ),
        original_rotation
    );
    assert_eq!(
        field(
            &mut fixture.editor,
            &wall.guid,
            "orr_physics3d::Body",
            "vel"
        ),
        original_velocity
    );
    assert_eq!(
        field(
            &mut fixture.editor,
            &wall.guid,
            "orr_physics3d::Collider",
            "friction"
        ),
        original_friction
    );
    let Value::Vec3(before_position) = original_body_position else {
        panic!("Body.pos is Vec3")
    };
    let Value::Vec3(after_position) = field(
        &mut fixture.editor,
        &wall.guid,
        "orr_physics3d::Body",
        "pos",
    ) else {
        panic!("Body.pos is Vec3")
    };
    assert_eq!(after_position.y, before_position.y);
    let Value::Vec3(before_half) = original_half else {
        panic!("box half extents are Vec3")
    };
    let Value::Vec3(after_half) = field(
        &mut fixture.editor,
        &wall.guid,
        "orr_physics3d::Collider",
        "shape.half_extents",
    ) else {
        panic!("box half extents are Vec3")
    };
    assert_eq!(after_half.y, before_half.y);
    let updated_checksum = fixture.editor.checksum();
    assert_ne!(updated_checksum, original_checksum);

    assert!(fixture.editor.undo());
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), original_checksum);
    assert!(fixture.editor.redo());
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), updated_checksum);
}

#[test]
fn invalid_room_wall_update_rolls_back_without_changing_the_scene() {
    let mut fixture = fixture(false);
    let wall = fixture.editor.read_room_walls().unwrap().remove(0);
    let checksum = fixture.editor.checksum();

    assert!(fixture
        .editor
        .update_room_wall(
            &wall.guid,
            "outside bounds",
            [FP::from_int(17), FP::from_int(12)],
            wall.half_extents,
        )
        .is_err());
    fixture.editor.sync();

    assert_eq!(fixture.editor.checksum(), checksum);
    assert!(!fixture.editor.sim().in_tx);
    assert!(!fixture.editor.history().in_tx);
}

#[test]
fn unchanged_room_wall_update_does_not_create_history() {
    let mut fixture = fixture(false);
    let guid = fixture
        .editor
        .spawn_room_wall(
            "unchanged wall",
            [FP::from_int(10), FP::from_int(10)],
            [FP::HALF, FP::HALF],
        )
        .expect("create named wall");
    fixture.editor.sync();
    let wall = fixture
        .editor
        .read_room_walls()
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.guid == guid)
        .expect("named wall");
    let checksum = fixture.editor.checksum();
    let can_undo = fixture.editor.history().can_undo;

    fixture
        .editor
        .update_room_wall(&wall.guid, &wall.name, wall.center, wall.half_extents)
        .expect("unchanged wall update");
    fixture.editor.sync();

    assert_eq!(fixture.editor.checksum(), checksum);
    assert_eq!(fixture.editor.history().can_undo, can_undo);
}

#[test]
fn duplicate_room_wall_uses_a_fresh_guid_and_clones_non_position_fields() {
    let mut fixture = fixture(false);
    let wall = fixture.editor.read_room_walls().unwrap().remove(0);
    let original_rotation = field(
        &mut fixture.editor,
        &wall.guid,
        "orr_physics3d::Body",
        "rot",
    );
    let original_velocity = field(
        &mut fixture.editor,
        &wall.guid,
        "orr_physics3d::Body",
        "vel",
    );
    let checksum = fixture.editor.checksum();

    let duplicate_guid = fixture
        .editor
        .duplicate_room_wall(
            &wall.guid,
            "wall copy",
            [FP::from_int(15), FP::from_int(15)],
        )
        .expect("duplicate Room wall");
    assert_ne!(duplicate_guid, wall.guid);
    fixture.editor.sync();
    let duplicate = fixture
        .editor
        .read_room_walls()
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.guid == duplicate_guid)
        .expect("fresh wall GUID");
    assert_eq!(duplicate.name, "wall copy");
    assert_eq!(duplicate.center, [FP::from_int(15), FP::from_int(15)]);
    assert_eq!(duplicate.half_extents, wall.half_extents);
    assert_eq!(
        field(
            &mut fixture.editor,
            &duplicate_guid,
            "orr_physics3d::Body",
            "rot"
        ),
        original_rotation
    );
    assert_eq!(
        field(
            &mut fixture.editor,
            &duplicate_guid,
            "orr_physics3d::Body",
            "vel"
        ),
        original_velocity
    );
    let duplicate_checksum = fixture.editor.checksum();
    assert_ne!(duplicate_checksum, checksum);

    assert!(fixture.editor.undo());
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), checksum);
    assert!(fixture.editor.redo());
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), duplicate_checksum);
}

#[test]
fn wall_removal_requires_a_saved_project_and_refuses_a_bound_guid() {
    let mut fixture = fixture(true);
    let wall = fixture.editor.read_room_walls().unwrap().remove(0);
    let checksum = fixture.editor.checksum();

    let error = fixture.editor.remove_room_wall(&wall.guid).unwrap_err();

    assert!(error.contains("model binding"));
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), checksum);
    assert!(fixture
        .editor
        .read_room_walls()
        .unwrap()
        .iter()
        .any(|candidate| candidate.guid == wall.guid));
    assert!(!fixture.editor.sim().in_tx);
    assert!(!fixture.editor.history().in_tx);
}

#[test]
fn unsaved_unbound_wall_can_be_removed_and_undone_without_saving_between_edits() {
    let mut fixture = fixture(false);
    let guid = fixture
        .editor
        .spawn_room_wall(
            "unsaved wall",
            [FP::from_int(10), FP::from_int(10)],
            [FP::HALF, FP::HALF],
        )
        .expect("create unsaved wall");
    fixture.editor.sync();
    let created_checksum = fixture.editor.checksum();

    fixture
        .editor
        .remove_room_wall(&guid)
        .expect("remove unsaved wall");
    fixture.editor.sync();
    let removed_checksum = fixture.editor.checksum();
    assert_ne!(removed_checksum, created_checksum);
    assert!(!fixture
        .editor
        .read_room_walls()
        .unwrap()
        .iter()
        .any(|wall| wall.guid == guid));

    assert!(fixture.editor.undo());
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), created_checksum);
    assert!(fixture
        .editor
        .read_room_walls()
        .unwrap()
        .iter()
        .any(|wall| wall.guid == guid));
    assert!(fixture.editor.redo());
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), removed_checksum);
}

#[test]
fn room_wall_changes_are_refused_during_play() {
    let mut fixture = fixture(false);
    let wall = fixture.editor.read_room_walls().unwrap().remove(0);
    assert!(fixture.editor.start_play());

    assert!(fixture
        .editor
        .update_room_wall(&wall.guid, "play edit", wall.center, wall.half_extents,)
        .is_err());
}

#[test]
fn unbound_wall_removal_admits_the_saved_project_and_is_undoable() {
    let mut fixture = fixture(false);
    let wall = fixture.editor.read_room_walls().unwrap().remove(0);
    let original_checksum = fixture.editor.checksum();

    fixture
        .editor
        .remove_room_wall(&wall.guid)
        .expect("remove unbound wall");
    fixture.editor.sync();

    assert!(!fixture
        .editor
        .read_room_walls()
        .unwrap()
        .iter()
        .any(|candidate| candidate.guid == wall.guid));
    let removed_checksum = fixture.editor.checksum();
    assert_ne!(removed_checksum, original_checksum);
    assert!(fixture.editor.undo());
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), original_checksum);
    assert!(fixture
        .editor
        .read_room_walls()
        .unwrap()
        .iter()
        .any(|candidate| candidate.guid == wall.guid));
    assert!(fixture.editor.redo());
    fixture.editor.sync();
    assert_eq!(fixture.editor.checksum(), removed_checksum);
}

#[test]
fn generic_delete_cannot_bypass_room_wall_admission() {
    let mut fixture = fixture(false);
    let wall = fixture.editor.read_room_walls().unwrap().remove(0);
    let original_checksum = fixture.editor.checksum();
    fixture.editor.select(Some(
        Target::parse(&wall.guid).expect("persistent wall GUID"),
    ));

    assert!(!fixture.editor.delete_selected());
    fixture.editor.sync();

    assert_eq!(fixture.editor.checksum(), original_checksum);
    assert!(fixture
        .editor
        .read_room_walls()
        .unwrap()
        .iter()
        .any(|candidate| candidate.guid == wall.guid));
}
