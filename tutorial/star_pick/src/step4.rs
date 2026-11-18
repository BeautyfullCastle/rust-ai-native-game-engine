//! Step 4: two windows on a local server. One window is one player.
//!
//! `cargo run -p star_pick --release --bin step4 -- --host`
//! `cargo run -p star_pick --release --bin step4 -- --connect 127.0.0.1:7777`
//!
//! New in this step: the `Players` trait and `player_signals` (the game gets to know who is in
//! the game), an avatar that a prefab makes when `Added` arrives, and the same `App` code on a
//! server (`--host`, `--connect`). The game has no player at the start any more.

use std::cell::RefCell;
use std::f32::consts::FRAC_PI_4;
use std::rc::Rc;

use bytemuck::{Pod, Zeroable};
use orr_app::{
    fp_to_f32, solid_quad, App, Camera, EntityView, EventState, Extracted, Extractor, FrameView, InterpMode, KeyCode, RenderItem, SceneView,
    Shape, SoundHandle, Style, Transform2, Vec2, View2D, ViewContext,
};
use orr_sim::{
    fn_systems, fp, player_signals, prefab, register_player_link, spawn_prefab, ComponentRegistryBuilder, Entity, Frame, Game, PlayerLink,
    PlayerSignal, PlayerSlot, Players, Prefab, PrefabId, SimContext, Simulation, System, FP,
};

use crate::sound;

// ---- simulation: FP only, no f32 ----

