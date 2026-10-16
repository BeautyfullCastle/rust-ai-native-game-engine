//! Opt-in, sphere-only terrain scene. No implicit floor, walls, rain or recycling.
//! Scene admission installs the immutable heightfield in the Frame before play.
//! Simulation never resolves paths or accesses an editor asset cache.
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{fp, FP};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{Game, SimContext, System};
use orr_terrain::Terrain;

use crate::yard3d_game::{NoCommand, NoEvent, YardInput};

pub const PIN_NAME: &str = "TerrainScenePin";
/// Pins both the authored asset identity and all 256 revision bits. The source
/// path is host-only metadata, relative to the scene's original directory.
/// Strings are length-delimited UTF-8 with canonical zero tails because scene
/// reflection deliberately supports only POD values.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct TerrainScenePin {
    pub source_len: u32,
    pub identity_len: u32,
    pub source_path: [u8; 256],
    pub asset_id: [u8; 256],
    pub revision: [u8; 32],
    #[reflect(range = "0..=100")]
    pub friction: FP,
    #[reflect(range = "0..=1")]
    pub restitution: FP,
    pub layer: u32,
    pub mask: u32,
}

impl TerrainScenePin {
    pub fn new(source_path: &str, terrain: &Terrain) -> Result<Self, String> {
        if source_path.is_empty() || source_path.len() > 256 {
            return Err("terrain source path must contain 1..=256 UTF-8 bytes".into());
        }
        let mut pin = Self::zeroed();
        pin.source_len = source_path.len() as u32;
        pin.identity_len = terrain.asset_id().len() as u32;
        pin.source_path[..source_path.len()].copy_from_slice(source_path.as_bytes());
        pin.asset_id[..terrain.asset_id().len()].copy_from_slice(terrain.asset_id().as_bytes());
        pin.revision = terrain.revision();
        pin.friction = fp!(0.6);
        pin.layer = 1;
        pin.mask = u32::MAX;
        Ok(pin)
    }

    pub fn source(&self) -> Result<&str, String> {
        decode_text(&self.source_path, self.source_len, "source path")
    }

    pub fn identity(&self) -> Result<&str, String> {
        decode_text(&self.asset_id, self.identity_len, "asset identity")
    }

    pub fn collider(&self) -> orr_terrain_physics3d::asset::HeightfieldCollider {
        orr_terrain_physics3d::asset::HeightfieldCollider {
            revision: self.revision,
            friction: self.friction,
            restitution: self.restitution,
            layer: self.layer,
            mask: self.mask,
        }
    }
}

fn decode_text<'a>(bytes: &'a [u8; 256], length: u32, name: &str) -> Result<&'a str, String> {
    let length = length as usize;
    if length == 0 || length > bytes.len() || bytes[length..].iter().any(|&b| b != 0) {
        return Err(format!(
            "terrain {name} has an invalid length or nonzero tail"
        ));
    }
    std::str::from_utf8(&bytes[..length]).map_err(|_| format!("terrain {name} is not UTF-8"))
}

/// Runtime failure is authoritative and visible through reflection. A failed
/// terrain step leaves bodies unchanged and halts further physics steps rather
/// than silently switching to convex-only physics or an invisible floor.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct TerrainStepStatus {
    /// 0 = running; 1 = terrain step rejected. Stop and reload the scene to retry.
    pub failed: u32,
    pub error_len: u32,
    pub failed_tick: u64,
    pub error: [u8; 256],
}

impl TerrainStepStatus {
    pub fn message(&self) -> String {
        String::from_utf8_lossy(&self.error[..(self.error_len as usize).min(self.error.len())])
            .into_owned()
    }
}

