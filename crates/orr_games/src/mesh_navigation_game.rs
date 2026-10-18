use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::FP;
use orr_navigation::mesh::StaticMeshGraph;
use orr_navigation_runtime::mesh::{
    decode_asset, AgentSpec, MeshRuntime, RuntimeState, WorldBinding,
};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{Game, PlayerSlot, SimContext, System};

use crate::yard3d_game::{NoCommand, NoEvent, YardInput};

pub const PIN_NAME: &str = "MeshScenePin";
pub const MAX_EMBEDDED_GRAPH_BYTES: usize = 8_192;
pub const MESH_SET_GOAL: u32 = 1 << 31;
const MAX_TARGET_COORDINATE_RAW: i64 = 256 * FP::SCALE;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct MeshScenePin {
    #[reflect(range = "0..=8192")]
    pub graph_len: u32,
    #[reflect(skip)]
    pub reserved: u32,
    pub graph_bytes: [u8; MAX_EMBEDDED_GRAPH_BYTES],
    pub graph_revision: [u8; 32],
    pub start: [FP; 3],
    pub goal: [FP; 3],
    pub max_vertical: FP,
    pub distance_per_tick: FP,
}

impl MeshScenePin {
    pub fn new(graph: &StaticMeshGraph, spec: AgentSpec) -> Result<Self, String> {
        validate_spec(spec)?;
        let graph_bytes = graph.to_bytes();
        if !(48..=MAX_EMBEDDED_GRAPH_BYTES).contains(&graph_bytes.len()) {
            return Err("static mesh graph must contain 48..=8192 bytes".into());
        }
        let loaded = decode_asset(&graph_bytes).map_err(|error| error.to_string())?;
        if loaded.revision() != graph.revision() {
            return Err("static mesh asset revision does not match its canonical graph".into());
        }
        loaded
            .find_path(
                spec.start,
                spec.goal,
                spec.max_vertical,
                orr_navigation::SearchBudget {
                    max_expansions: loaded.triangles().len() as u32,
                },
            )
            .map_err(|error| error.to_string())?;
        let mut pin = Self::zeroed();
        pin.graph_len = u32::try_from(graph_bytes.len())
            .map_err(|_| "static mesh asset exceeds the embedded byte limit")?;
        pin.graph_bytes[..graph_bytes.len()].copy_from_slice(&graph_bytes);
        pin.graph_revision = graph.revision();
        pin.start = spec.start;
        pin.goal = spec.goal;
        pin.max_vertical = spec.max_vertical;
        pin.distance_per_tick = spec.distance_per_tick;
        pin.graph()?;
        Ok(pin)
    }

    pub fn graph(&self) -> Result<StaticMeshGraph, String> {
        let length = self.graph_len as usize;
        if !(8..=MAX_EMBEDDED_GRAPH_BYTES).contains(&length)
            || self.reserved != 0
            || self.graph_bytes[length..].iter().any(|byte| *byte != 0)
        {
            return Err(
                "mesh scene pin has an invalid graph length, reserved value or nonzero tail".into(),
            );
        }
        validate_spec(self.spec())?;
        let bytes = &self.graph_bytes[..length];
        let graph = decode_asset(bytes).map_err(|error| error.to_string())?;
        if graph.revision() != self.graph_revision || graph.to_bytes() != bytes {
            return Err(
                "mesh scene pin graph bytes do not match the pinned canonical revision".into(),
            );
        }
        Ok(graph)
    }

    pub fn spec(&self) -> AgentSpec {
        AgentSpec {
            start: self.start,
            goal: self.goal,
            max_vertical: self.max_vertical,
            distance_per_tick: self.distance_per_tick,
        }
    }

    pub fn binding(&self) -> WorldBinding {
        WorldBinding {
            world_revision: self.graph_revision,
            graph_revision: self.graph_revision,
        }
    }