pub const HALF: FP = fp!(4.5);
const STEP: FP = fp!(0.1);
const REACH: FP = fp!(0.75);
const SPAWN_EVERY: u64 = 45;
const MAX_STARS: u32 = 6;
/// The room has two seats.
pub const PLAYERS: u8 = 2;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Pod, Zeroable)]
pub struct Move {
    pub dx: i32,
    pub dy: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Pos {
    pub x: FP,
    pub y: FP,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Score {
    pub points: u32,
    pub _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Star {
    pub born: u64,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Picked {
    pub x: FP,
    pub y: FP,
    pub slot: u32,
    pub points: u32,
}

/// How many avatar colors there are.
pub const COLOR_COUNT: u32 = 4;
/// `Choice::color` for "the color of my seat".
pub const SEAT_COLOR: u32 = u32::MAX;

/// The start data of a player: the color it picked, as an index into the color list. A window
/// sends it when it joins (`App::player_data`), and the server gives it to every window in the
/// `Added` signal of the player, so all windows make the same avatar.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
pub struct Choice {
    pub color: u32,
}

impl Default for Choice {
    fn default() -> Self {
        Self { color: SEAT_COLOR }
    }
}

impl Choice {
    /// The color of the avatar: the pick, or the color of the seat when the pick is no color.
    fn color_of_avatar(self, slot: PlayerSlot) -> u32 {
        if self.color < COLOR_COUNT {
            self.color
        } else {
            u32::from(slot.0) % COLOR_COUNT
        }
    }
}

/// On the avatar: the index of its color, from the start data of the player.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Tint {
    pub color: u32,
}

/// What `setup` needs. The prefab is the recipe of a player entity: its components and their
/// first values. It is baked before the run, because a system cannot read the config.
pub struct Setup {
    pub avatars: Vec<Prefab>,
}

impl Default for Setup {
    fn default() -> Self {
        let registry = Simulation::<StarPick>::build_registry();
        let avatar = Prefab::new()
            .with(&registry, Pos { x: FP::ZERO, y: FP::ZERO })
            .and_then(|prefab| prefab.with(&registry, Score { points: 0, _pad: 0 }))
            .and_then(|prefab| prefab.with(&registry, PlayerLink { player: PlayerSlot::NONE })) // `Added` fills this in
            .expect("StarPick registers these components");
        Self { avatars: vec![avatar] }
    }
}

pub struct StarPick;

impl Game for StarPick {
    type Input = Move;
    type Command = ();
    type Event = Picked;
    type Config = Setup;

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<Pos>("Pos");
        builder.register_component::<Score>("Score");
        builder.register_component::<Star>("Star");
        builder.register_component::<Tint>("Tint");
        register_player_link(builder);
        prefab::register(builder); // the place where the frame keeps its prefabs
    }

    fn setup(frame: &mut Frame, config: &Setup) {
        // No player exists yet: `Added` makes them.
        prefab::install(frame, &config.avatars).expect("StarPick registers prefabs");
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        // `player_signals` goes first: the players of this tick exist before the other systems run.
        let mut systems = vec![player_signals::<Self>()];
        systems.extend(fn_systems([spawn_stars, move_players, pick_stars]));
        systems
    }
}

impl Players for StarPick {
    /// The start data of a player: the color it picked.
    type Player = Choice;

    fn on_player(frame: &mut Frame, signal: PlayerSignal<Choice>) {
        match signal {
            PlayerSignal::Added { slot, data } => {
                if avatar_of(frame, slot).is_some() {
                    return; // the same player never gets two avatars
                }
                let Some(avatar) = spawn_prefab(frame, PrefabId(0)) else { return };
                if let Some(link) = frame.get_mut::<PlayerLink>(avatar) {
                    link.player = slot;
                }
                frame.add(avatar, Tint { color: data.color_of_avatar(slot) });
                if let Some(pos) = frame.get_mut::<Pos>(avatar) {
                    pos.x = FP::from_int(4 * i32::from(slot.0) - 2); // slot 0 at -2, slot 1 at 2
                }
            }
            // Step 5 handles these two.
            PlayerSignal::Disconnected { .. } | PlayerSignal::Reconnected { .. } => {}
            PlayerSignal::Removed { slot } => {
                if let Some(avatar) = avatar_of(frame, slot) {
                    frame.despawn(avatar);
                }
            }
        }
    }
}

/// The avatar of a player: the entity whose `PlayerLink` names the slot.
pub fn avatar_of(frame: &mut Frame, slot: PlayerSlot) -> Option<Entity> {
    frame.query::<(&PlayerLink,)>().find(|(_, (link,))| link.player == slot).map(|(entity, _)| entity)
}

fn spawn_stars(ctx: &mut SimContext<StarPick>) {
    if ctx.tick % SPAWN_EVERY != 0 || ctx.frame.count::<Star>() >= MAX_STARS {
        return;
    }
    let edge = HALF - fp!(0.5);
    let x = ctx.rng().range_fp(-edge, edge);
    let y = ctx.rng().range_fp(-edge, edge);
    let star = ctx.frame.spawn();
    ctx.frame.add(star, Pos { x, y });
    ctx.frame.add(star, Star { born: ctx.tick });
}

fn move_players(ctx: &mut SimContext<StarPick>) {
    let inputs = ctx.inputs;
    for (_, (pos, link)) in ctx.frame.query::<(&mut Pos, &PlayerLink)>() {
        if link.player.0 >= inputs.player_count() {
            continue;
        }
        let input = inputs.input(link.player);
        pos.x = (pos.x + STEP.mul_int(input.dx.clamp(-1, 1))).clamp(-HALF, HALF);
        pos.y = (pos.y + STEP.mul_int(input.dy.clamp(-1, 1))).clamp(-HALF, HALF);
    }
}

fn pick_stars(ctx: &mut SimContext<StarPick>) {
    let stars: Vec<(Entity, Pos)> = ctx.frame.query::<(&Pos, &Star)>().map(|(star, (pos, _))| (star, *pos)).collect();
    let mut picked: Vec<Entity> = Vec::new();
    let mut events: Vec<Picked> = Vec::new();
    for (_, (pos, score, link)) in ctx.frame.query::<(&Pos, &mut Score, &PlayerLink)>() {
        for (star, at) in &stars {
            let near = (pos.x - at.x).abs() < REACH && (pos.y - at.y).abs() < REACH;
            if near && !picked.contains(star) {
                picked.push(*star);
                score.points += 1;
                events.push(Picked { x: at.x, y: at.y, slot: u32::from(link.player.0), points: score.points });
            }
        }
    }
    for star in picked {
        ctx.frame.despawn(star);
    }
    for event in events {
        ctx.emit(event);
    }
}

// ---- view ----

const FIELD: [f32; 4] = [0.09, 0.10, 0.15, 1.0];
const COLORS: [[f32; 4]; COLOR_COUNT as usize] = [[0.35, 0.70, 0.95, 1.0], [0.95, 0.55, 0.30, 1.0], [0.45, 0.85, 0.50, 1.0], [0.75, 0.50, 0.90, 1.0]];
const STAR: [f32; 4] = [1.0, 0.85, 0.25, 1.0];
const RING_TICKS: u64 = 40;
const KEEP_TICKS: u64 = 240;
const TRAIL_DOTS: usize = 12;

fn color_of(index: u32) -> [f32; 4] {
    COLORS[index as usize % COLORS.len()]
}

/// The color of the avatar of `slot`; the color of the seat while it has no avatar.
fn avatar_color(frame: FrameView<'_>, slot: u32) -> [f32; 4] {
    let tint = frame.iter::<PlayerLink>().find(|(_, link)| u32::from(link.player.0) == slot).and_then(|(entity, _)| frame.get::<Tint>(entity));
    color_of(tint.map_or(slot, |tint| tint.color))
}

pub struct Ring {
    pub picked: Picked,
    pub start: u64,
    pub color: [f32; 4],
    /// The pick-up sound. A rollback that takes the pick-up back stops it.
    pub sound: Option<SoundHandle>,
}

pub struct Trail {
    pub entity: Entity,
    pub color: [f32; 4],
    pub dots: Vec<Vec2>,
}

#[derive(Default)]
pub struct Fx {
    pub now: u64,
    pub scores: [u32; 2],
    pub rings: Vec<Ring>,
    pub trails: Vec<Trail>,
}

pub type Shared = Rc<RefCell<Fx>>;

fn disc(center: Vec2, radius: f32, color: [f32; 4]) -> RenderItem {
    RenderItem {
        entity: Entity::NONE,
        transform: Transform2::new(center, 0.0),
        style: Style { shape: Shape::Circle, size: radius, half_y: 0.0, color },
    }
}

pub struct Look {
    pub fx: Shared,
}

impl Extractor for Look {
    #[allow(clippy::float_arithmetic)]
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted>) {
        for (entity, pos) in frame.iter::<Pos>() {
            let at = Vec2::new(fp_to_f32(pos.x), fp_to_f32(pos.y));
            let (style, mode, turn) = match (frame.get::<Star>(entity), frame.get::<PlayerLink>(entity)) {
                (Some(star), _) => {
                    let age = frame.tick().saturating_sub(star.born).min(10) as f32;
                    (Style { shape: Shape::Quad, size: 0.08 + 0.017 * age, half_y: 0.0, color: STAR }, InterpMode::None, FRAC_PI_4)
                }
                (None, link) => {
                    let color = color_of(frame.get::<Tint>(entity).map_or_else(|| link.map_or(0, |link| u32::from(link.player.0)), |tint| tint.color));
                    (Style { shape: Shape::Quad, size: 0.4, half_y: 0.0, color }, InterpMode::Prediction, 0.0)
                }
            };
            out.push(Extracted { entity, transform: Transform2::new(at, turn), mode, style });
        }
    }
}

impl View2D for Look {
    fn camera(&self, _frame: FrameView<'_>) -> Camera {
        Camera::new([0.0, 0.0], 5.5)
    }

