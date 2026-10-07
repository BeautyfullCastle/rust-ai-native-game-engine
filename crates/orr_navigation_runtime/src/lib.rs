//! Optional frame-owned, deterministic navigation for one bounded point agent.
#![deny(clippy::float_arithmetic)]
#![deny(clippy::disallowed_types)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame, FrameList};
use orr_fp::FP;
use orr_navigation::{
    AgentProfile, NavigationError, NavigationPath, NavigationStatus, Navigator, NavigatorSnapshot,
    SearchBudget, TerrainGraph,
};
use orr_terrain::Terrain;

pub const MAX_SIDE: u32 = 17;
pub const MAX_ROUTE_TRIANGLES: usize = 512;
/// Exact maximum canonical terrain/graph/route encoding sizes for the adapter limits.
pub const MAX_TERRAIN_BYTES: usize =
    8 + 2 + orr_terrain::MAX_ASSET_ID_BYTES + 8 + 24 + 17 * 17 * 8 + 16 * 16;
pub const MAX_GRAPH_BYTES: usize = 8
    + 32
    + 2
    + orr_terrain::MAX_ASSET_ID_BYTES
    + 32
    + 4
    + 17 * 17 * 24
    + 4
    + MAX_ROUTE_TRIANGLES * (4 + 3 * 4 + 1 + 3 * (4 + 2 * 4));
