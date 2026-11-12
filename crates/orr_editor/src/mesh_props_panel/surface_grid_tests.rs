use super::*;
use crate::{game::EditorGame, mesh_art::Snapshot, model::Mode};
use orr_fp::fp;
use orr_navigation::AgentProfile;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

fn graph(vertices: &[[FP; 3]], faces: &[[u32; 3]]) -> StaticMeshGraph {
    StaticMeshGraph::bake(
        vertices,
        faces,
        AgentProfile {
            max_slope: fp!(4),
            ..Default::default()
        },
    )
    .unwrap()
}

fn flat_graph(height: FP) -> StaticMeshGraph {
    graph(
        &[
            [FP::ZERO, height, FP::ZERO],
            [fp!(32), height, FP::ZERO],
            [FP::ZERO, height, fp!(32)],
            [fp!(32), height, fp!(32)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
    )
}

fn snapshot(graph: &StaticMeshGraph) -> Snapshot {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    Snapshot::from_package(&root, "foreground.glb", "LICENSE.txt", graph.revision()).unwrap()
}

fn document(graph: &StaticMeshGraph, position: [f32; 3]) -> Document {
    Document {
        props: vec![Prop {
            id: 1,
            name: "Source".into(),
            art: snapshot(graph).with_transform(position, 0.5).unwrap(),
            #[cfg(feature = "mesh-prop-shading")]
            shading: None,
        }],
    }
}

fn config(columns: usize, rows: usize) -> Config {
    Config::parse(
        &columns.to_string(),
        &rows.to_string(),
        &["1".into(), "1".into()],
        "4",
        "0",
    )
    .unwrap()
}

fn document_state(document: &Document) -> serde_json::Value {
    serde_json::Value::Array(
        document
            .props
            .iter()
            .map(|prop| {
                let value = serde_json::json!({
                    "id": prop.id, "name": prop.name,
                    "art": prop.art.encode_embedded("mesh.scene.yaml").unwrap(),
                });
                #[cfg(feature = "mesh-prop-shading")]
                let value = {
                    let mut value = value;
                    value["shading"] = serde_json::to_value(&prop.shading).unwrap();
                    value
                };
                value
            })
            .collect(),
    )
}

fn panel_state(panel: &Panel) -> serde_json::Value {
    serde_json::json!({
        "document": document_state(&panel.document),
        "history": panel.history.iter().map(|entry| (entry.id, document_state(&entry.document))).collect::<Vec<_>>(),
        "cursor": panel.cursor, "next_history": panel.next_history_id,
        "next_prop": panel.next_prop_id, "selected": panel.selected,
        "saved_history": panel.saved_history_id,
    })
}

fn editor_panel() -> (tempfile::TempDir, Editor, Panel) {
    let directory = tempfile::tempdir().unwrap();
    let graph = flat_graph(FP::ZERO);
    let mut scene_document = orr_remote::mesh_navigation::authoring::create_mesh_scene(
        &graph,
        orr_remote::mesh_navigation::AgentSpec {
            start: [fp!(0.25), FP::ZERO, fp!(0.25)],
            goal: [fp!(1.75), FP::ZERO, fp!(0.25)],
            max_vertical: FP::ZERO,
            distance_per_tick: fp!(0.25),
        },
    )
    .unwrap();
    let scene = directory
        .path()
        .canonicalize()
        .unwrap()
        .join("surface-grid.scene.yaml");
    std::fs::write(&scene, scene_document.save_yaml()).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::MeshNavigation).unwrap();
    editor.sync();
    let mut panel = Panel::default();
    panel.sync(&editor);
    panel.reset_history(document(&graph, [0.25, 0.0, 0.25]), Some(0));
    panel.selected = Some(1);
    panel.copy_selection_fields();
    assert!(panel.surface_grid_context(&editor, false).is_ok());
    (directory, editor, panel)
}

#[test]
fn literal_slope_heights_preserve_source_art_rotation_scale_tint_and_materials() {
    let graph = graph(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(8), fp!(4), FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(8)],
            [fp!(8), fp!(4), fp!(8)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
    );
    let mut original = document(&graph, [0.25, 1.75, 0.25]);
    let material = original.props[0].art.source_model_arc().source().primitives[0].material;
    let mut slot = crate::mesh_material_slots::Settings::identity(
        material,
        &original.props[0].art.source_model_arc().source().materials[material as usize],
    );
    slot.tint = [0.8, 0.7, 0.6, 1.0];
    original.props[0].art = original.props[0]
        .art
        .clone()
        .with_rotation(crate::mesh_rotation::from_degrees([17.0, 31.0, 53.0]).unwrap())
        .unwrap()
        .with_tint([0.6, 0.7, 0.8, 1.0])
        .unwrap()
        .with_surface(Some(crate::mesh_surface::Settings {
            material,
            uv_offset: [0.25, 0.5],
            uv_scale: [2.0, 3.0],
            uv_quarter_turns: 1,
            pivot: [0.125, 0.25, 0.375],
        }))
        .unwrap()
        .with_material_slots(vec![slot])
        .unwrap();
    #[cfg(feature = "mesh-prop-shading")]
    {
        original.props[0].shading = Some(
            crate::mesh_prop_shading::Settings::new(
                vec![crate::mesh_prop_shading::Slot {
                    material,
                    metallic: 0.7,
                    roughness: 0.3,
                }],
                original.props[0].art.source_model_arc(),
            )
            .unwrap(),
        );
    }
    let before = document_state(&original);
    let stamped = stamp_prop_surface_grid(&original, 1, 1, &graph, config(3, 2)).unwrap();
    let expected = [
        [0.25, 1.75, 0.25],
        [1.25, 0.625, 0.25],
        [2.25, 1.125, 0.25],
        [0.25, 0.125, 1.25],
        [1.25, 0.625, 1.25],
        [2.25, 1.125, 1.25],
    ];
    assert_eq!(document_state(&original), before);
    assert_eq!(document_state(&stamped)[0], before[0]);
    assert_eq!(
        stamped
            .props
            .iter()
            .map(|prop| prop.id)
            .collect::<BTreeSet<_>>()
            .len(),
        6
    );
    assert_eq!(
        stamped
            .props
            .iter()
            .map(|prop| &prop.name)
            .collect::<BTreeSet<_>>()
            .len(),
        6
    );
    for (prop, position) in stamped.props.iter().zip(expected) {
        assert_eq!(prop.art.offset(), position);
        assert_eq!(prop.art.scale(), original.props[0].art.scale());
        assert_eq!(prop.art.rotation(), original.props[0].art.rotation());
        assert_eq!(
            prop.art.render_identity(),
            original.props[0].art.render_identity()
        );
        assert_eq!(prop.art.tint(), original.props[0].art.tint());
        assert_eq!(prop.art.surface(), original.props[0].art.surface());
        assert_eq!(
            prop.art.material_slots(),
            original.props[0].art.material_slots()
        );
        assert_eq!(
            prop.art.source_model_arc().to_bytes().unwrap(),
            original.props[0].art.source_model_arc().to_bytes().unwrap()
        );
        #[cfg(feature = "mesh-prop-shading")]
        assert_eq!(prop.shading, original.props[0].shading);
    }
}

#[test]
fn stacked_layers_choose_nearest_height_and_lowest_triangle_key_on_ties() {
    // The upper layer starts at X=-1, giving its faces lower canonical keys.
    let graph = graph(
        &[
            [fp!(-1), fp!(4), fp!(-1)],
            [fp!(8), fp!(4), fp!(-1)],
            [fp!(-1), fp!(4), fp!(8)],
            [fp!(8), fp!(4), fp!(8)],
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(8), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(8)],
            [fp!(8), FP::ZERO, fp!(8)],
        ],
        &[[0, 2, 3], [0, 3, 1], [4, 6, 7], [4, 7, 5]],
    );
    for (source_y, expected_y) in [(0.5, 0.0), (3.5, 4.0), (2.0, 4.0)] {
        let original = document(&graph, [0.25, source_y, 0.25]);
        let stamped = stamp_prop_surface_grid(&original, 1, 2, &graph, config(2, 1)).unwrap();
        assert_eq!(stamped.props[1].art.offset(), [1.25, expected_y, 0.25]);
        assert_eq!(stamped.props[0].art.offset()[1], source_y);
    }
}