    #[allow(clippy::float_arithmetic)]
    fn backdrop(&self, _frame: FrameView<'_>, out: &mut Vec<RenderItem>) {
        let fx = self.fx.borrow();
        out.push(solid_quad([0.0, 0.0], [4.5, 4.5], FIELD));
        for trail in &fx.trails {
            for (i, dot) in trail.dots.iter().enumerate() {
                let fade = 0.15 + 0.05 * i as f32;
                out.push(solid_quad([dot.x, dot.y], [0.08, 0.08], [trail.color[0], trail.color[1], trail.color[2], fade]));
            }
        }
        for ring in &fx.rings {
            let age = fx.now.saturating_sub(ring.start);
            if age < RING_TICKS {
                let t = age as f32 / RING_TICKS as f32;
                let center = Vec2::new(fp_to_f32(ring.picked.x), fp_to_f32(ring.picked.y));
                let color = ring.color;
                out.push(disc(center, 0.3 + 0.9 * t, [color[0], color[1], color[2], 0.6 * (1.0 - t)]));
            }
        }
    }
}

/// Only the player of this window leaves a trail. `is_local` uses the slot the server gave.
pub struct TrailView {
    pub fx: Shared,
}

impl EntityView<StarPick> for TrailView {
    fn on_create(&mut self, ctx: &mut ViewContext<'_, StarPick>, entity: Entity) {
        let Some(link) = ctx.predicted().get::<PlayerLink>(entity) else { return };
        if ctx.is_local(link.player) {
            let color = avatar_color(ctx.predicted(), u32::from(link.player.0));
            self.fx.borrow_mut().trails.push(Trail { entity, color, dots: Vec::new() });
        }
    }

    #[allow(clippy::float_arithmetic)]
    fn on_update(&mut self, _ctx: &mut ViewContext<'_, StarPick>, entity: Entity, pose: Option<Transform2>) {
        let Some(pose) = pose else { return };
        let mut fx = self.fx.borrow_mut();
        let Some(trail) = fx.trails.iter_mut().find(|trail| trail.entity == entity) else { return };
        if trail.dots.last().is_none_or(|last| (pose.pos - *last).length() > 0.25) {
            trail.dots.push(pose.pos);
            if trail.dots.len() > TRAIL_DOTS {
                trail.dots.remove(0);
            }
        }
    }

