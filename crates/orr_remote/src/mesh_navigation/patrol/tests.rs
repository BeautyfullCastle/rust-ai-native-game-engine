use super::*;
use crate::mesh_navigation::{authoring::create_mesh_scene, AgentSpec, MeshScenePin, PIN_NAME};
use bytemuck::Zeroable;
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
fn point(x: i32, z: i32) -> [FP; 3] {
    [FP::from_int(x), FP::ZERO, FP::from_int(z)]
}
fn graph() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[point(0, 0), point(0, 4), point(4, 0), point(4, 4)],
        &[[0, 1, 2], [2, 1, 3]],
        AgentProfile::default(),
    )
    .unwrap()
}
fn document() -> EditorDoc {
    create_mesh_scene(
        &graph(),
        AgentSpec {
            start: point(1, 1),
            goal: point(3, 3),
            max_vertical: FP::ZERO,
            distance_per_tick: fp!(0.25),
        },
    )
    .unwrap()
}
fn config() -> MeshPatrol {
    let mut c = MeshPatrol::zeroed();
    c.enabled = 1;
    c.count = 2;
    c.looped = 1;
    c.wait_ticks = 2;
    c.points[0] = point(1, 2);
    c.points[1] = point(3, 2);
    c.graph_revision = graph().revision();
    c
}
#[test]
fn reflected_patch_and_atomic_authoring_admit_saved_route_without_runtime_authority() {
    let mut doc = document();
    assert!(from_view(FrameView::of(doc.frame())).unwrap().is_none());
    let c = config();
    let view = mesh_from_view(FrameView::of(doc.frame())).unwrap();
    let request = patch_request(&c, &view).unwrap();
    assert_eq!(request["name"], PATROL_NAME);
    assert_eq!(request["path"], "");
    assert_eq!(request["value"]["points"][0], json!([1, 0, 2]));
    assert!(doc.types().get(RUN_NAME).is_none());
    set_patrol(&mut doc, &c).unwrap();
    let read = from_view(FrameView::of(doc.frame())).unwrap().unwrap();
    assert_eq!(read.config, c);
    assert_eq!(read.run, MeshPatrolRun::zeroed());
    assert_eq!(doc.frame().singleton::<MeshScenePin>().goal, c.points[0]);
    let bytes = doc.frame().to_bytes();
    let yaml = doc.save_yaml();
    let reopened = super::super::mesh_doc_from_yaml(&yaml).unwrap();
    assert_eq!(reopened.frame().to_bytes(), bytes);
    assert_eq!(
        from_view(FrameView::of(reopened.frame()))
            .unwrap()
            .unwrap()
            .config,
        c
    );
}
#[test]
fn invalid_later_segments_flags_tails_and_graph_pins_leave_document_and_frame_unchanged() {
    let mut doc = document();
    let c = config();
    set_patrol(&mut doc, &c).unwrap();
    let frame = doc.frame().to_bytes();
    let yaml = doc.save_yaml();
    for change in 0..8 {
        let mut invalid = c;
        match change {
            0 => invalid.points[1] = point(99, 99),
            1 => invalid.points[7] = point(1, 1),
            2 => invalid.wait_ticks = 3601,
            3 => invalid.looped = 2,
            4 => invalid.count = 9,
            5 => invalid.enabled = 2,
            6 => invalid.graph_revision[0] ^= 1,
            _ => {
                invalid.enabled = 0;
            }
        }
        assert!(set_patrol(&mut doc, &invalid).is_err(), "case {change}");
        assert_eq!(doc.frame().to_bytes(), frame);
        assert_eq!(doc.save_yaml(), yaml);
    }
}
#[test]
fn patrol_undo_redo_disable_and_reopen_are_one_authoring_transaction() {
    let mut doc = document();
    let before = doc.frame().to_bytes();
    let c = config();
    set_patrol(&mut doc, &c).unwrap();
    let enabled = doc.frame().to_bytes();
    doc.undo().unwrap();
    assert_eq!(doc.frame().to_bytes(), before);
    doc.redo().unwrap();
    assert_eq!(doc.frame().to_bytes(), enabled);
    set_patrol(&mut doc, &MeshPatrol::zeroed()).unwrap();
    assert!(from_view(FrameView::of(doc.frame())).unwrap().is_none());
    doc.undo().unwrap();
    assert_eq!(doc.frame().to_bytes(), enabled);
}
#[test]
fn runtime_singleton_is_not_an_authored_reflected_scene_component() {
    let yaml="schema: orr.scene/1\nsingletons:\n  MeshPatrolRun: {cursor: 1, wait_remaining: 0, paused: 0, completed: 0, arrived: 0, reserved: [0, 0, 0]}\nentities: {}\n";
    assert!(super::super::mesh_doc_from_yaml(yaml).is_err());
    let doc = document();
    let mut frame = doc.frame().clone();
    let mut run = MeshPatrolRun::zeroed();
    run.reserved[0] = 1;
    frame.set_singleton(run);
    assert!(from_view(FrameView::of(&frame)).is_err());
}
#[test]
fn existing_pin_edits_preserve_patrol_and_stale_graph_replacement_rejects_atomically() {
    let mut doc = document();
    let c = config();
    set_patrol(&mut doc, &c).unwrap();
    let mut pin = *doc.frame().singleton::<MeshScenePin>();
    pin.distance_per_tick = fp!(0.125);
    let value = doc
        .types()
        .get(PIN_NAME)
        .unwrap()
        .read(bytemuck::bytes_of(&pin));
    doc.apply(
        Op::SetSingletonField {
            singleton: PIN_NAME.into(),
            path: String::new(),
            value,
        },
        Origin::User,
    )
    .unwrap();
    assert_eq!(
        from_view(FrameView::of(doc.frame()))
            .unwrap()
            .unwrap()
            .config,
        c
    );
    let frame = doc.frame().to_bytes();
    let yaml = doc.save_yaml();
    let graph = StaticMeshGraph::bake(
        &[point(0, 0), point(0, 5), point(5, 0), point(5, 5)],
        &[[0, 1, 2], [2, 1, 3]],
        AgentProfile::default(),
    )
    .unwrap();
    let pin = MeshScenePin::new(&graph, pin.spec()).unwrap();
    let value = doc
        .types()
        .get(PIN_NAME)
        .unwrap()
        .read(bytemuck::bytes_of(&pin));
    assert!(doc
        .apply_atomic_batch(
            "replace pinned graph",
            vec![Op::SetSingletonField {
                singleton: PIN_NAME.into(),
                path: String::new(),
                value
            }],
            Origin::User
        )
        .is_err());
    assert_eq!(doc.frame().to_bytes(), frame);
    assert_eq!(doc.save_yaml(), yaml);
}