#[test]
fn all_queries_use_original_pivot_y_instead_of_chaining_previous_hits() {
    let graph = graph(
        &[
            [FP::ZERO, FP::ZERO, FP::ZERO],
            [fp!(3), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(3)],
            [fp!(3), FP::ZERO, fp!(3)],
            [FP::ZERO, fp!(8), FP::ZERO],
            [fp!(3), fp!(2), FP::ZERO],
            [FP::ZERO, fp!(8), fp!(3)],
            [fp!(3), fp!(2), fp!(3)],
        ],
        &[[0, 2, 3], [0, 3, 1], [4, 6, 7], [4, 7, 5]],
    );
    let original = document(&graph, [0.0, 3.0, 0.5]);
    let stamped = stamp_prop_surface_grid(&original, 1, 2, &graph, config(3, 1)).unwrap();
    assert_eq!(stamped.props[1].art.offset(), [1.0, 0.0, 0.5]);
    assert_eq!(stamped.props[2].art.offset(), [2.0, 4.0, 0.5]);
}

#[test]
fn missing_later_cell_rejects_entire_grid_without_mutating_source() {
    let graph = flat_graph(FP::ZERO);
    let original = document(&graph, [30.5, 0.0, 0.25]);
    let before = document_state(&original);
    let error = stamp_prop_surface_grid(&original, 1, 2, &graph, config(3, 1))
        .err()
        .unwrap();
    assert!(error.contains("[2,0]"), "{error}");
    assert_eq!(document_state(&original), before);
}