    fn on_destroy(&mut self, _ctx: &mut ViewContext<'_, StarPick>, entity: Entity) {
        self.fx.borrow_mut().trails.retain(|trail| trail.entity != entity);
    }
}

pub struct Juice {
    pub fx: Shared,
}

impl SceneView<StarPick> for Juice {
    fn on_event(&mut self, ctx: &mut ViewContext<'_, StarPick>, event: &Picked, state: EventState) {
        let mut fx = self.fx.borrow_mut();
        match state {
            EventState::Predicted | EventState::Verified => {
                if !fx.rings.iter().any(|ring| ring.picked == *event) {
                    let sound = ctx.audio().play_named(sound::PICKUP); // a short ding, at once
                    let color = avatar_color(ctx.predicted(), event.slot);
                    fx.rings.push(Ring { picked: *event, start: ctx.tick(), color, sound });
                }
            }
            EventState::Canceled => {
                for sound in fx.rings.iter().filter(|ring| ring.picked == *event).filter_map(|ring| ring.sound) {
                    ctx.audio().stop(sound);
                }
                fx.rings.retain(|ring| ring.picked != *event);
            }
        }
    }

    /// A resync throws away the events that were predicted, and no `Canceled` comes for them.
    /// Stop their sounds and forget their rings. The new frame brings the pick-ups again.
    fn on_resync(&mut self, ctx: &mut ViewContext<'_, StarPick>) {
        let mut fx = self.fx.borrow_mut();
        for sound in fx.rings.drain(..).filter_map(|ring| ring.sound) {
            ctx.audio().stop(sound);
        }
    }

    #[allow(clippy::float_arithmetic)]
    fn on_update(&mut self, ctx: &mut ViewContext<'_, StarPick>) {
        let mut fx = self.fx.borrow_mut();
        fx.now = ctx.tick();
        if let Some(verified) = ctx.verified() {
            let mut scores = [0; 2];
            for (entity, score) in verified.iter::<Score>() {
                let slot = verified.get::<PlayerLink>(entity).map_or(0, |link| usize::from(link.player.0));
                if let Some(total) = scores.get_mut(slot) {
                    *total = score.points;
                }
            }
            fx.scores = scores;
        }
        // The score of each player as text in the top left corner: player 1 on the first line, player 2 on the second.
        for (slot, score) in fx.scores.iter().enumerate() {
            let line = 14.0 + 30.0 * slot as f32;
            let color = avatar_color(ctx.predicted(), slot as u32);
            ctx.draw().text([16.0, line], 21.0, color, &format!("PLAYER {} SCORE {}", slot + 1, score));
        }
        let now = fx.now;
        fx.rings.retain(|ring| now < ring.start + KEEP_TICKS);
    }
}

/// W A S D or the arrow keys: in a window of its own, a player may use either hand.
pub fn read_keys(keys: &orr_app::InputContext) -> Move {
    Move {
        dx: keys.axis(&[KeyCode::KeyA, KeyCode::ArrowLeft], &[KeyCode::KeyD, KeyCode::ArrowRight]),
        dy: keys.axis(&[KeyCode::KeyS, KeyCode::ArrowDown], &[KeyCode::KeyW, KeyCode::ArrowUp]),
    }
}

/// The color this window asks for: `STAR_COLOR=0` to `STAR_COLOR=3` in the environment. Without it,
/// the avatar gets the color of its seat.
pub fn chosen_color() -> Choice {
    std::env::var("STAR_COLOR").ok().and_then(|text| text.parse().ok()).map_or_else(Choice::default, |color| Choice { color })
}

pub fn app() -> App<StarPick> {
    let fx = Shared::default();
    let for_trails = fx.clone();
    App::<StarPick>::new() // `Setup::default()` bakes the avatar prefab
        .players(PLAYERS) // two seats: the server waits until both are taken
        .player_data(chosen_color()) // goes to the server, which gives it to every window
        .input(|keys, _slot| read_keys(keys))
        .view(Look { fx: fx.clone() })
        .entity_view::<Score>(move || TrailView { fx: for_trails.clone() })
        .scene_view(Juice { fx })
        .sound(sound::PICKUP, sound::pickup())
        .title("Star Pick 4")
}
