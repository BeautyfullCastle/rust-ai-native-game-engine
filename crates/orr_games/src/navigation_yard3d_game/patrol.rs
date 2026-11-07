//! Frame-owned deterministic terrain waypoint patrol; authored scene pin stays immutable.
use super::*;
use orr_navigation::{NavigationStatus, Navigator, SearchBudget};
use orr_navigation_runtime::RuntimeAgent;
pub const PATROL_NAME: &str = "TerrainPatrol";
pub const RUN_NAME: &str = "TerrainPatrolRun";
pub const MAX_POINTS: usize = 8;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct TerrainPatrol {
    #[reflect(range = "0..=1")]
    pub enabled: u32,
    #[reflect(range = "0..=8")]
    pub count: u32,
    #[reflect(range = "0..=1")]
    pub looped: u32,
    #[reflect(range = "0..=3600")]
    pub wait_ticks: u32,
    pub points: [[FP; 2]; MAX_POINTS],
    pub graph_revision: [u8; 32],
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct TerrainPatrolRun {
    pub cursor: u32,
    pub wait_remaining: u32,
    pub completed: u32,
    pub arrived: u32,
    pub cycled: u32,
    pub reserved: [u32; 3],
}
impl TerrainPatrol {
    pub fn disabled(&self) -> bool {
        *self == Self::zeroed()
    }
}
pub fn register(b: &mut ComponentRegistryBuilder) {
    b.register_singleton::<TerrainPatrol>(PATROL_NAME);
    b.register_singleton::<TerrainPatrolRun>(RUN_NAME);
}
pub fn register_reflect(t: &mut TypeRegistry) {
    t.register_singleton::<TerrainPatrol>(PATROL_NAME);
}
fn ensure(frame: &Frame) -> Result<(), String> {
    if frame.registry().singleton_id::<TerrainPatrol>().is_none()
        || frame
            .registry()
            .singleton_id::<TerrainPatrolRun>()
            .is_none()
    {
        return Err("terrain patrol registry missing singletons".into());
    }
    Ok(())
}
pub fn read_config(f: &Frame) -> Result<TerrainPatrol, String> {
    ensure(f)?;
    Ok(*f.singleton::<TerrainPatrol>())
}
pub fn read_run(f: &Frame) -> Result<TerrainPatrolRun, String> {
    ensure(f)?;
    Ok(*f.singleton::<TerrainPatrolRun>())
}
pub fn validate_config(
    c: &TerrainPatrol,
    t: &Terrain,
    g: &TerrainGraph,
    start: [FP; 2],
) -> Result<(), String> {
    if c.disabled() {
        return Ok(());
    }
    let n = c.count as usize;
    if c.enabled != 1
        || n == 0
        || n > MAX_POINTS
        || c.looped > 1
        || c.wait_ticks > 3600
        || c.graph_revision != g.revision()
        || c.points[n..].iter().any(|p| *p != [FP::ZERO; 2])
        || c.points[..n]
            .iter()
            .flatten()
            .any(|v| v.raw().unsigned_abs() > orr_navigation::MAX_COORDINATE_RAW as u64)
    {
        return Err("terrain patrol config flags, points, revision or tail invalid".into());
    }
    // Each derived leg must begin at the exact authored XZ, not a snapped point.
    // TerrainGraph currently preserves admitted XZ; keep that contract explicit here.
    for point in std::iter::once(start).chain(c.points[..n].iter().copied()) {
        let projected = g.project(t, point).map_err(|e| {
            format!("terrain patrol point must project at its exact authored XZ: {e}")
        })?;
        if [projected.position[0], projected.position[2]] != point {
            return Err("terrain patrol points must preserve exact authored XZ; clamped points are unsupported".into());
        }
    }
    let check = |a, b| -> Result<(), String> {
        let path = g
            .find_path(
                t,
                a,
                b,
                SearchBudget {
                    max_expansions: orr_navigation_runtime::MAX_ROUTE_TRIANGLES as u32,
                },
            )
            .map_err(|e| e.to_string())?;
        if path.corridor().len() > orr_navigation_runtime::MAX_ROUTE_TRIANGLES {
            return Err("terrain patrol route exceeds runtime bound".into());
        }
        Ok(())
    };
    check(start, c.points[0])?;
    for p in c.points[..n].windows(2) {
        check(p[0], p[1])?;
    }
    if c.looped == 1 {
        check(c.points[n - 1], c.points[0])?;
    }
    Ok(())
}
pub fn validate_run(c: &TerrainPatrol, r: &TerrainPatrolRun) -> Result<(), String> {
    if c.disabled() {
        return if *r == TerrainPatrolRun::zeroed() {
            Ok(())
        } else {
            Err("disabled terrain patrol requires zero runtime".into())
        };
    }
    if c.enabled != 1
        || c.count == 0
        || c.count as usize > MAX_POINTS
        || c.looped > 1
        || c.wait_ticks > 3600
        || r.cursor >= c.count
        || r.wait_remaining > c.wait_ticks
        || r.completed > 1
        || r.arrived > 1
        || r.cycled > 1
        || r.reserved != [0; 3]
        || (r.cycled == 1 && c.looped == 0)
        || (r.arrived == 0 && r.wait_remaining != 0)
        || (r.completed == 1
            && (c.looped != 0
                || r.cursor + 1 != c.count
                || r.arrived != 1
                || r.wait_remaining != 0))
    {
        return Err("terrain patrol runtime state invalid".into());
    }
    Ok(())
}
pub fn validate_runtime_parts(
    c: &TerrainPatrol,
    r: &TerrainPatrolRun,
    pin: &NavigationScenePin,
    t: &Terrain,
    g: &TerrainGraph,
    n: &Navigator,
    spec: &AgentSpec,
) -> Result<(), String> {
    validate_run(c, r)?;
    if t.asset_id() != pin.identity()?
        || t.revision() != pin.terrain_revision
        || g.revision() != pin.graph_revision
        || g.profile().max_slope != pin.agent.max_slope
        || spec.max_slope != pin.agent.max_slope
        || spec.distance_per_tick != pin.agent.distance_per_tick
    {
        return Err("terrain patrol runtime dependencies or speed differ from authored pin".into());
    }
    validate_config(c, t, g, pin.agent.start)?;
    if c.disabled() {
        if *spec != pin.agent.to_runtime() {
            return Err("disabled terrain patrol runtime differs from authored agent".into());
        }
        return Ok(());
    }
    let start = if r.cursor > 0 {
        c.points[r.cursor as usize - 1]
    } else if r.cycled == 1 {
        c.points[c.count as usize - 1]
    } else {
        pin.agent.start
    };
    if spec.start != start
        || spec.goal != c.points[r.cursor as usize]
        || ((r.arrived == 1 || r.completed == 1) && n.status() != NavigationStatus::Arrived)
    {
        return Err("terrain patrol derived leg does not match cursor".into());
    }
    Ok(())
}
fn decoded(f: &Frame) -> Result<(Terrain, TerrainGraph, Navigator, AgentSpec), String> {
    let state = *f.singleton::<RuntimeState>();
    if !state.is_active() {
        return Err("terrain patrol requires admitted runtime".into());
    }
    let a = f
        .get::<RuntimeAgent>(state.agent)
        .ok_or("terrain patrol agent missing")?;
    orr_navigation_runtime::decode_scene(
        &state,
        a,
        f.list(state.terrain_bytes),
        f.list(state.graph_bytes),
        f.list(state.navigator_bytes),
    )
    .map_err(|e| e.to_string())
}
pub fn validate_frame(f: &Frame, pin: NavigationScenePin) -> Result<(), String> {
    let c = read_config(f)?;
    let r = read_run(f)?;
    let (t, g, n, s) = decoded(f)?;
    validate_runtime_parts(&c, &r, &pin, &t, &g, &n, &s)
}
/// Reuse all list/entity handles. Validate a local replacement before any Frame writes.
fn replan(f: &mut Frame, goal: [FP; 2]) -> Result<(), String> {
    let state = *f.singleton::<RuntimeState>();
    let (t, g, mut n, old) = decoded(f)?;
    let p = n.position();
    let spec = AgentSpec {
        start: [p[0], p[2]],
        goal,
        ..old
    };
    n.replan(
        &g,
        &t,
        goal,
        SearchBudget {
            max_expansions: orr_navigation_runtime::MAX_ROUTE_TRIANGLES as u32,
        },
    )
    .map_err(|e| e.to_string())?;
    let snapshot = n.snapshot();
    let agent = RuntimeAgent::from(spec);
    orr_navigation_runtime::decode_scene(
        &state,
        &agent,
        f.list(state.terrain_bytes),
        f.list(state.graph_bytes),
        snapshot.as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    f.list_clear(state.navigator_bytes);
    for byte in snapshot.as_bytes() {
        f.list_push(state.navigator_bytes, *byte);
    }
    f.add(state.agent, agent);
    Ok(())
}
pub fn admit(f: &mut Frame, pin: NavigationScenePin) -> Result<(), String> {
    let c = read_config(f)?;
    if read_run(f)? != TerrainPatrolRun::zeroed() {
        return Err("TerrainPatrolRun must be zero in authored scene".into());
    }
    if pin == NavigationScenePin::zeroed() {
        if c.disabled()
            && *f.singleton::<RuntimeState>() == RuntimeState::zeroed()
            && f.count::<RuntimeAgent>() == 0
            && f.alive_count() == 0
        {
            return Ok(());
        }
        return Err("enabled terrain patrol requires an admitted terrain pin".into());
    }
    let (t, g, _, s) = decoded(f)?;
    if s != pin.agent.to_runtime() {
        return Err("terrain patrol admission requires original agent spec".into());
    }
    validate_config(&c, &t, &g, pin.agent.start)?;
    if c.disabled() {
        return validate_frame(f, pin);
    }
    let mut candidate = f.clone();
    replan(&mut candidate, c.points[0])?;
    candidate.set_singleton(TerrainPatrolRun::zeroed());
    validate_frame(&candidate, pin)?;
    *f = candidate;
    Ok(())
}
pub fn pre_step(f: &mut Frame, pin: NavigationScenePin) -> Result<bool, String> {
    validate_frame(f, pin)?;
    let c = read_config(f)?;
    let mut r = read_run(f)?;
    if c.disabled() {
        return Ok(true);
    }
    if r.completed == 1 {
        return Ok(false);
    }
    if r.arrived == 1 {
        if r.wait_remaining > 0 {
            r.wait_remaining -= 1;
            f.set_singleton(r);
            return Ok(false);
        }
        if r.cursor + 1 == c.count {
            if c.looped == 0 {
                r.completed = 1;
                f.set_singleton(r);
                return Ok(false);
            }
            r.cursor = 0;
            r.cycled = 1;
        } else {
            r.cursor += 1;
        }
        replan(f, c.points[r.cursor as usize])?;
        r.arrived = 0;
        r.wait_remaining = 0;
        f.set_singleton(r);
    }
    Ok(true)
}
pub fn post_step(f: &mut Frame, pin: NavigationScenePin) -> Result<(), String> {
    let c = read_config(f)?;
    let mut r = read_run(f)?;
    validate_run(&c, &r)?;
    if !c.disabled() && r.completed == 0 {
        let state = *f.singleton::<RuntimeState>();
        let a = NavigationRuntime::agent(f, state.agent).map_err(|e| e.to_string())?;
        if a.status == NavigationStatus::Arrived && r.arrived == 0 {
            r.arrived = 1;
            r.wait_remaining = c.wait_ticks;
            f.set_singleton(r);
        }
    }
    validate_frame(f, pin)
}
pub(super) fn tick(f: &mut Frame, pin: NavigationScenePin) -> Result<(), String> {
    let mut candidate = f.clone();
    if pre_step(&mut candidate, pin)? {
        NavigationRuntime::step(&mut candidate).map_err(|e| e.to_string())?;
    }
    post_step(&mut candidate, pin)?;
    *f = candidate;
    Ok(())
}
#[cfg(test)]
mod tests;