/// Strict game-level policy: every authored physics object is a dynamic sphere,
/// even if its collision filter would currently exclude the terrain.
pub fn validate_scene_frame(frame: &Frame) -> Result<(), String> {
    let (entities, colliders) = frame.dense::<orr_physics3d::Collider>();
    for (entity, collider) in entities.iter().zip(colliders) {
        let body = frame
            .get::<orr_physics3d::Body>(*entity)
            .ok_or("terrain sphere collider requires a Body")?;
        if body.kind != orr_physics3d::BODY_DYNAMIC
            || collider.shape.kind != orr_physics3d::SHAPE_SPHERE
        {
            return Err("TerrainYard3D admits dynamic spheres only; boxes, capsules and static floor proxies are unsupported".into());
        }
    }
    if frame
        .dense::<orr_physics3d::Body>()
        .0
        .iter()
        .any(|entity| !frame.has::<orr_physics3d::Collider>(*entity))
    {
        return Err("terrain sphere Body requires a Collider".into());
    }
    orr_terrain_physics3d::validate_frame(frame).map_err(|e| e.to_string())
}

pub struct TerrainYard3D;

pub fn register_reflect(types: &mut TypeRegistry) {
    orr_physics3d::register_reflect(types);
    types.register_singleton::<TerrainScenePin>(PIN_NAME);
    types.register_singleton::<TerrainStepStatus>("TerrainStepStatus");
}

impl Game for TerrainYard3D {
    type Input = YardInput;
    type Command = NoCommand;
    type Event = NoEvent;
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        orr_physics3d::register(builder);
        orr_terrain_physics3d::asset::register(builder);
        builder.register_singleton::<TerrainScenePin>(PIN_NAME);
        builder.register_singleton::<TerrainStepStatus>("TerrainStepStatus");
    }

    fn setup(frame: &mut Frame, _: &()) {
        // Only a host-admitted authored scene can run. In particular setup
        // supplies no collision surface or mixed Yard3D fixture population.
        let config = orr_terrain_physics3d::terrain_config();
        orr_physics3d::init(frame, config);
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        // Player spawn and rain are intentionally absent in this first slice:
        // neither replay nor network input can introduce unsupported shapes.
        vec![Box::new(TerrainPhysics::default())]
    }
}

#[derive(Default)]
struct TerrainPhysics {
    scratch: orr_physics3d::Scratch,
}
impl System<TerrainYard3D> for TerrainPhysics {
    fn name(&self) -> &'static str {
        "TerrainPhysics3d"
    }
    fn run(&mut self, ctx: &mut SimContext<TerrainYard3D>) {
        if ctx.frame.singleton::<TerrainStepStatus>().failed != 0 {
            return;
        }
        let asset = ctx
            .frame
            .singleton::<orr_terrain_physics3d::asset::TerrainAsset>();
        let admitted = asset.present == 1
            && asset.revision == ctx.frame.singleton::<TerrainScenePin>().revision;
        let result = if admitted {
            orr_terrain_physics3d::terrain_step(ctx.frame, &mut self.scratch)
                .map_err(|e| e.to_string())
        } else {
            Err("TerrainYard3D requires an admitted, fully pinned terrain asset".into())
        };
        if let Err(message) = result {
            let mut status = TerrainStepStatus::zeroed();
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
mod tests {
    use super::*;
    use orr_sim::{Simulation, TickInputs};

    #[test]
    fn missing_admission_reports_failure_without_installing_a_fallback_floor() {
        let mut sim = Simulation::<TerrainYard3D>::new((), 60, 42);
        assert_eq!(sim.frame().alive_count(), 0);
        sim.step(&TickInputs::new(1, 2));
        let status = sim.frame().singleton::<TerrainStepStatus>();
        assert_eq!(status.failed, 1);
        assert_eq!(status.failed_tick, 1);
        assert!(status.message().contains("requires an admitted"));
        assert_eq!(sim.frame().alive_count(), 0);
        sim.step(&TickInputs::new(2, 2));
        assert_eq!(sim.frame().singleton::<TerrainStepStatus>().failed_tick, 1);
    }
}
