//! Closed, opt-in flat-room simulation. The sphere's center stays at fixed Y;
//! this is not a general character controller and does not run the physics solver.
//! Authoring admission is initial-only. Runtime and restart must retain the
//! admitted Frame and never call setup to replace an authored room.
#![deny(clippy::float_arithmetic)]
#![deny(clippy::disallowed_types)]

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPVec3, FrameRng, FP};
use orr_physics3d::{
    Body, Collider, ContactCache, PhysicsConfig, PhysicsState, QueryFilter, Shape,
};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{decode_pod, encode_pod, Game, PlayerSlot, SimCommand, SimContext, System};

pub const TICK_RATE: u32 = 60;
pub const INTERACT: u32 = 1;
pub const PLAYER: u32 = 1;
pub const KEY: u32 = 2;
pub const EXIT: u32 = 3;
pub const WALL: u32 = 4;
pub const FLOOR: u32 = 5;
pub const WALL_LAYER: u32 = 1;
pub const OTHER_LAYER: u32 = 2;
pub const MAX_WALLS: usize = 64;
pub const PLAYER_RADIUS: FP = fp!(0.4);
pub const PLAYER_Y: FP = FP::HALF;
pub const SKIN: FP = FP(64);
pub const CENTER_BOUND: FP = fp!(16);
pub const INTERACT_DISTANCE: FP = fp!(1.25);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct RoomInput {
    #[reflect(range = "-1..=1")]
    pub move_x: i8,
    #[reflect(range = "-1..=1")]
    pub move_z: i8,
    #[reflect(skip)]
    pub reserved: u16,
    #[reflect(flags = "interact=1")]
    pub buttons: u32,
}
impl RoomInput {
    /// Invalid network/replay samples neutralize the whole sample, including
    /// held-button state; no partial salvage of malformed input is permitted.
    pub fn sanitized(self) -> Self {
        if !(-1..=1).contains(&self.move_x)
            || !(-1..=1).contains(&self.move_z)
            || self.reserved != 0
            || self.buttons & !INTERACT != 0
        {
            Self::default()
        } else {
            self
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct RoomActor {
    pub kind: u32,
    pub ordinal: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct RoomRun {
    #[reflect(skip)]
    pub key_collected: u32,
    #[reflect(skip)]
    pub won: u32,
    #[reflect(skip)]
    pub previous_buttons: u32,
    #[reflect(skip)]
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct NoCommand {
    pub reserved: u32,
}
impl SimCommand for NoCommand {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_pod(self, out);
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        let command: Self = decode_pod(bytes)?;
        (command.reserved == 0).then_some(command)
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct NoEvent {
    pub reserved: u32,
}

/// Canonical tiny room for tests/demos only. Authored scenes use from_frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct RoomConfig;
pub struct RoomEscapeV1;

pub fn register_reflect(types: &mut TypeRegistry) {
    // Reuse the existing bake hook for the hidden contact-list handle. The
    // validator rejects authored config changes instead of silently fixing them.
    orr_physics3d::register_reflect(types);
    types.register_component::<RoomActor>("RoomEscapeV1::Actor");
    types.register_singleton::<RoomRun>("RoomEscapeV1::Run");
}
impl Game for RoomEscapeV1 {
    type Input = RoomInput;
    type Command = NoCommand;
    type Event = NoEvent;
    type Config = RoomConfig;
    fn register(builder: &mut ComponentRegistryBuilder) {
        orr_physics3d::register(builder);
        builder.register_component::<RoomActor>("RoomEscapeV1::Actor");
        builder.register_singleton::<RoomRun>("RoomEscapeV1::Run");
    }
    fn setup(frame: &mut Frame, _: &RoomConfig) {
        orr_physics3d::init(frame, PhysicsConfig::default());
        frame.set_singleton(RoomRun::default());
        add_actor(
            frame,
            PLAYER,
            FPVec3::new(FP::ZERO, PLAYER_Y, FP::ZERO),
            Shape::sphere(PLAYER_RADIUS),
        );
        add_actor(
            frame,
            KEY,
            FPVec3::new(FP::ONE, PLAYER_Y, FP::ZERO),
            Shape::sphere(fp!(0.2)),
        );
        add_actor(
            frame,
            EXIT,
            FPVec3::new(fp!(3), PLAYER_Y, FP::ZERO),
            Shape::sphere(fp!(0.5)),
        );
        for (x, z) in [(5, 0), (-5, 0), (0, 5), (0, -5)] {
            let half = if x == 0 {
                FPVec3::new(fp!(5), FP::ONE, fp!(0.25))
            } else {
                FPVec3::new(fp!(0.25), FP::ONE, fp!(5))
            };
            add_actor(
                frame,
                WALL,
                FPVec3::new(FP::from_int(x), PLAYER_Y, FP::from_int(z)),
                Shape::cuboid(half.x, half.y, half.z),
            );
        }
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(RoomSystem)]
    }
}
fn add_actor(frame: &mut Frame, kind: u32, position: FPVec3, shape: Shape) -> Entity {
    let body = if kind == PLAYER {
        Body::new_kinematic(position)
    } else {
        Body::new_static(position)
    };
    let entity = orr_physics3d::spawn_body(frame, body, canonical_collider(kind, shape));
    frame.add(entity, RoomActor { kind, ordinal: 0 });
    entity
}
fn canonical_collider(kind: u32, shape: Shape) -> Collider {
    Collider::new(shape).with_filter(
        if kind == WALL {
            WALL_LAYER
        } else {
            OTHER_LAYER
        },
        0,
    )
}
fn same_bytes<T: Pod>(a: &T, b: &T) -> bool {
    bytemuck::bytes_of(a) == bytemuck::bytes_of(b)
}
fn bounded(value: FP, min: FP, max: FP) -> bool {
    value >= min && value <= max
}

/// Validate before any query or arithmetic involving authored values. The
/// enclosing scene admission must additionally reject duplicate declarations
/// and unsupported authored fields before baking erases that information.
pub fn validate_initial_frame(frame: &Frame) -> Result<(), String> {
    let registry = frame.registry();
    let rng = usize::from(registry.singleton_id::<FrameRng>().is_some());
    if registry.component_count() != 3
        || registry.singleton_count() as usize != 2 + rng
        || registry.list_count() != 1
        || registry.component_id::<RoomActor>().is_none()
        || registry.component_id::<Body>().is_none()
        || registry.component_id::<Collider>().is_none()
        || registry.singleton_id::<RoomRun>().is_none()
        || registry.singleton_id::<PhysicsState>().is_none()
        || registry.list_id::<ContactCache>().is_none()
    {
        return Err(
            "RoomEscapeV1 requires its exact closed component/singleton/list schema".into(),
        );
    }
    if frame.tick() != 0 || *frame.singleton::<RoomRun>() != RoomRun::default() {
        return Err("room initial tick and run state must be zero".into());
    }
    let physics = frame.singleton::<PhysicsState>();
    if !same_bytes(&physics.config, &PhysicsConfig::default())
        || physics.contacts.index != 0
        || physics.contacts.version != 1
        || !frame.list_is_alive(physics.contacts)
        || !frame.list(physics.contacts).is_empty()
    {
        return Err(
            "room physics storage must be canonically initialized with an empty cache".into(),
        );
    }
    if !(3..=(3 + MAX_WALLS as u32 + 1)).contains(&frame.alive_count()) {
        return Err("room requires three unique roles, at most 64 walls and one floor".into());
    }
    let mut counts = [0usize; 6];
    let mut player = FPVec3::ZERO;
    let mut walls = Vec::new();
    for entity in frame.entities() {
        let actor = frame
            .get::<RoomActor>(entity)
            .ok_or("every entity needs a room role")?;
        if !(PLAYER..=FLOOR).contains(&actor.kind) || actor.ordinal != 0 {
            return Err("room role kind or ordinal is invalid".into());
        }
        counts[actor.kind as usize] += 1;
        let body = frame
            .get::<Body>(entity)
            .ok_or("room role is missing Body")?;
        let collider = frame
            .get::<Collider>(entity)
            .ok_or("room role is missing Collider")?;
        let p = body.pos;
        // Raw range comparisons must precede subtraction, abs or normalization.
        if !bounded(p.x, -CENTER_BOUND, CENTER_BOUND)
            || !bounded(p.z, -CENTER_BOUND, CENTER_BOUND)
            || !bounded(p.y, fp!(-4), fp!(4))
        {
            return Err("room body center exceeds the admitted numeric envelope".into());
        }
        let canonical_body = if actor.kind == PLAYER {
            Body::new_kinematic(p)
        } else {
            Body::new_static(p)
        };
        if !same_bytes(body, &canonical_body) {
            return Err(
                "room bodies require canonical static/kinematic fields and identity rotation"
                    .into(),
            );
        }
        let shape = collider.shape;
        let canonical_shape = match actor.kind {
            PLAYER => {
                if p.y != PLAYER_Y {
                    return Err("player center Y must be 0.5".into());
                }
                player = p;
                Shape::sphere(PLAYER_RADIUS)
            }
            KEY | EXIT => {
                if !bounded(shape.radius, fp!(0.1), FP::ONE) {
                    return Err("key/exit sphere radius must be in 0.1..=1".into());
                }
                Shape::sphere(shape.radius)
            }
            WALL | FLOOR => {
                let h = shape.half;
                if [h.x, h.y, h.z]
                    .iter()
                    .any(|&v| !bounded(v, fp!(0.1), fp!(8)))
                {
                    return Err("wall/floor box half extents must be in 0.1..=8".into());
                }
                if actor.kind == WALL {
                    if p.y - h.y > PLAYER_Y - PLAYER_RADIUS - SKIN
                        || p.y + h.y < PLAYER_Y + PLAYER_RADIUS + SKIN
                    {
                        return Err("walls must span the full player height plus skin".into());
                    }
                    walls.push((p, h));
                }
                Shape::cuboid(h.x, h.y, h.z)
            }
            _ => unreachable!(),
        };
        if !same_bytes(collider, &canonical_collider(actor.kind, canonical_shape)) {
            return Err(
                "room collider shape, material, filter or reserved fields are noncanonical".into(),
            );
        }
    }
    if counts[PLAYER as usize] != 1
        || counts[KEY as usize] != 1
        || counts[EXIT as usize] != 1
        || counts[WALL as usize] > MAX_WALLS
        || counts[FLOOR as usize] > 1
    {
        return Err(
            "room requires exactly one player/key/exit, at most 64 walls and one floor".into(),
        );
    }
    for (p, h) in walls {
        // All inputs were bounded above; closest-point sphere/AABB distance.
        let delta = FPVec3::new(
            (player.x - p.x).abs() - h.x,
            (player.y - p.y).abs() - h.y,
            (player.z - p.z).abs() - h.z,
        );
        let outside = FPVec3::new(
            delta.x.max(FP::ZERO),
            delta.y.max(FP::ZERO),
            delta.z.max(FP::ZERO),
        );
        if outside.length_sq() <= (PLAYER_RADIUS + SKIN) * (PLAYER_RADIUS + SKIN) {
            return Err("initial player overlaps or touches a wall at radius plus skin".into());
        }
    }
    Ok(())
}

struct RoomSystem;
impl System<RoomEscapeV1> for RoomSystem {
    fn name(&self) -> &'static str {
        "RoomEscapeV1"
    }
    fn run(&mut self, ctx: &mut SimContext<RoomEscapeV1>) {
        let input =
            if ctx.inputs.player_count() > 0 && !ctx.inputs.flags(PlayerSlot(0)).disconnected {
                ctx.inputs.input(PlayerSlot(0)).sanitized()
            } else {
                RoomInput::default()
            };
        step_room(ctx.frame, input);
    }
}
fn step_room(frame: &mut Frame, input: RoomInput) {
    let mut run = *frame.singleton::<RoomRun>();
    if run.won != 0 {
        return;
    }
    let pressed = input.buttons & INTERACT != 0 && run.previous_buttons & INTERACT == 0;
    run.previous_buttons = input.buttons;
    let mut roles = [None; 3];
    for (entity, actor) in frame
        .dense::<RoomActor>()
        .0
        .iter()
        .zip(frame.dense::<RoomActor>().1)
    {
        if (PLAYER..=EXIT).contains(&actor.kind) {
            roles[(actor.kind - PLAYER) as usize] = Some(*entity);
        }
    }
    let [Some(player), Some(key), Some(exit)] = roles else {
        return;
    };
    let mut position = frame.get::<Body>(player).expect("admitted player body").pos;
    let direction = FPVec3::new(
        FP::from_int(i32::from(input.move_x)),
        FP::ZERO,
        FP::from_int(i32::from(input.move_z)),
    )
    .normalize_or_zero();
    let displacement = direction * FP::from_ratio(3, i64::from(TICK_RATE));
    // Fixed world X then Z, bounding each intended segment BEFORE querying.
    position = move_axis(frame, position, displacement.x, true);
    position = move_axis(frame, position, displacement.z, false);
    frame
        .get_mut::<Body>(player)
        .expect("admitted player body")
        .pos = position;
    if pressed {
        let target = if run.key_collected == 0 { key } else { exit };
        let target_position = frame.get::<Body>(target).expect("admitted target body").pos;
        if can_interact(frame, position, target_position) {
            if run.key_collected == 0 {
                run.key_collected = 1;
            } else {
                run.won = 1;
            }
        }
    }
    frame.set_singleton(run);
}
fn move_axis(frame: &mut Frame, origin: FPVec3, requested: FP, x_axis: bool) -> FPVec3 {
    let coordinate = if x_axis { origin.x } else { origin.z };
    let target = (coordinate + requested).clamp(-CENTER_BOUND, CENTER_BOUND);
    let delta = target - coordinate;
    if delta == FP::ZERO {
        return origin;
    }
    let sign = if delta > FP::ZERO { FP::ONE } else { -FP::ONE };
    let direction = if x_axis {
        FPVec3::new(sign, FP::ZERO, FP::ZERO)
    } else {
        FPVec3::new(FP::ZERO, FP::ZERO, sign)
    };
    let distance = delta.abs();
    let travel = match orr_physics3d::sphere_cast(
        frame,
        origin,
        direction,
        distance,
        PLAYER_RADIUS,
        QueryFilter { mask: WALL_LAYER },
    ) {
        Some(hit) => (hit.distance - SKIN).max(FP::ZERO).min(distance),
        None => distance,
    };
    origin + direction * travel
}
fn can_interact(frame: &mut Frame, from: FPVec3, to: FPVec3) -> bool {
    let delta = to - from;
    let distance_sq = delta.length_sq();
    if distance_sq > INTERACT_DISTANCE * INTERACT_DISTANCE {
        return false;
    }
    if distance_sq == FP::ZERO {
        return true;
    }
    orr_physics3d::raycast(
        frame,
        from,
        delta,
        distance_sq.sqrt(),
        QueryFilter { mask: WALL_LAYER },
    )
    .is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_sim::{Simulation, TickInputs};

    fn simulation() -> Simulation<RoomEscapeV1> {
        Simulation::new(RoomConfig, TICK_RATE, 123)
    }
    fn actor(frame: &Frame, kind: u32) -> Entity {
        frame
            .entities()
            .find(|&e| frame.get::<RoomActor>(e).is_some_and(|a| a.kind == kind))
            .unwrap()
    }
    fn position(sim: &Simulation<RoomEscapeV1>) -> FPVec3 {
        sim.frame()
            .get::<Body>(actor(sim.frame(), PLAYER))
            .unwrap()
            .pos
    }
    fn tick(sim: &mut Simulation<RoomEscapeV1>, input: RoomInput) {
        let mut inputs = TickInputs::new(sim.tick(), 1);
        inputs.set_input(PlayerSlot(0), input);
        sim.step(&inputs);
    }
    fn walk(x: i8, z: i8) -> RoomInput {
        RoomInput {
            move_x: x,
            move_z: z,
            ..RoomInput::default()
        }
    }
    fn press() -> RoomInput {
        RoomInput {
            buttons: INTERACT,
            ..RoomInput::default()
        }
    }

    #[test]
    fn canonical_setup_and_initial_only_admission() {
        let mut sim = simulation();
        validate_initial_frame(sim.frame()).unwrap();
        tick(&mut sim, RoomInput::default());
        assert!(validate_initial_frame(sim.frame()).is_err());
    }
    #[test]
    fn hostile_samples_are_wholly_neutral() {
        for invalid in [
            RoomInput {
                move_x: i8::MIN,
                ..press()
            },
            RoomInput {
                move_z: 2,
                ..press()
            },
            RoomInput {
                reserved: 1,
                ..press()
            },
            RoomInput {
                buttons: 3,
                ..walk(1, 1)
            },
        ] {
            let mut sim = simulation();
            let before = position(&sim);
            tick(&mut sim, invalid);
            assert_eq!(position(&sim), before);
            assert_eq!(*sim.frame().singleton::<RoomRun>(), RoomRun::default());
        }
    }
    #[test]
    fn walls_stop_motion_and_diagonal_is_normalized() {
        let mut diagonal = simulation();
        tick(&mut diagonal, walk(1, 1));
        let p = position(&diagonal);
        assert_eq!(p.x, p.z);
        assert!(p.x > fp!(0.03) && p.x < fp!(0.04));
        let mut straight = simulation();
        for _ in 0..400 {
            tick(&mut straight, walk(1, 1));
        }
        let p = position(&straight);
        assert!(p.x <= fp!(4.35) && p.z <= fp!(4.35));
        assert!(p.x > fp!(4.3) && p.z > fp!(4.3));
        assert_eq!(p.y, PLAYER_Y);
    }
    #[test]
    fn key_then_exit_requires_separate_edges_and_won_freezes() {
        let mut sim = simulation();
        let exit = actor(sim.frame(), EXIT);
        sim.frame_mut().get_mut::<Body>(exit).unwrap().pos =
            FPVec3::new(FP::ONE, PLAYER_Y, FP::ZERO);
        let count = sim.frame().alive_count();
        tick(&mut sim, press());
        assert_eq!(sim.frame().singleton::<RoomRun>().key_collected, 1);
        assert_eq!(sim.frame().singleton::<RoomRun>().won, 0);
        tick(&mut sim, press());
        assert_eq!(sim.frame().singleton::<RoomRun>().won, 0);
        tick(
            &mut sim,
            RoomInput {
                reserved: 1,
                ..press()
            },
        );
        tick(&mut sim, press());
        assert_eq!(sim.frame().singleton::<RoomRun>().won, 1);
        let p = position(&sim);
        let run = *sim.frame().singleton::<RoomRun>();
        tick(&mut sim, walk(-1, 1));
        assert_eq!(position(&sim), p);
        assert_eq!(*sim.frame().singleton::<RoomRun>(), run);
        assert_eq!(sim.frame().alive_count(), count);
    }
    #[test]
    fn wall_blocks_key_interaction_and_exit_stays_locked() {
        let mut sim = simulation();
        add_actor(
            sim.frame_mut(),
            WALL,
            FPVec3::new(fp!(0.6), PLAYER_Y, FP::ZERO),
            Shape::cuboid(fp!(0.1), FP::ONE, FP::ONE),
        );
        validate_initial_frame(sim.frame()).unwrap();
        tick(&mut sim, press());
        assert_eq!(sim.frame().singleton::<RoomRun>().key_collected, 0);
        let mut locked = simulation();
        let exit = actor(locked.frame(), EXIT);
        let key = actor(locked.frame(), KEY);
        locked.frame_mut().get_mut::<Body>(exit).unwrap().pos =
            FPVec3::new(FP::ONE, PLAYER_Y, FP::ZERO);
        locked.frame_mut().get_mut::<Body>(key).unwrap().pos.x = fp!(-3);
        tick(&mut locked, press());
        assert_eq!(locked.frame().singleton::<RoomRun>().won, 0);
    }
    #[test]
    fn snapshot_replay_and_initial_frame_restart_match_exact_bytes() {
        let mut a = simulation();
        let initial = a.frame().clone();
        for _ in 0..30 {
            tick(&mut a, walk(1, 0));
        }
        let snapshot = a.frame().clone();
        let mut b = Simulation::<RoomEscapeV1>::from_frame(&snapshot, TICK_RATE, 0).unwrap();
        for i in 0..120 {
            let input = if i % 11 == 0 { press() } else { walk(1, -1) };
            tick(&mut a, input);
            tick(&mut b, input);
            assert_eq!(a.frame().to_bytes(), b.frame().to_bytes());
        }
        let restarted = Simulation::<RoomEscapeV1>::from_frame(&initial, TICK_RATE, 0).unwrap();
        assert_eq!(restarted.frame().to_bytes(), initial.to_bytes());
        assert_eq!(restarted.frame().checksum(), initial.checksum());
    }
    #[test]
    fn rejects_noncanonical_and_unbounded_authoring_before_math() {
        let mut sim = simulation();
        let p = actor(sim.frame(), PLAYER);
        sim.frame_mut().get_mut::<Body>(p).unwrap().pos.x = FP(i64::MIN);
        assert!(validate_initial_frame(sim.frame()).is_err());
        let mut sim = simulation();
        let p = actor(sim.frame(), PLAYER);
        sim.frame_mut().get_mut::<Collider>(p).unwrap().shape._pad = 1;
        assert!(validate_initial_frame(sim.frame()).is_err());
        let mut sim = simulation();
        add_actor(
            sim.frame_mut(),
            WALL,
            FPVec3::new(FP::ZERO, PLAYER_Y, FP::ZERO),
            Shape::cuboid(FP::ONE, FP::ONE, FP::ONE),
        );
        assert!(validate_initial_frame(sim.frame()).is_err());
        let mut sim = simulation();
        add_actor(
            sim.frame_mut(),
            KEY,
            FPVec3::new(fp!(2), PLAYER_Y, FP::ZERO),
            Shape::sphere(fp!(0.2)),
        );
        assert!(validate_initial_frame(sim.frame()).is_err());
    }
    #[test]
    fn movement_bounds_each_segment_before_casting() {
        let mut sim = simulation();
        let walls: Vec<_> = sim
            .frame()
            .entities()
            .filter(|&e| sim.frame().get::<RoomActor>(e).unwrap().kind == WALL)
            .collect();
        for wall in walls {
            sim.frame_mut().despawn(wall);
        }
        let player = actor(sim.frame(), PLAYER);
        sim.frame_mut().get_mut::<Body>(player).unwrap().pos =
            FPVec3::new(CENTER_BOUND - FP(1), PLAYER_Y, -CENTER_BOUND + FP(1));
        validate_initial_frame(sim.frame()).unwrap();
        tick(&mut sim, walk(1, -1));
        assert_eq!(
            position(&sim),
            FPVec3::new(CENTER_BOUND, PLAYER_Y, -CENTER_BOUND)
        );
        for _ in 0..30 {
            tick(&mut sim, walk(1, -1));
        }
        assert_eq!(
            position(&sim),
            FPVec3::new(CENTER_BOUND, PLAYER_Y, -CENTER_BOUND)
        );
    }

    #[test]
    fn admission_rejects_hostile_hidden_fields_missing_roles_and_anonymous_entities() {
        let initial = simulation().frame().clone();
        for alter in [0, 1, 2, 3, 4, 5] {
            let mut frame = initial.clone();
            let player = actor(&frame, PLAYER);
            match alter {
                0 => {
                    frame.get_mut::<Body>(player).unwrap().rot.x = FP(i64::MIN);
                }
                1 => {
                    frame.get_mut::<Collider>(player).unwrap().layer = WALL_LAYER;
                }
                2 => {
                    frame.singleton_mut::<PhysicsState>().config.dt = FP(i64::MAX);
                }
                3 => {
                    frame.singleton_mut::<RoomRun>().reserved = 1;
                }
                4 => {
                    frame.despawn(actor(&frame, KEY));
                }
                _ => {
                    frame.spawn();
                }
            }
            assert!(validate_initial_frame(&frame).is_err());
        }
    }

    #[test]
    fn only_connected_slot_zero_controls_player() {
        let mut sim = simulation();
        let p = position(&sim);
        let mut inputs = TickInputs::new(sim.tick(), 2);
        inputs.set_input(
            PlayerSlot(1),
            RoomInput {
                buttons: INTERACT,
                ..walk(1, 1)
            },
        );
        sim.step(&inputs);
        assert_eq!(position(&sim), p);
        let mut inputs = TickInputs::new(sim.tick(), 1);
        inputs.set_input(
            PlayerSlot(0),
            RoomInput {
                buttons: INTERACT,
                ..walk(1, 1)
            },
        );
        inputs.set_flags(
            PlayerSlot(0),
            orr_sim::PlayerFlags {
                disconnected: true,
                predicted: false,
            },
        );
        sim.step(&inputs);
        assert_eq!(position(&sim), p);
        let inputs = TickInputs::new(sim.tick(), 0);
        sim.step(&inputs);
        assert_eq!(position(&sim), p);
        assert_eq!(*sim.frame().singleton::<RoomRun>(), RoomRun::default());
    }
}
