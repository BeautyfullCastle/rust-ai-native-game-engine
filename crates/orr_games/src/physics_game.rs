//! `PhysGame`: a physics stress scene for the sample. A walled box holds
//! hundreds to thousands of circles and boxes (`orr_physics`), static
//! obstacles, and one heavy kinematic paddle per player. Players push the
//! bodies around with their paddles, so the pile never settles and remote
//! input keeps mispredicting (which is what causes rollbacks).
//!
//! This is sim code: no floats, no wall clock, no hash maps.
#![deny(clippy::float_arithmetic)]
#![deny(clippy::disallowed_types)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPVec2, FrameRng, FP};
use orr_reflect::{Reflect, TypeRegistry};
use orr_physics::{
    spawn_body, Body, Collider, PhysicsConfig, PhysicsSystem, Shape, TriggerEvent, BODY_DYNAMIC,
};
use orr_sim::{decode_pod, encode_pod, Game, PlayerSlot, SimCommand, SimContext, System};

/// Fixed sim rate of the sample.
pub const TICK_RATE: u32 = 60;
/// Paddle speed in world units per second at full stick.
pub const PADDLE_SPEED: FP = FP(12 << 16);
/// Paddle turn rate in radians per second.
pub const PADDLE_SPIN: FP = FP(3 << 16);
/// Half size of a paddle (x, y).
pub const PADDLE_HALF: (FP, FP) = (FP(5 << 16), FP(45_875)); // 5.0, 0.7

/// `PhysInput::buttons` bit: shoot small balls out of the paddle.
pub const SHOOT: u32 = 1;

/// How the box is filled at the start.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SceneMode {
    /// Bodies start in the air in the upper half and fall onto the floor.
    #[default]
    Rain,
    /// Bodies start packed in a block on the floor.
    Pile,
    /// Like `Pile`, with two rotating bars near the floor that keep stirring.
    Mixer,
}

impl SceneMode {
    fn code(self) -> u32 {
        match self {
            SceneMode::Rain => 0,
            SceneMode::Pile => 1,
            SceneMode::Mixer => 2,
        }
    }
}

/// Parameters of a scene. Every peer must use the same values.
#[derive(Clone, Copy, Debug)]
pub struct PhysConfig {
    /// Dynamic bodies at the start.
    pub bodies: u32,
    pub mode: SceneMode,
    /// Bodies added per second while the game runs (0 = none).
    pub spawn_rate: u32,
    /// Spawning stops when this many entities are alive (walls and paddles count).
    pub max_entities: u32,
    /// Seed of the scene layout (the sim RNG seed is the session seed).
    pub layout_seed: u64,
    /// Paddles in the scene, one per slot (2 in the local samples; relay
    /// play sets the room's player count).
    pub paddles: u32,
}

impl PhysConfig {
    /// Reads the scene from a room config blob (see `orr_server::presets`).
    /// `player_count` is the room's, and sets the number of paddles.
    pub fn from_blob(blob: &[u8], player_count: u8) -> Option<Self> {
        let rest = blob.strip_prefix(b"PHY1")?;
        let bodies = u32::from_le_bytes(rest.get(0..4)?.try_into().ok()?);
        let mode = match *rest.get(4)? {
            0 => SceneMode::Rain,
            1 => SceneMode::Pile,
            2 => SceneMode::Mixer,
            _ => return None,
        };
        let spawn_rate = u32::from_le_bytes(rest.get(5..9)?.try_into().ok()?);
        let max_entities = u32::from_le_bytes(rest.get(9..13)?.try_into().ok()?);
        let layout_seed = u64::from_le_bytes(rest.get(13..21)?.try_into().ok()?);
        if rest.len() != 21 || bodies == 0 {
            return None;
        }
        Some(Self { bodies, mode, spawn_rate, max_entities, layout_seed, paddles: u32::from(player_count.max(1)) })
    }