#[test]
fn vertical_limit_is_applied_before_signed_height_offset() {
    let graph = flat_graph(FP::ZERO);
    let original = document(&graph, [0.25, 1.0, 0.25]);
    for (offset, height) in [("5", 5.0), ("-2", -2.0)] {
        let config = Config::parse("2", "1", &["1".into(), "1".into()], "1", offset).unwrap();
        let stamped = stamp_prop_surface_grid(&original, 1, 2, &graph, config).unwrap();
        assert_eq!(stamped.props[1].art.offset()[1], height);
        assert_eq!(stamped.props[0].art.offset()[1], 1.0);
    }
    let config = Config::parse(
        "2",
        "1",
        &["1".into(), "1".into()],
        "0.9999847412109375",
        "-1",
    )
    .unwrap();
    assert!(stamp_prop_surface_grid(&original, 1, 2, &graph, config).is_err());
}

#[test]
fn anchor_conversion_rounds_positive_and_negative_half_q16_away_from_zero() {
    assert_eq!(anchor_coordinate(1.0 / 131_072.0).unwrap().raw(), 1);
    assert_eq!(anchor_coordinate(-1.0 / 131_072.0).unwrap().raw(), -1);
    assert_eq!(anchor_coordinate(3.0 / 131_072.0).unwrap().raw(), 2);
    assert_eq!(anchor_coordinate(-3.0 / 131_072.0).unwrap().raw(), -2);
    assert_eq!(anchor_coordinate(0.1).unwrap().raw(), 6554);
    for value in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::MAX,
        -f32::MAX,
    ] {
        assert!(anchor_coordinate(value).is_err());
    }
}

#[test]
fn non_q16_source_is_byte_identical_while_new_cells_use_exact_decimal_spacing() {
    let graph = flat_graph(FP::ZERO);
    let original = document(&graph, [0.1, 0.1, 0.1]);
    let before = document_state(&original);
    let config = Config::parse(
        "2",
        "1",
        &["0.1000137329101562499".into(), "1".into()],
        "1",
        "0",
    )
    .unwrap();
    // Decimal just below 6554.5/65536 rounds to 6554, without an f32 parse.
    assert_eq!(config.spacing[0].raw(), 6554);
    let stamped = stamp_prop_surface_grid(&original, 1, 2, &graph, config).unwrap();
    assert_eq!(document_state(&stamped)[0], before[0]);
    assert_eq!(
        stamped.props[1].art.offset(),
        [13_108.0 / 65_536.0, 0.0, 6554.0 / 65_536.0]
    );
}

