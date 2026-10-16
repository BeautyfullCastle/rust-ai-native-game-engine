//! `Yard3D`: a 3D physics scene for the sample (`orr_physics3d`): a walled
//! yard with a box pyramid, a tower and a wall of boxes, a ramp, and rain of
//! spheres, capsules and small boxes. Players shoot balls along a camera ray
//! and drop bodies where they point, so the pile never settles and remote
//! input keeps mispredicting (which is what causes rollbacks).
//!
//! The ray comes in the input as integers (centimeters and thousandths), so
//! the game never sees a float.
//!
//! This is sim code: no floats, no wall clock, no hash maps.
#![deny(clippy::float_arithmetic)]
#![deny(clippy::disallowed_types)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPQuat, FPVec3, FrameRng, FP};
use orr_physics3d::{init, register, spawn_body, Body, Collider, PhysicsConfig, PhysicsSystem, Shape, BODY_DYNAMIC};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{decode_pod, encode_pod, Game, PlayerSlot, SimCommand, SimContext, System};

/// Fixed sim rate of the sample.
pub const TICK_RATE: u32 = 60;
/// Half size of the square floor.
pub const YARD_HALF: i32 = 24;
/// Muzzle speed of a shot, in units per second.
pub const SHOT_SPEED: i32 = 22;

/// `YardInput::buttons` bit: shoot a ball along the ray.
pub const SHOOT: u32 = 1;
/// Drop a box where the ray meets the floor.
pub const SPAWN_BOX: u32 = 2;
/// Drop a ball where the ray meets the floor.
pub const SPAWN_BALL: u32 = 4;
/// Drop a capsule where the ray meets the floor.
pub const SPAWN_CAPSULE: u32 = 8;

/// One player's input: held buttons and a camera ray.
///
/// `origin` is in centimeters, `dir` in thousandths of a unit (it does not
/// need to be normalized). Shots start at `origin + dir`; a spawn appears 8
/// units above the point where the ray meets the floor.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Pod, Zeroable)]
pub struct YardInput {
    pub buttons: u32,
    pub _pad: u32,
    pub origin: [i32; 3],
    pub dir: [i32; 3],
}

/// The game has no one-off commands; this exists to satisfy the `Game` trait.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Pod, Zeroable)]
pub struct NoCommand {
    pub _pad: u32,
}

impl SimCommand for NoCommand {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_pod(self, out)
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        decode_pod(bytes)
    }
}

/// The game has no sim events.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
pub struct NoEvent {
    pub _pad: u32,
}

/// Parameters of a scene. Every peer must use the same values.
#[derive(Clone, Copy, Debug)]
pub struct YardConfig {
    /// Bodies raining down at the start.
    pub bodies: u32,
    /// Bodies added per second while the game runs (0 = none).
    pub rain_per_second: u32,
    /// Spawning (rain and players) stops at this many entities.
    pub max_entities: u32,
    /// Seed of the scene layout (the sim RNG seed is the session seed).
    pub layout_seed: u64,
}

impl YardConfig {
    pub fn new(bodies: u32) -> Self {
        Self { bodies, rain_per_second: 6, max_entities: 2500, layout_seed: 0x5EED_CAFE }
    }
}

/// Scene constants, set once in `setup`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable, Reflect)]
pub struct Scene {
    pub rain_batch: u32,
    /// Ticks between rain drops; 0 = never.
    pub rain_interval: u32,
    pub max_entities: u32,
    #[reflect(skip)]
    pub _pad: u32,
    /// Seed of the rain: each drop is a pure function of `(rain_seed, tick, index)`, so a
    /// player's spawn never moves the rain of other ticks.
    pub rain_seed: u64,
}

pub struct Yard3D;

/// Registers the types used by Yard3D scene documents.
pub fn register_reflect(types: &mut TypeRegistry) {
    orr_physics3d::register_reflect(types);
    types.register_singleton::<Scene>("Yard3dScene");
}

impl Game for Yard3D {
    type Input = YardInput;
    type Command = NoCommand;
    type Event = NoEvent;
    type Config = YardConfig;

    fn register(builder: &mut ComponentRegistryBuilder) {
        register(builder);
        builder.register_singleton::<Scene>("Yard3dScene");
    }

