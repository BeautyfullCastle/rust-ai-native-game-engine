//! The builder and the window loop.

use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost, PlayerSlot};
use orr_ecs::Component;
use orr_reflect::{Scene, TypeRegistry};
use orr_render::orr_rhi::Wgpu;
use orr_render::{RenderList, WindowRenderer};
use orr_session::{PlayConfig, PlaySession};
use orr_sim::{Game, Simulation};
use orr_view::{RenderItem, ViewConfig, ViewWorld};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::cli::{Args, USAGE};
use crate::input::InputContext;
use crate::pace::{Advance, Outcome, Pacer};
use crate::shot::{render_rgba, write_png};
use crate::view2d::{fill_draw_list, SharedView, View2D};
use crate::views::{EntityFactory, SceneView, Views};

/// Why a run could not start or ended badly.
#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self(message)
    }
}

/// The first local player of an offline run (the others follow: slots `0..local_players`).
const LOCAL_SLOT: PlayerSlot = PlayerSlot(0);
const DEFAULT_TICK_RATE: u32 = 60;
const DEFAULT_SEED: u64 = 42;

type InputFn<G> = Box<dyn FnMut(&InputContext, PlayerSlot) -> <G as Game>::Input>;
type BotFn<G> = Box<dyn FnMut(PlayerSlot, u64) -> <G as Game>::Input>;
type Offline<G> = InProc<G, PlayHost<G>>;

struct SceneSource {
    path: PathBuf,
    register: fn(&mut TypeRegistry),
}

/// Runs game `G` in a window, offline: one or more local players on one screen, no server.
///
/// ```text
/// App::<MyGame>::new()
///     .input(|keys, _slot| MyInput::from(keys))
///     .view(MyLook)
///     .run()
///
/// // Two players on one keyboard (slot 0: WASD, slot 1: arrows):
/// App::<MyGame>::new()
///     .local_players(2)
///     .input(|keys, slot| MyInput::from(keys.stick_for_slot(slot)))
///     .view(MyLook)
///     .run()
/// ```
pub struct App<G: Game> {
    config: Option<G::Config>,
    scene: Option<SceneSource>,
    tick_rate: u32,
    players: u8,
    local_players: u8,
    seed: u64,
    title: String,
    input: Option<InputFn<G>>,
    bot: Option<BotFn<G>>,
    view: Option<Rc<dyn View2D>>,
    views: Views<G>,
}

impl<G: Game> App<G>
where
    G::Config: Default,
{
    /// A run of `G` with `G::Config::default()` as the setup argument.
    pub fn new() -> Self {
        Self::with_config(G::Config::default())
    }
}