    pub fn new(bodies: u32, mode: SceneMode) -> Self {
        Self { bodies, mode, spawn_rate: 0, max_entities: 20_000, layout_seed: 0x0DDB_A110, paddles: 2 }
    }
}

/// Size of the box for `bodies` dynamic bodies. The floor top is at `y = 0`,
/// the box spans `x` in `-half_w..half_w` and `y` in `0..height`, and is open at the top.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub half_w: FP,
    pub height: FP,
}

/// The box grows with the square root of the body count, so density stays about the same.
pub fn layout(bodies: u32) -> Layout {
    let side = (2 * bodies.isqrt()).max(40) as i32;
    Layout { half_w: FP::from_int(side) / 2, height: FP::from_int(side) }
}

// ---- input, command, event ----

/// One player's input: paddle stick, spin and buttons.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Pod, Zeroable, Reflect)]
pub struct PhysInput {
    /// -1, 0 or 1 (other values are clamped).
    pub axis_x: FP,
    pub axis_y: FP,
    /// -1, 0 or 1.
    pub spin: i32,
    /// Button bits.
    #[reflect(flags = "shoot=1")]
    pub buttons: u32,
}

impl PhysInput {
    pub fn new(axis_x: i32, axis_y: i32, spin: i32, shoot: bool) -> Self {
        Self {
            axis_x: FP::from_int(axis_x),
            axis_y: FP::from_int(axis_y),
            spin,
            buttons: if shoot { SHOOT } else { 0 },
        }
    }
}

