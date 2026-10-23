//! Separate opt-in hazard gameplay profile. Legacy MeshNavigation is unchanged.
//!
//! A hit rejects the entire attempted movement tick (including a goal request),
//! leaves the agent at its pre-tick position, and stops it. The authored lowest
//! hazard index touched along the attempted route wins. Hazards precede mission
//! collection and exit checks. Loss and win are terminal until snapshot reset.
//! Each attempted tick clones the bounded Frame for transactional validation;
//! this initial profile makes no performance claim.

use super::{
    apply_goal_requests, mission, record_failure, validate_step_status, MeshNavigation,
    MeshScenePin, MeshStepStatus,
};
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::FP;
use orr_navigation::NavigationStatus;
use orr_navigation_runtime::{
    hazards::{sweep_polyline, validate_hazards, SphereHazard, MAX_HAZARDS},
    mesh::MeshRuntime,
};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{Game, SimContext, System};

use crate::yard3d_game::{NoCommand, NoEvent, YardInput};

pub const HAZARDS_NAME: &str = "MeshHazards";
pub const RUN_NAME: &str = "MeshHazardRun";

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct HazardSphere {
    pub center: [FP; 3],
    pub radius: FP,
}

impl From<HazardSphere> for SphereHazard {
    fn from(value: HazardSphere) -> Self {
        Self {
            center: value.center,
            radius: value.radius,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct MeshHazards {
    #[reflect(range = "0..=1")]
    pub enabled: u32,
    #[reflect(range = "0..=8")]
    pub count: u32,
    pub spheres: [HazardSphere; MAX_HAZARDS],
    pub graph_revision: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct MeshHazardRun {
    #[reflect(range = "0..=1")]
    pub lost: u32,
    /// Zero when playing; otherwise the zero-based authored hazard index.
    pub hit_index: u32,
    /// Simulation tick of loss; zero in authored/playing state.
    pub lost_tick: u64,
}

impl MeshHazards {
    pub fn disabled(&self) -> bool {
        *self == Self::zeroed()
    }

    fn geometry(&self) -> Vec<SphereHazard> {
        self.spheres[..self.count as usize]
            .iter()
            .copied()
            .map(Into::into)
            .collect()
    }
}

/// Distinct registry/profile: never register hazard singletons in MeshNavigation.
pub struct MeshHazardNavigation;

impl Game for MeshHazardNavigation {
    type Input = YardInput;
    type Command = NoCommand;
    type Event = NoEvent;
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        MeshNavigation::register(builder);
        builder.register_singleton::<MeshHazards>(HAZARDS_NAME);
        builder.register_singleton::<MeshHazardRun>(RUN_NAME);
    }

    fn setup(_: &mut Frame, _: &()) {}

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(HazardStep)]
    }
}

pub fn register_reflect(types: &mut TypeRegistry) {
    super::register_reflect(types);
    types.register_singleton::<MeshHazards>(HAZARDS_NAME);
    types.register_singleton::<MeshHazardRun>(RUN_NAME);
}

fn ensure_registered(frame: &Frame) -> Result<(), String> {
    super::ensure_scene_schema(frame)?;
    if frame.registry().singleton_id::<MeshHazards>().is_none()
        || frame.registry().singleton_id::<MeshHazardRun>().is_none()
    {
        return Err("MeshHazardNavigation requires hazard singletons".into());
    }
    Ok(())
}

pub fn read_config(frame: &Frame) -> Result<MeshHazards, String> {
    ensure_registered(frame)?;
    Ok(*frame.singleton::<MeshHazards>())
}

pub fn read_run(frame: &Frame) -> Result<MeshHazardRun, String> {
    ensure_registered(frame)?;
    Ok(*frame.singleton::<MeshHazardRun>())
}

/// Spheres need not lie on the surface: a volume may overlap an admitted route.
/// Every unused entry and the complete disabled value must be canonical zero.
pub fn validate_config(config: &MeshHazards, pin: &MeshScenePin) -> Result<(), String> {
    if config.enabled == 0 {
        return if config.disabled() {
            Ok(())
        } else {
            Err("disabled MeshHazards must be canonical zero".into())
        };
    }
    if config.enabled != 1 || !(1..=MAX_HAZARDS as u32).contains(&config.count) {
        return Err("MeshHazards requires enabled=1 and 1..=8 spheres".into());
    }
    let graph = pin.graph()?;
    if config.graph_revision != graph.revision() {
        return Err("MeshHazards graph revision does not match scene pin".into());
    }
    if config.spheres[config.count as usize..]
        .iter()
        .any(|sphere| *sphere != HazardSphere::zeroed())
    {
        return Err("unused MeshHazards spheres must be zero".into());
    }
    validate_hazards(&config.geometry()).map_err(|error| error.to_string())
}

pub fn validate_run(config: &MeshHazards, run: &MeshHazardRun) -> Result<(), String> {
    if config.disabled() {
        return if *run == MeshHazardRun::zeroed() {
            Ok(())
        } else {
            Err("disabled MeshHazards requires canonical zero run state".into())
        };
    }
    if config.enabled != 1
        || !(1..=MAX_HAZARDS as u32).contains(&config.count)
        || run.lost > 1
        || (run.lost == 0 && (run.hit_index != 0 || run.lost_tick != 0))
        || (run.lost == 1 && (run.hit_index >= config.count || run.lost_tick == 0))
    {
        return Err("MeshHazardRun flags or hazard index are invalid".into());
    }
    Ok(())
}

pub fn validate_frame(frame: &Frame, pin: MeshScenePin) -> Result<(), String> {
    ensure_registered(frame)?;
    super::validate_scene_frame(frame)?;
    if *frame.singleton::<MeshScenePin>() != pin {
        return Err("MeshHazardNavigation pin does not match Frame authority".into());
    }
    let config = read_config(frame)?;
    let run = read_run(frame)?;
    validate_config(&config, &pin)?;
    validate_run(&config, &run)?;
    if run.lost == 1 {
        if run.lost_tick > frame.tick() {
            return Err("MeshHazardRun loss tick is in the future".into());
        }
        if mission::read_run(frame)?.won != 0 {
            return Err("MeshHazardNavigation cannot be both lost and won".into());
        }
        let agent = MeshRuntime::agent(frame, pin.binding()).map_err(|error| error.to_string())?;
        if agent.status != NavigationStatus::Stopped {
            return Err("lost MeshHazardNavigation agent must be stopped".into());
        }
    }
    Ok(())
}

/// Admission of authored state rejects runtime loss/win/progress. Reset uses the
/// host's original admitted Frame snapshot, restoring route, mission and hazards.
pub fn admit(frame: &Frame, pin: MeshScenePin) -> Result<(), String> {
    validate_frame(frame, pin)?;
    mission::admit(frame, pin)?;
    if read_run(frame)? != MeshHazardRun::zeroed() {
        return Err("authored MeshHazardRun must be zero".into());
    }
    Ok(())
}

struct HazardStep;

impl System<MeshHazardNavigation> for HazardStep {
    fn name(&self) -> &'static str {
        "MeshHazardStep"
    }

