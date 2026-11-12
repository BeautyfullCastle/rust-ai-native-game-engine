//! Move the courier to the goal, press action to deliver, release and press to restart.
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{FPVec2, FP};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{Game, PlayerSlot, SimContext, System};

pub const BUILD: u64 = 0x5354415254563201;
/// Released primary action; axes still move on every tick.
pub const MODE_RELEASED: u32 = 0;
/// Held primary action; axes still move on every tick.
pub const MODE_PRIMARY: u32 = 1;
/// Navigation-only edge pulse; mode 0 releases the shared latch.
pub const MODE_NAVIGATION_PULSE: u32 = 2;
pub const SCENE: &str = include_str!("../scenes/delivery.scene.yaml");
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct DeliveryInput {
    pub dx: i32,
    pub dy: i32,
    /// Control mode: released (0), primary action (1), or navigation pulse (2).
    #[reflect(range = "0..=2")]
    pub deliver: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
pub struct Courier {
    pub position: FPVec2,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
pub struct Delivery {
    pub goal: FPVec2,
    pub won: u32,
    /// Boolean latch shared by primary actions and navigation pulses.
    pub action_held: u32,
}
pub fn types() -> TypeRegistry {
    let mut t = TypeRegistry::new();
    t.register_component::<Courier>("Courier");
    t.register_singleton::<Delivery>("Delivery");
    t
}
pub struct DeliveryGame;
impl Game for DeliveryGame {
    type Input = DeliveryInput;
    type Command = crate::NoCommand;
    type Event = u32;
    type Config = ();
    fn register(b: &mut ComponentRegistryBuilder) {
        b.register_component::<Courier>("Courier");
        b.register_singleton::<Delivery>("Delivery");
    }
    fn setup(f: &mut Frame, _: &()) {
        let e = f.spawn();
        f.add(e, Courier::default());
        f.set_singleton(Delivery {
            goal: FPVec2::new(FP::from_raw(4 << 16), FP::ZERO),
            ..Delivery::default()
        });
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(step as fn(&mut SimContext<Self>))]
    }
}
fn step(c: &mut SimContext<DeliveryGame>) {
    let input = *c.inputs.input(PlayerSlot(0));
    // Binary/direct input bypasses reflected admission. Unknown modes must not
    // move, act, or release a previous latch.
    if input.deliver > MODE_NAVIGATION_PULSE {
        return;
    }
    let mut state = *c.frame.singleton::<Delivery>();
    let released = state.action_held == 0;
    let edge = input.deliver == MODE_PRIMARY && released;
    let pulse = input.deliver == MODE_NAVIGATION_PULSE;
    state.action_held = u32::from(input.deliver != MODE_RELEASED);
    let ids = c.frame.dense::<Courier>().0.to_vec();
    for id in ids {
        let courier = c.frame.get_mut::<Courier>(id).unwrap();
        if state.won != 0 {
            if edge {
                courier.position = FPVec2::ZERO;
                state.won = 0;
            }
        } else {
            let dx = i64::from(input.dx.clamp(-1, 1)) << 16;
            let dy = i64::from(input.dy.clamp(-1, 1)) << 16;
            if pulse {
                if released {
                    // Consume an overflowing pulse without moving either axis.
                    // Trace/replay/direct input cannot bypass this atomic rule.
                    if let (Some(x), Some(y)) = (
                        courier.position.x.raw().checked_add(dx),
                        courier.position.y.raw().checked_add(dy),
                    ) {
                        courier.position = FPVec2::new(FP::from_raw(x), FP::from_raw(y));
                    }
                }
            } else {
                // Preserve the original per-tick mode-0/1 arithmetic.
                courier.position.x += FP::from_raw(dx);
                courier.position.y += FP::from_raw(dy);
            }
            if edge && courier.position == state.goal {
                state.won = 1;
            }
        }
    }
    c.frame.set_singleton(state);
}
