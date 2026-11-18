//! The builder and the window loop.

use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use orr_bridge::{Bridge, BridgeConfig, BridgeError, BridgeEvent, FrameView, InProc, PlayHost, PlayerSlot, Snapshot, Threaded, ViewUpdate};
use orr_ecs::Component;
use orr_reflect::{Scene, TypeRegistry};
use orr_render::orr_rhi::Wgpu;
use orr_render::{Camera, Camera3D, RenderList, RenderList3D, Settings3D, WindowRenderer, WindowRenderer3D};
use orr_session::{PlayConfig, PlaySession};
use orr_sim::{Game, Simulation};
use orr_view::{RenderItem, RenderItem3, ViewConfig, ViewWorld, ViewWorld3};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::cli::{Args, RunMode, USAGE};
use crate::input::InputContext;
use crate::net::{self, Hosted, Join};
use crate::pace::{Advance, Outcome, Pacer};
use crate::models::ModelScene;
use crate::shot::{render_rgba, render_rgba3, write_png};
use crate::view2d::{fill_draw_list, SharedView, View2D};
use crate::view3d::{fill_draw_list3, ModelDraw, SharedView3, View3D};
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

/// The bridge of a run: the game inside this process (offline), or a relay client on its own thread.
enum Driver<G: Game> {
    Offline(Box<Offline<G>>),
    Net(Threaded<G>),
}

impl<G: Game> Driver<G> {
    /// Runs `n` ticks now. Only an offline run can: a relay client follows the clock of the server.
    fn step(&mut self, n: u32) {
        match self {
            Driver::Offline(bridge) => bridge.step(n),
            Driver::Net(bridge) => bridge.step(n),
        }
    }
}

macro_rules! delegate {
    ($self:ident.$method:ident($($arg:expr),*)) => {
        match $self {
            Driver::Offline(bridge) => bridge.$method($($arg),*),
            Driver::Net(bridge) => bridge.$method($($arg),*),
        }
    };
}

impl<G: Game> Bridge<G> for Driver<G> {
    fn tick_rate(&self) -> u32 {
        delegate!(self.tick_rate())
    }
    fn local_slot(&self) -> PlayerSlot {
        delegate!(self.local_slot())
    }
    fn player_count(&self) -> u8 {
        delegate!(self.player_count())
    }
    fn local_slots(&self) -> Vec<PlayerSlot> {
        delegate!(self.local_slots())
    }
    fn set_input(&mut self, player: PlayerSlot, input: G::Input) -> Result<(), BridgeError> {
        delegate!(self.set_input(player, input))
    }
    fn send_command(&mut self, command: G::Command) -> Result<(), BridgeError> {
        delegate!(self.send_command(command))
    }
    fn update(&mut self, elapsed: Duration) {
        delegate!(self.update(elapsed))
    }
    fn snapshot(&self) -> Option<Snapshot> {
        delegate!(self.snapshot())
    }
    fn drain_events(&mut self) -> Vec<BridgeEvent<G::Event>> {
        delegate!(self.drain_events())
    }
    fn poll_view(&mut self) -> ViewUpdate<G::Event> {
        delegate!(self.poll_view())
    }
    fn is_alive(&self) -> bool {
        delegate!(self.is_alive())
    }
}

struct SceneSource {
    path: PathBuf,
    register: fn(&mut TypeRegistry),
}

