use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FP};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile, NavigationError, NavigationStatus};
use orr_navigation_runtime::{
    mesh::{
        AgentSpec, AssetByte, MeshRuntime, RuntimeAgent, RuntimeError, RuntimeState, WorldBinding,
        MAX_GRAPH_BYTES,
    },
    NavigationRuntime,
};

fn point(x_coordinate: i64, y_coordinate: i64, z_coordinate: i64) -> [FP; 3] {
    [
        FP::from_raw(x_coordinate * 65_536),
        FP::from_raw(y_coordinate * 65_536),
        FP::from_raw(z_coordinate * 65_536),
    ]
}

fn graph() -> StaticMeshGraph {
    StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(1, 0, 0),
            point(0, 0, 1),
            point(1, 0, 1),
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap()
}

fn binding(graph: &StaticMeshGraph, world_byte: u8) -> WorldBinding {
    WorldBinding {
        world_revision: [world_byte; 32],
        graph_revision: graph.revision(),
    }
}

fn spec(start: [FP; 3], goal: [FP; 3]) -> AgentSpec {
    AgentSpec {
        start,
        goal,
        max_vertical: FP::ZERO,
        distance_per_tick: fp!(0.25),
    }
}

fn setup() -> (Frame, StaticMeshGraph, WorldBinding, AgentSpec) {
    let mut builder = ComponentRegistryBuilder::new();
    NavigationRuntime::register(&mut builder);
    MeshRuntime::register(&mut builder);
    let frame = Frame::new(builder.build());
    let graph = graph();
    let binding = binding(&graph, 1);
    let spec = spec(point(0, 0, 1), point(1, 0, 0));
    (frame, graph, binding, spec)
}

fn admit(
    frame: &mut Frame,
    graph: &StaticMeshGraph,
    binding: WorldBinding,
    spec: AgentSpec,
) -> Entity {
    MeshRuntime::admit(frame, graph, binding, spec).unwrap()
}

#[test]
fn mesh_runtime_co_registers_and_snapshots_route_progress_in_frame_bytes() {
    let (mut frame, graph, binding, spec) = setup();
    let zero_world_binding = WorldBinding {
        world_revision: [0; 32],
        graph_revision: graph.revision(),
    };
    let mut wrong_graph_binding = binding;
    wrong_graph_binding.graph_revision[0] ^= 1;
    let before_rejected_admissions = frame.checksum();
    for rejected_binding in [zero_world_binding, wrong_graph_binding] {
        assert_eq!(
            MeshRuntime::admit(&mut frame, &graph, rejected_binding, spec),
            Err(RuntimeError::StaleBinding)
        );
        assert_eq!(frame.checksum(), before_rejected_admissions);
    }
    let mut invalid_spec = spec;
    invalid_spec.distance_per_tick = fp!(-1);
    assert_eq!(
        MeshRuntime::admit(&mut frame, &graph, binding, invalid_spec),
        Err(RuntimeError::Navigation(NavigationError::InvalidDistance))
    );
    assert_eq!(frame.checksum(), before_rejected_admissions);

    let entity = admit(&mut frame, &graph, binding, spec);
    let initial = MeshRuntime::agent(&frame, binding).unwrap();
    assert_eq!(initial.entity, entity);
    assert_eq!(initial.spec, spec);
    assert_eq!(initial.binding, binding);
    assert_eq!(initial.position, spec.start);
    assert_eq!(initial.distance, FP::ZERO);
    assert_eq!(initial.status, NavigationStatus::Moving);
    MeshRuntime::validate(&frame, binding).unwrap();

    let state = *frame.singleton::<RuntimeState>();
    let stored_graph = frame
        .list(state.graph_bytes)
        .iter()
        .map(|byte| byte.value)
        .collect::<Vec<_>>();
    assert_eq!(stored_graph, graph.to_bytes());
    assert!(frame.list_is_alive(state.graph_bytes));

    let baseline = frame.clone();
    let restored = Frame::from_bytes(frame.registry().clone(), &frame.to_bytes()).unwrap();
    assert_eq!(restored.checksum(), frame.checksum());
    let mut first = frame.clone();
    let mut second = restored;
    for tick in 1..=8 {
        MeshRuntime::step(&mut first, binding).unwrap();
        MeshRuntime::step(&mut second, binding).unwrap();
        first.set_tick(tick);
        second.set_tick(tick);
        assert_eq!(first.checksum(), second.checksum());
        assert_eq!(first.to_bytes(), second.to_bytes());
        MeshRuntime::validate(&first, binding).unwrap();
    }
    let final_checksum = first.checksum();
    first.copy_from(&baseline);
    for tick in 1..=8 {
        MeshRuntime::step(&mut first, binding).unwrap();
        first.set_tick(tick);
    }
    assert_eq!(first.checksum(), final_checksum);
    assert_eq!(first.to_bytes(), second.to_bytes());
    assert_eq!(
        MeshRuntime::agent(&first, binding).unwrap(),
        MeshRuntime::agent(&second, binding).unwrap()
    );
}

