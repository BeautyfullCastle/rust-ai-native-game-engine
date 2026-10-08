//! Opt-in, scene-pinned point navigation. No physics solver, spawning, or input effects.
//! The complete route and its immutable dependencies live in the rollback Frame.
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::FP;
use orr_navigation::{AgentProfile, TerrainGraph};
use orr_navigation_runtime::{AgentSpec, NavigationRuntime, RuntimeState};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{Game, SimContext, System};
use orr_terrain::Terrain;

use crate::yard3d_game::{NoCommand, NoEvent, YardInput};

pub const PIN_NAME: &str = "NavigationScenePin";

/// The editor may deliberately stage an empty scene; a consuming runtime may not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionMode {
    EditorStaging,
    RequiredPin,
}

/// Pure pre-read admission shared by local editor and closed project consumers.
/// `authored_entities` and `explicit_pin` describe the document before baking.
/// No path is resolved or opened here, and failure never changes the Frame.
pub fn admission_pin(
    frame: &Frame,
    authored_entities: usize,
    explicit_pin: bool,
    mode: AdmissionMode,
) -> Result<Option<NavigationScenePin>, String> {
    if authored_entities != 0 || frame.alive_count() != 0 {
        return Err("NavigationYard3D rejects every authored entity, body and collider".into());
    }
    if bytemuck::bytes_of(frame.singleton::<NavigationStepStatus>())
        .iter()
        .any(|&byte| byte != 0)
    {
        return Err("runtime failure status cannot be authored into a scene".into());
    }
    let pin = *frame.singleton::<NavigationScenePin>();
    if !explicit_pin || bytemuck::bytes_of(&pin).iter().all(|&byte| byte == 0) {
        return match mode {
            AdmissionMode::EditorStaging => Ok(None),
            AdmissionMode::RequiredPin => {
                Err("navigation runtime requires an explicit nonzero scene pin".into())
            }
        };
    }
    validate_relative_source(pin.source()?)?;
    pin.identity()?;
    if pin.agent.distance_per_tick <= FP::ZERO {
        return Err("navigation distance per tick must be positive".into());
    }
    Ok(Some(pin))
}

/// Portable path syntax only. The host separately rejects symlinks and special
/// files while resolving this path relative to the original scene directory.
pub fn validate_relative_source(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part == ".orr")
        || !path.ends_with(".orrt")
        || path.rsplit('/').next() == Some(".orrt")
    {
        return Err("navigation source must be a confined scene-relative .orrt path".into());
    }
    Ok(())
}

/// Closed playground presentation contract, deliberately separate from the
/// wider legacy headless admission. Check every vertex, including intermediate
/// heights and hole vertices, before any route arithmetic or GPU conversion.
pub fn validate_project_bounds(
    terrain: &Terrain,
    agent: NavigationAgentSpec,
) -> Result<(), String> {
    const LIMIT_RAW: i64 = 256 * 65536;
    let bounded = |value: FP| (-LIMIT_RAW..=LIMIT_RAW).contains(&value.raw());
    if terrain.width() > orr_navigation_runtime::MAX_SIDE
        || terrain.depth() > orr_navigation_runtime::MAX_SIDE
    {
        return Err("navigation terrain exceeds the 17 by 17 point-agent scope".into());
    }
    if agent
        .start
        .into_iter()
        .chain(agent.goal)
        .any(|v| !bounded(v))
    {
        return Err("navigation project start and goal must be within +/-256".into());
    }
    for index in 0..terrain.width() * terrain.depth() {
        if terrain
            .vertex_position(index)
            .is_none_or(|position| position.into_iter().any(|v| !bounded(v)))
        {
            return Err("navigation project terrain XYZ must all be within +/-256".into());
        }
    }
    Ok(())
}

