//! Actual Editor widgets/save/reopen and production CPU placement; no GPU window.
use super::*;
use orr_model_bindings::model_bindings::{Document as Bindings, LocalTransform};
use orr_sample::room_character::{Document, ForwardAxis};
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click();
    settle(h);
}
fn position(app: &EditorApp, entity: orr_ecs::Entity) -> [f32; 3] {
    app.editor
        .yard_frame()
        .poses
        .iter()
        .find(|(id, _)| *id == entity)
        .unwrap()
        .1
        .pos
        .to_array()
}
fn expected_right(position: [f32; 3], local: LocalTransform) -> orr_model::animation::Matrix4 {
    let matrix = orr_model::animation::Trs {
        translation: local.translation,
        rotation: local.rotation,
        scale: local.scale,
    }
    .matrix();
    // Independent +90 degree Y rotation: (x,y,z)->(z,y,-x), then body translation.
    // This checks root-yaw-before-local composition, including the local anchor.
    std::array::from_fn(|column| {
        let value = matrix[column];
        [
            value[2] + if column == 3 { position[0] } else { 0.0 },
            value[1] + if column == 3 { position[1] } else { 0.0 },
            -value[0] + if column == 3 { position[2] } else { 0.0 },
            value[3],
        ]
    })
}
fn assert_matrix(actual: orr_model::animation::Matrix4, expected: orr_model::animation::Matrix4) {
    for column in 0..4 {
        for row in 0..4 {
            assert!(
                (actual[column][row] - expected[column][row]).abs() < 0.0001,
                "matrix[{column}][{row}] {} != {}",
                actual[column][row],
                expected[column][row]
            );
        }
    }
}
#[test]
fn facing_widgets_save_reopen_and_editor_native_placements_preserve_nonidentity_local_trs() {
    let f = Fixture::new();
    let path = f.root.join("room.models.json");
    let mut bindings: Bindings = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let player = f.guids[&PLAYER].to_string();
    let local = LocalTransform {
        translation: [0.3, 0.4, -0.2],
        rotation: [0.0, 0.24740396, 0.0, 0.9689124],
        scale: [0.75, 1.25, 1.5],
    };
    bindings.bindings.get_mut(&player).unwrap().transform = local;
    fs::write(&path, serde_json::to_vec(&bindings).unwrap()).unwrap();
    let mut h = f.app(false);
    settle(&mut h);
    wait(&mut h, "facing source coherent", |app| {
        app.editor.yard_rows_coherent()
    });
    let frame = frame_bytes(&mut h.state_mut().editor);
    let old = h.state().editor.room_character().unwrap().clone();
    if h.query_by_label("Face observed movement direction")
        .is_none()
    {
        click(&mut h, "Room character");
    }
    click(&mut h, "Face observed movement direction");
    click(&mut h, "Authored forward direction");
    click(&mut h, "Model forward +Z");
    assert_eq!(h.state().editor.room_character(), Some(&old));
    assert!(
        h.state()
            .room_character
            .as_ref()
            .unwrap()
            .has_unapplied_changes()
    );
    click(&mut h, "Apply character settings");
    let authored = h.state().editor.room_character().unwrap().clone();
    assert_eq!(authored.schema, 7);
    assert_eq!(
        authored.motion.unwrap().facing,
        Some(ForwardAxis::PositiveZ)
    );
    click(&mut h, "Undo character edit");
    assert_eq!(h.state().editor.room_character(), Some(&old));
    click(&mut h, "Redo character edit");
    assert_eq!(h.state().editor.room_character(), Some(&authored));
    click(&mut h, "Save character and model bindings");
    assert_eq!(frame_bytes(&mut h.state_mut().editor), frame);
    drop(h);
    assert_eq!(
        Document::parse(&fs::read(f.root.join("room.character.json")).unwrap()).unwrap(),
        authored
    );
    let prepared = PreparedProject::open_with_capabilities(
        &f.root,
        orr_sample::room_project::RoomProjectCapabilities {
            lighting: false,
            character: true,
        },
    )
    .unwrap();
    assert_eq!(
        prepared.models().document.bindings[&player].transform,
        local
    );
    let entity = prepared
        .scene()
        .index()
        .entity(&Guid::parse(&player).unwrap())
        .unwrap();
    let mut h = f.app(false);
    settle(&mut h);
    assert_eq!(h.state().editor.room_character(), Some(&authored));
    let mut cursor = orr_sample::room_character::PlaybackCursor::default();
    orr_sample::room_view::observe_character(
        &mut cursor,
        h.state().editor.snapshot().unwrap().predicted(),
        prepared.scene().index(),
        prepared.models(),
        Default::default(),
    )
    .unwrap();
    let start = position(h.state(), entity);
    click(&mut h, orr_editor::app::LBL_PLAY);
    wait(&mut h, "live facing play", |app| {
        app.editor.can_take_control()
    });
    click(&mut h, "Take control");
    wait(&mut h, "facing input claim", |app| {
        app.editor.input_phase() == Phase::Active
    });
    // Explicitly observe the production Editor placement at the initial Play
    // snapshot; this CPU harness has no GPU draw to perform that observation.
    assert_eq!(
        h.state()
            .models
            .animated_placements(&h.state().editor)
            .unwrap()
            .len(),
        1
    );
    let observed_tick = h.state().editor.snapshot().unwrap().predicted().tick();
    key(&mut h, egui::Key::D, true);
    // No animated_placements calls while the host advances: a CPU renderer
    // observing only received snapshots must turn from the forward-gap net XZ.
    wait(
        &mut h,
        "observed +X movement across skipped render observations",
        |app| {
            app.editor.yard_rows_coherent()
                && app.editor.snapshot().unwrap().predicted().tick() >= observed_tick + 4
                && position(app, entity)[0] > start[0] + 0.05
        },
    );
    key(&mut h, egui::Key::D, false);
    settle(&mut h);
    wait(&mut h, "facing placement coherent", |app| {
        app.editor.yard_rows_coherent()
    });
    let app = h.state();
    let frame = app.editor.snapshot().unwrap().predicted();
    assert!(frame.tick() >= observed_tick + 4);
    orr_sample::room_view::observe_character(
        &mut cursor,
        frame,
        prepared.scene().index(),
        prepared.models(),
        Default::default(),
    )
    .unwrap();
    let native = orr_sample::room_view::observed_character_placement(
        frame,
        prepared.scene().index(),
        prepared.models(),
        &cursor,
    )
    .unwrap()
    .unwrap();
    let editor = app
        .models
        .animated_placements(&app.editor)
        .unwrap()
        .into_iter()
        .find(|placed| placed.entity == entity)
        .unwrap();
    let expected = expected_right(position(app, entity), local);
    assert_matrix(editor.instance, expected);
    assert_matrix(native.transform, expected);
    assert_eq!(
        app.editor
            .yard_frame()
            .poses
            .iter()
            .find(|(id, _)| *id == entity)
            .unwrap()
            .1
            .rot
            .to_array(),
        [0.0, 0.0, 0.0, 1.0]
    );
}
