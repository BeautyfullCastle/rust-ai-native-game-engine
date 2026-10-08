//! Bounded, opt-in collection/dodge simulation. No Arena schema changes.
//!
//! All initial and current level state lives in the Frame. Contacts are discrete
//! inclusive AABBs after movement, not continuous collision detection. Hazard
//! contact wins over collection; completing the goal wins over this tick's
//! timeout. A fresh restart press resets the run without moving on that tick.
//! Terminal runs stop gameplay; simulation ticks and restart-edge tracking continue.
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{FPVec2, FP};
use orr_sim::{decode_pod, encode_pod, Game, PlayerSlot, SimCommand, SimContext, System};

pub const MAX_COLLECTIBLES: usize = 32;
pub const MAX_HAZARDS: usize = 16;
pub const HALF_EXTENT: FP = FP(256 << 16);
pub const RADIUS: FP = FP(2 << 16);
pub const RESTART: u32 = 1;
pub const PLAYING: u32 = 0;
pub const WON: u32 = 1;
pub const LOST_HAZARD: u32 = 2;
pub const LOST_TIMEOUT: u32 = 3;
pub const PLAYER: u32 = 0;
pub const COLLECTIBLE: u32 = 1;
pub const HAZARD: u32 = 2;
pub const EVENT_COLLECTED: u32 = 1;
pub const EVENT_FINISHED: u32 = 2;
pub const EVENT_RESTARTED: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct CollectInput {
    /// Clamped to [-1,1] per axis. Diagonal speed is intentionally per-axis.
    pub x: FP,
    pub y: FP,
    pub buttons: u32,
    pub reserved: u32,
}

