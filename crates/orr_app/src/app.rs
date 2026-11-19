//! The builder and the window loop.

use std::cell::RefCell;
use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use orr_audio::Clip;
use orr_bridge::{Bridge, BridgeConfig, BridgeError, BridgeEvent, FrameView, InProc, PlayHost, PlayerSlot, Snapshot, Threaded, ViewUpdate};
use orr_ecs::Component;
use orr_reflect::{Scene, TypeRegistry};
use orr_render::orr_rhi::Wgpu;
use orr_render::{Camera, Camera3D, RenderList, RenderList3D, Settings3D, WindowRenderer, WindowRenderer3D};
use orr_session::{PlayConfig, PlaySession};
use orr_sim::{Game, Players, Simulation};
use orr_view::{RenderItem, RenderItem3, ViewConfig, ViewWorld, ViewWorld3};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::cli::{Args, RunMode, USAGE};
use crate::font::{system_font, system_font_candidates, Font, FontHandle};
use crate::overlay::{Overlay, TextPass};
use crate::input::{Device, InputContext};
use crate::net::{self, Hosted, Join};
use crate::pace::{Advance, Outcome, Pacer};
use crate::models::ModelScene;
use crate::save::{LoadedSave, SaveError, SaveStore};
use crate::wav::{clip_from_wav, MAX_WAV_BYTES};
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

/// Which font draws the overlay text.
enum FontChoice {
    /// The first system font with Hangul, else the 5x7 bitmap font.
    System,
    /// Always the 5x7 bitmap font.
    Bitmap,
    Bytes(String, Vec<u8>),
    File(PathBuf),
}

struct SceneSource {
    path: PathBuf,
    register: fn(&mut TypeRegistry),
}

/// Where the bytes of a WAV sound come from (see [`App::sound_file`] and [`App::sound_wav`]).
enum WavSource {
    File(PathBuf),
    Bytes(Vec<u8>),
}

/// What [`App::saves`] asked for.
struct SaveSetup {
    game: String,
    version: u32,
    folder: Option<PathBuf>,
}

/// What a key does for the saves of an offline run (see [`App::save_key`], [`App::load_key`]).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Hot {
    Save(String),
    Load(String),
}

/// A bot that the host of every offline session (also one that a load starts) calls.
type SharedBot<G> = Rc<RefCell<BotFn<G>>>;

/// What an offline run needs to start a session again, from a save.
struct Parts<G: Game> {
    players: u8,
    local_players: u8,
    seed: u64,
    tick_rate: u32,
    bot: Option<SharedBot<G>>,
}

impl<G: Game> Parts<G> {
    /// The offline bridge of `session`. `announce` is the start data of the local players when the
    /// session is new and must announce its players; a session that comes from a save has them in its frame.
    fn bridge(&self, session: PlaySession<G>, announce: Option<Vec<u8>>) -> Offline<G> {
        let mut host = PlayHost::new(session, LOCAL_SLOT).with_extra_local_slots((1..self.local_players).map(PlayerSlot));
        if let Some(data) = announce {
            let local_players = self.local_players;
            host = host.with_announced_players_data(move |slot| if slot.0 < local_players { data.clone() } else { Vec::new() });
        }
        if let Some(bot) = &self.bot {
            let bot = bot.clone();
            host = host.with_bot(move |slot, tick| (bot.borrow_mut())(slot, tick));
        }
        InProc::new(host, BridgeConfig::default())
    }

    /// A new offline bridge that continues from `save`.
    fn bridge_from_save(&self, save: &LoadedSave) -> Result<Offline<G>, SaveError> {
        Ok(self.bridge(save.into_session::<G>(self.seed)?, None))
    }
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
    /// The start data of the local players, as bytes (see [`App::player_data`]); empty: the default.
    player_data: Vec<u8>,
    /// Devices given to slots by hand (see [`App::assign_device`]).
    devices: Vec<(PlayerSlot, Device)>,
    font: FontChoice,
    /// WAV sounds that are read when the run starts.
    wavs: Vec<(String, WavSource)>,
    saves: Option<SaveSetup>,
    hotkeys: Vec<(KeyCode, Hot)>,
    /// What the offline bridge was built from (set by `build_bridge`).
    parts: Option<Parts<G>>,
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
            player_data: Vec::new(),
            devices: Vec::new(),
            font: FontChoice::System,
            wavs: Vec::new(),
            saves: None,
            hotkeys: Vec::new(),
            parts: None,
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