#[test]
fn presentation_conversion_may_round_q16_without_claiming_exact_contact() {
    let graph = flat_graph(FP::from_raw(524_288 * FP::SCALE + 1));
    let original = document(&graph, [0.25, 524_288.0, 0.25]);
    let stamped = stamp_prop_surface_grid(&original, 1, 2, &graph, config(2, 1)).unwrap();
    assert_eq!(stamped.props[1].art.offset()[1], 524_288.0);
    assert_ne!(
        anchor_coordinate(stamped.props[1].art.offset()[1])
            .unwrap()
            .raw(),
        524_288 * FP::SCALE + 1
    );
}

#[test]
fn numeric_inputs_are_exact_bounded_and_reject_zero_rounded_spacing() {
    let parse = |spacing: &str, vertical: &str, offset: &str| {
        Config::parse("2", "1", &[spacing.into(), "1".into()], vertical, offset)
    };
    assert_eq!(
        parse("0.00000762939453125", "0", "-1000000")
            .unwrap()
            .spacing[0]
            .raw(),
        1
    );
    assert!(parse("1024", "1000000", "1000000").is_ok());
    for value in [
        "0",
        "-1",
        "0.0000076293945312499",
        "1024.0000152587890625",
        "NaN",
        "inf",
        "1e0",
        "+1",
        ".5",
        "01",
    ] {
        assert!(parse(value, "4", "0").is_err(), "{value}");
    }
    for value in ["-0.0000152587890625", "1000000.0000152587890625"] {
        assert!(parse("1", value, "0").is_err());
    }
    for value in ["-1000000.0000152587890625", "1000000.0000152587890625"] {
        assert!(parse("1", "4", value).is_err());
    }
    let at_bound = format!("1.{}", "0".repeat(46));
    assert_eq!(at_bound.len(), 48);
    assert!(parse(&at_bound, "4", "0").is_ok());
    assert!(parse(&(at_bound + "0"), "4", "0").is_err());
    for columns in ["0", "17", "-1", "1.5", &"1".repeat(49)] {
        assert!(Config::parse(columns, "2", &["1".into(), "1".into()], "4", "0").is_err());
    }
    assert!(Config::parse("1", "1", &["1".into(), "1".into()], "4", "0").is_err());
}

#[test]
fn checked_coordinates_reject_graph_and_final_art_overflow_before_f32_rounding() {
    assert!(cell_coordinate(FP::from_raw(MAX_COORDINATE_RAW), 1, FP::from_raw(1)).is_err());
    assert!(cell_coordinate(FP::ZERO, usize::MAX, FP::from_raw(i64::MAX)).is_err());
    assert!(graph_coordinate(i128::MAX).is_err());
    assert!(graph_coordinate(i128::from(i64::MAX) + 1).is_err());
    let graph = flat_graph(fp!(1000000));
    let original = document(&graph, [0.25, 1_000_000.0, 0.25]);
    let config = Config::parse(
        "2",
        "1",
        &["1".into(), "1".into()],
        "0",
        "0.0000152587890625",
    )
    .unwrap();
    let error = stamp_prop_surface_grid(&original, 1, 2, &graph, config)
        .err()
        .unwrap();
    assert!(error.contains("art coordinate bound"), "{error}");
}

#[test]
fn full_grid_capacity_and_identifier_wraparound_are_checked_atomically() {
    let graph = flat_graph(FP::ZERO);
    let original = document(&graph, [0.25, 0.0, 0.25]);
    let stamped = stamp_prop_surface_grid(&original, 1, u64::MAX, &graph, config(16, 8)).unwrap();
    assert_eq!(stamped.props.len(), 128);
    assert_eq!(stamped.props[1].id, u64::MAX);
    assert_eq!(stamped.props[2].id, 2);
    assert_eq!(
        stamped
            .props
            .iter()
            .map(|prop| prop.id)
            .collect::<BTreeSet<_>>()
            .len(),
        128
    );
    assert!(
        stamp_prop_surface_grid(&original, 1, 2, &graph, config(16, 16))
            .err()
            .unwrap()
            .contains("128-prop")
    );
    let mut nearly_full = original.clone();
    nearly_full
        .props
        .resize_with(127, || original.props[0].clone());
    assert!(
        stamp_prop_surface_grid(&nearly_full, 1, 2, &graph, config(3, 1))
            .err()
            .unwrap()
            .contains("128-prop")
    );
    assert_eq!(original.props.len(), 1);
}

