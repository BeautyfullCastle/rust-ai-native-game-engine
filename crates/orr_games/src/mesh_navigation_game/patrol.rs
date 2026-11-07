//! Bounded deterministic point-agent patrol. All progress belongs to the Frame.
use super::*;
use orr_navigation::{NavigationStatus, SearchBudget};
pub const PATROL_NAME: &str = "MeshPatrol";
pub const RUN_NAME: &str = "MeshPatrolRun";
pub const MAX_POINTS: usize = 8;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct MeshPatrol {
    #[reflect(range = "0..=1")]
    pub enabled: u32,
    #[reflect(range = "0..=8")]
    pub count: u32,
    #[reflect(range = "0..=1")]
    pub looped: u32,
    #[reflect(range = "0..=3600")]
    pub wait_ticks: u32,
    pub points: [[FP; 3]; MAX_POINTS],
    pub graph_revision: [u8; 32],
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct MeshPatrolRun {
    pub cursor: u32,
    pub wait_remaining: u32,
    pub paused: u32,
    pub completed: u32,
    pub arrived: u32,
    pub reserved: [u32; 3],
}
impl MeshPatrol {
    pub fn disabled(&self) -> bool {
        bytemuck::bytes_of(self).iter().all(|v| *v == 0)
    }
}
pub fn register(builder: &mut ComponentRegistryBuilder) {
    builder.register_singleton::<MeshPatrol>(PATROL_NAME);
    builder.register_singleton::<MeshPatrolRun>(RUN_NAME);
}
pub fn register_reflect(types: &mut TypeRegistry) {
    types.register_singleton::<MeshPatrol>(PATROL_NAME);
}
fn ensure_registered(frame: &Frame) -> Result<(), String> {
    if frame.registry().singleton_id::<MeshPatrol>().is_none()
        || frame.registry().singleton_id::<MeshPatrolRun>().is_none()
    {
        return Err("MeshPatrol frame registry is missing patrol singletons".into());
    }
    Ok(())
}
pub fn read_config(frame: &Frame) -> Result<MeshPatrol, String> {
    ensure_registered(frame)?;
    Ok(*frame.singleton::<MeshPatrol>())
}
pub fn read_run(frame: &Frame) -> Result<MeshPatrolRun, String> {
    ensure_registered(frame)?;
    Ok(*frame.singleton::<MeshPatrolRun>())
}
pub fn validate_config(
    config: &MeshPatrol,
    graph: &StaticMeshGraph,
    start: [FP; 3],
    max_vertical: FP,
) -> Result<(), String> {
    if config.disabled() {
        return Ok(());
    }
    let n = config.count as usize;
    if config.enabled != 1
        || n == 0
        || n > MAX_POINTS
        || config.looped > 1
        || config.wait_ticks > 3600
        || config.graph_revision != graph.revision()
        || config.points[n..].iter().any(|p| *p != [FP::ZERO; 3])
    {
        return Err("MeshPatrol config count, flags, revision or tail is invalid".into());
    }
    if config.points[..n]
        .iter()
        .flatten()
        .any(|v| v.raw().unsigned_abs() > (256 * FP::SCALE) as u64)
    {
        return Err("MeshPatrol point exceeds coordinate bounds".into());
    }
    let budget = SearchBudget {
        max_expansions: graph.triangles().len() as u32,
    };
    let segment = |a, b| -> Result<(), String> {
        let p = graph
            .find_path(a, b, max_vertical, budget)
            .map_err(|e| format!("MeshPatrol segment: {e}"))?;
        graph.path_length(&p).map_err(|e| e.to_string())?;
        Ok(())
    };
    segment(start, config.points[0])?;
    for pair in config.points[..n].windows(2) {
        segment(pair[0], pair[1])?;
    }
    if config.looped == 1 {
        segment(config.points[n - 1], config.points[0])?;
    }
    Ok(())
}
pub fn validate_run(c: &MeshPatrol, r: &MeshPatrolRun) -> Result<(), String> {
    if c.disabled() {
        if *r != MeshPatrolRun::zeroed() {
            return Err("disabled MeshPatrol requires zero runtime state".into());
        }
        return Ok(());
    }
    if c.enabled != 1
        || c.count == 0
        || c.count as usize > MAX_POINTS
        || c.looped > 1
        || c.wait_ticks > 3600
        || r.cursor >= c.count
        || r.wait_remaining > c.wait_ticks
        || r.paused > 1
        || r.completed > 1
        || r.arrived > 1
        || r.reserved != [0; 3]
        || (r.arrived == 0 && r.wait_remaining != 0)
        || (r.completed == 1
            && (c.looped != 0
                || r.cursor + 1 != c.count
                || r.arrived != 1
                || r.wait_remaining != 0))
    {
        return Err("MeshPatrol runtime state is malformed".into());
    }
    Ok(())
}
pub fn validate_frame(frame: &Frame, pin: MeshScenePin) -> Result<(), String> {
    let c = read_config(frame)?;
    let r = read_run(frame)?;
    validate_run(&c, &r)?;
    if c.disabled() {
        return Ok(());
    }
    let graph = pin.graph()?;
    validate_config(&c, &graph, pin.start, pin.max_vertical)?;
    let a = MeshRuntime::agent(frame, pin.binding()).map_err(|e| e.to_string())?;
    if r.paused == 0
        && (pin.goal != c.points[r.cursor as usize]
            || a.spec.goal != pin.goal
            || ((r.arrived == 1 || r.completed == 1)
                && a.status != NavigationStatus::Arrived
                && !terminal_hazard_stop(frame, pin, a.status)))
    {
        return Err("MeshPatrol runtime goal or arrival does not match patrol cursor".into());
    }
    Ok(())
}
// A hazard hit preserves the pre-tick cursor/wait/arrival exactly, then stops
// the agent. Only a valid terminal loss may explain this arrival/status pair.
fn terminal_hazard_stop(frame: &Frame, pin: MeshScenePin, status: NavigationStatus) -> bool {
    #[cfg(feature = "mesh-hazards")]
    {
        let proof = || -> Result<bool, String> {
            let config = super::hazards::read_config(frame)?;
            let run = super::hazards::read_run(frame)?;
            super::hazards::validate_config(&config, &pin)?;
            super::hazards::validate_run(&config, &run)?;
            Ok(status == NavigationStatus::Stopped
                && run.lost == 1
                && run.lost_tick <= frame.tick()
                && super::mission::read_run(frame)?.won == 0)
        };
        proof().unwrap_or(false)
    }
    #[cfg(not(feature = "mesh-hazards"))]
    {
        let _ = (frame, pin, status);
        false
    }
}
/// Called after the ordinary runtime is admitted. Authored runtime state must be zero.
pub fn admit(frame: &mut Frame, pin: &mut MeshScenePin) -> Result<(), String> {
    let c = read_config(frame)?;
    if read_run(frame)? != MeshPatrolRun::zeroed() {
        return Err("MeshPatrolRun must be zero in an authored scene".into());
    }
    if c.disabled() {
        return Ok(());
    }
    let graph = pin.graph()?;
    validate_config(&c, &graph, pin.start, pin.max_vertical)?;
    let mut next = frame.clone();
    let mut p = *pin;
    replan(&mut next, &mut p, c.points[0])?;
    next.set_singleton(MeshPatrolRun::zeroed());
    validate_frame(&next, p)?;
    *frame = next;
    *pin = p;
    Ok(())
}
fn replan(frame: &mut Frame, pin: &mut MeshScenePin, goal: [FP; 3]) -> Result<(), String> {
    let graph = pin.graph()?;
    let position = MeshRuntime::agent(frame, pin.binding())
        .map_err(|e| e.to_string())?
        .position;
    MeshRuntime::replan(
        frame,
        &graph,
        pin.binding(),
        goal,
        pin.max_vertical,
        pin.distance_per_tick,
    )
    .map_err(|e| e.to_string())?;
    pin.start = position;
    pin.goal = goal;
    frame.set_singleton(*pin);
    Ok(())
}
pub fn pause(frame: &mut Frame) -> Result<(), String> {
    let c = read_config(frame)?;
    let mut r = read_run(frame)?;
    validate_run(&c, &r)?;
    if !c.disabled() {
        r.paused = 1;
        frame.set_singleton(r);
    }
    Ok(())
}
pub fn pre_step(frame: &mut Frame, pin: &mut MeshScenePin) -> Result<bool, String> {
    validate_frame(frame, *pin)?;
    let c = read_config(frame)?;
    let mut r = read_run(frame)?;
    if c.disabled() || r.paused == 1 {
        return Ok(true);
    }
    if r.completed == 1 {
        return Ok(false);
    }
    if r.arrived == 1 {
        if r.wait_remaining > 0 {
            r.wait_remaining -= 1;
            frame.set_singleton(r);
            return Ok(false);
        }
        if r.cursor + 1 == c.count {
            if c.looped == 0 {
                r.completed = 1;
                frame.set_singleton(r);
                return Ok(false);
            }
            r.cursor = 0;
        } else {
            r.cursor += 1;
        }
        replan(frame, pin, c.points[r.cursor as usize])?;
        r.arrived = 0;
        r.wait_remaining = 0;
        frame.set_singleton(r);
    }
    Ok(true)
}
pub fn post_step(frame: &mut Frame, pin: MeshScenePin) -> Result<(), String> {
    let c = read_config(frame)?;
    let mut r = read_run(frame)?;
    validate_run(&c, &r)?;
    if c.disabled() || r.paused == 1 || r.completed == 1 {
        return Ok(());
    }
    let a = MeshRuntime::agent(frame, pin.binding()).map_err(|e| e.to_string())?;
    if a.status == NavigationStatus::Arrived && r.arrived == 0 {
        r.arrived = 1;
        r.wait_remaining = c.wait_ticks;
        frame.set_singleton(r);
    }
    validate_frame(frame, pin)
}
#[cfg(test)]
mod tests;