    fn run(&mut self, context: &mut SimContext<MeshHazardNavigation>) {
        let status = *context.frame.singleton::<MeshStepStatus>();
        if validate_step_status(status).is_ok() && status.failed != 0 {
            return;
        }
        let result = step(context.frame, context.inputs);
        if let Err(message) = result {
            record_failure(context.frame, context.tick, &message);
        }
    }
}

fn step(
    frame: &mut Frame,
    inputs: &orr_sim::TickInputs<YardInput, NoCommand>,
) -> Result<(), String> {
    let pin = *frame.singleton::<MeshScenePin>();
    validate_frame(frame, pin)?;
    if read_run(frame)?.lost != 0 || mission::read_run(frame)?.won != 0 {
        return Ok(());
    }
    let config = read_config(frame)?;
    let mut candidate = frame.clone();
    let mut next_pin = pin;
    apply_goal_requests(&mut candidate, inputs, &mut next_pin);
    let before =
        MeshRuntime::agent(&candidate, next_pin.binding()).map_err(|error| error.to_string())?;
    MeshRuntime::step(&mut candidate, next_pin.binding()).map_err(|error| error.to_string())?;
    let after =
        MeshRuntime::agent(&candidate, next_pin.binding()).map_err(|error| error.to_string())?;
    if config.enabled != 0 {
        let graph = next_pin.graph()?;
        let movement = graph
            .movement_polyline(&before.path, before.distance, after.distance)
            .map_err(|error| error.to_string())?;
        if let Some(hit_index) =
            sweep_polyline(&config.geometry(), &movement).map_err(|error| error.to_string())?
        {
            // Discard both input replan and tentative motion. Stop at the old
            // position rather than inventing a rounded time-of-impact point.
            let mut lost = frame.clone();
            MeshRuntime::stop(&mut lost, pin.binding()).map_err(|error| error.to_string())?;
            lost.set_singleton(MeshHazardRun {
                lost: 1,
                hit_index,
                lost_tick: frame.tick(),
            });
            validate_frame(&lost, pin)?;
            *frame = lost;
            return Ok(());
        }
    }
    mission::update(&mut candidate, &next_pin)?;
    validate_frame(&candidate, next_pin)?;
    *frame = candidate;
    Ok(())
}

#[cfg(test)]
#[path = "hazards/tests.rs"]
mod tests;