/// Admit an owned terrain snapshot using full asset/terrain/graph identity.
/// This preserves the wider existing headless coordinate contract; a bounded
/// presentation consumer must enforce its own narrower bounds before calling.
pub fn admit_scene(
    frame: &mut Frame,
    authored_entities: usize,
    explicit_pin: bool,
    terrain_bytes: Option<&[u8]>,
    mode: AdmissionMode,
) -> Result<(), String> {
    let Some(pin) = admission_pin(frame, authored_entities, explicit_pin, mode)? else {
        return Ok(());
    };
    let bytes = terrain_bytes.ok_or("navigation pinned terrain bytes are missing")?;
    let terrain = Terrain::load(bytes).map_err(|e| e.to_string())?;
    if terrain.asset_id() != pin.identity()? {
        return Err("navigation asset identity does not match the scene pin".into());
    }
    if terrain.revision() != pin.terrain_revision {
        return Err("navigation full terrain SHA-256 revision does not match the scene pin".into());
    }
    if terrain.width() > orr_navigation_runtime::MAX_SIDE
        || terrain.depth() > orr_navigation_runtime::MAX_SIDE
    {
        return Err("navigation terrain exceeds the 17 by 17 point-agent scope".into());
    }
    let graph = TerrainGraph::build(
        &terrain,
        AgentProfile {
            max_slope: pin.agent.max_slope,
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        },
    )
    .map_err(|e| e.to_string())?;
    if graph.revision() != pin.graph_revision {
        return Err("navigation full graph SHA-256 revision does not match the scene pin".into());
    }
    NavigationRuntime::admit(frame, &terrain, &graph, pin.agent.to_runtime())
        .map_err(|e| e.to_string())?;
    validate_scene_frame(frame)
}

/// The one point agent authored with this scene. Only slope is configurable:
/// radius, headroom, and step clearance are precisely zero.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct NavigationAgentSpec {
    pub start: [FP; 2],
    pub goal: [FP; 2],
    #[reflect(range = "0..=256")]
    pub max_slope: FP,
    #[reflect(range = "0..=256")]
    pub distance_per_tick: FP,
}
impl NavigationAgentSpec {
    pub fn to_runtime(self) -> AgentSpec {
        AgentSpec {
            start: self.start,
            goal: self.goal,
            max_slope: self.max_slope,
            distance_per_tick: self.distance_per_tick,
        }
    }
}
impl From<AgentSpec> for NavigationAgentSpec {
    fn from(spec: AgentSpec) -> Self {
        Self {
            start: spec.start,
            goal: spec.goal,
            max_slope: spec.max_slope,
            distance_per_tick: spec.distance_per_tick,
        }
    }
}

/// Full immutable asset identity and SHA-256 dependencies. The path is host-only,
/// confined relative to the original scene directory; no simulation step reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct NavigationScenePin {
    pub source_len: u32,
    pub identity_len: u32,
    pub source_path: [u8; 256],
    pub asset_id: [u8; 256],
    pub terrain_revision: [u8; 32],
    pub graph_revision: [u8; 32],
    pub agent: NavigationAgentSpec,
}
impl NavigationScenePin {
    pub fn new(
        source_path: &str,
        terrain: &Terrain,
        graph: &TerrainGraph,
        agent: NavigationAgentSpec,
    ) -> Result<Self, String> {
        if source_path.is_empty() || source_path.len() > 256 {
            return Err("navigation source path must contain 1..=256 UTF-8 bytes".into());
        }
        if graph.validate_terrain(terrain).is_err()
            || graph.profile()
                != (AgentProfile {
                    max_slope: agent.max_slope,
                    radius: FP::ZERO,
                    headroom: FP::ZERO,
                    max_step: FP::ZERO,
                })
        {
            return Err("navigation graph does not match the terrain and agent profile".into());
        }
        let mut pin = Self::zeroed();
        pin.source_len = source_path.len() as u32;
        pin.identity_len = terrain.asset_id().len() as u32;
        pin.source_path[..source_path.len()].copy_from_slice(source_path.as_bytes());
        pin.asset_id[..terrain.asset_id().len()].copy_from_slice(terrain.asset_id().as_bytes());
        pin.terrain_revision = terrain.revision();
        pin.graph_revision = graph.revision();
        pin.agent = agent;
        Ok(pin)
    }
    pub fn source(&self) -> Result<&str, String> {
        decode_text(&self.source_path, self.source_len, "source path")
    }
    pub fn identity(&self) -> Result<&str, String> {
        decode_text(&self.asset_id, self.identity_len, "asset identity")
    }
}

fn decode_text<'a>(bytes: &'a [u8; 256], length: u32, name: &str) -> Result<&'a str, String> {
    let length = length as usize;
    if length == 0 || length > bytes.len() || bytes[length..].iter().any(|&b| b != 0) {
        return Err(format!(
            "navigation {name} has an invalid length or nonzero tail"
        ));
    }
    std::str::from_utf8(&bytes[..length]).map_err(|_| format!("navigation {name} is not UTF-8"))
}

/// Authoritative halt on a rejected runtime step; stable in snapshots and replay.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct NavigationStepStatus {
    pub failed: u32,
    pub error_len: u32,
    pub failed_tick: u64,
    pub error: [u8; 256],
}
impl NavigationStepStatus {
    pub fn message(&self) -> String {
        String::from_utf8_lossy(&self.error[..(self.error_len as usize).min(self.error.len())])
            .into_owned()
    }
}

