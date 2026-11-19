//! Step 2: playing alone. Move the square (W A S D or the arrow keys), pick up the stars.
//!
//! New in this step: input, systems that move and score, an event, an `EntityView` (a trail
//! behind the player) and a `SceneView` (the score text, the pick-up ring and its sound), and
//! the game screens: a title, a pause, a win after `WIN_STARS` stars, and a restart.
//!
//! `cargo run -p star_pick --release --bin step2`

use std::cell::RefCell;
use std::f32::consts::FRAC_PI_4;
use std::rc::Rc;

use bytemuck::{Pod, Zeroable};
use orr_app::{
    fp_to_f32, solid_quad, App, Camera, EntityView, EventState, Extracted, Extractor, FrameView, InterpMode, KeyCode, RenderItem, SceneView,
    Shape, SoundHandle, Style, Transform2, Vec2, View2D, ViewContext,
};
use orr_app::{Anchor, Labels, Screen, Screens};
use orr_sim::{decode_pod, encode_pod, fn_systems, fp, ComponentRegistryBuilder, Entity, Frame, Game, PlayerSlot, SimCommand, SimContext, System, FP};

use crate::sound;

// ---- simulation: FP only, no f32 ----

/// Half of the field width and height.
pub const HALF: FP = fp!(4.5);
/// How far the player walks in one tick at full input.
const STEP: FP = fp!(0.1);
/// A star is picked when the player is closer than this on both axes.
const REACH: FP = fp!(0.75);
/// A new star appears every 45 ticks (0.75 s), up to 6 at the same time.
const SPAWN_EVERY: u64 = 45;
const MAX_STARS: u32 = 6;
/// The game is won with this many stars.
pub const WIN_STARS: u32 = 5;

/// The input of one player in one tick: -1, 0 or 1 on each axis.
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

/// The one command of the game: start a new round. The player goes back to the middle, the
/// score to 0, and every star is gone. The restart screen sends it (see `Juice`).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Pod, Zeroable)]
pub struct Restart {
    pub _pad: u32,
}

impl SimCommand for Restart {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_pod(self, out);
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        decode_pod(bytes)
    }
}

/// On the entity of a player.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Score {
    pub points: u32,
    pub _pad: u32,
}

/// On every star. `born` is the tick in which it appeared.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Star {
    pub born: u64,
}

/// Event: a star was picked. The window plays the effect from it.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Picked {
    pub x: FP,
    pub y: FP,
    pub slot: u32,
    /// The score of the player after this star.
    pub points: u32,
}

pub struct StarPick;

impl Game for StarPick {
    type Input = Move;
    type Command = Restart;
    type Event = Picked;
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<Pos>("Pos");
        builder.register_component::<Score>("Score");
        builder.register_component::<Star>("Star");
    }

    fn setup(frame: &mut Frame, _config: &()) {
        let player = frame.spawn();
        frame.add(player, Pos { x: FP::ZERO, y: FP::ZERO });
        frame.add(player, Score { points: 0, _pad: 0 });
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        // The order matters: a restart comes first, and a star that appears is not picked in the same tick.
        fn_systems([restart, spawn_stars, move_player, pick_stars])
    }
}

/// A new round on command: the same in a replay and on a server, because it is part of the game.
fn restart(ctx: &mut SimContext<StarPick>) {
    if ctx.inputs.commands().is_empty() {
        return;
    }
    let stars: Vec<Entity> = ctx.frame.query::<(&Star,)>().map(|(star, _)| star).collect();
    for star in stars {
        ctx.frame.despawn(star);
    }
    for (_, (pos, score)) in ctx.frame.query::<(&mut Pos, &mut Score)>() {
        *pos = Pos { x: FP::ZERO, y: FP::ZERO };
        score.points = 0;
    }
}

fn spawn_stars(ctx: &mut SimContext<StarPick>) {
    if ctx.tick % SPAWN_EVERY != 0 || ctx.frame.count::<Star>() >= MAX_STARS {
        return;
    }
    // The random numbers come from the frame, so every run with the same seed gets the same stars.
    let edge = HALF - fp!(0.5);
    let x = ctx.rng().range_fp(-edge, edge);
    let y = ctx.rng().range_fp(-edge, edge);
    let star = ctx.frame.spawn();
    ctx.frame.add(star, Pos { x, y });
    ctx.frame.add(star, Star { born: ctx.tick });
}