    pub fn is_empty(&self) -> bool {
        bytemuck::bytes_of(self).iter().all(|byte| *byte == 0)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct MeshStepStatus {
    pub failed: u32,
    pub error_len: u32,
    pub failed_tick: u64,
    pub error: [u8; 256],
}

impl MeshStepStatus {
    pub fn message(&self) -> String {
        String::from_utf8_lossy(&self.error[..(self.error_len as usize).min(self.error.len())])
            .into_owned()
    }
}

pub struct MeshNavigation;

pub fn register_reflect(types: &mut TypeRegistry) {
    orr_physics3d::register_reflect(types);
    types.register_singleton::<MeshScenePin>(PIN_NAME);
    types.register_singleton::<MeshStepStatus>("MeshStepStatus");
}

impl Game for MeshNavigation {
    type Input = YardInput;
    type Command = NoCommand;
    type Event = NoEvent;
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        orr_physics3d::register(builder);
        MeshRuntime::register(builder);
        builder.register_singleton::<MeshScenePin>(PIN_NAME);
        builder.register_singleton::<MeshStepStatus>("MeshStepStatus");
    }

    fn setup(_: &mut Frame, _: &()) {}

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(MeshStep)]
    }
}

pub fn validate_scene_frame(frame: &Frame) -> Result<(), String> {
    ensure_scene_schema(frame)?;
    if frame.count::<orr_physics3d::Body>() != 0 || frame.count::<orr_physics3d::Collider>() != 0 {
        return Err("MeshNavigation rejects physics bodies and colliders".into());
    }
    if frame.alive_count() != 1 || frame.count::<orr_navigation_runtime::mesh::RuntimeAgent>() != 1
    {
        return Err("MeshNavigation requires exactly one derived runtime agent".into());
    }
    let status = *frame.singleton::<MeshStepStatus>();
    validate_step_status(status)
        .map_err(|()| "MeshNavigation step status is malformed".to_string())?;
    if status.failed != 0 {
        return Err(format!(
            "MeshNavigation runtime failed: {}",
            status.message()
        ));
    }
    validate_runtime(frame, *frame.singleton::<MeshScenePin>())
}

struct MeshStep;

impl System<MeshNavigation> for MeshStep {
    fn name(&self) -> &'static str {
        "MeshStep"
    }

    fn run(&mut self, context: &mut SimContext<MeshNavigation>) {
        let step_status = *context.frame.singleton::<MeshStepStatus>();
        match validate_step_status(step_status) {
            Ok(()) if step_status.failed != 0 => return,
            Ok(()) => {}
            Err(()) => {
                record_failure(
                    context.frame,
                    context.tick,
                    "MeshNavigation step status is malformed",
                );
                return;
            }
        }
        let mut pin = *context.frame.singleton::<MeshScenePin>();
        let result = validate_runtime(context.frame, pin).and_then(|()| {
            apply_goal_input(context, &mut pin);
            MeshRuntime::step(context.frame, pin.binding()).map_err(|error| error.to_string())
        });
        if let Err(message) = result {
            record_failure(context.frame, context.tick, &message);
        }
    }
}

fn apply_goal_input(context: &mut SimContext<MeshNavigation>, pin: &mut MeshScenePin) {
    let graph = match pin.graph() {
        Ok(graph) => graph,
        Err(_) => return,
    };
    let binding = pin.binding();
    let initial_agent = match MeshRuntime::agent(context.frame, binding) {
        Ok(agent) => agent,
        Err(_) => return,
    };
    for player_index in 0..context.inputs.player_count() {
        let slot = PlayerSlot(player_index);
        if context.inputs.flags(slot).disconnected {
            continue;
        }
        let Some(goal) = requested_goal(*context.inputs.input(slot)) else {
            continue;
        };
        if goal == initial_agent.spec.goal
            && initial_agent.status != orr_navigation::NavigationStatus::Stopped
        {
            return;
        }
        if MeshRuntime::replan(
            context.frame,
            &graph,
            binding,
            goal,
            pin.max_vertical,
            pin.distance_per_tick,
        )
        .is_ok()
        {
            pin.start = initial_agent.position;
            pin.goal = goal;
            context.frame.set_singleton(*pin);
            return;
        }
    }
}