/// This closed first slice accepts no gameplay commands. Nonzero encodings fail.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct NoCommand {
    reserved: u32,
}
impl SimCommand for NoCommand {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_pod(self, out);
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        let value: Self = decode_pod(bytes)?;
        (value.reserved == 0).then_some(value)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct CollectEvent {
    pub kind: u32,
    /// Stable level ordinal for collection, otherwise zero.
    pub ordinal: u32,
    /// New score for collection, terminal phase for finish, zero for restart.
    pub value: u32,
}

/// A never-despawned level actor. Ordinals and initial state survive rollback
/// and restart; inactive collectibles remain available to the view as hidden.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct CollectActor {
    pub position: FPVec2,
    pub initial_position: FPVec2,
    pub velocity: FPVec2,
    pub initial_velocity: FPVec2,
    pub kind: u32,
    pub ordinal: u32,
    pub active: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct CollectRun {
    pub phase: u32,
    pub score: u32,
    pub elapsed_ticks: u32,
    pub time_limit_ticks: u32,
    pub goal: u32,
    pub restart_held: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HazardSpec {
    pub position: FPVec2,
    /// Each axis is in [-1,1] units/tick. Bounces inside the closed arena.
    pub velocity: FPVec2,
}

/// Validated immutable setup; no publicly mutable fields can bypass admission.
#[derive(Clone, Debug)]
pub struct CollectLevel {
    player: FPVec2,
    collectibles: Vec<FPVec2>,
    hazards: Vec<HazardSpec>,
    time_limit_ticks: u32,
}
impl CollectLevel {
    pub fn new(
        player: FPVec2,
        collectibles: Vec<FPVec2>,
        hazards: Vec<HazardSpec>,
        time_limit_ticks: u32,
    ) -> Result<Self, &'static str> {
        if collectibles.is_empty()
            || collectibles.len() > MAX_COLLECTIBLES
            || hazards.len() > MAX_HAZARDS
        {
            return Err("requires 1..=32 collectibles and at most 16 hazards");
        }
        if !(1..=36_000).contains(&time_limit_ticks) {
            return Err("time limit must be 1..=36000 ticks");
        }
        if !in_bounds(player)
            || collectibles.iter().any(|&p| !in_bounds(p))
            || hazards
                .iter()
                .any(|h| !in_bounds(h.position) || !velocity_valid(h.velocity))
        {
            return Err("actor position or hazard velocity out of bounds");
        }
        Ok(Self {
            player,
            collectibles,
            hazards,
            time_limit_ticks,
        })
    }
}
fn in_bounds(p: FPVec2) -> bool {
    let bound = HALF_EXTENT - RADIUS;
    p.x >= -bound && p.x <= bound && p.y >= -bound && p.y <= bound
}
fn velocity_valid(v: FPVec2) -> bool {
    v.x >= -FP::ONE && v.x <= FP::ONE && v.y >= -FP::ONE && v.y <= FP::ONE
}

pub struct CollectDodgeV1;
impl Game for CollectDodgeV1 {
    type Input = CollectInput;
    type Command = NoCommand;
    type Event = CollectEvent;
    type Config = CollectLevel;
    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<CollectActor>("CollectDodgeV1::Actor");
        builder.register_singleton::<CollectRun>("CollectDodgeV1::Run");
    }
    fn setup(frame: &mut Frame, level: &CollectLevel) {
        let mut add = |kind, ordinal, position, velocity| {
            let entity = frame.spawn();
            frame.add(
                entity,
                CollectActor {
                    position,
                    initial_position: position,
                    velocity,
                    initial_velocity: velocity,
                    kind,
                    ordinal,
                    active: 1,
                    reserved: 0,
                },
            );
        };
        add(PLAYER, 0, level.player, FPVec2::ZERO);
        for (ordinal, &position) in level.collectibles.iter().enumerate() {
            add(COLLECTIBLE, ordinal as u32, position, FPVec2::ZERO);
        }
        for (ordinal, hazard) in level.hazards.iter().enumerate() {
            add(HAZARD, ordinal as u32, hazard.position, hazard.velocity);
        }
        frame.set_singleton(CollectRun {
            phase: PLAYING,
            score: 0,
            elapsed_ticks: 0,
            time_limit_ticks: level.time_limit_ticks,
            goal: level.collectibles.len() as u32,
            restart_held: 0,
        });
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(CollectSystem)]
    }
}
struct CollectSystem;
impl System<CollectDodgeV1> for CollectSystem {
    fn name(&self) -> &'static str {
        "CollectDodgeV1"
    }
    fn run(&mut self, ctx: &mut SimContext<CollectDodgeV1>) {
        let input =
            if ctx.inputs.player_count() != 0 && !ctx.inputs.flags(PlayerSlot(0)).disconnected {
                *ctx.inputs.input(PlayerSlot(0))
            } else {
                CollectInput::default()
            };
        let held = u32::from(input.buttons & RESTART != 0);
        let mut run = *ctx.frame.singleton::<CollectRun>();
        let restart = held != 0 && run.restart_held == 0;
        run.restart_held = held;
        // Entity IDs are stable throughout this closed game. Explicit semantic
        // ordering avoids depending on dense storage's incidental order.
        let mut actors: Vec<_> = ctx
            .frame
            .query::<(&CollectActor,)>()
            .map(|(e, (a,))| (a.kind, a.ordinal, e))
            .collect();
        actors.sort_by_key(|&(kind, ordinal, _)| (kind, ordinal));
        if restart {
            for &(_, _, e) in &actors {
                let actor = ctx.frame.get_mut::<CollectActor>(e).expect("listed actor");
                actor.position = actor.initial_position;
                actor.velocity = actor.initial_velocity;
                actor.active = 1;
            }
            run.phase = PLAYING;
            run.score = 0;
            run.elapsed_ticks = 0;
            ctx.frame.set_singleton(run);
            ctx.emit(CollectEvent {
                kind: EVENT_RESTARTED,
                ordinal: 0,
                value: 0,
            });
            return;
        }
        if run.phase != PLAYING {
            ctx.frame.set_singleton(run);
            return;
        }
        run.elapsed_ticks += 1;
        let bound = HALF_EXTENT - RADIUS;
        for &(kind, _, e) in &actors {
            let actor = ctx.frame.get_mut::<CollectActor>(e).expect("listed actor");
            if kind == PLAYER {
                actor.position.x =
                    (actor.position.x + input.x.clamp(-FP::ONE, FP::ONE)).clamp(-bound, bound);
                actor.position.y =
                    (actor.position.y + input.y.clamp(-FP::ONE, FP::ONE)).clamp(-bound, bound);
            } else if kind == HAZARD {
                bounce(&mut actor.position.x, &mut actor.velocity.x, bound);
                bounce(&mut actor.position.y, &mut actor.velocity.y, bound);
            }
        }
        let player = actors
            .iter()
            .find(|&&(kind, _, _)| kind == PLAYER)
            .map(|&(_, _, e)| ctx.frame.get::<CollectActor>(e).expect("player").position)
            .expect("validated level contains one player");
        if actors.iter().any(|&(kind, _, e)| {
            kind == HAZARD
                && touches(
                    player,
                    ctx.frame.get::<CollectActor>(e).expect("hazard").position,
                )
        }) {
            run.phase = LOST_HAZARD;
        } else {
            for &(kind, ordinal, e) in &actors {
                if kind != COLLECTIBLE {
                    continue;
                }
                let actor = ctx.frame.get_mut::<CollectActor>(e).expect("collectible");
                if actor.active != 0 && touches(player, actor.position) {
                    actor.active = 0;
                    run.score += 1;
                    ctx.emit(CollectEvent {
                        kind: EVENT_COLLECTED,
                        ordinal,
                        value: run.score,
                    });
                }
            }
            if run.score == run.goal {
                run.phase = WON;
            } else if run.elapsed_ticks >= run.time_limit_ticks {
                run.phase = LOST_TIMEOUT;
            }
        }
        if run.phase != PLAYING {
            ctx.emit(CollectEvent {
                kind: EVENT_FINISHED,
                ordinal: 0,
                value: run.phase,
            });
        }
        ctx.frame.set_singleton(run);
    }
}
fn touches(a: FPVec2, b: FPVec2) -> bool {
    let diameter = RADIUS + RADIUS;
    (a.x - b.x).abs() <= diameter && (a.y - b.y).abs() <= diameter
}
fn bounce(position: &mut FP, velocity: &mut FP, bound: FP) {
    let next = *position + *velocity;
    if next > bound {
        *position = bound - (next - bound);
        *velocity = -*velocity;
    } else if next < -bound {
        *position = -bound + (-bound - next);
        *velocity = -*velocity;
    } else {
        *position = next;
    }
}
