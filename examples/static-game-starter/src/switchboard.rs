//! A distinct four-switch puzzle: select with horizontal input and toggle with action.
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{FPVec2, FP};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{Game, PlayerSlot, SimContext, System};
pub const BUILD: u64 = 0x5354415254455202;
pub const SCENE: &str = include_str!("../scenes/switchboard.scene.yaml");
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct SwitchInput {
    pub selection: FP,
    pub toggle: u32,
    #[reflect(skip)]
    pub pad: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
pub struct Cursor {
    pub position: FPVec2,
    pub selected: u32,
    pub held: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
pub struct Board {
    pub bits: u32,
    pub target: u32,
    pub solved: u32,
    pub moves: u32,
}
pub fn types() -> TypeRegistry {
    let mut t = TypeRegistry::new();
    t.register_component::<Cursor>("Cursor");
    t.register_singleton::<Board>("Board");
    t
}
pub struct SwitchGame;
impl Game for SwitchGame {
    type Input = SwitchInput;
    type Command = crate::NoCommand;
    type Event = u32;
    type Config = ();
    fn register(b: &mut ComponentRegistryBuilder) {
        b.register_component::<Cursor>("Cursor");
        b.register_singleton::<Board>("Board");
    }
    fn setup(f: &mut Frame, _: &()) {
        let e = f.spawn();
        f.add(e, Cursor::default());
        f.set_singleton(Board {
            target: 5,
            ..Board::default()
        });
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(step as fn(&mut SimContext<Self>))]
    }
}
fn step(c: &mut SimContext<SwitchGame>) {
    let input = *c.inputs.input(PlayerSlot(0));
    let mut board = *c.frame.singleton::<Board>();
    let ids = c.frame.dense::<Cursor>().0.to_vec();
    for id in ids {
        let cursor = c.frame.get_mut::<Cursor>(id).unwrap();
        if input.selection > FP::ZERO {
            cursor.selected = (cursor.selected + 1) % 4;
        }
        if input.selection < FP::ZERO {
            cursor.selected = (cursor.selected + 3) % 4;
        }
        cursor.position.x = FP::from_raw(i64::from(cursor.selected) * (2 << 16));
        if input.toggle != 0 && cursor.held == 0 {
            if board.solved != 0 {
                board.bits = 0;
                board.solved = 0;
                board.moves = 0;
            } else {
                board.bits ^= 1 << cursor.selected;
                board.moves = board.moves.saturating_add(1);
                board.solved = u32::from(board.bits == board.target);
            }
        }
        cursor.held = u32::from(input.toggle != 0);
    }
    c.frame.set_singleton(board);
}