#[test]
fn work_budget_accepts_exact_limit_and_rejects_first_over_and_multiply_overflow() {
    assert!(check_projection_work(32, 32_768).is_ok());
    assert!(check_projection_work(1, 1_048_576).is_ok());
    assert!(check_projection_work(1, 1_048_577).is_err());
    assert!(check_projection_work(33, 32_768).is_err());
    assert!(check_projection_work(usize::MAX, 2).is_err());
}

#[test]
fn panel_surface_grid_adds_one_undo_redo_entry_without_scene_or_sidecar_writes() {
    let (_directory, editor, mut panel) = editor_panel();
    let source = document_state(&panel.document);
    let checksum = editor.checksum();
    let scene_history = editor.history().entries.len();
    let scene = editor.path().unwrap();
    let scene_bytes = std::fs::read(&scene).unwrap();
    panel.stamp_surface_grid(&editor, false);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert_eq!(panel.document.props.len(), 12);
    assert_eq!(panel.history.len(), 2);
    assert_eq!(panel.selected, Some(1));
    assert_eq!(document_state(&panel.document)[0], source[0]);
    let after = document_state(&panel.document);
    panel.undo();
    assert_eq!(document_state(&panel.document), source);
    panel.redo();
    assert_eq!(document_state(&panel.document), after);
    assert_eq!(panel.history.len(), 2);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), scene_history);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    assert!(!crate::mesh_props::sidecar_path(&scene).unwrap().exists());
}

#[test]
fn direct_action_guards_play_stale_identity_and_pending_document() {
    let (_directory, mut editor, mut panel) = editor_panel();
    let epoch = panel.scene_epoch;
    let revision = panel.graph_revision;
    for case in 0..5 {
        match case {
            0 => panel.scene_epoch = Some(epoch.unwrap().wrapping_add(1)),
            1 => panel.graph_revision = Some([0xff; 32]),
            2 => panel.document_pending = true,
            3 => panel.context_stale = true,
            4 => panel.sidecar_error = Some("unreadable".into()),
            _ => unreachable!(),
        }
        let before = panel_state(&panel);
        panel.stamp_surface_grid(&editor, false);
        assert!(panel.error.is_some(), "case {case}");
        assert_eq!(panel_state(&panel), before, "case {case}");
        panel.scene_epoch = epoch;
        panel.graph_revision = revision;
        panel.document_pending = false;
        panel.context_stale = false;
        panel.sidecar_error = None;
    }
    assert!(editor.start_play());
    assert_eq!(editor.mode(), Mode::Play);
    let before = panel_state(&panel);
    panel.stamp_surface_grid(&editor, false);
    assert!(panel.error.is_some());
    assert_eq!(panel_state(&panel), before);
}

#[test]
fn final_context_check_rejects_scene_epoch_revision_checksum_and_new_pick() {
    let (_directory, editor, mut panel) = editor_panel();
    let context = panel.surface_grid_context(&editor, false).unwrap();
    for case in 0..5 {
        let mut expected = context.clone();
        match case {
            0 => expected.scene = expected.scene.with_file_name("another.scene.yaml"),
            1 => expected.scene_epoch = expected.scene_epoch.wrapping_add(1),
            2 => expected.graph_revision = [0xff; 32],
            3 => expected.checksum ^= 1,
            4 => {}
            _ => unreachable!(),
        }
        let before = panel_state(&panel);
        let candidate = panel.document.clone();
        assert!(panel
            .commit_surface_grid(&editor, case == 4, &expected, candidate)
            .is_err());
        assert_eq!(panel_state(&panel), before);
    }
}