fn requested_goal(input: YardInput) -> Option<[FP; 3]> {
    if input.buttons != MESH_SET_GOAL || input._pad != 0 || input.dir != [0; 3] {
        return None;
    }
    if input.origin.iter().any(|coordinate| {
        let raw = i64::from(*coordinate);
        !(-MAX_TARGET_COORDINATE_RAW..=MAX_TARGET_COORDINATE_RAW).contains(&raw)
    }) {
        return None;
    }
    Some(
        input
            .origin
            .map(|coordinate| FP::from_raw(i64::from(coordinate))),
    )
}

fn ensure_scene_schema(frame: &Frame) -> Result<(), String> {
    let registry = frame.registry();
    if registry.component_id::<orr_physics3d::Body>().is_none()
        || registry.component_id::<orr_physics3d::Collider>().is_none()
        || registry
            .component_id::<orr_navigation_runtime::mesh::RuntimeAgent>()
            .is_none()
        || registry.singleton_id::<MeshScenePin>().is_none()
        || registry.singleton_id::<MeshStepStatus>().is_none()
        || registry.singleton_id::<RuntimeState>().is_none()
        || registry
            .list_id::<orr_navigation_runtime::mesh::AssetByte>()
            .is_none()
    {
        return Err("MeshNavigation frame registry is missing required types".into());
    }
    Ok(())
}

fn validate_step_status(status: MeshStepStatus) -> Result<(), ()> {
    let error_len = status.error_len as usize;
    if error_len > status.error.len() || status.error[error_len..].iter().any(|byte| *byte != 0) {
        return Err(());
    }
    match status.failed {
        0 if status.error_len == 0 && status.failed_tick == 0 => Ok(()),
        1 if error_len > 0 && std::str::from_utf8(&status.error[..error_len]).is_ok() => Ok(()),
        _ => Err(()),
    }
}

fn validate_runtime(frame: &Frame, pin: MeshScenePin) -> Result<(), String> {
    if pin.is_empty() {
        return Err("MeshNavigation requires a nonempty admitted static mesh scene pin".into());
    }
    if frame.count::<orr_physics3d::Body>() != 0
        || frame.count::<orr_physics3d::Collider>() != 0
        || frame.alive_count() != 1
        || frame.count::<orr_navigation_runtime::mesh::RuntimeAgent>() != 1
    {
        return Err(
            "MeshNavigation frame contains unsupported entities or physics components".into(),
        );
    }
    let graph = pin.graph()?;
    let binding = pin.binding();
    if graph.revision() != binding.graph_revision {
        return Err("MeshNavigation graph does not match its world binding".into());
    }
    let runtime_state = *frame.singleton::<RuntimeState>();
    if runtime_state.binding != binding {
        return Err("MeshNavigation runtime binding does not match the saved scene pin".into());
    }
    MeshRuntime::validate(frame, binding).map_err(|error| error.to_string())?;
    let readback = MeshRuntime::agent(frame, binding).map_err(|error| error.to_string())?;
    if readback.spec != pin.spec() || readback.binding != binding {
        return Err("MeshNavigation runtime agent does not match the saved scene pin".into());
    }
    Ok(())
}

fn validate_spec(spec: AgentSpec) -> Result<(), String> {
    if spec.max_vertical.raw() < 0 || spec.distance_per_tick.raw() <= 0 {
        return Err(
            "mesh scene pin requires nonnegative vertical tolerance and positive per-tick distance"
                .into(),
        );
    }
    Ok(())
}

fn record_failure(frame: &mut Frame, tick: u64, message: &str) {
    let mut status = MeshStepStatus::zeroed();
    status.failed = 1;
    status.failed_tick = tick;
    let mut length = message.len().min(status.error.len());
    while !message.is_char_boundary(length) {
        length -= 1;
    }
    status.error[..length].copy_from_slice(&message.as_bytes()[..length]);
    status.error_len = length as u32;
    frame.set_singleton(status);
}

#[cfg(test)]
#[path = "mesh_play_input_tests.rs"]
mod mesh_play_input_tests;