/// A deterministic scripted player for verification runs: a pure function of
/// `(seed, tick, slot)` that holds a stick direction and spin for 30 ticks at
/// a time and shoots in short bursts. Integers only.
pub fn bot_input(seed: u64, tick: u64, slot: PlayerSlot) -> PhysInput {
    fn mix(mut z: u64) -> u64 {
        z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    let s = u64::from(slot.0);
    let h = mix(seed ^ mix((tick / 30) ^ (s << 40)));
    PhysInput::new((h % 3) as i32 - 1, ((h >> 8) % 3) as i32 - 1, ((h >> 16) % 3) as i32 - 1, tick % 90 < 10 && (h >> 24) % 2 == 0)
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

/// `PhysEvent::kind` of a shot (a paddle fired a ball): `a` is the slot, `b` the tick.
/// Trigger events carry `orr_physics::TRIGGER_ENTER` / `TRIGGER_EXIT` (small numbers).
pub const EVENT_SHOT: u32 = 100;

/// A sim event: a trigger overlap started or ended (unused by the scenes so far), or a
/// shot (`EVENT_SHOT`). Events are not part of the frame, so they never change a checksum;
/// a predicted shot is canceled when the confirmed input of the paddle's player did not shoot.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
pub struct PhysEvent {
    pub kind: u32,
    pub a: u32,
    pub b: u32,
}

fn map_trigger(e: TriggerEvent) -> PhysEvent {
    PhysEvent { kind: e.kind, a: e.a.index, b: e.b.index }
}

// ---- components ----

/// Marks the paddle of a player.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable, Reflect)]
pub struct PaddleTag {
    pub slot: u32,
}

/// Scene constants, set once in `setup`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable, Reflect)]
pub struct Scene {
    pub half_w: FP,
    pub height: FP,
    pub spawn_batch: u32,
    /// Ticks between spawns; 0 = never.
    pub spawn_interval: u32,
    pub max_entities: u32,
    pub mode: u32,
}

pub struct PhysGame;

/// Registers every reflected type `PhysGame` uses (physics types, `PaddleTag`
/// and the `Scene` singleton) under the names of its `ComponentRegistry`.
pub fn register_reflect(types: &mut TypeRegistry) {
    orr_physics::register_reflect(types);
    types.register_component::<PaddleTag>("PaddleTag");
    types.register_singleton::<Scene>("Scene");
}

impl Game for PhysGame {
    type Input = PhysInput;
    type Command = NoCommand;
    type Event = PhysEvent;
    type Config = PhysConfig;

    fn register(builder: &mut ComponentRegistryBuilder) {
        orr_physics::register(builder);
        builder.register_component::<PaddleTag>("PaddleTag");
        builder.register_singleton::<Scene>("Scene");
    }

    fn setup(frame: &mut Frame, config: &PhysConfig) {
        build_scene(frame, config);
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![
            Box::new(PaddleSystem),
            Box::new(SpawnSystem),
            Box::new(PhysicsSystem::<PhysGame>::new(map_trigger)),
            Box::new(BoundsSystem),
        ]
    }
}

// ---- scene construction ----

const CELL: FP = FP(88_474); // 1.35: a little more than the widest body (a 0.45 box has a 1.27 diagonal)

fn v2(x: FP, y: FP) -> FPVec2 {
    FPVec2::new(x, y)
}

/// A random dynamic body (circle or box, 0.25 to 0.45 in size) at `pos`.
fn random_body(rng: &mut FrameRng, pos: FPVec2, rotate: bool) -> (Body, Collider) {
    let size = rng.range_fp(fp!(0.25), fp!(0.45));
    if rng.next_u32() & 1 == 0 {
        let shape = Shape::circle(size);
        (Body::new_dynamic(pos, &shape, FP::ONE), Collider::new(shape).with_restitution(fp!(0.2)))
    } else {
        let hy = rng.range_fp(fp!(0.25), fp!(0.45));
        let shape = Shape::box_shape(size, hy);
        let angle = if rotate { rng.range_fp(-FP::PI, FP::PI) } else { FP::ZERO };
        (Body::new_dynamic(pos, &shape, FP::ONE).with_angle(angle), Collider::new(shape).with_restitution(fp!(0.1)))
    }
}

fn build_scene(frame: &mut Frame, cfg: &PhysConfig) {
    orr_physics::init(frame, PhysicsConfig { max_linear_speed: fp!(40), ..PhysicsConfig::default() });
    let Layout { half_w, height } = layout(cfg.bodies);
    let mut rng = FrameRng::new(cfg.layout_seed);

    let (spawn_batch, spawn_interval) = match cfg.spawn_rate {
        0 => (0, 0),
        r if r >= TICK_RATE => (r / TICK_RATE, 1),
        r => (1, TICK_RATE / r),
    };
    frame.set_singleton(Scene {
        half_w,
        height,
        spawn_batch,
        spawn_interval,
        max_entities: cfg.max_entities,
        mode: cfg.mode.code(),
    });

    // Floor and walls. Thick, so a fast body cannot cross them in one tick.
    let wall = |frame: &mut Frame, x: FP, y: FP, hx: FP, hy: FP| {
        spawn_body(frame, Body::new_static(v2(x, y), FP::ZERO), Collider::new(Shape::box_shape(hx, hy)).with_friction(fp!(0.4)));
    };
    wall(frame, FP::ZERO, -FP::TWO, half_w + fp!(3), FP::TWO);
    wall(frame, -half_w - FP::ONE, height, FP::ONE, height * 2);
    wall(frame, half_w + FP::ONE, height, FP::ONE, height * 2);

    // Things bodies must not start inside: (center, keep-out radius).
    let mut keep_out: Vec<(FPVec2, FP)> = Vec::new();

    // Static obstacles in the middle band: tilted bars and round pegs.
    let obstacles = ((height.to_int() / 12).max(4)) as u32;
    for i in 0..obstacles {
        let x = rng.range_fp(-half_w + fp!(6), half_w - fp!(6));
        let y = rng.range_fp(height * fp!(0.55), height * fp!(0.85));
        let pos = v2(x, y);
        if i % 2 == 0 {
            let angle = rng.range_fp(-fp!(0.8), fp!(0.8));
            let hx = fp!(2.5);
            spawn_body(frame, Body::new_static(pos, angle), Collider::new(Shape::box_shape(hx, fp!(0.4))));
            keep_out.push((pos, hx + FP::ONE));
        } else {
            let r = rng.range_fp(fp!(1.2), fp!(2));
            spawn_body(frame, Body::new_static(pos, FP::ZERO), Collider::new(Shape::circle(r)).with_restitution(fp!(0.4)));
            keep_out.push((pos, r + FP::ONE));
        }
    }

    // Where the block of bodies starts, and how tall it is (for the paddles).
    let cols = ((half_w * 2 - fp!(2)) / CELL).to_int().max(1) as u32;
    let rows = cfg.bodies.div_ceil(cols);
    let pile_top = fp!(0.6) + CELL * rows as i32;
    let first_row_y = if cfg.mode == SceneMode::Rain { height * fp!(0.5) } else { fp!(0.6) };

    // Rotating bars of the mixer.
    if cfg.mode == SceneMode::Mixer {
        let len = (half_w / 2 - FP::TWO).min(fp!(14));
        for (sx, omega) in [(-1, fp!(1.5)), (1, -fp!(1.5))] {
            let pos = v2(half_w / 2 * sx, fp!(6));
            let mut bar = Body::new_kinematic(pos);
            bar.omega = omega;
            spawn_body(frame, bar, Collider::new(Shape::box_shape(len, fp!(0.6))).with_friction(fp!(0.6)));
            keep_out.push((pos, len + FP::ONE));
        }
    }

    // One paddle per player, side by side above the block (or low, in the rain).
    let paddle_y = if cfg.mode == SceneMode::Rain { height * fp!(0.2) } else { (pile_top + fp!(4)).min(height * fp!(0.9)) };
    for slot in 0..cfg.paddles {
        let x = match (cfg.paddles, slot) {
            (2, 0) => -half_w / 2,
            (2, _) => half_w / 2,
            // More (or fewer) players: evenly spread across the box.
            (n, s) => -half_w + half_w * 2 * (s as i32 * 2 + 1) / (n as i32 * 2),
        };
        let pos = v2(x, paddle_y);
        let paddle = spawn_body(
            frame,
            Body::new_kinematic(pos),
            Collider::new(Shape::box_shape(PADDLE_HALF.0, PADDLE_HALF.1)).with_friction(fp!(0.6)),
        );
        frame.add(paddle, PaddleTag { slot });
        keep_out.push((pos, PADDLE_HALF.0 + FP::ONE));
    }

    // The bodies, on a grid. Cells that touch an obstacle or a paddle are skipped.
    let mut placed = 0;
    let mut k: u32 = 0;
    while placed < cfg.bodies {
        let (col, row) = (k % cols, k / cols);
        k += 1;
        let jitter = fp!(0.03);
        let x = -half_w + FP::ONE + CELL / 2 + CELL * col as i32 + rng.range_fp(-jitter, jitter);
        let y = first_row_y + CELL * row as i32 + rng.range_fp(-jitter, jitter);
        let pos = v2(x, y);
        if keep_out.iter().any(|&(c, r)| (c - pos).length_sq() < r * r) {
            continue;
        }
        let (mut body, mut collider) = random_body(&mut rng, pos, true);
        // The mixer also has capsules: every fourth body (the random stream is
        // used as before, so other scenes and body positions are unchanged).
        if cfg.mode == SceneMode::Mixer && placed % 4 == 3 {
            let shape = Shape::capsule(fp!(0.3), fp!(0.28));
            body = Body::new_dynamic(pos, &shape, FP::ONE).with_angle(body.angle);
            collider = Collider::new(shape).with_restitution(fp!(0.15));
        }
        if cfg.mode == SceneMode::Rain {
            body.vel = v2(rng.range_fp(-FP::ONE, FP::ONE), FP::ZERO);
        }
        spawn_body(frame, body, collider);
        placed += 1;
    }
}

// ---- systems ----

/// Turns each player's input into paddle velocity, and shoots balls on request.
struct PaddleSystem;

impl System<PhysGame> for PaddleSystem {
    fn name(&self) -> &'static str {
        "PaddleSystem"
    }

    fn run(&mut self, ctx: &mut SimContext<PhysGame>) {
        let scene = *ctx.frame.singleton::<Scene>();
        let player_count = u32::from(ctx.inputs.player_count());
        let limit_x = scene.half_w - PADDLE_HALF.0 - FP::ONE;
        let (min_y, max_y) = (fp!(2), scene.height * 2);
        let mut shots: Vec<(u32, FPVec2)> = Vec::new();
        for (_, (tag, body)) in ctx.frame.query::<(&PaddleTag, &mut Body)>() {
            if tag.slot >= player_count {
                continue;
            }
            let input = *ctx.inputs.input(PlayerSlot(tag.slot as u8));
            // Inputs come from the network or a replay file: never trust their range.
            let mut ax = input.axis_x.clamp(-FP::ONE, FP::ONE);
            let mut ay = input.axis_y.clamp(-FP::ONE, FP::ONE);
            if (body.pos.x >= limit_x && ax > FP::ZERO) || (body.pos.x <= -limit_x && ax < FP::ZERO) {
                ax = FP::ZERO;
            }
            if (body.pos.y >= max_y && ay > FP::ZERO) || (body.pos.y <= min_y && ay < FP::ZERO) {
                ay = FP::ZERO;
            }
            body.vel = v2(ax, ay) * PADDLE_SPEED;
            body.omega = PADDLE_SPIN * input.spin.clamp(-1, 1);
            if input.buttons & SHOOT != 0 && ctx.tick % 3 == u64::from(tag.slot) {
                shots.push((tag.slot, body.pos + v2(FP::ZERO, PADDLE_HALF.1 + fp!(1.2))));
            }
        }
        for (slot, pos) in shots {
            if ctx.frame.alive_count() >= scene.max_entities {
                break;
            }
            ctx.emit(PhysEvent { kind: EVENT_SHOT, a: slot, b: ctx.tick as u32 });
            let shape = Shape::circle(fp!(0.4));
            let body = Body::new_dynamic(pos, &shape, FP::ONE).with_velocity(v2(FP::ZERO, fp!(15)));
            spawn_body(ctx.frame, body, Collider::new(shape).with_restitution(fp!(0.2)));
        }
    }
}

/// Adds bodies at the top of the box at the configured rate.
struct SpawnSystem;

impl System<PhysGame> for SpawnSystem {
    fn name(&self) -> &'static str {
        "SpawnSystem"
    }

    fn run(&mut self, ctx: &mut SimContext<PhysGame>) {
        let scene = *ctx.frame.singleton::<Scene>();
        if scene.spawn_interval == 0 || ctx.tick % u64::from(scene.spawn_interval) != 0 {
            return;
        }
        for i in 0..scene.spawn_batch {
            if ctx.frame.alive_count() >= scene.max_entities {
                return;
            }
            let rng = ctx.rng();
            let x = rng.range_fp(-scene.half_w + fp!(2), scene.half_w - fp!(2));
            let vx = rng.range_fp(-fp!(2), fp!(2));
            let pos = v2(x, scene.height + FP::ONE + fp!(1.4) * i as i32);
            let (body, collider) = random_body(rng, pos, true);
            spawn_body(ctx.frame, body.with_velocity(v2(vx, -FP::TWO)), collider);
        }
    }
}