    /// The start data of the local players, for a game with [`Players`]: the `data` of the `Added`
    /// signal that creates their avatars (a color, a name, a character). Offline, every local
    /// player gets it and the other players get the default value. With `--host` and `--connect`,
    /// the window sends it when it joins and the server gives it to every window, in the same
    /// `Added` signal. Without this call the start data is `G::Player::default()`.
    ///
    /// `Players::Player` must be at most `orr_proto::MAX_PLAYER_DATA` (256) bytes; the run
    /// stops with an error before it starts otherwise.
    pub fn player_data(mut self, value: G::Player) -> Self
    where
        G: Players,
    {
        self.player_data = bytemuck::bytes_of(&value).to_vec();
        self
    }

    /// Plays `slot` with `device` for `InputContext::stick_of`, instead of the default
    /// (slot 0 keyboard, slot 1 first pad, slot 2 second pad).
    pub fn assign_device(mut self, slot: PlayerSlot, device: Device) -> Self {
        self.devices.push((slot, device));
        self
    }

    /// The font of the overlay text (`ctx.draw().text(..)`, `text_box(..)`, the [`crate::Screens`]):
    /// the bytes of a `.ttf`, `.otf` or `.ttc` file, which the game embeds with `include_bytes!`.
    /// `name` appears in messages. Without this call or [`App::font_file`] the run uses the system
    /// font with Hangul (on Windows `malgun.ttf`), and the 5x7 bitmap font (ASCII only) when
    /// there is none. A font that cannot be read stops the run with an error.
    pub fn font(mut self, name: &str, bytes: impl Into<Vec<u8>>) -> Self {
        self.font = FontChoice::Bytes(name.to_string(), bytes.into());
        self
    }

