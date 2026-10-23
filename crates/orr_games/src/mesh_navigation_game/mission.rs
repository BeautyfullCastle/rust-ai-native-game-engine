use super::{
    ensure_scene_schema, record_failure, validate_step_status, MeshNavigation, MeshScenePin,
    MeshStepStatus,
};
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::FP;
use orr_navigation::mesh::StaticMeshGraph;
use orr_navigation_runtime::mesh::MeshRuntime;
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{SimContext, System};

pub const MISSION_NAME: &str = "MeshMission";
pub const RUN_NAME: &str = "MeshMissionRun";
pub const MAX_MISSION_POINTS: usize = 8;
const MAX_COORDINATE_RAW: i64 = 256 * FP::SCALE;
const MAX_RADIUS_RAW: i64 = 4 * FP::SCALE;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct MissionPoint {
    pub position: [FP; 3],
    pub radius: FP,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct MeshMission {
    #[reflect(range = "0..=1")]
    pub enabled: u32,
    #[reflect(range = "0..=8")]
    pub count: u32,
    pub points: [MissionPoint; MAX_MISSION_POINTS],
    pub exit: MissionPoint,
    pub graph_revision: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct MeshMissionRun {
    pub collected: u32,
    #[reflect(range = "0..=1")]
    pub won: u32,
}

impl MeshMission {
    pub fn disabled(&self) -> bool {
        bytemuck::bytes_of(self).iter().all(|byte| *byte == 0)
    }
}

pub fn register(builder: &mut ComponentRegistryBuilder) {
    builder.register_singleton::<MeshMission>(MISSION_NAME);
    builder.register_singleton::<MeshMissionRun>(RUN_NAME);
}

pub fn register_reflect(types: &mut TypeRegistry) {
    types.register_singleton::<MeshMission>(MISSION_NAME);
    types.register_singleton::<MeshMissionRun>(RUN_NAME);
}

pub fn read_config(frame: &Frame) -> Result<MeshMission, String> {
    ensure_registered(frame)?;
    Ok(*frame.singleton::<MeshMission>())
}

pub fn read_run(frame: &Frame) -> Result<MeshMissionRun, String> {
    ensure_registered(frame)?;
    Ok(*frame.singleton::<MeshMissionRun>())
}

pub fn validate_config(
    config: &MeshMission,
    graph: &StaticMeshGraph,
    max_vertical: FP,
) -> Result<(), String> {
    if config.enabled == 0 {
        if config.disabled() {
            return Ok(());
        }
        return Err("disabled MeshMission must use the canonical all-zero value".into());
    }
    if config.enabled != 1 || !(1..=MAX_MISSION_POINTS as u32).contains(&config.count) {
        return Err("MeshMission requires enabled=1 and 1..=8 collection points".into());
    }
    if max_vertical.raw() < 0 || config.graph_revision != graph.revision() {
        return Err("MeshMission graph revision or projection tolerance is invalid".into());
    }
    for (index, point) in config.points.iter().enumerate() {
        if index < config.count as usize {
            validate_point(*point, graph, max_vertical)?;
        } else if *point != MissionPoint::zeroed() {
            return Err("unused MeshMission points must be zero".into());
        }
    }
    validate_point(config.exit, graph, max_vertical)
}

pub fn validate_run(config: &MeshMission, run: &MeshMissionRun) -> Result<(), String> {
    if config.enabled == 0 {
        return if config.disabled() && *run == MeshMissionRun::zeroed() {
            Ok(())
        } else {
            Err("disabled MeshMission requires a canonical zero run state".into())
        };
    }
    if config.enabled != 1 || !(1..=MAX_MISSION_POINTS as u32).contains(&config.count) {
        return Err("MeshMission run state has no valid enabled configuration".into());
    }
    let valid_mask = (1u32 << config.count) - 1;
    if run.collected & !valid_mask != 0
        || run.won > 1
        || (run.won == 1 && run.collected != valid_mask)
    {
        return Err("MeshMission run state has an invalid collected mask or win flag".into());
    }
    Ok(())
}

pub fn validate_frame(frame: &Frame, pin: MeshScenePin) -> Result<(), String> {
    ensure_scene_schema(frame)?;
    ensure_registered(frame)?;
    let config = read_config(frame)?;
    let run = read_run(frame)?;
    validate_run(&config, &run)?;
    if config.disabled() {
        return Ok(());
    }
    if pin.is_empty() {
        return Err("an enabled MeshMission requires an admitted mesh graph".into());
    }
    let graph = pin.graph()?;
    validate_config(&config, &graph, pin.max_vertical)
}

pub fn admit(frame: &Frame, pin: MeshScenePin) -> Result<(), String> {
    validate_frame(frame, pin)?;
    if read_run(frame)? != MeshMissionRun::zeroed() {
        return Err("MeshMissionRun is runtime state and must be zero in an authored scene".into());
    }
    Ok(())
}

pub fn update(frame: &mut Frame, pin: &MeshScenePin) -> Result<(), String> {
    ensure_scene_schema(frame)?;
    ensure_registered(frame)?;
    let config = read_config(frame)?;
    let mut run = read_run(frame)?;
    if config.disabled() {
        validate_run(&config, &run)?;
        return Ok(());
    }
    if pin.is_empty() {
        return Err("an enabled MeshMission requires an admitted mesh graph".into());
    }
    let graph = pin.graph()?;
    validate_config(&config, &graph, pin.max_vertical)?;
    validate_run(&config, &run)?;
    let agent = MeshRuntime::agent(frame, pin.binding()).map_err(|error| error.to_string())?;
    let mut next = run;
    for index in 0..config.count as usize {
        let point_mask = 1u32 << index;
        if next.collected & point_mask == 0 && within_radius(agent.position, config.points[index])?
        {
            next.collected |= point_mask;
        }
    }
    let all_collected = (1u32 << config.count) - 1;
    if next.won == 0
        && next.collected == all_collected
        && within_radius(agent.position, config.exit)?
    {
        next.won = 1;
    }
    validate_run(&config, &next)?;
    if next != run {
        run = next;
        frame.set_singleton(run);
    }
    Ok(())
}

pub(super) struct MissionStep;

impl System<MeshNavigation> for MissionStep {
    fn name(&self) -> &'static str {
        "MeshMissionStep"
    }

    fn run(&mut self, context: &mut SimContext<MeshNavigation>) {
        let status = *context.frame.singleton::<MeshStepStatus>();
        if validate_step_status(status).is_err() || status.failed != 0 {
            return;
        }
        let pin = *context.frame.singleton::<MeshScenePin>();
        if let Err(message) = update(context.frame, &pin) {
            record_failure(context.frame, context.tick, &message);
        }
    }
}

fn validate_point(
    point: MissionPoint,
    graph: &StaticMeshGraph,
    max_vertical: FP,
) -> Result<(), String> {
    if point.radius.raw() <= 0 || point.radius.raw() > MAX_RADIUS_RAW {
        return Err("MeshMission point radius must be positive and at most 4 units".into());
    }
    if point
        .position
        .iter()
        .any(|coordinate| !(-MAX_COORDINATE_RAW..=MAX_COORDINATE_RAW).contains(&coordinate.raw()))
    {
        return Err("MeshMission point coordinates must be within +/-256 units".into());
    }
    let projection = graph
        .project(point.position, max_vertical)
        .map_err(|error| format!("MeshMission point is not on the admitted graph: {error}"))?;
    if projection.position != point.position {
        return Err("MeshMission point must exactly match its graph projection".into());
    }
    Ok(())
}

fn within_radius(position: [FP; 3], point: MissionPoint) -> Result<bool, String> {
    let mut squared_distance = 0i128;
    for (position_axis, point_axis) in position.iter().zip(point.position) {
        let delta = i128::from(position_axis.raw()) - i128::from(point_axis.raw());
        let square = delta
            .checked_mul(delta)
            .ok_or("MeshMission distance overflow")?;
        squared_distance = squared_distance
            .checked_add(square)
            .ok_or("MeshMission distance overflow")?;
    }
    let radius = i128::from(point.radius.raw());
    let radius_squared = radius
        .checked_mul(radius)
        .ok_or("MeshMission radius overflow")?;
    Ok(squared_distance <= radius_squared)
}

fn ensure_registered(frame: &Frame) -> Result<(), String> {
    let registry = frame.registry();
    if registry.singleton_id::<MeshMission>().is_none()
        || registry.singleton_id::<MeshMissionRun>().is_none()
    {
        return Err("MeshMission frame registry is missing mission singletons".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "mission/tests.rs"]
mod tests;