#[test]
fn direct_action_rejects_form_candidate_prefab_material_gesture_and_placement_drafts() {
    let (_directory, editor, mut panel) = editor_panel();
    for case in 0..8 {
        match case {
            0 => panel.name = "Unapplied name".into(),
            1 => panel.candidate = Some(panel.document.props[0].art.clone()),
            2 => {
                let template = crate::mesh_prop_prefab::Template::capture(
                    "Pending prefab",
                    &panel.document.props[0].art,
                )
                .unwrap();
                panel.stage_prefab(template).unwrap();
            }
            3 => {
                let model = panel.document.props[0].art.source_model_arc();
                let material = model.source().primitives[0].material;
                let mut settings = crate::mesh_material_slots::Settings::identity(
                    material,
                    &model.source().materials[material as usize],
                );
                settings.tint[0] = 0.5;
                panel.material_slots_draft = vec![settings];
            }
            4 => {
                let camera = orr_render::Camera3D::orthographic([0.0, 5.0, 5.0], [0.0; 3], 4.0);
                panel.gesture = Some(super::super::PropGesture {
                    original: panel.document.clone(),
                    selected: 1,
                    drag: crate::mesh_prop_view::Drag::new(
                        &camera,
                        [0.0, 0.0],
                        (100, 100),
                        panel.document.props[0].art.offset(),
                    )
                    .unwrap(),
                });
            }
            5 => {
                panel.placement = Some(super::super::PropPlacement {
                    prop_id: 1,
                    scene: panel.scene.clone().unwrap(),
                    scene_epoch: panel.scene_epoch.unwrap(),
                    graph_revision: panel.graph_revision.unwrap(),
                    checksum: editor.sim().checksum,
                    position: None,
                })
            }
            6 => {} // Direct endpoint-picking guard.
            7 => panel.selected = None,
            _ => unreachable!(),
        }
        let before = panel_state(&panel);
        panel.stamp_surface_grid(&editor, case == 6);
        assert!(panel.error.is_some(), "case {case}");
        assert_eq!(panel_state(&panel), before, "case {case}");
        panel.selected = Some(1);
        panel.gesture = None;
        panel.placement = None;
        panel.copy_selection_fields();
    }
}

