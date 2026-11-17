//! `Roster`: a tiny game whose only world content comes from player signals.
//! It is the test game for [`orr_sim::Players`]: no avatar exists at the start,
//! `Added` spawns one (with the color from the start data), `Disconnected` and
//! `Reconnected` flip its `away` flag, `Removed` deletes it. Each avatar moves
//! along a line by the `dx` of its player's input, unless it is away.
//!
//! A `Census` singleton counts every signal the game received, so a test can
//! tell that a signal was applied once and in the right tick.

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_sim::{fn_systems, player_signals, Game, PlayerSignal, PlayerSlot, Players, SimContext, System};

/// The per-tick input of a player: how far to move along the line.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Pod, Zeroable)]
pub struct RosterInput {
    pub dx: i32,
}

/// The start data of a player (`Players::Player`).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Pod, Zeroable)]
pub struct Profile {
    pub color: u32,
    pub _pad: u32,
}

/// Event: an avatar moved.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Moved {
    pub slot: u32,
    pub x: i32,
}

/// One per player that is in the game.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
pub struct Avatar {
    pub slot: u32,
    pub x: i32,
    pub color: u32,
    /// 1 from `Disconnected` until `Reconnected`.
    pub away: u32,
}

/// How many signals of each kind the game has received.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Pod, Zeroable)]
pub struct Census {
    pub added: u32,
    pub disconnected: u32,
    pub reconnected: u32,
    pub removed: u32,
}

pub struct Roster;

impl Game for Roster {
    type Input = RosterInput;
    type Command = ();
    type Event = Moved;
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<Avatar>("Avatar");
        builder.register_singleton::<Census>("Census");
    }

    fn setup(frame: &mut Frame, _config: &()) {
        frame.set_singleton(Census::default());
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        let mut systems = vec![player_signals::<Self>()];
        systems.extend(fn_systems([move_avatars]));
        systems
    }
}

impl Players for Roster {
    type Player = Profile;

    fn on_player(frame: &mut Frame, signal: PlayerSignal<Profile>) {
        let census = frame.singleton_mut::<Census>();
        match signal {
            PlayerSignal::Added { .. } => census.added += 1,
            PlayerSignal::Disconnected { .. } => census.disconnected += 1,
            PlayerSignal::Reconnected { .. } => census.reconnected += 1,
            PlayerSignal::Removed { .. } => census.removed += 1,
        }
        let existing = avatar_of(frame, signal.slot());
        match (signal, existing) {
            (PlayerSignal::Added { slot, data }, None) => {
                let e = frame.spawn();
                frame.add(e, Avatar { slot: u32::from(slot.0), x: 0, color: data.color, away: 0 });
            }
            (PlayerSignal::Added { data, .. }, Some(e)) => {
                if let Some(a) = frame.get_mut::<Avatar>(e) {
                    a.color = data.color;
                    a.away = 0;
                }
            }
            (PlayerSignal::Disconnected { .. }, Some(e)) => {
                if let Some(a) = frame.get_mut::<Avatar>(e) {
                    a.away = 1;
                }
            }
            (PlayerSignal::Reconnected { .. }, Some(e)) => {
                if let Some(a) = frame.get_mut::<Avatar>(e) {
                    a.away = 0;
                }
            }
            (PlayerSignal::Removed { .. }, Some(e)) => {
                frame.despawn(e);
            }
            _ => {}
        }
    }
}

fn avatar_of(frame: &mut Frame, slot: PlayerSlot) -> Option<Entity> {
    frame.query::<(&Avatar,)>().find(|(_, (a,))| a.slot == u32::from(slot.0)).map(|(e, _)| e)
}

fn move_avatars(ctx: &mut SimContext<Roster>) {
    for slot in 0..ctx.inputs.player_count() {
        let dx = ctx.inputs.input(PlayerSlot(slot)).dx;
        if dx == 0 {
            continue;
        }
        let Some(e) = avatar_of(ctx.frame, PlayerSlot(slot)) else { continue };
        let Some(avatar) = ctx.frame.get_mut::<Avatar>(e) else { continue };
        if avatar.away != 0 {
            continue;
        }
        avatar.x += dx;
        let moved = Moved { slot: u32::from(slot), x: avatar.x };
        ctx.emit(moved);
    }
}