#[test]
fn stop_replan_arrival_and_binding_changes_are_explicit_and_atomic() {
    let (mut frame, old_graph, old_binding, agent_spec) = setup();
    let entity = admit(&mut frame, &old_graph, old_binding, agent_spec);
    MeshRuntime::step(&mut frame, old_binding).unwrap();
    let moving = MeshRuntime::agent(&frame, old_binding).unwrap();
    assert_ne!(moving.position, agent_spec.start);
    assert_eq!(moving.distance, fp!(0.25));

    MeshRuntime::stop(&mut frame, old_binding).unwrap();
    let stopped = MeshRuntime::agent(&frame, old_binding).unwrap();
    assert_eq!(stopped.status, NavigationStatus::Stopped);
    let stopped_checksum = frame.checksum();
    MeshRuntime::step(&mut frame, old_binding).unwrap();
    assert_eq!(frame.checksum(), stopped_checksum);

    let new_graph = StaticMeshGraph::bake(
        &[
            point(0, 0, 0),
            point(1, 0, 0),
            point(0, 0, 1),
            point(1, 0, 1),
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile {
            max_slope: FP::from_raw(1),
            ..AgentProfile::default()
        },
    )
    .unwrap();
    let new_binding = binding(&new_graph, 2);
    let mut invalid_binding = new_binding;
    invalid_binding.graph_revision[0] ^= 1;
    let before_failed_replan = frame.checksum();
    assert_eq!(
        MeshRuntime::replan(
            &mut frame,
            &new_graph,
            invalid_binding,
            point(0, 0, 0),
            FP::ZERO,
            fp!(0.25)
        ),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(frame.checksum(), before_failed_replan);

    let prior_position = stopped.position;
    MeshRuntime::replan(
        &mut frame,
        &new_graph,
        new_binding,
        point(0, 0, 0),
        FP::ZERO,
        fp!(0.25),
    )
    .unwrap();
    let replanned = MeshRuntime::agent(&frame, new_binding).unwrap();
    assert_eq!(replanned.entity, entity);
    assert_eq!(replanned.spec.start, prior_position);
    assert_eq!(replanned.spec.goal, point(0, 0, 0));
    assert_eq!(replanned.binding, new_binding);
    assert_eq!(replanned.distance, FP::ZERO);
    assert_eq!(replanned.status, NavigationStatus::Moving);
    assert_eq!(
        MeshRuntime::validate(&frame, old_binding),
        Err(RuntimeError::StaleBinding)
    );

    for _ in 0..16 {
        if MeshRuntime::agent(&frame, new_binding).unwrap().status == NavigationStatus::Arrived {
            break;
        }
        MeshRuntime::step(&mut frame, new_binding).unwrap();
    }
    let arrived = MeshRuntime::agent(&frame, new_binding).unwrap();
    assert_eq!(arrived.status, NavigationStatus::Arrived);
    assert_eq!(arrived.position, point(0, 0, 0));
    assert_eq!(
        arrived.distance,
        new_graph.path_length(&arrived.path).unwrap()
    );

    let mut stale_world = new_binding;
    stale_world.world_revision[0] ^= 1;
    let arrived_checksum = frame.checksum();
    assert_eq!(
        MeshRuntime::validate(&frame, stale_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(
        MeshRuntime::step(&mut frame, stale_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(
        MeshRuntime::agent(&frame, stale_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(
        MeshRuntime::remove(&mut frame, stale_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(frame.checksum(), arrived_checksum);
    MeshRuntime::stop(&mut frame, new_binding).unwrap();
    let stopped_checksum = frame.checksum();
    assert_eq!(
        MeshRuntime::validate(&frame, stale_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(
        MeshRuntime::stop(&mut frame, stale_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(
        MeshRuntime::remove(&mut frame, stale_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(frame.checksum(), stopped_checksum);
    let zero_world = WorldBinding {
        world_revision: [0; 32],
        graph_revision: new_graph.revision(),
    };
    assert_eq!(
        MeshRuntime::validate(&frame, zero_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(
        MeshRuntime::step(&mut frame, zero_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(
        MeshRuntime::agent(&frame, zero_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(
        MeshRuntime::stop(&mut frame, zero_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(
        MeshRuntime::remove(&mut frame, zero_world),
        Err(RuntimeError::StaleBinding)
    );
    assert_eq!(frame.checksum(), stopped_checksum);
}

#[test]
fn zero_step_preserves_moving_cursor_and_max_distance_arrives_without_overflow() {
    let (mut frame, graph, binding, mut agent_spec) = setup();
    agent_spec.distance_per_tick = FP::ZERO;
    let entity = admit(&mut frame, &graph, binding, agent_spec);
    let before = MeshRuntime::agent(&frame, binding).unwrap();
    assert_eq!(before.status, NavigationStatus::Moving);
    let checksum = frame.checksum();
    MeshRuntime::step(&mut frame, binding).unwrap();
    let after = MeshRuntime::agent(&frame, binding).unwrap();
    assert_eq!(after.status, NavigationStatus::Moving);
    assert_eq!(after.distance, FP::ZERO);
    assert_eq!(after.position, agent_spec.start);
    assert_eq!(frame.checksum(), checksum);
    assert_eq!(after.entity, entity);

    let (mut immediate_frame, immediate_graph, immediate_binding, mut immediate_spec) = setup();
    immediate_spec.distance_per_tick = FP::from_raw(i64::MAX);
    let immediate_entity = admit(
        &mut immediate_frame,
        &immediate_graph,
        immediate_binding,
        immediate_spec,
    );
    MeshRuntime::step(&mut immediate_frame, immediate_binding).unwrap();
    let arrived = MeshRuntime::agent(&immediate_frame, immediate_binding).unwrap();
    assert_eq!(arrived.entity, immediate_entity);
    assert_eq!(arrived.status, NavigationStatus::Arrived);
    assert_eq!(arrived.position, immediate_spec.goal);
    assert_eq!(
        arrived.distance,
        immediate_graph.path_length(&arrived.path).unwrap()
    );

    let (mut same_point_frame, same_point_graph, same_point_binding, _) = setup();
    let same_position = point(0, 0, 1);
    let same_point_spec = spec(same_position, same_position);
    let same_point_entity = admit(
        &mut same_point_frame,
        &same_point_graph,
        same_point_binding,
        same_point_spec,
    );
    let same_point = MeshRuntime::agent(&same_point_frame, same_point_binding).unwrap();
    assert_eq!(same_point.entity, same_point_entity);
    assert_eq!(same_point.status, NavigationStatus::Arrived);
    assert_eq!(same_point.distance, FP::ZERO);
    assert_eq!(same_point.position, same_position);
}

#[test]
fn frame_tampering_and_failed_operations_never_partially_mutate_state() {
    let (mut frame, graph, binding, agent_spec) = setup();
    let entity = admit(&mut frame, &graph, binding, agent_spec);
    let original = frame.clone();

    for invalid_binding in [
        WorldBinding {
            world_revision: [0; 32],
            graph_revision: graph.revision(),
        },
        WorldBinding {
            world_revision: binding.world_revision,
            graph_revision: [9; 32],
        },
    ] {
        frame.copy_from(&original);
        assert_eq!(
            MeshRuntime::admit(&mut frame, &graph, invalid_binding, agent_spec),
            Err(RuntimeError::AlreadyActive)
        );
        assert_eq!(frame.checksum(), original.checksum());
        assert_eq!(
            MeshRuntime::validate(&frame, invalid_binding),
            Err(RuntimeError::StaleBinding)
        );
        assert_eq!(frame.checksum(), original.checksum());
    }

    frame.copy_from(&original);
    let before_failed_route = frame.checksum();
    let before_failed_route_bytes = frame.to_bytes();
    assert!(MeshRuntime::replan(
        &mut frame,
        &graph,
        WorldBinding {
            world_revision: [2; 32],
            graph_revision: graph.revision(),
        },
        point(9, 0, 9),
        FP::ZERO,
        fp!(0.25)
    )
    .is_err());
    assert_eq!(frame.checksum(), before_failed_route);
    assert_eq!(frame.to_bytes(), before_failed_route_bytes);

    for tamper in 0..4 {
        frame.copy_from(&original);
        let agent = frame.get_mut::<RuntimeAgent>(entity).unwrap();
        match tamper {
            0 => agent.status = 7,
            1 => agent.reserved = 1,
            2 => agent.position[0] += FP::ONE,
            _ => agent.distance = FP::from_raw(i64::MAX),
        }
        let before = frame.checksum();
        assert!(MeshRuntime::validate(&frame, binding).is_err());
        assert_eq!(frame.checksum(), before);
        assert!(MeshRuntime::step(&mut frame, binding).is_err());
        assert_eq!(frame.checksum(), before);
    }

    frame.copy_from(&original);
    let graph_handle = frame.singleton::<RuntimeState>().graph_bytes;
    frame.list_mut(graph_handle)[0].value ^= 1;
    let before = frame.checksum();
    assert_eq!(
        MeshRuntime::validate(&frame, binding),
        Err(RuntimeError::Navigation(NavigationError::MalformedAsset))
    );
    assert_eq!(frame.checksum(), before);
    assert!(MeshRuntime::step(&mut frame, binding).is_err());
    assert_eq!(frame.checksum(), before);

    frame.copy_from(&original);
    let graph_handle = frame.singleton::<RuntimeState>().graph_bytes;
    for (offset, value) in (40..44).zip(u32::MAX.to_le_bytes()) {
        frame.list_mut(graph_handle)[offset].value = value;
    }
    let before_forged_count = frame.checksum();
    assert_eq!(
        MeshRuntime::validate(&frame, binding),
        Err(RuntimeError::Limit)
    );
    assert_eq!(frame.checksum(), before_forged_count);

    frame.copy_from(&original);
    let state = *frame.singleton::<RuntimeState>();
    frame.list_clear(state.graph_bytes);
    for _ in 0..=MAX_GRAPH_BYTES {
        frame.list_push(state.graph_bytes, AssetByte { value: 0 });
    }
    let before = frame.checksum();
    assert_eq!(
        MeshRuntime::validate(&frame, binding),
        Err(RuntimeError::Limit)
    );
    assert_eq!(frame.checksum(), before);
}

#[test]
fn stale_list_handles_and_recycled_entities_do_not_alias_runtime_state() {
    let (mut frame, graph, binding, agent_spec) = setup();
    admit(&mut frame, &graph, binding, agent_spec);
    let stale_list = frame.singleton::<RuntimeState>().graph_bytes;
    frame.list_free(stale_list);
    let replacement_list = frame.alloc_list::<AssetByte>();
    assert_eq!(replacement_list.index, stale_list.index);
    assert_ne!(replacement_list.version, stale_list.version);
    assert!(!frame.list_is_alive(stale_list));
    let before_stale_read = frame.checksum();
    assert_eq!(
        MeshRuntime::validate(&frame, binding),
        Err(RuntimeError::InvalidState)
    );
    assert_eq!(frame.checksum(), before_stale_read);

    let (mut clean_frame, clean_graph, clean_binding, clean_spec) = setup();
    let original_entity = admit(&mut clean_frame, &clean_graph, clean_binding, clean_spec);
    MeshRuntime::remove(&mut clean_frame, clean_binding).unwrap();
    assert!(!clean_frame.exists(original_entity));
    assert!(!clean_frame.singleton::<RuntimeState>().is_active());
    let reused_entity = clean_frame.spawn();
    assert_eq!(reused_entity.index, original_entity.index);
    assert_ne!(reused_entity.version, original_entity.version);
    assert_eq!(
        MeshRuntime::agent(&clean_frame, clean_binding),
        Err(RuntimeError::InvalidState)
    );

    let (mut manually_damaged, damaged_graph, damaged_binding, damaged_spec) = setup();
    let damaged_entity = admit(
        &mut manually_damaged,
        &damaged_graph,
        damaged_binding,
        damaged_spec,
    );
    assert!(manually_damaged.despawn(damaged_entity));
    let replacement_entity = manually_damaged.spawn();
    assert_eq!(replacement_entity.index, damaged_entity.index);
    assert_ne!(replacement_entity.version, damaged_entity.version);
    let before_missing_agent = manually_damaged.checksum();
    assert_eq!(
        MeshRuntime::step(&mut manually_damaged, damaged_binding),
        Err(RuntimeError::MissingAgent)
    );
    assert_eq!(manually_damaged.checksum(), before_missing_agent);
    assert_eq!(
        MeshRuntime::validate(&manually_damaged, damaged_binding),
        Err(RuntimeError::MissingAgent)
    );
    assert_eq!(manually_damaged.checksum(), before_missing_agent);
}

#[test]
fn registration_and_coexistence_fail_closed_without_panics() {
    let unregistered_builder = ComponentRegistryBuilder::new();
    let unregistered_frame = Frame::new(unregistered_builder.build());
    let (_, unregistered_graph, unregistered_binding, unregistered_spec) = setup();
    let before_unregistered = unregistered_frame.to_bytes();
    let validate_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MeshRuntime::validate(&unregistered_frame, unregistered_binding)
    }));
    assert_eq!(validate_result.unwrap(), Err(RuntimeError::Unregistered));
    let agent_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MeshRuntime::agent(&unregistered_frame, unregistered_binding)
    }));
    assert_eq!(agent_result.unwrap(), Err(RuntimeError::Unregistered));

    let mut unregistered_frame = unregistered_frame;
    let admit_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MeshRuntime::admit(
            &mut unregistered_frame,
            &unregistered_graph,
            unregistered_binding,
            unregistered_spec,
        )
    }));
    assert_eq!(admit_result.unwrap(), Err(RuntimeError::Unregistered));
    let step_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MeshRuntime::step(&mut unregistered_frame, unregistered_binding)
    }));
    assert_eq!(step_result.unwrap(), Err(RuntimeError::Unregistered));
    let stop_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MeshRuntime::stop(&mut unregistered_frame, unregistered_binding)
    }));
    assert_eq!(stop_result.unwrap(), Err(RuntimeError::Unregistered));
    let remove_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MeshRuntime::remove(&mut unregistered_frame, unregistered_binding)
    }));
    assert_eq!(remove_result.unwrap(), Err(RuntimeError::Unregistered));
    let replan_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MeshRuntime::replan(
            &mut unregistered_frame,
            &unregistered_graph,
            unregistered_binding,
            unregistered_spec.goal,
            unregistered_spec.max_vertical,
            unregistered_spec.distance_per_tick,
        )
    }));
    assert_eq!(replan_result.unwrap(), Err(RuntimeError::Unregistered));
    assert_eq!(unregistered_frame.to_bytes(), before_unregistered);

    let mut builder = ComponentRegistryBuilder::new();
    NavigationRuntime::register(&mut builder);
    MeshRuntime::register(&mut builder);
    let mut frame = Frame::new(builder.build());
    assert!(!frame.singleton::<RuntimeState>().is_active());
    assert_eq!(frame.count::<RuntimeAgent>(), 0);
    let legacy_list = frame.alloc_list::<u8>();
    let mesh_list = frame.alloc_list::<AssetByte>();
    assert!(frame.list_is_alive(legacy_list));
    assert!(frame.list_is_alive(mesh_list));
}

#[test]
fn runtime_caps_source_vertices_faces_and_encoded_assets_before_admission() {
    let (mut frame, _, _, agent_spec) = setup();
    let vertex_graph = {
        let mut vertices = vec![
            point(0, 0, 0),
            point(1, 0, 0),
            point(0, 0, 1),
            point(1, 0, 1),
        ];
        for index in 0..1_021 {
            vertices.push(point(10 + index, 0, 10));
        }
        StaticMeshGraph::bake(&vertices, &[[0, 2, 3], [0, 3, 1]], AgentProfile::default()).unwrap()
    };
    let vertex_binding = binding(&vertex_graph, 3);
    let before_vertices = frame.checksum();
    assert_eq!(
        MeshRuntime::admit(&mut frame, &vertex_graph, vertex_binding, agent_spec),
        Err(RuntimeError::Limit)
    );
    assert_eq!(frame.checksum(), before_vertices);

    let width = 17usize;
    let depth = 18usize;
    let mut vertices = Vec::new();
    for z_index in 0..depth {
        for x_index in 0..width {
            vertices.push(point(x_index as i64, 0, z_index as i64));
        }
    }
    let mut faces = Vec::new();
    for z_index in 0..(depth - 1) {
        for x_index in 0..(width - 1) {
            let top_left = (z_index * width + x_index) as u32;
            let bottom_left = top_left + width as u32;
            faces.push([top_left, bottom_left, bottom_left + 1]);
            faces.push([top_left, bottom_left + 1, top_left + 1]);
        }
    }
    assert_eq!(faces.len(), 544);
    let face_graph = StaticMeshGraph::bake(&vertices, &faces, AgentProfile::default()).unwrap();
    let face_binding = binding(&face_graph, 4);
    let before_faces = frame.checksum();
    assert_eq!(
        MeshRuntime::admit(&mut frame, &face_graph, face_binding, agent_spec),
        Err(RuntimeError::Limit)
    );
    assert_eq!(frame.checksum(), before_faces);

    let mut filtered_vertices = Vec::new();
    for z_index in 0..depth {
        for x_index in 0..width {
            filtered_vertices.push(point(x_index as i64, x_index as i64, z_index as i64));
        }
    }
    let filtered_graph = StaticMeshGraph::bake(
        &filtered_vertices,
        &faces,
        AgentProfile {
            max_slope: FP::ZERO,
            ..AgentProfile::default()
        },
    )
    .unwrap();
    assert!(filtered_graph.triangles().is_empty());
    let filtered_binding = binding(&filtered_graph, 5);
    let before_filtered = frame.checksum();
    assert_eq!(
        MeshRuntime::admit(&mut frame, &filtered_graph, filtered_binding, agent_spec),
        Err(RuntimeError::Limit)
    );
    assert_eq!(frame.checksum(), before_filtered);

    let (mut active_frame, active_graph, active_binding, active_spec) = setup();
    admit(
        &mut active_frame,
        &active_graph,
        active_binding,
        active_spec,
    );
    let state = *active_frame.singleton::<RuntimeState>();
    active_frame.list_clear(state.graph_bytes);
    for _ in 0..=MAX_GRAPH_BYTES {
        active_frame.list_push(state.graph_bytes, AssetByte { value: 0 });
    }
    let before_oversize = active_frame.checksum();
    assert_eq!(
        MeshRuntime::step(&mut active_frame, active_binding),
        Err(RuntimeError::Limit)
    );
    assert_eq!(active_frame.checksum(), before_oversize);
}
