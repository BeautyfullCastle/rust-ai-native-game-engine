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
