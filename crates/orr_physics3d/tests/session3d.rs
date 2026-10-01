//! A small headless 3D game ("Yard") through `orr_session`: two peers over
//! a latency loopback, with rollbacks and re-simulation of the 3D physics,
//! must agree on every verified checksum and with one plain simulation fed
//! the same inputs.
//!
//! The game is sim code: integers and `FP` only.
#![deny(clippy::float_arithmetic)]

use std::collections::BTreeMap;

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{fp, FPQuat, FPVec3, FrameRng, FP};
use orr_physics3d::{
    init, register, spawn_body, Body, Collider, PhysicsConfig, PhysicsSystem, Shape, BODY_DYNAMIC,
};
use orr_session::{compare_checksums, AdvanceResult, LoopbackNetwork, Session, SessionConfig};
use orr_sim::{decode_pod, encode_pod, Game, PlayerSlot, SimCommand, SimContext, Simulation, System, TickInputs};

/// One player's input: push direction and a fire button.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Pod, Zeroable)]
struct YardInput {
    x: i32,
    z: i32,
    fire: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Pod, Zeroable)]
struct NoCommand {
    _pad: u32,
}

impl SimCommand for NoCommand {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_pod(self, out)
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        decode_pod(bytes)
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct NoEvent {
    _pad: u32,
}

/// Marks the pusher (kinematic block) of a player.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Pusher {
    slot: u32,
}

struct Yard;

const PUSH_SPEED: FP = FP::from_raw(4 << 16);

struct PusherSystem;

impl System<Yard> for PusherSystem {
    fn name(&self) -> &'static str {
        "PusherSystem"
    }

    fn run(&mut self, ctx: &mut SimContext<Yard>) {
        let players = u32::from(ctx.inputs.player_count());
        let mut shots = Vec::new();
        for (_, (tag, body)) in ctx.frame.query::<(&Pusher, &mut Body)>() {
            if tag.slot >= players {
                continue;
            }
            let input = *ctx.inputs.input(PlayerSlot(tag.slot as u8));
            let (x, z) = (input.x.clamp(-1, 1), input.z.clamp(-1, 1));
            body.vel = FPVec3::new(PUSH_SPEED * x, FP::ZERO, PUSH_SPEED * z);
            if input.fire != 0 && ctx.tick % 4 == u64::from(tag.slot) {
                shots.push(body.pos + FPVec3::new(FP::ZERO, fp!(1.5), FP::ZERO));
            }
        }
        for pos in shots {
            if ctx.frame.alive_count() < 400 {
                let s = Shape::sphere(fp!(0.3));
                spawn_body(ctx.frame, Body::new_dynamic(pos, &s, FP::ONE).with_velocity(FPVec3::new(FP::ZERO, fp!(3), FP::ZERO)), Collider::new(s));
            }
        }
    }
}

impl Game for Yard {
    type Input = YardInput;
    type Command = NoCommand;
    type Event = NoEvent;
    type Config = u32; // dynamic bodies

    fn register(builder: &mut ComponentRegistryBuilder) {
        register(builder);
        builder.register_component::<Pusher>("Pusher");
    }

    fn setup(frame: &mut Frame, bodies: &u32) {
        init(frame, PhysicsConfig::default());
        let wall = |frame: &mut Frame, pos: FPVec3, half: FPVec3| {
            spawn_body(frame, Body::new_static(pos), Collider::new(Shape::cuboid(half.x, half.y, half.z)).with_friction(fp!(0.5)));
        };
        wall(frame, FPVec3::new(FP::ZERO, -FP::HALF, FP::ZERO), FPVec3::new(fp!(12), FP::HALF, fp!(12)));
        for (sx, sz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let pos = FPVec3::new(fp!(12) * sx, fp!(2), fp!(12) * sz);
            let half = if sx != 0 { FPVec3::new(FP::HALF, fp!(2), fp!(12)) } else { FPVec3::new(fp!(12), fp!(2), FP::HALF) };
            wall(frame, pos, half);
        }
        // One kinematic pusher per player, at opposite sides.
        for slot in 0..2u32 {
            let pos = FPVec3::new(fp!(-6) + fp!(12) * slot as i32, fp!(0.5), FP::ZERO);
            let e = spawn_body(frame, Body::new_kinematic(pos), Collider::new(Shape::cuboid(fp!(0.8), fp!(0.5), fp!(2))));
            frame.add(e, Pusher { slot });
        }
        // A tower, then mixed bodies dropped around it.
        for i in 0..6 {
            let s = Shape::cuboid(FP::HALF, FP::HALF, FP::HALF);
            let y = fp!(0.5) + FP::from_int(i) * fp!(1.001);
            spawn_body(frame, Body::new_dynamic(FPVec3::new(FP::ZERO, y, FP::ZERO), &s, FP::ONE), Collider::new(s));
        }
        let mut rng = FrameRng::new(0x7A2D);
        for i in 0..*bodies as i32 {
            let x = rng.range_fp(fp!(-8), fp!(8));
            let z = rng.range_fp(fp!(-8), fp!(8));
            let pos = FPVec3::new(x, fp!(3) + FP::from_int(i % 6), z);
            let rot = FPQuat::from_axis_angle(FPVec3::Y, rng.range_fp(fp!(-3), fp!(3)));
            match i % 3 {
                0 => {
                    let s = Shape::sphere(fp!(0.35));
                    spawn_body(frame, Body::new_dynamic(pos, &s, FP::ONE), Collider::new(s).with_restitution(fp!(0.2)));
                }
                1 => {
                    let s = Shape::cuboid(fp!(0.4), fp!(0.3), fp!(0.5));
                    spawn_body(frame, Body::new_dynamic(pos, &s, FP::ONE).with_rotation(rot), Collider::new(s));
                }
                _ => {
                    let s = Shape::capsule(fp!(0.4), fp!(0.25));
                    spawn_body(frame, Body::new_dynamic(pos, &s, FP::ONE).with_rotation(rot), Collider::new(s));
                }
            }
        }
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(PusherSystem), Box::new(PhysicsSystem::<Yard>::new())]
    }
}