    /// The same with a font file that is read when the run starts.
    pub fn font_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.font = FontChoice::File(path.into());
        self
    }

    /// Draws the overlay text with the 5x7 bitmap font (ASCII capitals, digits, a few signs)
    /// even when a system font exists: the same picture on every machine.
    pub fn bitmap_font(mut self) -> Self {
        self.font = FontChoice::Bitmap;
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
    /// WASD and slot 1 to the arrow keys. `InputContext::stick_of` follows the controllers too:
    /// slot 0 the keyboard, slot 1 the first pad, slot 2 the second pad, and so on.
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

    /// Registers a sound under `name` for `ctx.audio().play_named(name)` in the views. The clip is made
    /// once, here (generate its samples with `Clip::from_stereo`). The run opens the system audio
    /// device when the first sound plays; without a device the run is silent (said once on stderr),
    /// and a `--screenshot` run never opens one.
    pub fn sound(mut self, name: &str, clip: Clip) -> Self {
        self.views.sound(name, clip);
        self
    }

    /// Registers the WAV file `path` under `name` for `ctx.audio().play_named(name)` and for
    /// [`App::on_event_sound`]. 16-bit PCM or 32-bit float, mono or stereo, 8000 to 192000 Hz,
    /// at most 10 seconds (see [`crate::clip_from_wav`]); the mixer resamples it to the output rate.
    /// The file is read when the run starts (a relative path is relative to the working directory).
    /// A file that is missing or refused stops the run with an error that names the sound and the file.
    pub fn sound_file(mut self, name: &str, path: impl Into<PathBuf>) -> Self {
        self.wavs.push((name.to_string(), WavSource::File(path.into())));
        self
    }

    /// Like [`App::sound_file`] with the bytes of a WAV file, which the game embeds with
    /// `include_bytes!`: the run needs no file next to the program.
    pub fn sound_wav(mut self, name: &str, bytes: impl Into<Vec<u8>>) -> Self {
        self.wavs.push((name.to_string(), WavSource::Bytes(bytes.into())));
        self
    }

    /// Plays the sound that `name_of` names (a sound of [`App::sound`], [`App::sound_file`] or
    /// [`App::sound_wav`]) for each event of the game: `|event| Some("pickup")`. One place does it all:
    /// a `Predicted` event starts its sound, a `Canceled` event stops it, an event that a rollback keeps
    /// sounds once, and a resync stops the sounds that were not confirmed (see [`Views::on_event_sound`]).
    /// A view then needs no sound code of its own.
    pub fn on_event_sound<S: Into<String>>(mut self, name_of: impl Fn(&G::Event) -> Option<S> + 'static) -> Self {
        self.views.on_event_sound(name_of);
        self
    }

    /// Gives an offline run save slots for `game`, a stable id of 1 to 64 letters, digits, `-` or `_`
    /// (it names the folder: `%LOCALAPPDATA%\orrery\games\<game>\saves` on Windows, see
    /// [`SaveStore::default_dir`]), and `version`, a number of yours that you raise when old saves no longer fit
    /// the game. Views save and load through `ctx.save_game(slot, label)` and `ctx.load_game(slot)`;
    /// [`App::save_key`], [`App::load_key`] and [`App::checkpoint_on`] do it without view code. A save is the
    /// verified frame of the simulation; a load starts a new offline session from it (the checksum is the saved one).
    /// A `--host` or `--connect` run has no slots: networked games do not load saves.
    pub fn saves(mut self, game: &str, version: u32) -> Self {
        self.saves = Some(SaveSetup { game: game.to_string(), version, folder: None });
        self
    }

    /// The same save slots in `folder` instead of the per-user data folder. Call it after [`App::saves`].
    pub fn save_folder(mut self, folder: impl Into<PathBuf>) -> Self {
        if let Some(setup) = &mut self.saves {
            setup.folder = Some(folder.into());
        }
        self
    }

    /// Pressing `key` saves the verified frame into `slot` (needs [`App::saves`]; the result is
    /// `ctx.save_notice()`). A key that is held does not repeat the save.
    pub fn save_key(mut self, key: KeyCode, slot: &str) -> Self {
        self.hotkeys.push((key, Hot::Save(slot.to_string())));
        self
    }

    /// Pressing `key` continues from the save in `slot` (needs [`App::saves`]).
    pub fn load_key(mut self, key: KeyCode, slot: &str) -> Self {
        self.hotkeys.push((key, Hot::Load(slot.to_string())));
        self
    }

    /// A checkpoint: whenever a verified event gets `Some(label)` from `label_of`, the run saves into
    /// `slot` (needs [`App::saves`]). A game that wants a checkpoint emits an event and names it here.
    pub fn checkpoint_on(mut self, slot: &str, label_of: impl Fn(&G::Event) -> Option<String> + 'static) -> Self {
        self.views.checkpoint_on(slot, label_of);
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

    /// The font of the run (see [`App::font`]); `None` for the bitmap font.
    fn resolve_font(&mut self) -> Result<Option<FontHandle>, Error> {
        match std::mem::replace(&mut self.font, FontChoice::System) {
            FontChoice::Bitmap => Ok(None),
            FontChoice::Bytes(name, bytes) => Font::from_bytes(&name, bytes).map(|f| Some(FontHandle::new(f))).map_err(|e| Error(e.to_string())),
            FontChoice::File(path) => Font::from_file(&path).map(|f| Some(FontHandle::new(f))).map_err(|e| Error(e.to_string())),
            FontChoice::System => {
                let font = system_font();
                if font.is_none() {
                    let tried: Vec<String> = system_font_candidates().iter().map(|p| p.display().to_string()).collect();
                    eprintln!("overlay text: no system font was found ({}); drawing with the 5x7 bitmap font (ASCII only). Give a font with App::font or App::font_file", tried.join(", "));
                }
                Ok(font)
            }
        }
    }

    /// Reads the WAV sounds and registers their clips. The first file that fails stops the run.
    fn resolve_sounds(&mut self) -> Result<(), Error> {
        for (name, source) in std::mem::take(&mut self.wavs) {
            let (bytes, origin) = match source {
                WavSource::Bytes(bytes) => (bytes, "embedded bytes".to_string()),
                WavSource::File(path) => {
                    let origin = path.display().to_string();
                    let file = std::fs::File::open(&path).map_err(|e| Error(format!("sound {name:?}: cannot open {origin}: {e}")))?;
                    let mut bytes = Vec::new();
                    file.take(MAX_WAV_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| Error(format!("sound {name:?}: cannot read {origin}: {e}")))?;
                    (bytes, origin)
                }
            };
            let clip = clip_from_wav(&bytes).map_err(|e| Error(format!("sound {name:?} ({origin}): {e}")))?;
            self.views.sound(&name, clip);
        }
        Ok(())
    }

    /// The save slots of an offline run (see [`App::saves`]). Errors name what is wrong.
    fn resolve_saves(&mut self, parts: &Parts<G>) -> Result<(), Error> {
        let Some(setup) = self.saves.take() else { return Ok(()) };
        let store = match setup.folder {
            Some(folder) => SaveStore::at(folder, &setup.game, setup.version),
            None => SaveStore::new(&setup.game, setup.version),
        }
        .map_err(|e| Error(format!("save slots: {e}")))?;
        self.views.enable_saves(store, parts.players, parts.tick_rate);
        Ok(())
    }

    fn build_bridge(&mut self) -> Result<Offline<G>, Error> {
        self.check_player_data()?;
        let players = self.players.max(self.local_players);
        let cfg = PlayConfig::new(players, self.seed, self.tick_rate);
        let session = match self.scene.take() {
            Some(source) => session_from_scene::<G>(cfg, &source)?,
            None => PlaySession::<G>::new(cfg, self.config.take().ok_or_else(|| Error("the app was already run".to_string()))?),
        };
        // Offline, the app is the "server": it announces every player in tick 1.
        let data = std::mem::take(&mut self.player_data);
        let parts = Parts {
            players,
            local_players: self.local_players,
            seed: self.seed,
            tick_rate: self.tick_rate,
            bot: self.bot.take().map(|bot| Rc::new(RefCell::new(bot))),
        };
        let bridge = parts.bridge(session, Some(data));
        self.parts = Some(parts);
        Ok(bridge)
    }

    /// The start data must fit a `Hello`, also offline, so that a game runs the same way in both modes.
    fn check_player_data(&self) -> Result<(), Error> {
        let (len, max) = (self.player_data.len(), orr_proto::MAX_PLAYER_DATA);
        if len > max {
            return Err(Error(format!("the player data is {len} bytes, more than the {max} bytes a server takes")));
        }
        Ok(())
    }

    /// What a networked run needs from the builder: the number of players the room waits for
    /// (every window plays one slot, so the larger of `players` and `local_players`) and the
    /// config of the game.
    fn net_parts(&mut self) -> Result<(u8, G::Config), Error> {
        self.check_player_data()?;
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
        let join = Join { addr: server.client_address(), slot: Some(0), token: 0, player_data: std::mem::take(&mut self.player_data) };
        let joined = net::join::<G>(join, config)?;
        Ok((Driver::Net(joined.bridge), server))
    }

    /// `--connect`: this window is a client of the server at `addr`.
    fn build_joined(&mut self, addr: String, args: &Args) -> Result<Driver<G>, Error> {
        let (_, config) = self.net_parts()?;
        let join = Join { addr: addr.clone(), slot: args.slot, token: args.token, player_data: std::mem::take(&mut self.player_data) };
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
        let font = self.resolve_font()?;
        self.views.set_font(font);
        self.resolve_sounds()?;
        let mode = args.mode()?;
        let (bridge, server) = match &mode {
            RunMode::Offline => (Driver::Offline(Box::new(self.build_bridge()?)), None),
            RunMode::Host { bind } => {
                let (bridge, server) = self.build_hosted(*bind)?;
                (bridge, Some(server))
            }
            RunMode::Connect { addr } => (self.build_joined(addr.clone(), &args)?, None),
        };
        let parts = self.parts.take();
        match &parts {
            Some(parts) => self.resolve_saves(parts)?,
            None if self.saves.is_some() => eprintln!("save slots: a networked run has none (networked games do not load saves)"),
            None => {}
        }
        let event_loop = EventLoop::new().map_err(|e| Error(format!("event loop: {e}")))?;
        event_loop.set_control_flow(ControlFlow::Poll);
        let networked = mode != RunMode::Offline;
        let local = if networked {
            LocalInput::networked(input, bridge.local_slots())
        } else {
            LocalInput::new(input, bridge.local_slots())
        };
        if args.screenshot.is_some() {
            self.views.mute_sound();
        } else {
            self.views.use_sound_device();
        }
        let mut keys = InputContext::default();
        for (slot, device) in &self.devices {
            keys.assign(*slot, *device);
        }
        let mut runner = Runner {
            bridge,
            display,
            local,
            keys,
            #[cfg(feature = "gamepad")]
            pads: crate::gamepad::Pads::open(),
            pacer: if networked { Pacer::new(&args).following_clock() } else { Pacer::new(&args) },
            args,
            title: self.title,
            window: None,
            epoch: None,
            views: self.views,
            parts,
            hotkeys: if networked { Vec::new() } else { std::mem::take(&mut self.hotkeys) },
            hot: Vec::new(),
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

    /// The new bridge knows no input yet: send every input again.
    fn forget(&mut self) {
        self.sent.iter_mut().for_each(|sent| *sent = None);
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
    /// Draws the text of the overlay over the picture; made with the renderer.
    text: Option<TextPass>,
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
            text: None,
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
    text: Option<TextPass>,
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
            text: None,
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
            Display::D2(d) => {
                let renderer = WindowRenderer::new(window, size, vsync)?;
                d.text = Some(TextPass::new(renderer.renderer.rhi().clone(), renderer.renderer.format()));
                d.renderer = Some(renderer);
            }
            Display::D3(d) => {
                let renderer = WindowRenderer3D::new(window, size, vsync, Settings3D::default())?;
                d.text = Some(TextPass::new(renderer.renderer.rhi().clone(), renderer.renderer.format()));
                d.renderer = Some(renderer);
            }
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

    /// Builds the draw list of `frame`, adds the world items of the views' `overlay` to it, draws
    /// it and then the overlay text on top. `false` when there is no renderer yet.
    fn draw(&mut self, frame: FrameView<'_>, overlay: &Overlay) -> bool {
        match self {
            Display::D2(d) => {
                d.camera = fill_draw_list(&d.view, &d.world, frame, &mut d.items, &mut d.list);
                overlay.add_to_list(&mut d.list);
                let d = &mut **d;
                let (Some(renderer), Some(text)) = (d.renderer.as_mut(), d.text.as_mut()) else { return false };
                renderer.render_with_overlay(&d.list, &d.camera, |_, view, size| text.draw(view, size, overlay));
                true
            }
            Display::D3(d) => {
                let camera = fill_draw_list3(&d.view, &d.world, frame, &mut d.items, &mut d.list, &mut d.models);
                overlay.add_to_list3(&mut d.list);
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
                        let text = d.text.as_mut().expect("made with the renderer");
                        renderer.render_with(false, |_, target, size| {
                            scene.draw(target, size, list, &camera, models, assets);
                            text.draw(target, size, overlay);
                        });
                    }
                    None => {
                        let d = &mut **d;
                        let (renderer, text) = (d.renderer.as_mut().expect("checked above"), d.text.as_mut().expect("made with the renderer"));
                        let list = &d.list;
                        renderer.render_with(false, |renderer3, target, size| {
                            renderer3.draw(target, size, list, &camera);
                            text.draw(target, size, overlay);
                        });
                    }
                }
                true
            }
        }
    }

    /// The last drawn list and overlay, drawn again into a picture of exactly `size` pixels (RGBA8).
    fn capture(&mut self, size: (u32, u32), overlay: &Overlay) -> Option<Vec<u8>> {
        match self {
            Display::D2(d) => {
                let d = &mut **d;
                let (renderer, text) = (d.renderer.as_mut()?, d.text.as_mut()?);
                Some(render_rgba(&mut renderer.renderer, text, overlay, &d.list, &d.camera, size))
            }
            Display::D3(d) => {
                let camera = d.camera?;
                d.renderer.as_ref()?;
                let view = d.view.0.clone();
                let composed = d.model_scene().is_some();
                if let Some(assets) = view.view_assets().filter(|_| composed) {
                    let d = &mut **d;
                    let scene = d.scene.as_mut().expect("made above");
                    let text = d.text.as_mut()?;
                    return Some(scene.capture(size, &d.list, &camera, &d.models, assets, |view, size| text.draw(view, size, overlay)));
                }
                let d = &mut **d;
                let (renderer, text) = (d.renderer.as_mut()?, d.text.as_mut()?);
                Some(render_rgba3(&mut renderer.renderer, text, overlay, &d.list, &camera, size))
            }
        }
    }
}

struct Runner<G: Game> {
    bridge: Driver<G>,
    display: Display,
    local: LocalInput<G>,
    keys: InputContext,
    /// The controllers; their events go to `keys` before every frame.
    #[cfg(feature = "gamepad")]
    pads: crate::gamepad::Pads,
    pacer: Pacer,
    args: Args,
    title: String,
    window: Option<Arc<Window>>,
    /// The last timeline epoch shown (a change means a jump, not a movement).
    epoch: Option<u64>,
    views: Views<G>,
    /// What a load needs to start a session again; `None` in a networked run.
    parts: Option<Parts<G>>,
    hotkeys: Vec<(KeyCode, Hot)>,
    /// The hotkeys pressed since the last frame.
    hot: Vec<Hot>,
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

        #[cfg(feature = "gamepad")]
        {
            let mut events = Vec::new();
            self.pads.poll(&mut events);
            for event in events {
                self.keys.apply_pad(event);
            }
        }
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

        let screen = self.screen_size();
        self.views.set_screen_size(screen);
        if !self.views.is_empty() {
            self.display.dispatch(&mut self.views, &mut self.bridge, &update);
        }
        self.run_hotkeys();
        self.apply_load();
        let Some(snapshot) = snapshot else { return };

        self.views.poll_audio();
        if !self.display.draw(snapshot.predicted(), self.views.overlay()) {
            return;
        }

        if self.pacer.frame_done(snapshot.tick()) == Outcome::Capture {
            if let (Some(path), Some(rgba)) = (&self.args.screenshot, self.display.capture(self.args.size, self.views.overlay())) {
                if let Err(e) = write_png(path, self.args.size, &rgba) {
                    self.error = Some(e);
                }
            }
            event_loop.exit();
        }
    }

    /// Does the saves and loads that keys asked for since the last frame.
    fn run_hotkeys(&mut self) {
        for hot in std::mem::take(&mut self.hot) {
            // The result is the notice that the views can show; a failed one changes nothing else.
            let _ = match hot {
                Hot::Save(slot) => self.views.save_game(&slot, "").map(drop),
                Hot::Load(slot) => self.views.load_game(&slot).map(drop),
            };
        }
    }

    /// A load that was accepted this frame: a new offline session replaces the old one.
    fn apply_load(&mut self) {
        let Some(save) = self.views.take_load() else { return };
        let Some(parts) = &self.parts else { return };
        match parts.bridge_from_save(&save) {
            Ok(bridge) => {
                self.bridge = Driver::Offline(Box::new(bridge));
                self.epoch = None;
                self.display.reset();
                self.local.forget();
                self.views.session_replaced();
            }
            Err(error) => self.views.report_save_failure(error),
        }
    }

    /// The size in pixels that the overlay lays its text out for: the picture of `--screenshot`,
    /// else the window.
    fn screen_size(&self) -> [f32; 2] {
        let (w, h) = match (&self.args.screenshot, &self.window) {
            (None, Some(window)) => {
                let size = window.inner_size();
                (size.width, size.height)
            }
            _ => self.args.size,
        };
        [w as f32, h as f32]
    }

    fn key(&mut self, code: KeyCode, down: bool, repeat: bool, event_loop: &ActiveEventLoop) {
        if code == KeyCode::Escape && down {
            event_loop.exit();
        } else {
            if down && !repeat {
                self.hot.extend(self.hotkeys.iter().filter(|(key, _)| *key == code).map(|(_, hot)| hot.clone()));
            }
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
                    self.key(code, event.state == ElementState::Pressed, event.repeat, event_loop);
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
    use orr_testgame::roster::{Avatar, Profile, Roster};

    use super::*;
    use crate::screens::{Action, Labels, Screens};

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
    fn the_player_data_of_the_app_is_the_start_data_of_every_local_player_and_not_of_the_others() {
        let mut bridge = App::<Roster>::new().players(3).local_players(2).player_data(Profile { color: 5, _pad: 0 }).build_bridge().unwrap();
        bridge.step(3);
        let snapshot = bridge.snapshot().unwrap();
        let mut avatars: Vec<(u32, u32)> = snapshot.predicted().dense::<Avatar>().1.iter().map(|a| (a.slot, a.color)).collect();
        avatars.sort_unstable();
        assert_eq!(avatars, [(0, 5), (1, 5), (2, 0)]);
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

    /// Runs 60 frames with W and D held. The input function is `poll`; `before` runs first in
    /// each frame with the frame number.
    fn held_run(mut poll: impl FnMut(&InputContext, PlayerSlot) -> PhysInput + 'static, mut before: impl FnMut(u32)) -> u64 {
        let mut bridge = physics().players(1).build_bridge().unwrap();
        let mut local = LocalInput::<PhysGame>::new(Box::new(move |keys, slot| poll(keys, slot)), bridge.local_slots());
        let mut keys = InputContext::default();
        keys.set(KeyCode::KeyD, true);
        keys.set(KeyCode::KeyW, true);
        for frame in 0..60 {
            before(frame);
            local.send(&keys, &mut bridge);
            bridge.step(1);
        }
        bridge.snapshot().unwrap().predicted().checksum()
    }

    fn walk(keys: &InputContext) -> PhysInput {
        let stick = keys.stick_for_slot(PlayerSlot(0));
        PhysInput::new(stick.x, stick.y, 0, stick.action)
    }

    #[test]
    fn a_screen_sends_the_neutral_input_to_the_simulation_and_the_checksum_proves_it() {
        // Frames 20..40 show the pause screen while the keys stay held.
        let screens = Screens::playing(Labels::default());
        let (for_input, for_frames) = (screens.clone(), screens.clone());
        let paused_run = held_run(
            move |keys, _| for_input.input(keys, walk),
            move |frame| {
                if frame == 20 || frame == 40 {
                    assert!(for_frames.act(Action::Pause));
                }
            },
        );

        // The same frames with the neutral input written by hand, and no screens.
        let frame = Rc::new(Cell::new(0u32));
        let counted = frame.clone();
        let neutral_run = held_run(
            move |keys, _| if (20..40).contains(&counted.get()) { PhysInput::default() } else { walk(keys) },
            move |n| frame.set(n),
        );
        assert_eq!(paused_run, neutral_run, "the simulation saw neutral input while the screen was shown");

        // The same frames with live input all the way: the keys do move the player.
        let live_run = held_run(|keys, _| walk(keys), |_| {});
        assert_ne!(paused_run, live_run, "the held keys would have changed the run");
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

    #[test]
    fn the_font_choices_resolve_to_a_font_or_the_bitmap_font_and_a_bad_font_is_an_error() {
        assert!(physics().bitmap_font().resolve_font().unwrap().is_none(), "the bitmap font is a choice, not a failure");
        assert!(physics().font("junk", vec![1, 2, 3]).resolve_font().unwrap_err().to_string().contains("junk"));
        let missing = std::env::temp_dir().join("orr_app_no_such_font.ttf");
        assert!(physics().font_file(&missing).resolve_font().unwrap_err().to_string().contains("orr_app_no_such_font.ttf"));
        // The default is the system font when there is one, and still a run when there is none.
        assert_eq!(physics().resolve_font().unwrap().is_some(), crate::font::system_font().is_some());
        if let Some(path) = crate::font::system_font_candidates().into_iter().find(|p| p.exists()) {
            let font = physics().font_file(&path).resolve_font().unwrap().expect("a font file gives a font");
            let bytes = std::fs::read(&path).unwrap();
            let from_bytes = physics().font("copy", bytes).resolve_font().unwrap().unwrap();
            assert_eq!(font.advance('가', 20.0), from_bytes.advance('가', 20.0));
        }
    }

    /// A 16-bit mono WAV file of `samples` at 22050 Hz.
    fn wav_bytes(samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        for field in [1u16, 1] {
            out.extend_from_slice(&field.to_le_bytes());
        }
        out.extend_from_slice(&22_050u32.to_le_bytes());
        out.extend_from_slice(&44_100u32.to_le_bytes());
        for field in [2u16, 16] {
            out.extend_from_slice(&field.to_le_bytes());
        }
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn wav_sounds_are_read_when_the_run_starts_and_a_bad_one_stops_it_with_its_name() {
        let dir = std::env::temp_dir().join(format!("orr_app_wav_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("pickup.wav");
        std::fs::write(&good, wav_bytes(&[0, 8000, -8000, 0])).unwrap();
        let bad = dir.join("bad.wav");
        std::fs::write(&bad, b"RIFF....nope").unwrap();

        let mut app = physics().sound_file("pickup", &good).sound_wav("embedded", wav_bytes(&[1, 2, 3, 4]));
        assert!(!app.views.has_sound("pickup"), "nothing is read before the run");
        app.resolve_sounds().unwrap();
        assert!(app.views.has_sound("pickup") && app.views.has_sound("embedded"));

        let missing = physics().sound_file("gone", dir.join("missing.wav")).resolve_sounds().unwrap_err().to_string();
        assert!(missing.contains("gone") && missing.contains("missing.wav"), "{missing}");
        let refused = physics().sound_file("broken", &bad).resolve_sounds().unwrap_err().to_string();
        assert!(refused.contains("broken") && refused.contains("bad.wav") && refused.contains("WAV"), "{refused}");
        let embedded = physics().sound_wav("junk", vec![1, 2, 3]).resolve_sounds().unwrap_err().to_string();
        assert!(embedded.contains("junk") && embedded.contains("embedded bytes"), "{embedded}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_load_starts_a_new_offline_bridge_that_has_the_saved_checksum_and_the_same_future() {
        let dir = std::env::temp_dir().join(format!("orr_app_load_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bot = |slot: PlayerSlot, tick: u64| {
            assert_eq!(slot, PlayerSlot(1));
            PhysInput::new(1, 0, 0, tick % 2 == 0)
        };

        // The straight run: 60 ticks.
        let mut straight = physics().bot(bot).build_bridge().unwrap();
        straight.step(60);
        let straight_sum = straight.snapshot().unwrap().predicted().checksum();

        // The run that saves at tick 30 and goes on from the load. The new bridge must not announce the players again.
        let mut app = physics().bot(bot);
        let mut bridge = app.build_bridge().unwrap();
        let parts = app.parts.take().expect("an offline bridge leaves its parts");
        bridge.step(30);
        let mut views = Views::<PhysGame>::new();
        views.enable_saves(SaveStore::at(&dir, "physics", 1).unwrap(), parts.players, parts.tick_rate);
        let update = bridge.poll_view();
        views.dispatch(&mut bridge, &update, &[]);
        let saved = views.save_game("quick", "tick 30").unwrap();
        assert_eq!(saved.tick, 30);
        bridge.step(10); // the old run moves on; the load must not see it

        let loaded = views.load_game("quick").unwrap();
        assert_eq!(loaded.checksum, saved.checksum);
        let save = views.take_load().expect("an accepted load");
        let mut restarted = parts.bridge_from_save(&save).unwrap();
        let first = restarted.poll_view().snapshot.expect("a snapshot at once");
        assert_eq!((first.tick(), first.predicted().checksum()), (30, saved.checksum), "the checksum after the load is the saved one");
        restarted.step(30);
        assert_eq!(restarted.snapshot().unwrap().tick(), 60);
        assert_eq!(restarted.snapshot().unwrap().predicted().checksum(), straight_sum, "30 ticks, a save, a load and 30 ticks are 60 ticks");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_load_of_a_game_with_players_does_not_announce_them_again() {
        let dir = std::env::temp_dir().join(format!("orr_app_load_players_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::<Roster>::new().players(3).local_players(2).player_data(Profile { color: 5, _pad: 0 });
        let mut bridge = app.build_bridge().unwrap();
        let parts = app.parts.take().unwrap();
        bridge.step(5);
        let mut views = Views::<Roster>::new();
        views.enable_saves(SaveStore::at(&dir, "roster", 1).unwrap(), parts.players, parts.tick_rate);
        let update = bridge.poll_view();
        views.dispatch(&mut bridge, &update, &[]);
        views.save_game("p", "").unwrap();
        views.load_game("p").unwrap();
        let mut restarted = parts.bridge_from_save(&views.take_load().unwrap()).unwrap();
        restarted.step(5);
        let snapshot = restarted.snapshot().unwrap();
        let mut avatars: Vec<(u32, u32)> = snapshot.predicted().dense::<Avatar>().1.iter().map(|a| (a.slot, a.color)).collect();
        avatars.sort_unstable();
        assert_eq!(avatars, [(0, 5), (1, 5), (2, 0)], "three avatars, not six");
        let _ = std::fs::remove_dir_all(dir);
    }
}
