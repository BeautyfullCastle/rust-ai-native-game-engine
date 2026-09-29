//! The arena test game seen by the view layer: which entities to draw and
//! how, the local input mapping, and a loopback two-peer session with a bot.

use orr_bridge::{BridgeConfig, LoopbackPair, PlayerSlot, FrameView};
use orr_ecs::Entity;
use orr_fp::{FrameRng, FP};
use orr_session::SessionConfig;
use orr_testgame::{
    Arena, ArenaConfig, ArenaInput, Bullet, PlayerTag, Position, SpawnBulletCmd, ARENA_HALF, BULLET_RADIUS, FIRE, PLAYER_RADIUS,
};
use orr_view::{fp_to_f32, fp_to_vec2, Extracted, Extractor, InterpMode, RenderItem, Shape, Style, Transform2, Vec2};

/// Slot of the player at the keyboard.
pub const LOCAL_SLOT: u8 = 0;

const PLAYER_COLORS: [[f32; 4]; 2] = [[0.25, 0.6, 1.0, 1.0], [1.0, 0.55, 0.2, 1.0]];
const BULLET_COLORS: [[f32; 4]; 2] = [[0.7, 0.85, 1.0, 1.0], [1.0, 0.8, 0.55, 1.0]];

/// Reads players and bullets out of an arena frame.
/// The local player and all bullets are predicted; other players use `remote_mode`.
pub struct ArenaExtractor {
    pub remote_mode: InterpMode,
}

impl Extractor for ArenaExtractor {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted>) {
        for (entity, position) in frame.iter::<Position>() {
            let transform = Transform2::new(fp_to_vec2(position.pos), 0.0);
            if let Some(tag) = frame.get::<PlayerTag>(entity) {
                let mode = if tag.slot == u32::from(LOCAL_SLOT) { InterpMode::Prediction } else { self.remote_mode };
                let color = PLAYER_COLORS[tag.slot as usize % PLAYER_COLORS.len()];
                let style = Style { shape: Shape::Circle, size: fp_to_f32(PLAYER_RADIUS), color };
                out.push(Extracted { entity, transform, mode, style });
            } else if let Some(bullet) = frame.get::<Bullet>(entity) {
                let color = BULLET_COLORS[bullet.owner_slot as usize % BULLET_COLORS.len()];
                let style = Style { shape: Shape::Circle, size: fp_to_f32(BULLET_RADIUS), color };
                out.push(Extracted { entity, transform, mode: InterpMode::Prediction, style });
            }
        }
    }
}

/// The dark square the game is played on. Drawn first, not part of the sim.
pub fn arena_floor() -> RenderItem {
    RenderItem {
        entity: Entity::NONE,
        transform: Transform2::new(Vec2::ZERO, 0.0),
        style: Style { shape: Shape::Quad, size: fp_to_f32(ARENA_HALF), color: [0.07, 0.07, 0.11, 1.0] },
    }
}

fn fire_commands(owner: u32, input: &ArenaInput) -> Vec<SpawnBulletCmd> {
    if input.buttons & FIRE != 0 {
        vec![SpawnBulletCmd { owner }]
    } else {
        Vec::new()
    }
}

/// Bridge settings for the arena: a set fire bit spawns a bullet command.
pub fn arena_bridge_config() -> BridgeConfig<Arena> {
    BridgeConfig::default().with_commands_from_input(|input| fire_commands(u32::from(LOCAL_SLOT), input))
}

/// The keys of the local player.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Keys {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub fire: bool,
}

impl Keys {
    pub fn to_input(self) -> ArenaInput {
        let axis = |neg: bool, pos: bool| FP::from_int(i32::from(pos) - i32::from(neg));
        ArenaInput::new(axis(self.left, self.right), axis(self.down, self.up), self.fire)
    }
}

/// Network conditions of the loopback link, in sim ticks one way.
#[derive(Clone, Copy, Debug)]
pub struct Loopback {
    pub latency_ticks: u64,
    pub jitter_ticks: u64,
}

impl Default for Loopback {
    fn default() -> Self {
        // 6 ticks is 100 ms one way: well above the input delay of 2, so rollbacks happen often.
        Self { latency_ticks: 6, jitter_ticks: 2 }
    }
}

/// A wandering, shooting bot: picks a new direction every 30 to 90 ticks and fires every 20 ticks.
pub fn bot(seed: u64) -> impl FnMut(u64) -> (ArenaInput, Vec<SpawnBulletCmd>) {
    let mut rng = FrameRng::new(seed);
    let (mut ax, mut ay) = (1, 0);
    let mut left = 0u32;
    move |tick| {
        if left == 0 {
            ax = rng.range_i32(-1, 2);
            ay = rng.range_i32(-1, 2);
            left = 30 + rng.next_u32() % 60;
        }
        left -= 1;
        let input = ArenaInput::new(FP::from_int(ax), FP::from_int(ay), tick % 20 == 0);
        let commands = fire_commands(1, &input);
        (input, commands)
    }
}

/// The local peer plus a bot peer, joined by a simulated-latency loopback link.
pub fn loopback_pair(net: Loopback) -> LoopbackPair<Arena> {
    LoopbackPair::new(
        || ArenaConfig { player_count: 2 },
        SessionConfig::new(2, PlayerSlot(LOCAL_SLOT), 42, 60),
        SessionConfig::new(2, PlayerSlot(1), 42, 60),
        net.latency_ticks,
        net.jitter_ticks,
        777,
        bot(1234),
    )
}