fn move_player(ctx: &mut SimContext<StarPick>) {
    let input = *ctx.inputs.input(PlayerSlot(0));
    let (dx, dy) = (input.dx.clamp(-1, 1), input.dy.clamp(-1, 1)); // never trust an input value
    for (_, (pos, _)) in ctx.frame.query::<(&mut Pos, &Score)>() {
        pos.x = (pos.x + STEP.mul_int(dx)).clamp(-HALF, HALF);
        pos.y = (pos.y + STEP.mul_int(dy)).clamp(-HALF, HALF);
    }
}

fn pick_stars(ctx: &mut SimContext<StarPick>) {
    let stars: Vec<(Entity, Pos)> = ctx.frame.query::<(&Pos, &Star)>().map(|(star, (pos, _))| (star, *pos)).collect();
    let mut picked: Vec<Entity> = Vec::new();
    let mut events: Vec<Picked> = Vec::new();
    for (_, (pos, score)) in ctx.frame.query::<(&Pos, &mut Score)>() {
        for (star, at) in &stars {
            let near = (pos.x - at.x).abs() < REACH && (pos.y - at.y).abs() < REACH;
            if near && !picked.contains(star) {
                picked.push(*star);
                score.points += 1;
                events.push(Picked { x: at.x, y: at.y, slot: 0, points: score.points });
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

// ---- view: the window reads the simulation, it never changes it ----

const FIELD: [f32; 4] = [0.09, 0.10, 0.15, 1.0];
const PLAYER: [f32; 4] = [0.35, 0.70, 0.95, 1.0];
const STAR: [f32; 4] = [1.0, 0.85, 0.25, 1.0];
/// The ring grows for 40 ticks and is remembered for 240.
const RING_TICKS: u64 = 40;
const KEEP_TICKS: u64 = 240;
const TRAIL_DOTS: usize = 12;

/// A ring that grows where a star was picked.
pub struct Ring {
    pub picked: Picked,
    pub start: u64,
    /// The pick-up sound. A rollback that takes the pick-up back stops it.
    pub sound: Option<SoundHandle>,
}

pub struct Trail {
    pub entity: Entity,
    pub dots: Vec<Vec2>,
}

/// What the views share. The `SceneView` and the `EntityView` write it, `Look` draws it.
#[derive(Default)]
pub struct Fx {
    pub now: u64,
    /// The score of the newest verified frame.
    pub score: u32,
    pub rings: Vec<Ring>,
    pub trails: Vec<Trail>,
    /// The title, pause and win screens. `None`: no screens, the game just plays.
    pub screens: Option<Screens>,
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
            let (style, mode, turn) = match frame.get::<Star>(entity) {
                // A new star grows for 10 ticks. A star never moves, so it needs no smoothing.
                Some(star) => {
                    let age = frame.tick().saturating_sub(star.born).min(10) as f32;
                    (Style { shape: Shape::Quad, size: 0.08 + 0.017 * age, half_y: 0.0, color: STAR }, InterpMode::None, FRAC_PI_4)
                }
                None => (Style { shape: Shape::Quad, size: 0.4, half_y: 0.0, color: PLAYER }, InterpMode::Prediction, 0.0),
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
        for (i, dot) in fx.trails.iter().flat_map(|trail| trail.dots.iter()).enumerate() {
            let fade = 0.15 + 0.05 * (i % TRAIL_DOTS) as f32;
            out.push(solid_quad([dot.x, dot.y], [0.08, 0.08], [PLAYER[0], PLAYER[1], PLAYER[2], fade]));
        }
        for ring in &fx.rings {
            let age = fx.now.saturating_sub(ring.start);
            if age < RING_TICKS {
                let t = age as f32 / RING_TICKS as f32;
                let center = Vec2::new(fp_to_f32(ring.picked.x), fp_to_f32(ring.picked.y));
                out.push(disc(center, 0.3 + 0.9 * t, [STAR[0], STAR[1], STAR[2], 0.6 * (1.0 - t)]));
            }
        }
    }
}

/// One per player: leaves a trail of dots behind the square.
pub struct TrailView {
    pub fx: Shared,
}

impl EntityView<StarPick> for TrailView {
    fn on_create(&mut self, _ctx: &mut ViewContext<'_, StarPick>, entity: Entity) {
        self.fx.borrow_mut().trails.push(Trail { entity, dots: Vec::new() });
    }

    #[allow(clippy::float_arithmetic)]
    fn on_update(&mut self, _ctx: &mut ViewContext<'_, StarPick>, entity: Entity, pose: Option<Transform2>) {
        let Some(pose) = pose else { return };
        let mut fx = self.fx.borrow_mut();
        let Some(trail) = fx.trails.iter_mut().find(|trail| trail.entity == entity) else { return };
        // A new dot only when the square moved a bit, so a standing square leaves none.
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

/// One for the whole screen: the pick-up effect and the score.
pub struct Juice {
    pub fx: Shared,
}

impl SceneView<StarPick> for Juice {
    fn on_event(&mut self, ctx: &mut ViewContext<'_, StarPick>, event: &Picked, state: EventState) {
        let mut fx = self.fx.borrow_mut();
        match state {
            // Predicted: the game guessed it. Start the effect at once, a rollback may still undo it.
            // Verified: it is certain. Offline, an event is always Verified at first, so start here too
            // (unless Predicted already did).
            EventState::Predicted | EventState::Verified => {
                // The win comes from a verified pick-up, never from a guess that a rollback may undo.
                if state == EventState::Verified && event.points >= WIN_STARS {
                    if let Some(screens) = &fx.screens {
                        screens.win();
                    }
                }
                if !fx.rings.iter().any(|ring| ring.picked == *event) {
                    let sound = ctx.audio().play_named(sound::PICKUP); // a short ding, at once
                    fx.rings.push(Ring { picked: *event, start: ctx.tick(), sound });
                }
            }
            // Canceled: a rollback took the pick-up back. Stop the effect.
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

    fn on_update(&mut self, ctx: &mut ViewContext<'_, StarPick>) {
        let mut fx = self.fx.borrow_mut();
        fx.now = ctx.tick();
        // The score comes from the verified frame: it never jumps back.
        if let Some(verified) = ctx.verified() {
            fx.score = verified.iter::<Score>().map(|(_, score)| score.points).sum();
        }
        // The score in the top left corner and the goal at the top middle, but not behind the title.
        // The overlay text is drawn with the system Korean font (or the font of `App::font`), so Hangul works.
        if fx.screens.as_ref().is_none_or(|screens| screens.screen() != Screen::Title) {
            ctx.draw().text([16.0, 14.0], 21.0, STAR, &format!("점수 {}", fx.score));
            ctx.draw().text_anchored(Anchor::Top, [0.0, 14.0], 21.0, [1.0, 1.0, 1.0, 0.85], &format!("별 {WIN_STARS}개를 모으세요"));
        }
        if let Some(screens) = &fx.screens {
            // A new round (the start, a restart) is a command to the game.
            if screens.take_restart() {
                let _ = ctx.send_command(Restart::default());
            }
            screens.draw(ctx.draw());
        }
        let now = fx.now;
        fx.rings.retain(|ring| now < ring.start + KEEP_TICKS);
    }
}

/// W A S D or the arrow keys.
pub fn read_keys(keys: &orr_app::InputContext) -> Move {
    Move {
        dx: keys.axis(&[KeyCode::KeyA, KeyCode::ArrowLeft], &[KeyCode::KeyD, KeyCode::ArrowRight]),
        dy: keys.axis(&[KeyCode::KeyS, KeyCode::ArrowDown], &[KeyCode::KeyW, KeyCode::ArrowUp]),
    }
}

/// The screens of the game. Enter or Space starts, P pauses, R restarts. While a screen is
/// shown the game gets the neutral input, so a menu key never moves the square.
pub fn screens() -> Screens {
    Screens::new(Labels {
        title: "별 줍기".to_string(),
        title_hint: "Enter: 시작".to_string(),
        paused: "일시정지".to_string(),
        paused_hint: "P: 계속   R: 다시 시작".to_string(),
        win: "승리!".to_string(),
        lose: "패배".to_string(),
        end_hint: "Enter 또는 R: 다시 하기".to_string(),
    })
}

pub fn app() -> App<StarPick> {
    let screens = screens();
    let fx = Shared::default();
    fx.borrow_mut().screens = Some(screens.clone());
    let for_trails = fx.clone();
    App::<StarPick>::new()
        .input(move |keys, _slot| screens.input(keys, read_keys))
        .view(Look { fx: fx.clone() })
        .entity_view::<Score>(move || TrailView { fx: for_trails.clone() })
        .scene_view(Juice { fx })
        .sound(sound::PICKUP, sound::pickup())
        .title("Star Pick 2")
}