    fn setup(frame: &mut Frame, config: &YardConfig) {
        build_scene(frame, config);
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(PlayerSystem), Box::new(RainSystem), Box::new(PhysicsSystem::<Yard3D>::new()), Box::new(BoundsSystem)]
    }
}

fn v3(x: FP, y: FP, z: FP) -> FPVec3 {
    FPVec3::new(x, y, z)
}

/// A random dynamic body: a ball, a capsule or a small box, 0.25 to 0.5 in size.
fn random_body(rng: &mut FrameRng, pos: FPVec3) -> (Body, Collider) {
    let kind = rng.next_u32() % 20;
    let rot = FPQuat::from_axis_angle(FPVec3::Y, rng.range_fp(-fp!(3), fp!(3)))
        .hamilton_mul(FPQuat::from_axis_angle(FPVec3::X, rng.range_fp(-fp!(1.5), fp!(1.5))));
    if kind < 9 {
        let s = Shape::sphere(rng.range_fp(fp!(0.28), fp!(0.5)));
        (Body::new_dynamic(pos, &s, FP::ONE), Collider::new(s).with_restitution(fp!(0.3)))
    } else if kind < 17 {
        let s = Shape::capsule(rng.range_fp(fp!(0.25), fp!(0.5)), rng.range_fp(fp!(0.2), fp!(0.3)));
        (Body::new_dynamic(pos, &s, FP::ONE).with_rotation(rot), Collider::new(s).with_restitution(fp!(0.15)))
    } else {
        let s = Shape::cuboid(rng.range_fp(fp!(0.25), fp!(0.45)), rng.range_fp(fp!(0.25), fp!(0.4)), rng.range_fp(fp!(0.25), fp!(0.45)));
        (Body::new_dynamic(pos, &s, FP::ONE).with_rotation(rot), Collider::new(s).with_restitution(fp!(0.1)))
    }
}

fn static_box(frame: &mut Frame, pos: FPVec3, half: FPVec3, rot: FPQuat) {
    spawn_body(frame, Body::new_static(pos).with_rotation(rot), Collider::new(Shape::cuboid(half.x, half.y, half.z)).with_friction(fp!(0.6)));
}

fn crate_box(frame: &mut Frame, pos: FPVec3) {
    let s = Shape::cuboid(FP::HALF, FP::HALF, FP::HALF);
    spawn_body(frame, Body::new_dynamic(pos, &s, FP::ONE), Collider::new(s).with_friction(fp!(0.6)));
}

