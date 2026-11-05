//! A distinct four-switch puzzle: select with horizontal input and toggle with action.
use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{FPVec2, FP};
use orr_reflect::{Reflect, TypeRegistry};
use orr_sim::{Game, PlayerSlot, SimContext, System};
pub const BUILD: u64 = 0x5354415254563202;
/// Released primary action; selection still changes on every tick.
pub const MODE_RELEASED: u32 = 0;
/// Held primary action; selection still changes on every tick.
pub const MODE_PRIMARY: u32 = 1;
/// Navigation-only edge pulse; mode 0 releases the shared latch.
pub const MODE_NAVIGATION_PULSE: u32 = 2;
pub const SCENE: &str = include_str!("../scenes/switchboard.scene.yaml");
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable, Reflect)]
pub struct SwitchInput {
    pub selection: FP,
    /// Control mode: released (0), primary action (1), or navigation pulse (2).
    #[reflect(range = "0..=2")]
    pub toggle: u32,
    #[reflect(skip)]
    pub pad: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Reflect)]
pub struct Cursor {
    pub position: FPVec2,
    pub selected: u32,
    /// Boolean latch shared by primary actions and navigation pulses.
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
    // Binary/direct input bypasses reflected admission. Unknown modes leave
    // the board, cursor, and previous latch unchanged.
    if input.toggle > MODE_NAVIGATION_PULSE {
        return;
    }
    let mut board = *c.frame.singleton::<Board>();
    let ids = c.frame.dense::<Cursor>().0.to_vec();
    for id in ids {
        let cursor = c.frame.get_mut::<Cursor>(id).unwrap();
        if input.toggle != MODE_NAVIGATION_PULSE || cursor.held == 0 {
            if input.toggle == MODE_NAVIGATION_PULSE {
                // New pulse semantics stay bounded even for raw, non-admitted
                // state. Keep the old mode-0/1 arithmetic unchanged.
                cursor.selected %= 4;
            }
            if input.selection > FP::ZERO {
                cursor.selected = (cursor.selected + 1) % 4;
            }
            if input.selection < FP::ZERO {
                cursor.selected = (cursor.selected + 3) % 4;
            }
            cursor.position.x = FP::from_raw(i64::from(cursor.selected) * (2 << 16));
        }
        if input.toggle == MODE_PRIMARY && cursor.held == 0 {
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
        cursor.held = u32::from(input.toggle != MODE_RELEASED);
    }
    c.frame.set_singleton(board);
}
