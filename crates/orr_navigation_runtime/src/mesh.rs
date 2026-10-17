use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame, FrameList};
use orr_fp::FP;
use orr_navigation::{
    mesh::{MeshPath, StaticMeshGraph},
    NavigationError, NavigationStatus, SearchBudget,
};
use std::fmt;

pub const MAX_VERTICES: usize = 1_024;
pub const MAX_TRIANGLES: usize = 512;
pub const MAX_GRAPH_BYTES: usize = 48 + 24 * MAX_VERTICES + 12 * MAX_TRIANGLES;
const MAX_SEARCH_EXPANSIONS: u32 = MAX_TRIANGLES as u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct WorldBinding {
    pub world_revision: [u8; 32],
    pub graph_revision: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct AgentSpec {
    pub start: [FP; 3],
    pub goal: [FP; 3],
    pub max_vertical: FP,
    pub distance_per_tick: FP,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(transparent)]
pub struct AssetByte {
    pub value: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct RuntimeAgent {
    pub spec: AgentSpec,
    pub position: [FP; 3],
    pub distance: FP,
    pub status: u32,
    pub reserved: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct RuntimeState {
    pub agent: Entity,
    pub graph_bytes: FrameList<AssetByte>,
    pub binding: WorldBinding,
}

impl RuntimeState {
    pub fn is_active(self) -> bool {
        self.graph_bytes.version != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    Unregistered,
    AlreadyActive,
    MissingAgent,
    InvalidState,
    Limit,
    StaleBinding,
    Navigation(NavigationError),
}

impl From<NavigationError> for RuntimeError {
    fn from(error: NavigationError) -> Self {
        Self::Navigation(error)
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentReadback {
    pub entity: Entity,
    pub spec: AgentSpec,
    pub binding: WorldBinding,
    pub position: [FP; 3],
    pub distance: FP,
    pub status: NavigationStatus,
    pub path: MeshPath,
}

pub struct MeshRuntime;

impl MeshRuntime {
    pub fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_singleton::<RuntimeState>("orr_navigation_runtime::mesh::RuntimeState");
        builder.register_component::<RuntimeAgent>("orr_navigation_runtime::mesh::RuntimeAgent");
        builder.register_list::<AssetByte>("orr_navigation_runtime::mesh::AssetByte");
    }

    pub fn admit(
        frame: &mut Frame,
        graph: &StaticMeshGraph,
        binding: WorldBinding,
        spec: AgentSpec,
    ) -> Result<Entity, RuntimeError> {
        ensure_registered(frame)?;
        if frame.singleton::<RuntimeState>().is_active() || frame.count::<RuntimeAgent>() != 0 {
            return Err(RuntimeError::AlreadyActive);
        }
        validate_binding(binding, graph.revision())?;
        validate_spec(spec)?;
        check_graph_shape(graph)?;
        let graph_bytes = graph.to_bytes();
        let canonical_graph = decode_asset(&graph_bytes)?;
        if canonical_graph.revision() != binding.graph_revision {
            return Err(RuntimeError::StaleBinding);
        }
        let path = make_path(&canonical_graph, spec)?;
        let path_length = canonical_graph.path_length(&path)?;
        let initial_position = canonical_graph.point_at_distance(&path, FP::ZERO)?;
        let status = if path_length == FP::ZERO {
            STATUS_ARRIVED
        } else {
            STATUS_MOVING
        };

        let graph_handle = frame.alloc_list::<AssetByte>();
        for value in graph_bytes {
            frame.list_push(graph_handle, AssetByte { value });
        }
        let entity = frame.spawn();
        frame.add(
            entity,
            RuntimeAgent {
                spec,
                position: initial_position,
                distance: FP::ZERO,
                status,
                reserved: 0,
            },
        );
        frame.set_singleton(RuntimeState {
            agent: entity,
            graph_bytes: graph_handle,
            binding,
        });
        Ok(entity)
    }

    pub fn validate(frame: &Frame, binding: WorldBinding) -> Result<(), RuntimeError> {
        Self::checked(frame, binding).map(|_| ())
    }

    pub fn agent(frame: &Frame, binding: WorldBinding) -> Result<AgentReadback, RuntimeError> {
        let (state, agent, _, path) = Self::checked(frame, binding)?;
        Ok(AgentReadback {
            entity: state.agent,
            spec: agent.spec,
            binding: state.binding,
            position: agent.position,
            distance: agent.distance,
            status: navigation_status(agent.status)?,
            path,
        })
    }

    pub fn step(frame: &mut Frame, binding: WorldBinding) -> Result<(), RuntimeError> {
        let (state, mut agent, graph, path) = Self::checked(frame, binding)?;
        if agent.status == STATUS_STOPPED || agent.status == STATUS_ARRIVED {
            return Ok(());
        }
        let length = graph.path_length(&path)?;
        let next_distance = (i128::from(agent.distance.raw())
            + i128::from(agent.spec.distance_per_tick.raw()))
        .min(i128::from(length.raw()));
        let next_distance =
            FP::from_raw(i64::try_from(next_distance).map_err(|_| RuntimeError::Limit)?);
        let next_position = graph.point_at_distance(&path, next_distance)?;
        agent.distance = next_distance;
        agent.position = next_position;
        agent.status = if next_distance == length {
            STATUS_ARRIVED
        } else {
            STATUS_MOVING
        };
        frame.add(state.agent, agent);
        Ok(())
    }

    pub fn stop(frame: &mut Frame, binding: WorldBinding) -> Result<(), RuntimeError> {
        let (state, mut agent, _, _) = Self::checked(frame, binding)?;
        agent.status = STATUS_STOPPED;
        frame.add(state.agent, agent);
        Ok(())
    }

    pub fn replan(
        frame: &mut Frame,
        graph: &StaticMeshGraph,
        binding: WorldBinding,
        goal: [FP; 3],
        max_vertical: FP,
        distance_per_tick: FP,
    ) -> Result<(), RuntimeError> {
        ensure_registered(frame)?;
        let stored_state = *frame.singleton::<RuntimeState>();
        if !stored_state.is_active() {
            return Err(RuntimeError::InvalidState);
        }
        let (state, old_agent, _, _) = Self::checked(frame, stored_state.binding)?;
        validate_binding(binding, graph.revision())?;
        let spec = AgentSpec {
            start: old_agent.position,
            goal,
            max_vertical,
            distance_per_tick,
        };
        validate_spec(spec)?;
        check_graph_shape(graph)?;
        let graph_bytes = graph.to_bytes();
        let canonical_graph = decode_asset(&graph_bytes)?;
        if canonical_graph.revision() != binding.graph_revision {
            return Err(RuntimeError::StaleBinding);
        }
        let path = make_path(&canonical_graph, spec)?;
        let length = canonical_graph.path_length(&path)?;
        let position = canonical_graph.point_at_distance(&path, FP::ZERO)?;
        let status = if length == FP::ZERO {
            STATUS_ARRIVED
        } else {
            STATUS_MOVING
        };

        let new_graph_handle = frame.alloc_list::<AssetByte>();
        for value in graph_bytes {
            frame.list_push(new_graph_handle, AssetByte { value });
        }
        frame.add(
            state.agent,
            RuntimeAgent {
                spec,
                position,
                distance: FP::ZERO,
                status,
                reserved: 0,
            },
        );
        frame.set_singleton(RuntimeState {
            agent: state.agent,
            graph_bytes: new_graph_handle,
            binding,
        });
        frame.list_free(state.graph_bytes);
        Ok(())
    }

    pub fn remove(frame: &mut Frame, binding: WorldBinding) -> Result<(), RuntimeError> {
        let (state, _, _, _) = Self::checked(frame, binding)?;
        if !frame.despawn(state.agent) {
            return Err(RuntimeError::MissingAgent);
        }
        frame.list_free(state.graph_bytes);
        frame.set_singleton(RuntimeState::zeroed());
        Ok(())
    }

    fn checked(
        frame: &Frame,
        expected_binding: WorldBinding,
    ) -> Result<(RuntimeState, RuntimeAgent, StaticMeshGraph, MeshPath), RuntimeError> {
        ensure_registered(frame)?;
        let state = *frame.singleton::<RuntimeState>();
        if !state.is_active() || state.agent.is_none() {
            return Err(RuntimeError::InvalidState);
        }
        if state.binding != expected_binding {
            return Err(RuntimeError::StaleBinding);
        }
        validate_binding(expected_binding, expected_binding.graph_revision)?;
        if !frame.exists(state.agent) || frame.count::<RuntimeAgent>() != 1 {
            return Err(RuntimeError::MissingAgent);
        }
        let agent = *frame
            .get::<RuntimeAgent>(state.agent)
            .ok_or(RuntimeError::MissingAgent)?;
        if !frame.list_is_alive(state.graph_bytes) {
            return Err(RuntimeError::InvalidState);
        }
        let (graph, path) = decode_state(
            &state,
            &agent,
            frame.list(state.graph_bytes),
            expected_binding,
        )?;
        Ok((state, agent, graph, path))
    }
}

pub fn decode_asset(bytes: &[u8]) -> Result<StaticMeshGraph, RuntimeError> {
    check_graph_bytes(bytes)?;
    StaticMeshGraph::from_bytes(bytes).map_err(RuntimeError::Navigation)
}

pub fn decode_readback(
    state: &RuntimeState,
    agent: &RuntimeAgent,
    stored_bytes: &[AssetByte],
    expected_binding: WorldBinding,
) -> Result<AgentReadback, RuntimeError> {
    let (_, path) = decode_state(state, agent, stored_bytes, expected_binding)?;
    Ok(AgentReadback {
        entity: state.agent,
        spec: agent.spec,
        binding: state.binding,
        position: agent.position,
        distance: agent.distance,
        status: navigation_status(agent.status)?,
        path,
    })
}

fn decode_state(
    state: &RuntimeState,
    agent: &RuntimeAgent,
    stored_bytes: &[AssetByte],
    expected_binding: WorldBinding,
) -> Result<(StaticMeshGraph, MeshPath), RuntimeError> {
    if !state.is_active() || state.agent.is_none() {
        return Err(RuntimeError::InvalidState);
    }
    if state.binding != expected_binding {
        return Err(RuntimeError::StaleBinding);
    }
    validate_binding(expected_binding, expected_binding.graph_revision)?;
    if stored_bytes.len() > MAX_GRAPH_BYTES {
        return Err(RuntimeError::Limit);
    }
    let graph_bytes: Vec<u8> = stored_bytes.iter().map(|byte| byte.value).collect();
    let graph = decode_asset(&graph_bytes)?;
    if graph.revision() != state.binding.graph_revision {
        return Err(RuntimeError::StaleBinding);
    }
    let path = validate_agent(&graph, agent)?;
    Ok((graph, path))
}

const STATUS_STOPPED: u32 = 0;
const STATUS_MOVING: u32 = 1;
const STATUS_ARRIVED: u32 = 2;

fn ensure_registered(frame: &Frame) -> Result<(), RuntimeError> {
    let registry = frame.registry();
    if registry.singleton_id::<RuntimeState>().is_none()
        || registry.component_id::<RuntimeAgent>().is_none()
        || registry.list_id::<AssetByte>().is_none()
    {
        return Err(RuntimeError::Unregistered);
    }
    Ok(())
}

fn validate_binding(binding: WorldBinding, graph_revision: [u8; 32]) -> Result<(), RuntimeError> {
    if binding.world_revision == [0; 32] || binding.graph_revision != graph_revision {
        return Err(RuntimeError::StaleBinding);
    }
    Ok(())
}

fn check_graph_shape(graph: &StaticMeshGraph) -> Result<(), RuntimeError> {
    if graph.vertices().len() > MAX_VERTICES || graph.triangles().len() > MAX_TRIANGLES {
        return Err(RuntimeError::Limit);
    }
    Ok(())
}

fn validate_spec(spec: AgentSpec) -> Result<(), RuntimeError> {
    if spec.max_vertical.raw() < 0 || spec.distance_per_tick.raw() < 0 {
        return Err(RuntimeError::Navigation(NavigationError::InvalidDistance));
    }
    Ok(())
}

fn check_graph_bytes(bytes: &[u8]) -> Result<(), RuntimeError> {
    if bytes.len() > MAX_GRAPH_BYTES || bytes.len() < 48 {
        return Err(RuntimeError::Limit);
    }
    if bytes.get(..8) != Some(b"ORRMESH\x01".as_slice()) {
        return Err(RuntimeError::Navigation(NavigationError::MalformedAsset));
    }
    let vertex_count = u32::from_le_bytes(
        bytes[40..44]
            .try_into()
            .map_err(|_| RuntimeError::InvalidState)?,
    ) as usize;
    if vertex_count > MAX_VERTICES {
        return Err(RuntimeError::Limit);
    }
    let face_count_offset = 44usize
        .checked_add(vertex_count.checked_mul(24).ok_or(RuntimeError::Limit)?)
        .ok_or(RuntimeError::Limit)?;
    let face_count_end = face_count_offset
        .checked_add(4)
        .ok_or(RuntimeError::Limit)?;
    let face_count = u32::from_le_bytes(
        bytes
            .get(face_count_offset..face_count_end)
            .ok_or(RuntimeError::Navigation(NavigationError::MalformedAsset))?
            .try_into()
            .map_err(|_| RuntimeError::InvalidState)?,
    ) as usize;
    if face_count > MAX_TRIANGLES {
        return Err(RuntimeError::Limit);
    }
    let expected_len = 48usize
        .checked_add(vertex_count.checked_mul(24).ok_or(RuntimeError::Limit)?)
        .and_then(|length| length.checked_add(face_count.checked_mul(12)?))
        .ok_or(RuntimeError::Limit)?;
    if bytes.len() != expected_len {
        return Err(RuntimeError::Navigation(NavigationError::MalformedAsset));
    }
    Ok(())
}

fn make_path(graph: &StaticMeshGraph, spec: AgentSpec) -> Result<MeshPath, RuntimeError> {
    graph
        .find_path(
            spec.start,
            spec.goal,
            spec.max_vertical,
            SearchBudget {
                max_expansions: MAX_SEARCH_EXPANSIONS,
            },
        )
        .map_err(RuntimeError::Navigation)
}

fn validate_agent(graph: &StaticMeshGraph, agent: &RuntimeAgent) -> Result<MeshPath, RuntimeError> {
    if agent.reserved != 0 || agent.status > STATUS_ARRIVED {
        return Err(RuntimeError::InvalidState);
    }
    validate_spec(agent.spec)?;
    let path = make_path(graph, agent.spec)?;
    let length = graph.path_length(&path)?;
    if agent.distance.raw() < 0 || agent.distance > length {
        return Err(RuntimeError::Navigation(NavigationError::InvalidDistance));
    }
    if graph.point_at_distance(&path, agent.distance)? != agent.position {
        return Err(RuntimeError::InvalidState);
    }
    match agent.status {
        STATUS_STOPPED => {}
        STATUS_MOVING if agent.distance < length => {}
        STATUS_ARRIVED if agent.distance == length => {}
        _ => return Err(RuntimeError::InvalidState),
    }
    Ok(path)
}

fn navigation_status(status: u32) -> Result<NavigationStatus, RuntimeError> {
    match status {
        STATUS_STOPPED => Ok(NavigationStatus::Stopped),
        STATUS_MOVING => Ok(NavigationStatus::Moving),
        STATUS_ARRIVED => Ok(NavigationStatus::Arrived),
        _ => Err(RuntimeError::InvalidState),
    }
}