fn build_scene(frame: &mut Frame, cfg: &YardConfig) {
    init(frame, PhysicsConfig::default());
    let (rain_batch, rain_interval) = match cfg.rain_per_second {
        0 => (0, 0),
        r if r >= TICK_RATE => (r / TICK_RATE, 1),
        r => (1, TICK_RATE / r),
    };
    frame.set_singleton(Scene { rain_batch, rain_interval, max_entities: cfg.max_entities, _pad: 0, rain_seed: cfg.layout_seed ^ 0x7A1D });

    // Floor (top at y = 0) and four low walls.
    let half = FP::from_int(YARD_HALF);
    static_box(frame, v3(FP::ZERO, -FP::ONE, FP::ZERO), v3(half + FP::TWO, FP::ONE, half + FP::TWO), FPQuat::IDENTITY);
    for (sx, sz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let pos = v3((half + FP::HALF) * sx, fp!(1.5), (half + FP::HALF) * sz);
        let h = if sx != 0 { v3(FP::HALF, fp!(1.5), half + FP::ONE) } else { v3(half + FP::ONE, fp!(1.5), FP::HALF) };
        static_box(frame, pos, h, FPQuat::IDENTITY);
    }
    // A ramp.
    static_box(
        frame,
        v3(fp!(-14), fp!(1.2), fp!(8)),
        v3(fp!(3.5), fp!(0.25), fp!(2.5)),
        FPQuat::from_axis_angle(FPVec3::Z, fp!(0.42)),
    );

    // A pyramid of boxes: 6 layers, 1 box less per layer.
    for layer in 0..6 {
        let n = 6 - layer;
        for i in 0..n {
            let x = fp!(-15) + (FP::from_int(i) - FP::from_int(n - 1) / 2) * fp!(1.02);
            crate_box(frame, v3(x, fp!(0.5) + FP::from_int(layer) * fp!(1.001), fp!(-14)));
        }
    }
    // A tower.
    for i in 0..10 {
        crate_box(frame, v3(fp!(15), fp!(0.5) + FP::from_int(i) * fp!(1.001), fp!(-15)));
    }
    // A wall, 7 wide and 5 high, bricks offset every other row.
    for row in 0..5 {
        for i in 0..7 {
            let shift = if row % 2 == 1 { FP::HALF } else { FP::ZERO };
            let x = fp!(13) + FP::from_int(i) * fp!(1.02) + shift;
            crate_box(frame, v3(x, fp!(0.5) + FP::from_int(row) * fp!(1.001), fp!(2)));
        }
    }

    // Rain: a jittered 3D grid above the yard.
    let mut rng = FrameRng::new(cfg.layout_seed);
    let (cols, spacing) = (12u32, fp!(1.5));
    for i in 0..cfg.bodies {
        let (c, layer) = (i % (cols * cols), i / (cols * cols));
        let (cx, cz) = (c % cols, c / cols);
        let jitter = fp!(0.35);
        let x = (FP::from_int(cx as i32) - fp!(5.5)) * spacing + rng.range_fp(-jitter, jitter);
        let z = (FP::from_int(cz as i32) - fp!(5.5)) * spacing + rng.range_fp(-jitter, jitter);
        let y = fp!(9) + FP::from_int(layer as i32) * fp!(1.5) + rng.range_fp(-jitter, jitter);
        let (body, collider) = random_body(&mut rng, v3(x, y, z));
        spawn_body(frame, body, collider);
    }
}

/// Where a ray from `origin` along `dir` meets the plane y = 0, if it points down.
fn floor_hit(origin: FPVec3, dir: FPVec3) -> Option<FPVec3> {
    if dir.y >= -fp!(0.01) {
        return None;
    }
    let t = origin.y / -dir.y;
    (t > FP::ZERO).then(|| origin + dir * t)
}

/// Turns each player's input into shots and spawns.
struct PlayerSystem;

impl System<Yard3D> for PlayerSystem {
    fn name(&self) -> &'static str {
        "PlayerSystem"
    }

    fn run(&mut self, ctx: &mut SimContext<Yard3D>) {
        let scene = *ctx.frame.singleton::<Scene>();
        let players = u32::from(ctx.inputs.player_count());
        let half = FP::from_int(YARD_HALF - 2);
        for slot in 0..players {
            let input = *ctx.inputs.input(PlayerSlot(slot as u8));
            if input.buttons == 0 || ctx.tick % 4 != u64::from(slot % 4) || ctx.frame.alive_count() >= scene.max_entities {
                continue;
            }
            // Inputs come from the network or a replay file: never trust their range.
            let clamp = |v: [i32; 3], lim: i32| v.map(|c| c.clamp(-lim, lim));
            let origin = clamp(input.origin, 20_000);
            let dir = clamp(input.dir, 1_000);
            let origin = v3(FP::from_int(origin[0]) / 100, FP::from_int(origin[1]) / 100, FP::from_int(origin[2]) / 100);
            let dir = v3(FP::from_int(dir[0]) / 1000, FP::from_int(dir[1]) / 1000, FP::from_int(dir[2]) / 1000);
            if input.buttons & SHOOT != 0 {
                let s = Shape::sphere(fp!(0.4));
                let body = Body::new_dynamic(origin + dir * FP::TWO, &s, fp!(2)).with_velocity(dir * FP::from_int(SHOT_SPEED));
                spawn_body(ctx.frame, body, Collider::new(s).with_restitution(fp!(0.2)));
            }
            let spawn_kind = input.buttons & (SPAWN_BOX | SPAWN_BALL | SPAWN_CAPSULE);
            if spawn_kind != 0 {
                let Some(hit) = floor_hit(origin, dir) else { continue };
                let (x, z) = (hit.x.clamp(-half, half), hit.z.clamp(-half, half));
                let rng = ctx.rng();
                let pos = v3(x + rng.range_fp(-fp!(0.2), fp!(0.2)), fp!(8), z + rng.range_fp(-fp!(0.2), fp!(0.2)));
                let rot = FPQuat::from_axis_angle(FPVec3::X, rng.range_fp(-fp!(1.5), fp!(1.5)));
                let (body, collider) = if spawn_kind & SPAWN_BOX != 0 {
                    let s = Shape::cuboid(fp!(0.4), fp!(0.4), fp!(0.4));
                    (Body::new_dynamic(pos, &s, FP::ONE).with_rotation(rot), Collider::new(s))
                } else if spawn_kind & SPAWN_CAPSULE != 0 {
                    let s = Shape::capsule(fp!(0.45), fp!(0.25));
                    (Body::new_dynamic(pos, &s, FP::ONE).with_rotation(rot), Collider::new(s).with_restitution(fp!(0.15)))
                } else {
                    let s = Shape::sphere(fp!(0.4));
                    (Body::new_dynamic(pos, &s, FP::ONE), Collider::new(s).with_restitution(fp!(0.3)))
                };
                spawn_body(ctx.frame, body, collider);
            }
        }
    }
}