pub const MAX_NAVIGATOR_BYTES: usize = 8
    + 32
    + 32
    + 24
    + 1
    + 4
    + 1
    + 32
    + 32
    + 8
    + 4
    + MAX_ROUTE_TRIANGLES * 4
    + 4
    + (MAX_ROUTE_TRIANGLES - 1) * (4 + 2 * 4 + 3 * 8)
    + 4
    + (MAX_ROUTE_TRIANGLES + 1) * 3 * 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct AgentSpec {
    pub start: [FP; 2],
    pub goal: [FP; 2],
    pub max_slope: FP,
    pub distance_per_tick: FP,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct RuntimeAgent {
    pub start_x_raw: i64,
    pub start_z_raw: i64,
    pub goal_x_raw: i64,
    pub goal_z_raw: i64,
    pub max_slope_raw: i64,
    pub distance_per_tick_raw: i64,
}
impl From<AgentSpec> for RuntimeAgent {
    fn from(spec: AgentSpec) -> Self {
        Self {
            start_x_raw: spec.start[0].raw(),
            start_z_raw: spec.start[1].raw(),
            goal_x_raw: spec.goal[0].raw(),
            goal_z_raw: spec.goal[1].raw(),
            max_slope_raw: spec.max_slope.raw(),
            distance_per_tick_raw: spec.distance_per_tick.raw(),
        }
    }
}
impl From<RuntimeAgent> for AgentSpec {
    fn from(agent: RuntimeAgent) -> Self {
        Self {
            start: [
                FP::from_raw(agent.start_x_raw),
                FP::from_raw(agent.start_z_raw),
            ],
            goal: [
                FP::from_raw(agent.goal_x_raw),
                FP::from_raw(agent.goal_z_raw),
            ],
            max_slope: FP::from_raw(agent.max_slope_raw),
            distance_per_tick: FP::from_raw(agent.distance_per_tick_raw),
        }
    }
}

/// All persistent references. Zero version means no route has been admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct RuntimeState {
    pub agent: Entity,
    pub terrain_bytes: FrameList<u8>,
    pub graph_bytes: FrameList<u8>,
    pub navigator_bytes: FrameList<u8>,
}
impl RuntimeState {
    pub fn is_active(self) -> bool {
        self.terrain_bytes.version != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    AlreadyActive,
    MissingAgent,
    InvalidState,
    InvalidTerrain,
    Limit,
    Navigation(NavigationError),
}
impl From<NavigationError> for RuntimeError {
    fn from(error: NavigationError) -> Self {
        Self::Navigation(error)
    }
}
impl core::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RuntimeError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentReadback {
    pub entity: Entity,
    pub spec: AgentSpec,
    pub position: [FP; 3],
    pub status: NavigationStatus,
    pub next_waypoint: u32,
    pub path: Option<NavigationPath>,
}

fn check_bounds(
    terrain: &Terrain,
    graph: &TerrainGraph,
    spec: AgentSpec,
) -> Result<(), RuntimeError> {
    if terrain.width() > MAX_SIDE
        || terrain.depth() > MAX_SIDE
        || graph.triangles().len() > MAX_ROUTE_TRIANGLES
    {
        return Err(RuntimeError::Limit);
    }
    if spec.distance_per_tick.raw() < 0 {
        return Err(RuntimeError::Navigation(NavigationError::InvalidDistance));
    }
    graph.validate_terrain(terrain)?;
    if graph.profile()
        != (AgentProfile {
            max_slope: spec.max_slope,
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        })
    {
        return Err(RuntimeError::Navigation(NavigationError::ProfileMismatch));
    }
    Ok(())
}

/// Reconstructs canonical dependencies and route from exactly the Frame's byte lists.
/// This is independent of FrameView and can be used by the editor's read-only view.
pub fn decode_scene(
    state: &RuntimeState,
    agent: &RuntimeAgent,
    terrain_bytes: &[u8],
    graph_bytes: &[u8],
    navigator_bytes: &[u8],
) -> Result<(Terrain, TerrainGraph, Navigator, AgentSpec), RuntimeError> {
    if !state.is_active() || state.agent.is_none() {
        return Err(RuntimeError::InvalidState);
    }
    if terrain_bytes.len() > MAX_TERRAIN_BYTES
        || graph_bytes.len() > MAX_GRAPH_BYTES
        || navigator_bytes.len() > MAX_NAVIGATOR_BYTES
    {
        return Err(RuntimeError::Limit);
    }
    let spec = AgentSpec::from(*agent);
    let terrain = Terrain::load(terrain_bytes).map_err(|_| RuntimeError::InvalidTerrain)?;
    // Reject an oversized dependency before graph reconstruction (which can
    // otherwise traverse the larger core navigation limits).
    if terrain.width() > MAX_SIDE || terrain.depth() > MAX_SIDE {
        return Err(RuntimeError::Limit);
    }
    let profile = AgentProfile {
        max_slope: spec.max_slope,
        radius: FP::ZERO,
        headroom: FP::ZERO,
        max_step: FP::ZERO,
    };
    let graph = TerrainGraph::load(graph_bytes, &terrain, profile)?;
    check_bounds(&terrain, &graph, spec)?;
    let snapshot = NavigatorSnapshot::from_bytes(navigator_bytes)?;
    let mut navigator = Navigator::new(&graph, &terrain, spec.start)?;
    navigator.restore(&graph, &terrain, &snapshot)?;
    let path = navigator.path().ok_or(RuntimeError::InvalidState)?;
    if path.corridor().len() > MAX_ROUTE_TRIANGLES
        || path.waypoints().first().copied() != Some(graph.project(&terrain, spec.start)?.position)
        || path.waypoints().last().copied() != Some(graph.project(&terrain, spec.goal)?.position)
    {
        return Err(RuntimeError::InvalidState);
    }
    Ok((terrain, graph, navigator, spec))
}

pub fn decode_readback(
    state: &RuntimeState,
    agent: &RuntimeAgent,
    terrain_bytes: &[u8],
    graph_bytes: &[u8],
    navigator_bytes: &[u8],
) -> Result<AgentReadback, RuntimeError> {
    let (_, _, navigator, spec) =
        decode_scene(state, agent, terrain_bytes, graph_bytes, navigator_bytes)?;
    Ok(AgentReadback {
        entity: state.agent,
        spec,
        position: navigator.position(),
        status: navigator.status(),
        next_waypoint: navigator.next_waypoint(),
        path: navigator.path().cloned(),
    })
}

pub struct NavigationRuntime;
impl NavigationRuntime {
    pub fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_singleton::<RuntimeState>("orr_navigation_runtime::RuntimeState");
        builder.register_component::<RuntimeAgent>("orr_navigation_runtime::RuntimeAgent");
        builder.register_list::<u8>("orr_navigation_runtime::bytes");
    }