/// Runs game `G` in a window. By default it runs offline: one or more local players on one
/// screen, no server. `--host` and `--connect` (see `--help`) play the same game on a relay
/// server instead, with one player per window; the builder code does not change.
///
/// ```text
/// App::<MyGame>::new()
///     .input(|keys, _slot| MyInput::from(keys))
///     .view(MyLook)
///     .run()
///
/// // The same game in 3D, with the default look (`Look3D`):
/// App::<MyGame>::new()
///     .input(|keys, _slot| MyInput::from(keys))
///     .view3d(Look3D::default())
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
    view3: Option<Rc<dyn View3D>>,
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
            view3: None,
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

    /// What to draw in 2D: see [`View2D`]. A run draws either in 2D or in 3D ([`App::view3d`]), not both.
    pub fn view(mut self, view: impl View2D + 'static) -> Self {
        self.view = Some(Rc::new(view));
        self
    }

    /// What to draw in 3D: see [`View3D`] and the ready-made [`crate::Look3D`]. Entity views, scene
    /// views, players, `--host`, `--connect` and `--screenshot` work as in 2D. Entity views get
    /// [`crate::EntityView::on_update3`] instead of `on_update`.
    pub fn view3d(mut self, view: impl View3D + 'static) -> Self {
        self.view3 = Some(Rc::new(view));
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
        // Offline, the app is the "server": it announces every player in tick 1.
        let mut host =
            PlayHost::new(session, LOCAL_SLOT).with_extra_local_slots((1..self.local_players).map(PlayerSlot)).with_announced_players();
        if let Some(bot) = self.bot.take() {
            host = host.with_bot(bot);
        }
        Ok(InProc::new(host, BridgeConfig::default()))
    }

    /// What a networked run needs from the builder: the number of players the room waits for
    /// (every window plays one slot, so the larger of `players` and `local_players`) and the
    /// config of the game.
    fn net_parts(&mut self) -> Result<(u8, G::Config), Error> {
        if self.scene.is_some() {
            return Err(Error("a scene file is not supported with --host or --connect yet".to_string()));
        }
        let config = self.config.take().ok_or_else(|| Error("the app was already run".to_string()))?;
        Ok((self.players.max(self.local_players), config))
    }

    /// `--host`: a relay server in this process, and a client of it for this window (slot 0).
    fn build_hosted(&mut self, bind: std::net::SocketAddr) -> Result<(Driver<G>, Hosted), Error> {
        let (players, config) = self.net_parts()?;
        let room = net::room_config::<G>(players, self.tick_rate, self.seed);
        let server = Hosted::start(bind, room, self.seed)?;
        eprintln!("hosting {players} players on {} (other windows: --connect {})", server.addr(), server.client_address());
        let join = Join { addr: server.client_address(), slot: Some(0), token: 0 };
        let joined = net::join::<G>(join, config)?;
        Ok((Driver::Net(joined.bridge), server))
    }

    /// `--connect`: this window is a client of the server at `addr`.
    fn build_joined(&mut self, addr: String, args: &Args) -> Result<Driver<G>, Error> {
        let (_, config) = self.net_parts()?;
        let join = Join { addr: addr.clone(), slot: args.slot, token: args.token };
        let joined = net::join::<G>(join, config)?;
        eprintln!(
            "playing as slot {} of {}. To take the seat again after closing this window: --connect {} --slot {} --token {}",
            joined.slot, joined.players, addr, joined.slot, joined.token
        );
        Ok(Driver::Net(joined.bridge))
    }

    fn run_window(mut self, args: Args) -> Result<(), Error> {
        let display = match (self.view.take(), self.view3.take()) {
            (Some(view), None) => Display::D2(Box::new(Display2::new(view))),
            (None, Some(view)) => Display::D3(Box::new(Display3::new(view))),
            (None, None) => return Err(Error("no view: call .view(..) for 2D or .view3d(..) for 3D before .run()".to_string())),
            (Some(_), Some(_)) => return Err(Error("both .view(..) and .view3d(..) are set: a run draws in 2D or in 3D, not both".to_string())),
        };
        let input = self.input.take().ok_or_else(|| Error("no input: call .input(..) before .run()".to_string()))?;
        let mode = args.mode()?;
        let (bridge, server) = match &mode {
            RunMode::Offline => (Driver::Offline(Box::new(self.build_bridge()?)), None),
            RunMode::Host { bind } => {
                let (bridge, server) = self.build_hosted(*bind)?;
                (bridge, Some(server))
            }
            RunMode::Connect { addr } => (self.build_joined(addr.clone(), &args)?, None),
        };
        let event_loop = EventLoop::new().map_err(|e| Error(format!("event loop: {e}")))?;
        event_loop.set_control_flow(ControlFlow::Poll);
        let networked = mode != RunMode::Offline;
        let local = if networked {
            LocalInput::networked(input, bridge.local_slots())
        } else {
            LocalInput::new(input, bridge.local_slots())
        };
        let mut runner = Runner {
            bridge,
            display,
            local,
            keys: InputContext::default(),
            pacer: if networked { Pacer::new(&args).following_clock() } else { Pacer::new(&args) },
            args,
            title: self.title,
            window: None,
            epoch: None,
            views: self.views,
            last_frame: Instant::now(),
            error: None,
            _server: server,
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
    /// The slots the bridge takes input for.
    slots: Vec<PlayerSlot>,
    /// The slot number the input function is given for each of them.
    polled: Vec<PlayerSlot>,
    sent: Vec<Option<G::Input>>,
}

impl<G: Game> LocalInput<G> {
    /// The input function gets the slots themselves.
    fn new(poll: InputFn<G>, slots: Vec<PlayerSlot>) -> Self {
        let polled = slots.clone();
        let sent = vec![None; slots.len()];
        Self { poll, slots, polled, sent }
    }

    /// A relay client: the server picked the slot, so the input function gets the number of the
    /// local player instead (0 for the one player of this window, which is the WASD layout).
    fn networked(poll: InputFn<G>, slots: Vec<PlayerSlot>) -> Self {
        let polled = (0..slots.len()).map(|i| PlayerSlot(i as u8)).collect();
        let sent = vec![None; slots.len()];
        Self { poll, slots, polled, sent }
    }

    /// Polls every local slot and sends the inputs that changed.
    fn send(&mut self, keys: &InputContext, bridge: &mut dyn Bridge<G>) {
        for ((slot, polled), sent) in self.slots.iter().zip(&self.polled).zip(&mut self.sent) {
            let input = (self.poll)(keys, *polled);
            if *sent != Some(input) {
                *sent = Some(input);
                let _ = bridge.set_input(*slot, input);
            }
        }
    }
}

/// What a run draws: the 2D look or the 3D look, each with its view world, draw list and renderer.
enum Display {
    D2(Box<Display2>),
    D3(Box<Display3>),
}

struct Display2 {
    view: SharedView,
    world: ViewWorld<SharedView>,
    /// The poses of the view world, read again each frame for the entity views.
    poses: Vec<RenderItem>,
    items: Vec<RenderItem>,
    list: RenderList,
    camera: Camera,
    renderer: Option<WindowRenderer<Wgpu>>,
}

impl Display2 {
    fn new(view: Rc<dyn View2D>) -> Self {
        let view = SharedView(view);
        Self {
            world: ViewWorld::new(view.clone(), ViewConfig::default()),
            view,
            poses: Vec::new(),
            items: Vec::new(),
            list: RenderList::new(),
            camera: Camera::new([0.0, 0.0], 1.0),
            renderer: None,
        }
    }
}

struct Display3 {
    view: SharedView3,
    world: ViewWorld3<SharedView3>,
    poses: Vec<RenderItem3>,
    items: Vec<RenderItem3>,
    list: RenderList3D,
    /// The models of the view assets that the entities of the last drawn frame ask for.
    models: Vec<ModelDraw>,
    /// Draws shapes and models together; made when the first model is drawn.
    scene: Option<ModelScene>,
    scene_failed: bool,
    camera: Option<Camera3D>,
    renderer: Option<WindowRenderer3D<Wgpu>>,
}

impl Display3 {
    fn new(view: Rc<dyn View3D>) -> Self {
        let view = SharedView3(view);
        Self {
            world: ViewWorld3::new(view.clone(), ViewConfig::default()),
            view,
            poses: Vec::new(),
            items: Vec::new(),
            list: RenderList3D::new(),
            models: Vec::new(),
            scene: None,
            scene_failed: false,
            camera: None,
            renderer: None,
        }
    }

    /// The scene that draws models with the shapes, made on first use. `None` when the frame has
    /// no models, or when the GPU state for them could not be made (said once on stderr).
    fn model_scene(&mut self) -> Option<&mut ModelScene> {
        if self.models.is_empty() || self.scene_failed {
            return None;
        }
        if self.scene.is_none() {
            let renderer = &self.renderer.as_ref()?.renderer;
            match ModelScene::new(renderer.rhi().clone(), renderer.format()) {
                Ok(scene) => self.scene = Some(scene),
                Err(error) => {
                    eprintln!("models are not drawn: {error}");
                    self.scene_failed = true;
                }
            }
        }
        self.scene.as_mut()
    }
}

impl Display {
    /// Creates the renderer of the look for `window`.
    fn open(&mut self, window: Arc<Window>, size: (u32, u32), vsync: bool) -> Result<(), String> {
        match self {
            Display::D2(d) => d.renderer = Some(WindowRenderer::new(window, size, vsync)?),
            Display::D3(d) => d.renderer = Some(WindowRenderer3D::new(window, size, vsync, Settings3D::default())?),
        }
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        match self {
            Display::D2(d) => d.renderer.iter_mut().for_each(|r| r.resize(width, height)),
            Display::D3(d) => d.renderer.iter_mut().for_each(|r| r.resize(width, height)),
        }
    }

    /// A jump of the timeline: the entities show their new place at once.
    fn reset(&mut self) {
        match self {
            Display::D2(d) => d.world.reset(),
            Display::D3(d) => d.world.reset(),
        }
    }

    /// Moves the view world `dt` seconds forward with what the bridge delivered.
    fn advance<E>(&mut self, dt: f32, update: &ViewUpdate<E>) {
        // The entity views diff the poses themselves, so nobody reads the lifecycle queue: drop it every frame.
        match self {
            Display::D2(d) => {
                d.world.update_from_bridge(dt, update);
                drop(d.world.take_lifecycle());
            }
            Display::D3(d) => {
                d.world.update_from_bridge(dt, update);
                drop(d.world.take_lifecycle());
            }
        }
    }

    /// Calls the entity views and scene views with the poses of the view world.
    fn dispatch<G: Game>(&mut self, views: &mut Views<G>, bridge: &mut dyn Bridge<G>, update: &ViewUpdate<G::Event>) {
        match self {
            Display::D2(d) => {
                d.poses.clear();
                d.world.render_items(&mut d.poses);
                views.dispatch(bridge, update, &d.poses);
            }
            Display::D3(d) => {
                d.poses.clear();
                d.world.render_items(&mut d.poses);
                views.dispatch3(bridge, update, &d.poses);
            }
        }
    }

    /// Builds the draw list of `frame` and draws it. `false` when there is no renderer yet.
    fn draw(&mut self, frame: FrameView<'_>) -> bool {
        match self {
            Display::D2(d) => {
                d.camera = fill_draw_list(&d.view, &d.world, frame, &mut d.items, &mut d.list);
                d.renderer.as_mut().map(|r| r.render(&d.list, &d.camera)).is_some()
            }
            Display::D3(d) => {
                let camera = fill_draw_list3(&d.view, &d.world, frame, &mut d.items, &mut d.list, &mut d.models);
                d.camera = Some(camera);
                if d.renderer.is_none() {
                    return false;
                }
                // Models and shapes go through one composed pass; a frame without models keeps the plain pass.
                let view = d.view.0.clone();
                let composed = d.model_scene().is_some();
                match view.view_assets().filter(|_| composed) {
                    Some(assets) => {
                        let d = &mut **d;
                        let (scene, renderer) = (d.scene.as_mut().expect("made above"), d.renderer.as_mut().expect("checked above"));
                        let (list, models) = (&d.list, &d.models);
                        renderer.render_with(false, |shapes, target, size| scene.draw(shapes, target, size, list, &camera, models, assets));
                    }
                    None => {
                        d.renderer.as_mut().expect("checked above").render(&d.list, &camera);
                    }
                }
                true
            }
        }
    }

    /// The last drawn list, drawn again into a picture of exactly `size` pixels (RGBA8).
    fn capture(&mut self, size: (u32, u32)) -> Option<Vec<u8>> {
        match self {
            Display::D2(d) => d.renderer.as_mut().map(|r| render_rgba(&mut r.renderer, &d.list, &d.camera, size)),
            Display::D3(d) => {
                let camera = d.camera?;
                d.renderer.as_ref()?;
                let view = d.view.0.clone();
                let composed = d.model_scene().is_some();
                if let Some(assets) = view.view_assets().filter(|_| composed) {
                    let d = &mut **d;
                    let (scene, renderer) = (d.scene.as_mut().expect("made above"), d.renderer.as_mut().expect("checked above"));
                    return Some(scene.capture(&mut renderer.renderer, size, &d.list, &camera, &d.models, assets));
                }
                d.renderer.as_mut().map(|r| render_rgba3(&mut r.renderer, &d.list, &camera, size))
            }
        }
    }
}

struct Runner<G: Game> {
    bridge: Driver<G>,
    display: Display,
    local: LocalInput<G>,
    keys: InputContext,
    pacer: Pacer,
    args: Args,
    title: String,
    window: Option<Arc<Window>>,
    /// The last timeline epoch shown (a change means a jump, not a movement).
    epoch: Option<u64>,
    views: Views<G>,
    last_frame: Instant,
    error: Option<String>,
    /// The relay server of `--host`. Declared last so it stops after the bridge left the room.
    _server: Option<Hosted>,
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
                self.display.reset();
            }
        }
        self.display.advance(self.pacer.view_dt(real.as_secs_f32(), self.bridge.tick_rate()), &update);

        if !self.views.is_empty() {
            self.display.dispatch(&mut self.views, &mut self.bridge, &update);
        }
        let Some(snapshot) = snapshot else { return };

        if !self.display.draw(snapshot.predicted()) {
            return;
        }

        if self.pacer.frame_done(snapshot.tick()) == Outcome::Capture {
            if let (Some(path), Some(rgba)) = (&self.args.screenshot, self.display.capture(self.args.size)) {
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
        if self.window.is_some() {
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
        match self.display.open(window.clone(), (size.width, size.height), self.args.vsync) {
            Ok(()) => {
                self.window = Some(window);
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
                self.display.resize(size.width, size.height);
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
        if let Some(window) = &self.window {
            window.request_redraw();
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
        let no_input_3d = physics().view3d(crate::Look3D::default()).run_window(Args::default()).unwrap_err();
        assert!(no_input_3d.to_string().contains(".input("), "{no_input_3d}");
    }

    #[test]
    fn a_run_cannot_draw_in_2d_and_in_3d_at_once() {
        let both = physics().input(|_, _| PhysInput::default()).view(NoLook).view3d(crate::Look3D::default()).run_window(Args::default()).unwrap_err();
        assert!(both.to_string().contains(".view3d("), "{both}");
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