/// Drops new bodies from the sky at the configured rate.
struct RainSystem;

impl System<Yard3D> for RainSystem {
    fn name(&self) -> &'static str {
        "RainSystem"
    }

    fn run(&mut self, ctx: &mut SimContext<Yard3D>) {
        let scene = *ctx.frame.singleton::<Scene>();
        if scene.rain_interval == 0 || ctx.tick % u64::from(scene.rain_interval) != 0 {
            return;
        }
        let lim = FP::from_int(YARD_HALF - 4);
        for i in 0..scene.rain_batch {
            if ctx.frame.alive_count() >= scene.max_entities {
                return;
            }
            let mut rng = FrameRng::with_stream(scene.rain_seed ^ ctx.tick, u64::from(i));
            let pos = v3(rng.range_fp(-lim, lim), fp!(20) + fp!(1.4) * i as i32, rng.range_fp(-lim, lim));
            let (body, collider) = random_body(&mut rng, pos);
            spawn_body(ctx.frame, body.with_velocity(v3(FP::ZERO, -FP::TWO, FP::ZERO)), collider);
        }
    }
}

/// Puts a body that left the yard back into the sky.
struct BoundsSystem;

impl System<Yard3D> for BoundsSystem {
    fn name(&self) -> &'static str {
        "BoundsSystem"
    }

    fn run(&mut self, ctx: &mut SimContext<Yard3D>) {
        let lim = FP::from_int(YARD_HALF + 4);
        let escaped: Vec<Entity> = ctx
            .frame
            .query::<(&Body,)>()
            .filter(|(_, (b,))| b.kind == BODY_DYNAMIC && (b.pos.y < -fp!(6) || b.pos.x.abs() > lim || b.pos.z.abs() > lim))
            .map(|(e, _)| e)
            .collect();
        for e in escaped {
            let l = FP::from_int(YARD_HALF - 4);
            let (x, z) = (ctx.rng().range_fp(-l, l), ctx.rng().range_fp(-l, l));
            if let Some(b) = ctx.frame.get_mut::<Body>(e) {
                b.pos = v3(x, fp!(20), z);
                b.vel = FPVec3::ZERO;
                b.omega = FPVec3::ZERO;
            }
        }
    }
}

/// Number of dynamic bodies in a frame.
pub fn dynamic_count(frame: &mut Frame) -> u32 {
    frame.query::<(&Body,)>().filter(|(_, (b,))| b.kind == BODY_DYNAMIC).count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_downward_ray_meets_the_floor() {
        let hit = floor_hit(v3(FP::ZERO, fp!(10), FP::ZERO), v3(fp!(0.5), -fp!(0.5), FP::ZERO)).unwrap();
        assert_eq!(hit.y, FP::ZERO);
        assert!((hit.x - fp!(10)).abs() < fp!(0.01));
        assert!(floor_hit(v3(FP::ZERO, fp!(10), FP::ZERO), v3(FP::ONE, fp!(0.5), FP::ZERO)).is_none());
    }
}