    /// Validates everything, including the bounded complete route, before touching Frame.
    pub fn admit(
        frame: &mut Frame,
        terrain: &Terrain,
        graph: &TerrainGraph,
        spec: AgentSpec,
    ) -> Result<Entity, RuntimeError> {
        if frame.singleton::<RuntimeState>().is_active() || frame.count::<RuntimeAgent>() != 0 {
            return Err(RuntimeError::AlreadyActive);
        }
        check_bounds(terrain, graph, spec)?;
        let canonical_graph = TerrainGraph::load(&graph.cook(), terrain, graph.profile())?;
        if canonical_graph.revision() != graph.revision() {
            return Err(RuntimeError::InvalidState);
        }
        let mut navigator = Navigator::new(graph, terrain, spec.start)?;
        navigator.replan(
            graph,
            terrain,
            spec.goal,
            SearchBudget {
                max_expansions: MAX_ROUTE_TRIANGLES as u32,
            },
        )?;
        if navigator
            .path()
            .is_none_or(|p| p.corridor().len() > MAX_ROUTE_TRIANGLES)
        {
            return Err(RuntimeError::Limit);
        }
        let terrain_bytes = terrain.cook();
        let graph_bytes = graph.cook();
        let navigator_bytes = navigator.snapshot();
        let terrain_handle = frame.alloc_list::<u8>();
        let graph_handle = frame.alloc_list::<u8>();
        let navigator_handle = frame.alloc_list::<u8>();
        for byte in terrain_bytes {
            frame.list_push(terrain_handle, byte);
        }
        for byte in graph_bytes {
            frame.list_push(graph_handle, byte);
        }
        for &byte in navigator_bytes.as_bytes() {
            frame.list_push(navigator_handle, byte);
        }
        let entity = frame.spawn();
        frame.add(entity, RuntimeAgent::from(spec));
        frame.set_singleton(RuntimeState {
            agent: entity,
            terrain_bytes: terrain_handle,
            graph_bytes: graph_handle,
            navigator_bytes: navigator_handle,
        });
        Ok(entity)
    }

    fn checked(frame: &Frame) -> Result<(RuntimeState, RuntimeAgent), RuntimeError> {
        let state = *frame.singleton::<RuntimeState>();
        if !state.is_active() || !frame.exists(state.agent) || frame.count::<RuntimeAgent>() != 1 {
            return Err(RuntimeError::MissingAgent);
        }
        let agent = *frame
            .get::<RuntimeAgent>(state.agent)
            .ok_or(RuntimeError::MissingAgent)?;
        for handle in [
            state.terrain_bytes,
            state.graph_bytes,
            state.navigator_bytes,
        ] {
            if !frame.list_is_alive(handle) {
                return Err(RuntimeError::InvalidState);
            }
        }
        if state.terrain_bytes == state.graph_bytes
            || state.terrain_bytes == state.navigator_bytes
            || state.graph_bytes == state.navigator_bytes
        {
            return Err(RuntimeError::InvalidState);
        }
        Ok((state, agent))
    }
    pub fn validate(frame: &Frame) -> Result<(), RuntimeError> {
        let (state, agent) = Self::checked(frame)?;
        decode_scene(
            &state,
            &agent,
            frame.list(state.terrain_bytes),
            frame.list(state.graph_bytes),
            frame.list(state.navigator_bytes),
        )?;
        Ok(())
    }
    pub fn agent(frame: &Frame, entity: Entity) -> Result<AgentReadback, RuntimeError> {
        let (state, agent) = Self::checked(frame)?;
        if entity != state.agent {
            return Err(RuntimeError::MissingAgent);
        }
        decode_readback(
            &state,
            &agent,
            frame.list(state.terrain_bytes),
            frame.list(state.graph_bytes),
            frame.list(state.navigator_bytes),
        )
    }
    /// Rebuilds heap helpers from Frame each tick; only the final validated state is written.
    pub fn step(frame: &mut Frame) -> Result<(), RuntimeError> {
        let (state, agent) = Self::checked(frame)?;
        let (terrain, graph, mut navigator, spec) = decode_scene(
            &state,
            &agent,
            frame.list(state.terrain_bytes),
            frame.list(state.graph_bytes),
            frame.list(state.navigator_bytes),
        )?;
        navigator.advance(&graph, &terrain, spec.distance_per_tick)?;
        let snapshot = navigator.snapshot();
        frame.list_clear(state.navigator_bytes);
        for &byte in snapshot.as_bytes() {
            frame.list_push(state.navigator_bytes, byte);
        }
        Ok(())
    }
    pub fn remove(frame: &mut Frame, entity: Entity) -> Result<(), RuntimeError> {
        let (state, _) = Self::checked(frame)?;
        if state.agent != entity {
            return Err(RuntimeError::MissingAgent);
        }
        frame.list_free(state.terrain_bytes);
        frame.list_free(state.graph_bytes);
        frame.list_free(state.navigator_bytes);
        frame.despawn(entity);
        frame.set_singleton(RuntimeState::zeroed());
        Ok(())
    }
}