fn cooked_art(graph: &StaticMeshGraph, draws: usize, license_len: usize) -> Snapshot {
    let directory = tempfile::tempdir().unwrap();
    let mut source = snapshot(graph).source_model_arc().source().clone();
    source.asset_id = "model.orrmodel".into();
    let primitive = source.primitives[0].clone();
    source.primitives = (0..draws)
        .map(|index| {
            let mut primitive = primitive.clone();
            primitive.id = format!("model.orrmodel#node=0/mesh=0/primitive={index}");
            primitive
        })
        .collect();
    let model = orr_model::StaticModel::new(source).unwrap();
    let manifest = orr_package::Manifest {
        schema: 1,
        name: "surface-grid-budget".into(),
        version: "1.0.0".into(),
        engine: format!("={}", orr_package::Runtime::content_only().engine_version),
        capabilities: BTreeSet::from(["models".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from(["model.orrmodel".into(), "LICENSE.txt".into()]),
    };
    std::fs::write(
        directory.path().join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("model.orrmodel"),
        model.to_bytes().unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("LICENSE.txt"),
        "L".repeat(license_len),
    )
    .unwrap();
    Snapshot::from_package(
        directory.path(),
        "model.orrmodel",
        "LICENSE.txt",
        graph.revision(),
    )
    .unwrap()
}

#[test]
fn final_document_draw_and_sidecar_budget_failures_preserve_panel_state() {
    for (draws, license_len, expected_error) in [(29, 1, "draw limit"), (1, 200_000, "16 MiB")] {
        let (_directory, editor, mut panel) = editor_panel();
        let graph = mesh_from_view(editor.snapshot().unwrap().predicted())
            .unwrap()
            .graph;
        panel.document.props[0].art = cooked_art(&graph, draws, license_len);
        panel.reset_history(panel.document.clone(), Some(0));
        panel.copy_selection_fields();
        panel.grid_columns = "16".into();
        panel.grid_rows = "8".into();
        panel.grid_spacing = ["1".into(), "1".into()];
        validate_document(&panel.document, graph.revision()).unwrap();
        let before = panel_state(&panel);
        panel.stamp_surface_grid(&editor, false);
        assert!(
            panel.error.as_deref().unwrap().contains(expected_error),
            "{:?}",
            panel.error
        );
        assert_eq!(panel_state(&panel), before);
    }
}

#[cfg(feature = "mesh-prop-shading")]
#[test]
fn composed_shading_appearance_budget_is_revalidated_for_entire_candidate() {
    let graph = flat_graph(FP::ZERO);
    let mut original = document(&graph, [0.25, 0.0, 0.25]);
    let art = original.props[0].art.clone();
    let material = art.source_model_arc().source().primitives[0].material;
    original.props = (0..7)
        .map(|index| Prop {
            id: index + 1,
            name: format!("Appearance {index}"),
            art: art.clone(),
            shading: Some(
                crate::mesh_prop_shading::Settings::new(
                    vec![crate::mesh_prop_shading::Slot {
                        material,
                        metallic: index as f32 / 10.0,
                        roughness: 0.5,
                    }],
                    art.source_model_arc(),
                )
                .unwrap(),
            ),
        })
        .collect();
    let before = document_state(&original);
    let error = stamp_prop_surface_grid(&original, 1, 8, &graph, config(2, 1))
        .err()
        .unwrap();
    assert!(error.contains("six-appearance"), "{error}");
    assert_eq!(document_state(&original), before);
}

#[test]
fn full_admitted_graph_obeys_work_limit_before_attempting_any_projection() {
    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    for row in 0..=128 {
        for column in 0..=128 {
            vertices.push([
                FP::from_raw(column * 16_384),
                FP::ZERO,
                FP::from_raw(row * 16_384),
            ]);
        }
    }
    for row in 0..128_u32 {
        for column in 0..128_u32 {
            let a = row * 129 + column;
            faces.extend([[a, a + 129, a + 130], [a, a + 130, a + 1]]);
        }
    }
    let graph = graph(&vertices, &faces);
    assert_eq!(graph.triangles().len(), 32_768);
    let original = document(&graph, [0.25, 0.0, 0.25]);
    let stamped = stamp_prop_surface_grid(&original, 1, 2, &graph, config(11, 3)).unwrap();
    assert_eq!(stamped.props.len(), 33); // 32 projections * 32,768 triangles = 1,048,576 work units.
                                         // If a projection were attempted this source would miss, so the error proves
                                         // the whole request's work is checked before looking at individual cells.
    let outside = document(&graph, [100.0, 0.0, 100.0]);
    let error = stamp_prop_surface_grid(&outside, 1, 2, &graph, config(7, 5))
        .err()
        .unwrap();
    assert!(error.contains("work limit"), "{error}");
}

#[test]
fn source_cell_need_not_hit_a_surface_and_is_never_projected() {
    let graph = flat_graph(FP::ZERO);
    let original = document(&graph, [-0.25, 0.0, 0.25]);
    let before = document_state(&original);
    let stamped = stamp_prop_surface_grid(&original, 1, 2, &graph, config(2, 1)).unwrap();
    assert_eq!(document_state(&stamped)[0], before[0]);
    assert_eq!(stamped.props[1].art.offset(), [0.75, 0.0, 0.25]);
}

#[test]
fn panel_numeric_and_missing_cell_failures_preserve_history_ids_and_selection() {
    let (_directory, editor, mut panel) = editor_panel();
    let before = panel_state(&panel);
    panel.grid_spacing[0] = "NaN".into();
    panel.stamp_surface_grid(&editor, false);
    assert!(panel.error.is_some());
    assert_eq!(panel_state(&panel), before);
    panel.grid_spacing[0] = "20".into();
    panel.stamp_surface_grid(&editor, false);
    assert!(panel.error.as_deref().unwrap().contains("[2,0]"));
    assert_eq!(panel_state(&panel), before);
}

#[test]
fn direct_action_and_final_check_reject_an_open_editor_transaction() {
    let (_directory, mut editor, mut panel) = editor_panel();
    let admitted = panel.surface_grid_context(&editor, false).unwrap();
    editor
        .host_call(
            "tx.begin",
            serde_json::json!({"label": "surface grid guard"}),
        )
        .unwrap();
    editor.sync();
    assert!(editor.history().in_tx);
    assert!(editor.sim().in_tx);
    let before = panel_state(&panel);
    panel.stamp_surface_grid(&editor, false);
    assert!(panel.error.is_some());
    assert_eq!(panel_state(&panel), before);
    let candidate = panel.document.clone();
    assert!(panel
        .commit_surface_grid(&editor, false, &admitted, candidate)
        .is_err());
    assert_eq!(panel_state(&panel), before);
    editor
        .host_call("tx.rollback", serde_json::Value::Null)
        .unwrap();
    editor.sync();
    assert!(panel.surface_grid_context(&editor, false).is_ok());
}