/// Puts a body that left the box (squeezed through a wall by a paddle) back at the top.
struct BoundsSystem;

impl System<PhysGame> for BoundsSystem {
    fn name(&self) -> &'static str {
        "BoundsSystem"
    }

    fn run(&mut self, ctx: &mut SimContext<PhysGame>) {
        let scene = *ctx.frame.singleton::<Scene>();
        let limit_x = scene.half_w + fp!(3);
        let (min_y, max_y) = (-scene.height, scene.height * 3);
        let escaped: Vec<Entity> = ctx
            .frame
            .query::<(&Body,)>()
            .filter(|(_, (b,))| b.kind == BODY_DYNAMIC && (b.pos.x.abs() > limit_x || b.pos.y < min_y || b.pos.y > max_y))
            .map(|(e, _)| e)
            .collect();
        for e in escaped {
            let x = ctx.rng().range_fp(-scene.half_w + fp!(2), scene.half_w - fp!(2));
            if let Some(b) = ctx.frame.get_mut::<Body>(e) {
                b.pos = v2(x, scene.height + FP::ONE);
                b.vel = FPVec2::ZERO;
                b.omega = FP::ZERO;
            }
        }
    }
}

/// Number of moving (dynamic) bodies in a frame.
pub fn dynamic_count(frame: &mut Frame) -> u32 {
    frame.query::<(&Body,)>().filter(|(_, (b,))| b.kind == BODY_DYNAMIC).count() as u32
}