impl<G: Game> Default for App<G>
where
    G::Config: Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> App<G> {
    /// A run of `G` that starts from `G::setup(frame, &config)`.
    pub fn with_config(config: G::Config) -> Self {
        Self {
            config: Some(config),
            scene: None,
            tick_rate: DEFAULT_TICK_RATE,
            players: 1,
            local_players: 1,
            seed: DEFAULT_SEED,
            title: "Orrery".to_string(),
            input: None,
            bot: None,
            view: None,
            views: Views::new(),
        }
    }

    /// Starts from a scene file instead of `G::setup`. `register` adds the reflected
    /// types of `G` (for example the game's `register_reflect` function).
    pub fn scene(mut self, path: impl Into<PathBuf>, register: fn(&mut TypeRegistry)) -> Self {
        self.scene = Some(SceneSource { path: path.into(), register });
        self
    }

    /// Ticks per second of the simulation (default 60). The `Game` trait has no tick rate of its own.
    pub fn tick_rate(mut self, ticks_per_second: u32) -> Self {
        self.tick_rate = ticks_per_second.max(1);
        self
    }

    /// Players in the simulation (default 1). Slots `0..local_players` are local
    /// (see [`App::local_players`]); the others get default input unless [`App::bot`] drives them.
    pub fn players(mut self, count: u8) -> Self {
        self.players = count.max(1);
        self
    }

    /// How many players share this screen (default 1): slots `0..n` are local and
    /// every frame the [`App::input`] function is called for each of them. Slots
    /// from `n` up to [`App::players`] are not local; the bot drives them. The
    /// simulation has at least `n` players, so `local_players(2)` alone makes a
    /// two-player game. A count of 0 is read as 1.
    ///
    /// [`crate::ViewContext::local_players`] and `is_local` report all of them.
    pub fn local_players(mut self, count: u8) -> Self {
        self.local_players = count.max(1);
        self
    }

    /// Seed of the simulation random numbers (default 42).
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// The window title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Turns the held keys into the input of a local player. Called every frame, once for
    /// each local slot, with that slot. `InputContext::stick_for_slot` maps slot 0 to
    /// WASD and slot 1 to the arrow keys.
    pub fn input(mut self, poll: impl FnMut(&InputContext, PlayerSlot) -> G::Input + 'static) -> Self {
        self.input = Some(Box::new(poll));
        self
    }

    /// Input of every slot that is not local: `bot(slot, tick)` runs before each tick.
    pub fn bot(mut self, bot: impl FnMut(PlayerSlot, u64) -> G::Input + 'static) -> Self {
        self.bot = Some(Box::new(bot));
        self
    }

    /// What to draw: see [`View2D`].
    pub fn view(mut self, view: impl View2D + 'static) -> Self {
        self.view = Some(Rc::new(view));
        self
    }

    /// One [`crate::EntityView`] for every entity that has component `C` in the newest
    /// predicted frame. `factory` makes a view for each new entity (`MyView::default` works).
    pub fn entity_view<C: Component>(mut self, factory: impl EntityFactory<G> + 'static) -> Self {
        self.views.entity_view::<C>(factory);
        self
    }

    /// A [`crate::SceneView`]: one object for the whole scene. Several are allowed and are called in the order added.
    pub fn scene_view(mut self, view: impl SceneView<G> + 'static) -> Self {
        self.views.scene_view(view);
        self
    }

    /// Reads the command line (see `--help`) and runs until the window closes,
    /// Escape is pressed, or the requested screenshot is saved.
    pub fn run(self) -> Result<(), Error> {
        let args = Args::from_env()?;
        if args.help {
            println!("{USAGE}");
            return Ok(());
        }
        self.run_window(args)
    }

    fn build_bridge(&mut self) -> Result<Offline<G>, Error> {
        let players = self.players.max(self.local_players);
        let cfg = PlayConfig::new(players, self.seed, self.tick_rate);
        let session = match self.scene.take() {
            Some(source) => session_from_scene::<G>(cfg, &source)?,
            None => PlaySession::<G>::new(cfg, self.config.take().ok_or_else(|| Error("the app was already run".to_string()))?),
        };
        let mut host = PlayHost::new(session, LOCAL_SLOT).with_extra_local_slots((1..self.local_players).map(PlayerSlot));
        if let Some(bot) = self.bot.take() {
            host = host.with_bot(bot);
        }
        Ok(InProc::new(host, BridgeConfig::default()))
    }

    fn run_window(mut self, args: Args) -> Result<(), Error> {
        let view = self.view.take().ok_or_else(|| Error("no view: call .view(..) before .run()".to_string()))?;
        let input = self.input.take().ok_or_else(|| Error("no input: call .input(..) before .run()".to_string()))?;
        let bridge = self.build_bridge()?;
        let event_loop = EventLoop::new().map_err(|e| Error(format!("event loop: {e}")))?;
        event_loop.set_control_flow(ControlFlow::Poll);
        let local = LocalInput::new(input, bridge.local_slots());
        let shared = SharedView(view);
        let mut runner = Runner {
            bridge,
            world: ViewWorld::new(shared.clone(), ViewConfig::default()),
            view: shared,
            local,
            keys: InputContext::default(),
            pacer: Pacer::new(&args),
            args,
            title: self.title,
            gfx: None,
            epoch: None,
            views: self.views,
            poses: Vec::new(),
            items: Vec::new(),
            list: RenderList::new(),
            last_frame: Instant::now(),
            error: None,
        };
        event_loop.run_app(&mut runner).map_err(|e| Error(format!("run: {e}")))?;
        runner.error.map_or(Ok(()), |e| Err(Error(e)))
    }
}

fn session_from_scene<G: Game>(cfg: PlayConfig, source: &SceneSource) -> Result<PlaySession<G>, Error> {
    let text = std::fs::read_to_string(&source.path).map_err(|e| Error(format!("read {}: {e}", source.path.display())))?;
    let mut types = TypeRegistry::new();
    (source.register)(&mut types);
    let mismatch = types.check_against(&Simulation::<G>::build_registry());
    if !mismatch.is_empty() {
        return Err(Error(format!("the registered types do not match the game: {mismatch:?}")));
    }
    let scene = Scene::parse(&text, &types).map_err(|e| Error(format!("{}: {e}", source.path.display())))?;
    let mut bake_error = None;
    let session = PlaySession::<G>::from_setup(cfg, |frame| {
        if let Err(e) = scene.bake(&types, frame) {
            bake_error = Some(e.to_string());
        }
    });
    match bake_error {
        Some(e) => Err(Error(format!("{}: {e}", source.path.display()))),
        None => Ok(session),
    }
}

/// The input function and what it last sent for each local slot.
struct LocalInput<G: Game> {
    poll: InputFn<G>,
    slots: Vec<PlayerSlot>,
    sent: Vec<Option<G::Input>>,
}