/// A scripted player: holds a direction for 20 ticks at a time and shoots in
/// bursts. A pure function of `(seed, tick, slot)`.
fn bot(seed: u64, tick: u64, slot: u8) -> YardInput {
    let mut rng = FrameRng::new(seed ^ ((tick / 20) << 8) ^ u64::from(slot));
    YardInput { x: rng.range_i32(-1, 2), z: rng.range_i32(-1, 2), fire: u32::from(tick % 60 < 15), _pad: 0 }
}

fn headless_checksum(inputs: &BTreeMap<u64, (YardInput, YardInput)>, bodies: u32, seed: u64, up_to: u64) -> u64 {
    let mut sim = Simulation::<Yard>::new(bodies, 60, seed);
    for tick in 1..=up_to {
        let (a, b) = inputs.get(&tick).copied().unwrap_or_default();
        let mut ti = TickInputs::<YardInput, NoCommand>::new(tick, 2);
        ti.set_input(PlayerSlot(0), a);
        ti.set_input(PlayerSlot(1), b);
        sim.step(&ti);
    }
    sim.checksum()
}

#[test]
fn two_peers_with_rollbacks_agree_and_match_a_plain_simulation() {
    const TICKS: u64 = 600;
    const BODIES: u32 = 60;
    const SEED: u64 = 42;
    let (end_a, end_b, clock) = LoopbackNetwork::new::<Yard>(4, 1, 777);
    let cfg_a = SessionConfig::new(2, PlayerSlot(0), SEED, 60);
    let cfg_b = SessionConfig::new(2, PlayerSlot(1), SEED, 60);
    let input_delay = u64::from(cfg_a.input_delay);
    let mut a = Session::<Yard, _>::new(BODIES, cfg_a, end_a);
    let mut b = Session::<Yard, _>::new(BODIES, cfg_b, end_b);

    let mut scripted: BTreeMap<u64, (YardInput, YardInput)> = BTreeMap::new();
    let (mut rollbacks, mut ca, mut cb) = (0u32, Vec::new(), Vec::new());
    for call in 1..=TICKS + 30 {
        let target = call + input_delay;
        let (ia, ib) = (bot(1, target, 0), bot(2, target, 1));
        scripted.insert(target, (ia, ib));
        clock.tick();
        for r in [a.advance(ia, Vec::new()), b.advance(ib, Vec::new())] {
            if let AdvanceResult::Advanced { rollback: Some(_), .. } = r {
                rollbacks += 1;
            }
        }
        ca.extend(a.checksums().iter().skip(ca.len()).copied());
        cb.extend(b.checksums().iter().skip(cb.len()).copied());
    }
    assert!(!ca.is_empty(), "no checkpoints recorded");
    let desyncs = compare_checksums(&ca, &cb);
    assert!(desyncs.is_empty(), "peers desynced: {desyncs:?}");
    assert!(rollbacks > 0, "latency above the input delay must cause rollbacks");

    let (last_tick, last_cs) = *ca.last().unwrap();
    assert_eq!(headless_checksum(&scripted, BODIES, SEED, last_tick), last_cs, "session disagrees with a plain simulation");

    // The scene really simulated: bodies spread over the yard, shots spawned.
    let mut sim = Simulation::<Yard>::new(BODIES, 60, SEED);
    for tick in 1..=300 {
        let (ia, ib) = scripted.get(&tick).copied().unwrap_or_default();
        let mut ti = TickInputs::<YardInput, NoCommand>::new(tick, 2);
        ti.set_input(PlayerSlot(0), ia);
        ti.set_input(PlayerSlot(1), ib);
        sim.step(&ti);
    }
    let (_, bodies) = sim.frame().dense::<Body>();
    let dynamic = bodies.iter().filter(|b| b.kind == BODY_DYNAMIC).count();
    assert!(dynamic as u32 > BODIES, "shots should have added bodies, got {dynamic}");
}