/// There are no authored entities in this game. The only live entity is the
/// derived runtime agent, created transactionally by the bake admission hook.
pub fn validate_scene_frame(frame: &Frame) -> Result<(), String> {
    if frame.count::<orr_physics3d::Body>() != 0 || frame.count::<orr_physics3d::Collider>() != 0 {
        return Err("NavigationYard3D rejects all physics bodies and colliders".into());
    }
    NavigationRuntime::validate(frame).map_err(|e| e.to_string())?;
    if frame.alive_count() != 1 {
        return Err("NavigationYard3D admits exactly one derived agent".into());
    }
    Ok(())
}

pub struct NavigationYard3D;
pub fn register_reflect(types: &mut TypeRegistry) {
    orr_physics3d::register_reflect(types);
    types.register_singleton::<NavigationScenePin>(PIN_NAME);
    types.register_singleton::<NavigationStepStatus>("NavigationStepStatus");
}
impl Game for NavigationYard3D {
    type Input = YardInput;
    type Command = NoCommand;
    type Event = NoEvent;
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        orr_physics3d::register(builder); // read-only Yard extractor compatibility
        NavigationRuntime::register(builder);
        builder.register_singleton::<NavigationScenePin>(PIN_NAME);
        builder.register_singleton::<NavigationStepStatus>("NavigationStepStatus");
    }
    fn setup(_: &mut Frame, _: &()) {
        // Empty means authoring only. Admission owns the sole agent and full route.
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(NavigationStep)]
    }
}

