//! A user-authored collect-then-deliver game using the public static starter API.
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{FP, FPVec2};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{Game, PlayerSlot, SimContext, System};
use orr_static_starter::game_contract::Starter;

pub const BUILD: u64 = 0x434f4c4c444c5631;
pub const MODE_RELEASED: u32 = 0;
pub const MODE_PRIMARY: u32 = 1;
/// One navigation edge. The following neutral tick releases the shared action latch.
pub const MODE_NAVIGATION_PULSE: u32 = 2;
const MAX_SCENE_UNIT: i64 = 16;
pub const SCENE: &str = include_str!("../scenes/collect-delivery.scene.yaml");

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct CollectInput {
    #[reflect(range = "-1..=1")]
    pub dx: i32,
    #[reflect(range = "-1..=1")]
    pub dy: i32,
    #[reflect(range = "0..=2")]
    pub action: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
pub struct Courier {
    pub position: FPVec2,
}

/// Authored objective positions and initial state are all in the saved scene.
/// Play-time values are part of the same deterministic frame and restart restores
/// the authored pose and objectives while clearing cargo and terminal outcomes.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
pub struct CollectDeliveryState {
    pub start: FPVec2,
    pub pickup: FPVec2,
    pub destination: FPVec2,
    pub hazard: FPVec2,
    pub item_available: u32,
    pub carrying: u32,
    pub won: u32,
    pub lost: u32,
    /// Explicit zero padding keeps the reflected/serialized POD layout exact.
    pub reserved: u32,
    /// 0 = released, 1 = primary held, 2 = movement pulse held.
    #[reflect(range = "0..=2")]
    pub action_held: u32,
}

fn point(x: i32, y: i32) -> FPVec2 {
    FPVec2::new(FP::from_int(x), FP::from_int(y))
}

impl CollectDeliveryState {
    fn initial() -> Self {
        Self {
            start: point(0, 0),
            pickup: point(2, 0),
            destination: point(4, 0),
            hazard: point(2, 1),
            item_available: 1,
            ..Self::default()
        }
    }
}

pub fn types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    types.register_component::<Courier>("Courier");
    types.register_singleton::<CollectDeliveryState>("CollectDeliveryState");
    types
}

pub struct CollectDeliveryGame;

impl Game for CollectDeliveryGame {
    type Input = CollectInput;
    type Command = orr_static_starter::NoCommand;
    type Event = u32;
    type Config = ();

    fn register(registry: &mut ComponentRegistryBuilder) {
        registry.register_component::<Courier>("Courier");
        registry.register_singleton::<CollectDeliveryState>("CollectDeliveryState");
    }

    fn setup(frame: &mut Frame, _: &()) {
        let entity = frame.spawn();
        frame.add(
            entity,
            Courier {
                position: point(0, 0),
            },
        );
        frame.set_singleton(CollectDeliveryState::initial());
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(step as fn(&mut SimContext<Self>))]
    }
}

fn step(context: &mut SimContext<CollectDeliveryGame>) {
    let input = *context.inputs.input(PlayerSlot(0));
    if !CollectDeliveryGame::valid_input(&input) {
        return;
    }

    let mut state = *context.frame.singleton::<CollectDeliveryState>();
    let released = state.action_held == MODE_RELEASED;
    let edge = input.action == MODE_PRIMARY && released;
    let ids = context.frame.dense::<Courier>().0.to_vec();
    if ids.len() != 1 {
        return;
    }

    let current = context
        .frame
        .get::<Courier>(ids[0])
        .expect("registered courier")
        .position;
    if state.won != 0 || state.lost != 0 {
        if edge {
            context
                .frame
                .get_mut::<Courier>(ids[0])
                .expect("registered courier")
                .position = state.start;
            state.item_available = 1;
            state.carrying = 0;
            state.won = 0;
            state.lost = 0;
        }
        state.action_held = input.action;
        context.frame.set_singleton(state);
        return;
    }

    let moves_this_tick = input.action != MODE_NAVIGATION_PULSE || released;
    let delta_x = if moves_this_tick {
        i64::from(input.dx) << 16
    } else {
        0
    };
    let delta_y = if moves_this_tick {
        i64::from(input.dy) << 16
    } else {
        0
    };
    // Overflow rejects the complete tick, including action-latch changes.
    let (Some(next_x), Some(next_y)) = (
        current.x.raw().checked_add(delta_x),
        current.y.raw().checked_add(delta_y),
    ) else {
        return;
    };
    let next = FPVec2::new(FP::from_raw(next_x), FP::from_raw(next_y));

    context
        .frame
        .get_mut::<Courier>(ids[0])
        .expect("registered courier")
        .position = next;
    state.action_held = input.action;

    if next == state.hazard {
        state.lost = 1;
    } else if edge && state.item_available != 0 && next == state.pickup {
        state.item_available = 0;
        state.carrying = 1;
    } else if edge && state.carrying != 0 && next == state.destination {
        state.won = 1;
    }
    context.frame.set_singleton(state);
}

fn whole_bounded(point: FPVec2) -> bool {
    [point.x.raw(), point.y.raw()].into_iter().all(|raw| {
        raw.rem_euclid(1 << 16) == 0
            && raw / (1 << 16) >= -MAX_SCENE_UNIT
            && raw / (1 << 16) <= MAX_SCENE_UNIT
    })
}

fn valid_scene(frame: &Frame) -> bool {
    let (entities, couriers) = frame.dense::<Courier>();
    if entities.len() != 1 || couriers.len() != 1 {
        return false;
    }
    let state = frame.singleton::<CollectDeliveryState>();
    let objectives = [state.start, state.pickup, state.destination, state.hazard];
    if objectives.iter().any(|point| !whole_bounded(*point))
        || objectives
            .iter()
            .enumerate()
            .any(|(index, point)| objectives[index + 1..].contains(point))
        || couriers[0].position != state.start
        || state.item_available != 1
        || state.carrying != 0
        || state.won != 0
        || state.lost != 0
        || state.reserved != 0
        || state.action_held != MODE_RELEASED
    {
        return false;
    }
    true
}

impl orr_static_starter::game_contract::Starter for CollectDeliveryGame {
    const ID: &'static str = "CollectDeliveryV1";
    const ALIAS: &'static str = "collect-delivery";
    const INPUT: &'static str = "CollectInput";
    const BUILD: u64 = BUILD;
    const SCENE: &'static str = SCENE;

    fn types() -> TypeRegistry {
        types()
    }

    fn validate(frame: &Frame) -> bool {
        valid_scene(frame)
    }

    fn valid_input(input: &Self::Input) -> bool {
        let axes_bounded = (-1..=1).contains(&input.dx) && (-1..=1).contains(&input.dy);
        let has_motion = input.dx != 0 || input.dy != 0;
        axes_bounded
            && match input.action {
                MODE_RELEASED | MODE_PRIMARY => !has_motion,
                MODE_NAVIGATION_PULSE => has_motion,
                _ => false,
            }
    }

    fn won(frame: &Frame) -> bool {
        frame.singleton::<CollectDeliveryState>().won == 1
    }
}