/// Verification metrics of a `PhysGame` frame (see `orr_sim::Metrics`), all
/// integers or `FP`. Names: `dynamic_bodies`, `sleeping_bodies`,
/// `lost_bodies` (dynamic bodies below the floor top, `y < 0`, or outside the
/// side walls), `mean_height`, `max_height`, `min_height` (of dynamic bodies,
/// 0 if none), `max_speed`, `kinetic_energy` (sum of `m * v^2 / 2`, mass from
/// `1 / inv_mass`).
pub struct PhysMetrics;

impl orr_sim::Metrics for PhysMetrics {
    fn sample(&self, frame: &Frame) -> Vec<(String, orr_sim::MetricValue)> {
        use orr_sim::MetricValue::{Fixed, Int};
        let half_w = if frame.registry().singleton_id::<Scene>().is_some() { frame.singleton::<Scene>().half_w } else { FP::MAX };
        let (mut dynamic, mut sleeping, mut lost) = (0i64, 0i64, 0i64);
        let (mut sum_y, mut max_y, mut min_y) = (0i128, FP::ZERO, FP::ZERO);
        let (mut max_v2, mut energy) = (FP::ZERO, FP::ZERO);
        let (_, bodies) = frame.dense::<Body>();
        for b in bodies.iter().filter(|b| b.kind == BODY_DYNAMIC) {
            if dynamic == 0 {
                (max_y, min_y) = (b.pos.y, b.pos.y);
            }
            dynamic += 1;
            sleeping += i64::from(b.sleep & orr_physics::SLEEP_FLAG != 0);
            lost += i64::from(b.pos.y < FP::ZERO || b.pos.x.abs() > half_w);
            sum_y += i128::from(b.pos.y.raw());
            max_y = max_y.max(b.pos.y);
            min_y = min_y.min(b.pos.y);
            let v2 = b.vel.length_sq();
            max_v2 = max_v2.max(v2);
            if b.inv_mass > FP::ZERO {
                energy += (v2 / b.inv_mass) / 2;
            }
        }
        let mean = if dynamic > 0 { FP((sum_y / i128::from(dynamic)) as i64) } else { FP::ZERO };
        let name = |s: &str| s.to_string();
        vec![
            (name("dynamic_bodies"), Int(dynamic)),
            (name("sleeping_bodies"), Int(sleeping)),
            (name("lost_bodies"), Int(lost)),
            (name("mean_height"), Fixed(mean)),
            (name("max_height"), Fixed(max_y)),
            (name("min_height"), Fixed(min_y)),
            (name("max_speed"), Fixed(max_v2.sqrt())),
            (name("kinetic_energy"), Fixed(energy)),
        ]
    }
}