struct NavigationStep;
impl System<NavigationYard3D> for NavigationStep {
    fn name(&self) -> &'static str {
        "NavigationStep"
    }
    fn run(&mut self, ctx: &mut SimContext<NavigationYard3D>) {
        if ctx.frame.singleton::<NavigationStepStatus>().failed != 0 {
            return;
        }
        let pin = *ctx.frame.singleton::<NavigationScenePin>();
        let state = *ctx.frame.singleton::<RuntimeState>();
        let result = (|| {
            if !state.is_active() {
                return Err("NavigationYard3D requires an admitted navigation route".into());
            }
            let agent = ctx
                .frame
                .get::<orr_navigation_runtime::RuntimeAgent>(state.agent)
                .ok_or_else(|| "navigation agent is missing".to_string())?;
            let (terrain, graph, _, spec) = orr_navigation_runtime::decode_scene(
                &state,
                agent,
                ctx.frame.list(state.terrain_bytes),
                ctx.frame.list(state.graph_bytes),
                ctx.frame.list(state.navigator_bytes),
            )
            .map_err(|e| e.to_string())?;
            if terrain.asset_id() != pin.identity()?
                || terrain.revision() != pin.terrain_revision
                || graph.revision() != pin.graph_revision
                || spec != pin.agent.to_runtime()
            {
                return Err("navigation runtime no longer matches the scene pin".into());
            }
            NavigationRuntime::step(ctx.frame).map_err(|e| e.to_string())
        })();
        if let Err(message) = result {
            let mut status = NavigationStepStatus::zeroed();
            status.failed = 1;
            status.failed_tick = ctx.tick;
            let mut length = message.len().min(status.error.len());
            while !message.is_char_boundary(length) {
                length -= 1;
            }
            status.error[..length].copy_from_slice(&message.as_bytes()[..length]);
            status.error_len = length as u32;
            ctx.frame.set_singleton(status);
        }
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use orr_fp::fp;
    use orr_sim::Simulation;

    fn fixture() -> (Frame, Vec<u8>) {
        let terrain =
            Terrain::load(include_bytes!("../../../scenes/navigation/point_demo.orrt")).unwrap();
        let agent = NavigationAgentSpec {
            start: [fp!(0.25), fp!(0.75)],
            goal: [fp!(0.75), fp!(0.25)],
            max_slope: FP::ONE,
            distance_per_tick: fp!(0.125),
        };
        let graph = TerrainGraph::build(&terrain, AgentProfile::default()).unwrap();
        let mut frame = Frame::new(Simulation::<NavigationYard3D>::build_registry());
        frame.set_singleton(
            NavigationScenePin::new("navigation/point_demo.orrt", &terrain, &graph, agent).unwrap(),
        );
        (frame, terrain.cook())
    }

    #[test]
    fn editor_staging_is_explicitly_distinct_from_required_runtime_pin() {
        let mut frame = Frame::new(Simulation::<NavigationYard3D>::build_registry());
        let before = frame.to_bytes();
        admit_scene(&mut frame, 0, false, None, AdmissionMode::EditorStaging).unwrap();
        assert_eq!(frame.to_bytes(), before);
        assert!(admit_scene(&mut frame, 0, false, None, AdmissionMode::RequiredPin).is_err());
        assert!(admit_scene(&mut frame, 0, true, None, AdmissionMode::RequiredPin).is_err());
        assert_eq!(frame.to_bytes(), before);
    }

    #[test]
    fn full_revisions_identity_and_authored_state_are_checked_before_mutation() {
        for field in [
            "terrain", "graph", "identity", "distance", "status", "entity", "tail",
        ] {
            let (mut frame, bytes) = fixture();
            let mut pin = *frame.singleton::<NavigationScenePin>();
            match field {
                "terrain" => pin.terrain_revision[31] ^= 1,
                "graph" => pin.graph_revision[31] ^= 1,
                "identity" => pin.asset_id[0] ^= 1,
                "distance" => pin.agent.distance_per_tick = FP::ZERO,
                "status" => frame.singleton_mut::<NavigationStepStatus>().failed = 1,
                "entity" => {
                    frame.spawn();
                }
                "tail" => pin.source_path[255] = 1,
                _ => unreachable!(),
            }
            frame.set_singleton(pin);
            let before = frame.to_bytes();
            assert!(
                admit_scene(
                    &mut frame,
                    0,
                    true,
                    Some(&bytes),
                    AdmissionMode::RequiredPin
                )
                .is_err(),
                "{field}"
            );
            assert_eq!(frame.to_bytes(), before, "{field}");
        }
        let (mut frame, bytes) = fixture();
        assert!(admit_scene(
            &mut frame,
            1,
            true,
            Some(&bytes),
            AdmissionMode::RequiredPin
        )
        .is_err());
        assert!(admit_scene(&mut frame, 0, true, None, AdmissionMode::RequiredPin).is_err());
        admit_scene(
            &mut frame,
            0,
            true,
            Some(&bytes),
            AdmissionMode::RequiredPin,
        )
        .unwrap();
        validate_scene_frame(&frame).unwrap();
        assert_eq!(frame.alive_count(), 1);
    }

    #[test]
    fn portable_pin_paths_reject_every_escape_spelling() {
        for path in [
            "",
            "/a.orrt",
            "../a.orrt",
            "a/../b.orrt",
            "./a.orrt",
            "a//b.orrt",
            "a\\b.orrt",
            "C:a.orrt",
            ".orr/a.orrt",
            "a\0.orrt",
            "a.ORRT",
            ".orrt",
        ] {
            assert!(validate_relative_source(path).is_err(), "{path:?}");
        }
        validate_relative_source("navigation/point_demo.orrt").unwrap();
    }

    #[test]
    fn closed_bounds_check_intermediate_xyz_but_do_not_narrow_headless_admission() {
        let (mut frame, _) = fixture();
        let mut agent = frame.singleton::<NavigationScenePin>().agent;
        let origin = fp!(300);
        let terrain = Terrain::new(
            "wide".into(),
            2,
            2,
            [origin; 2],
            FP::ONE,
            vec![FP::ZERO; 4],
            vec![false],
        )
        .unwrap();
        agent.start = [origin + fp!(0.25), origin + fp!(0.75)];
        agent.goal = [origin + fp!(0.75), origin + fp!(0.25)];
        let graph = TerrainGraph::build(&terrain, AgentProfile::default()).unwrap();
        frame.set_singleton(NavigationScenePin::new("wide.orrt", &terrain, &graph, agent).unwrap());
        assert!(validate_project_bounds(&terrain, agent).is_err());
        admit_scene(
            &mut frame,
            0,
            true,
            Some(&terrain.cook()),
            AdmissionMode::RequiredPin,
        )
        .unwrap();

        agent.start = [FP::ZERO; 2];
        agent.goal = [FP::ONE; 2];
        let mut heights = vec![FP::ZERO; 9];
        heights[4] = fp!(256);
        let at_limit = Terrain::new(
            "bounds".into(),
            3,
            3,
            [FP::ZERO; 2],
            FP::ONE,
            heights.clone(),
            vec![false; 4],
        )
        .unwrap();
        validate_project_bounds(&at_limit, agent).unwrap();
        heights[4] = FP::from_raw(fp!(256).raw() + 1);
        let over_limit = Terrain::new(
            "bounds".into(),
            3,
            3,
            [FP::ZERO; 2],
            FP::ONE,
            heights,
            vec![false; 4],
        )
        .unwrap();
        assert!(validate_project_bounds(&over_limit, agent).is_err());
    }
}