impl<G: Game> LocalInput<G> {
    fn new(poll: InputFn<G>, slots: Vec<PlayerSlot>) -> Self {
        let sent = vec![None; slots.len()];
        Self { poll, slots, sent }
    }

    /// Polls every local slot and sends the inputs that changed.
    fn send(&mut self, keys: &InputContext, bridge: &mut dyn Bridge<G>) {
        for (slot, sent) in self.slots.iter().zip(&mut self.sent) {
            let input = (self.poll)(keys, *slot);
            if *sent != Some(input) {
                *sent = Some(input);
                let _ = bridge.set_input(*slot, input);
            }
        }
    }
}

struct Gfx {
    window: Arc<Window>,
    renderer: WindowRenderer<Wgpu>,
}

struct Runner<G: Game> {
    bridge: Offline<G>,
    world: ViewWorld<SharedView>,
    view: SharedView,
    local: LocalInput<G>,
    keys: InputContext,
    pacer: Pacer,
    args: Args,
    title: String,
    gfx: Option<Gfx>,
    /// The last timeline epoch shown (a change means a jump, not a movement).
    epoch: Option<u64>,
    views: Views<G>,
    /// The poses of the view world, read again each frame for the entity views.
    poses: Vec<RenderItem>,
    items: Vec<RenderItem>,
    list: RenderList,
    last_frame: Instant,
    error: Option<String>,
}

impl<G: Game> Runner<G> {
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let real = now - self.last_frame;
        self.last_frame = now;

        self.local.send(&self.keys, &mut self.bridge);

        let head = self.bridge.snapshot().map_or(0, |s| s.tick());
        match self.pacer.advance(head) {
            Advance::Realtime => self.bridge.update(real),
            Advance::Steps(n) => self.bridge.step(n),
            Advance::Frozen => {}
        }

        let update = self.bridge.poll_view();
        let snapshot = update.snapshot.clone();
        if let Some(epoch) = snapshot.as_ref().map(|s| s.timeline().map(|t| t.epoch)) {
            if epoch != self.epoch {
                self.epoch = epoch;
                self.world.reset();
            }
        }
        self.world.update_from_bridge(self.pacer.view_dt(real.as_secs_f32(), self.bridge.tick_rate()), &update);
        // The entity views diff the poses themselves, so nobody reads this queue: drop it every frame.
        drop(self.world.take_lifecycle());

        if !self.views.is_empty() {
            self.poses.clear();
            self.world.render_items(&mut self.poses);
            self.views.dispatch(&mut self.bridge, &update, &self.poses);
        }
        let Some(snapshot) = snapshot else { return };

        let camera = fill_draw_list(&self.view, &self.world, snapshot.predicted(), &mut self.items, &mut self.list);
        let Some(gfx) = &mut self.gfx else { return };
        gfx.renderer.render(&self.list, &camera);

        if self.pacer.frame_done(snapshot.tick()) == Outcome::Capture {
            if let Some(path) = &self.args.screenshot {
                let rgba = render_rgba(&mut gfx.renderer.renderer, &self.list, &camera, self.args.size);
                if let Err(e) = write_png(path, self.args.size, &rgba) {
                    self.error = Some(e);
                }
            }
            event_loop.exit();
        }
    }

    fn key(&mut self, code: KeyCode, down: bool, event_loop: &ActiveEventLoop) {
        if code == KeyCode::Escape && down {
            event_loop.exit();
        } else {
            self.keys.set(code, down);
        }
    }
}

