//! Move the courier to the goal, press action to deliver, release and press to restart.
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{FPVec2, FP};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{Game, PlayerSlot, SimContext, System};

pub const BUILD: u64 = 0x5354415254455201;
pub const SCENE: &str = include_str!("../scenes/delivery.scene.yaml");
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct DeliveryInput {
    pub dx: i32,
    pub dy: i32,
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
    let mut state = *c.frame.singleton::<Delivery>();
    let edge = input.deliver != 0 && state.action_held == 0;
    state.action_held = u32::from(input.deliver != 0);
    let ids = c.frame.dense::<Courier>().0.to_vec();
    for id in ids {
        let courier = c.frame.get_mut::<Courier>(id).unwrap();
        if state.won != 0 {
            if edge {
                courier.position = FPVec2::ZERO;
                state.won = 0;
            }
        } else {
            courier.position.x += FP::from_raw(i64::from(input.dx.clamp(-1, 1)) << 16);
            courier.position.y += FP::from_raw(i64::from(input.dy.clamp(-1, 1)) << 16);
            if edge && courier.position == state.goal {
                state.won = 1;
            }
        }
    }
    c.frame.set_singleton(state);
}