impl<G: Game> ApplicationHandler for Runner<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let (w, h) = self.args.size;
        let attributes = Window::default_attributes().with_title(self.title.as_str()).with_inner_size(LogicalSize::new(w, h));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(format!("create_window: {e}"));
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        match WindowRenderer::new(window.clone(), (size.width, size.height), self.args.vsync) {
            Ok(renderer) => {
                self.gfx = Some(Gfx { window, renderer });
                self.last_frame = Instant::now();
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gfx) = &mut self.gfx {
                    gfx.renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(false) => self.keys.release_all(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.key(code, event.state == ElementState::Pressed, event_loop);
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(gfx) = &self.gfx {
            gfx.window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use orr_games::physics_game::{register_reflect, PhysConfig, PhysGame, PhysInput, SceneMode, TICK_RATE};

    use super::*;

    const DEMO_SCENE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/physics_demo.scene.yaml");

    fn physics() -> App<PhysGame> {
        App::with_config(PhysConfig::new(40, SceneMode::Rain)).tick_rate(TICK_RATE).players(2)
    }

    #[test]
    fn the_offline_bridge_steps_and_asks_the_bot_once_per_tick_for_every_other_slot() {
        let asked = Rc::new(Cell::new(0u64));
        let counter = asked.clone();
        let mut bridge = physics()
            .bot(move |slot, tick| {
                assert_eq!(slot, PlayerSlot(1));
                counter.set(counter.get() + 1);
                PhysInput::new(1, 0, 0, tick % 2 == 0)
            })
            .build_bridge()
            .unwrap();

        bridge.step(30);

        assert_eq!(bridge.snapshot().unwrap().tick(), 30);
        assert_eq!(bridge.tick_rate(), TICK_RATE);
        assert_eq!(bridge.player_count(), 2);
        assert_eq!(bridge.local_slot(), PlayerSlot(0));
        assert_eq!(asked.get(), 30);
    }

    #[test]
    fn local_players_are_polled_each_frame_and_the_bot_gets_only_the_other_slots() {
        let asked = Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = asked.clone();
        let mut bridge = physics()
            .players(3)
            .local_players(2)
            .bot(move |slot, _tick| {
                log.borrow_mut().push(slot);
                PhysInput::default()
            })
            .build_bridge()
            .unwrap();
        assert_eq!(bridge.player_count(), 3);
        assert_eq!(bridge.local_slots(), vec![PlayerSlot(0), PlayerSlot(1)]);

        let polled = Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = polled.clone();
        let mut local = LocalInput::<PhysGame>::new(
            Box::new(move |keys, slot| {
                seen.borrow_mut().push(slot);
                let stick = keys.stick_for_slot(slot);
                PhysInput::new(stick.x, stick.y, 0, stick.action)
            }),
            bridge.local_slots(),
        );
        let mut keys = InputContext::default();
        keys.set(KeyCode::KeyD, true);
        keys.set(KeyCode::ArrowUp, true);
        local.send(&keys, &mut bridge);
        local.send(&keys, &mut bridge);
        assert_eq!(*polled.borrow(), vec![PlayerSlot(0), PlayerSlot(1), PlayerSlot(0), PlayerSlot(1)]);
        assert_eq!(local.sent, vec![Some(PhysInput::new(1, 0, 0, false)), Some(PhysInput::new(0, 1, 0, false))]);

        bridge.step(5);
        assert_eq!(*asked.borrow(), vec![PlayerSlot(2); 5]);
    }

    #[test]
    fn local_players_raise_the_player_count_and_the_default_is_one() {
        let bridge = physics().players(1).local_players(2).build_bridge().unwrap();
        assert_eq!((bridge.player_count(), bridge.local_slots()), (2, vec![PlayerSlot(0), PlayerSlot(1)]));
        let bridge = physics().build_bridge().unwrap();
        assert_eq!(bridge.local_slots(), vec![PlayerSlot(0)]);
    }

    #[test]
    fn two_runs_of_the_same_app_reach_the_same_checksum() {
        let checksum = || {
            let mut bridge = physics().build_bridge().unwrap();
            bridge.step(45);
            bridge.snapshot().unwrap().predicted().checksum()
        };
        assert_eq!(checksum(), checksum());
    }

    #[test]
    fn a_scene_file_replaces_the_game_setup() {
        let mut types = TypeRegistry::new();
        register_reflect(&mut types);
        let scene = Scene::parse(&std::fs::read_to_string(DEMO_SCENE).unwrap(), &types).unwrap();

        let mut bridge = physics().scene(DEMO_SCENE, register_reflect).build_bridge().unwrap();
        bridge.step(1);

        let snapshot = bridge.snapshot().unwrap();
        assert_eq!(snapshot.predicted().alive_count() as usize, scene.entities.len());
    }

    #[test]
    fn scene_problems_are_errors_that_name_the_problem() {
        let missing = physics().scene("no/such.scene.yaml", register_reflect).build_bridge().err().unwrap();
        assert!(missing.to_string().contains("no/such.scene.yaml"), "{missing}");

        fn register_nothing(_: &mut TypeRegistry) {}
        let unregistered = physics().scene(DEMO_SCENE, register_nothing).build_bridge().err().unwrap();
        assert!(unregistered.to_string().contains("unknown component"), "{unregistered}");
    }

    #[test]
    fn a_run_without_a_view_or_input_is_refused_before_a_window_opens() {
        let no_view = physics().input(|_, _| PhysInput::default()).run_window(Args::default()).unwrap_err();
        assert!(no_view.to_string().contains(".view("), "{no_view}");
        let no_input = physics().view(NoLook).run_window(Args::default()).unwrap_err();
        assert!(no_input.to_string().contains(".input("), "{no_input}");
    }

    struct NoLook;

    impl orr_view::Extractor for NoLook {
        fn extract(&self, _frame: orr_bridge::FrameView<'_>, _out: &mut Vec<orr_view::Extracted>) {}
    }

    impl View2D for NoLook {
        fn camera(&self, _frame: orr_bridge::FrameView<'_>) -> orr_render::Camera {
            orr_render::Camera::new([0.0, 0.0], 1.0)
        }
    }
}
