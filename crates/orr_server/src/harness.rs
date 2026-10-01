//! Test harness (feature `harness`): a relay server and any number of relay
//! clients on one simulated network and one virtual clock. Everything is a
//! pure function of the seed: no wall clock, no threads.
//!
//! Used by the end-to-end tests in this crate and in `orr_sample`.
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

use orr_proto::netsim::{PathParams, SimEndpoint, SimLink, SimNet};
use orr_proto::{Bundle, Welcome};
use orr_session::{ClientState, DumpCollector, RelayClient, RelayClientConfig};
use orr_sim::{Game, PlayerSlot, SimCommand, Simulation, TickInputs};

use crate::{AcceptAll, AuthoritativeConfig, InputValidator, RelayServer, RoomConfig, ServerSim, SharedDumps};

thread_local! {
    static CURRENT_CLIENT: Cell<usize> = const { Cell::new(usize::MAX) };
}

/// Index of the harness client whose `update` is running (`usize::MAX`
/// outside of one). A test game can read it to misbehave on one client only
/// (to force a desync).
pub fn current_client() -> usize {
    CURRENT_CLIENT.with(Cell::get)
}

/// The local player's input for `(client index, tick)`.
pub type Script<G> = Rc<dyn Fn(usize, u64) -> (<G as Game>::Input, Vec<<G as Game>::Command>)>;

/// One client to add to a harness.
#[derive(Clone)]
pub struct ClientSpec {
    pub path: PathParams,
    /// Speed of this client's clock in ppm of real time (1_000_000 = exact;
    /// 990_000 = 1% slow).
    pub clock_ppm: u64,
    pub want_slot: Option<PlayerSlot>,
    pub token: u64,
    pub build_id: u64,
    /// Inputs per packet the client repeats (`None` = the client default).
    pub input_redundancy: Option<u32>,
}

impl ClientSpec {
    pub fn new(path: PathParams) -> Self {
        Self { path, clock_ppm: 1_000_000, want_slot: None, token: 0, build_id: 1, input_redundancy: None }
    }
}

pub struct HarnessClient<G: Game> {
    pub client: RelayClient<G, SimLink>,
    pub link: orr_proto::ConnId,
    pub dumps: DumpCollector,
    clock_ppm: u64,
    started_at_us: u64,
}

impl<G: Game> HarnessClient<G> {
    fn local_now(&self, now_us: u64) -> u64 {
        (now_us - self.started_at_us) * self.clock_ppm / 1_000_000
    }
}

type ConfigFn<G> = Rc<dyn Fn(&Welcome) -> <G as Game>::Config>;

pub struct Harness<G: Game> {
    pub net: SimNet,
    pub server: RelayServer<SimEndpoint, Box<dyn InputValidator>>,
    pub clients: Vec<HarnessClient<G>>,
    /// The `.orrd` dumps the server wrote (authoritative rooms).
    pub server_dumps: SharedDumps,
    pub now_us: u64,
    pub room: u64,
    /// Length of one harness step.
    pub step_us: u64,
    script: Script<G>,
    make_config: ConfigFn<G>,
}

impl<G: Game> Harness<G> {
    pub fn new(
        seed: u64,
        room_cfg: RoomConfig,
        script: Script<G>,
        make_config: impl Fn(&Welcome) -> G::Config + 'static,
    ) -> Self {
        Self::with_validator(seed, room_cfg, script, make_config, Box::new(AcceptAll))
    }

    /// Relay + Validate: the server checks inputs with `validator`.
    pub fn with_validator(
        seed: u64,
        room_cfg: RoomConfig,
        script: Script<G>,
        make_config: impl Fn(&Welcome) -> G::Config + 'static,
        validator: Box<dyn InputValidator>,
    ) -> Self {
        Self::build(seed, room_cfg, None, script, make_config, validator)
    }

    /// An authoritative room: the server also simulates `sim` (see
    /// `RelayServer::create_authoritative_room`).
    pub fn authoritative(
        seed: u64,
        room_cfg: RoomConfig,
        auth: AuthoritativeConfig,
        sim: Box<dyn ServerSim>,
        script: Script<G>,
        make_config: impl Fn(&Welcome) -> G::Config + 'static,
    ) -> Self {
        Self::with_authoritative_validator(seed, room_cfg, auth, sim, script, make_config, Box::new(AcceptAll))
    }

    /// Authoritative, with an input validator as well.
    pub fn with_authoritative_validator(
        seed: u64,
        room_cfg: RoomConfig,
        auth: AuthoritativeConfig,
        sim: Box<dyn ServerSim>,
        script: Script<G>,
        make_config: impl Fn(&Welcome) -> G::Config + 'static,
        validator: Box<dyn InputValidator>,
    ) -> Self {
        Self::build(seed, room_cfg, Some((auth, sim)), script, make_config, validator)
    }

    fn build(
        seed: u64,
        room_cfg: RoomConfig,
        auth: Option<(AuthoritativeConfig, Box<dyn ServerSim>)>,
        script: Script<G>,
        make_config: impl Fn(&Welcome) -> G::Config + 'static,
        validator: Box<dyn InputValidator>,
    ) -> Self {
        let net = SimNet::new(seed);
        let mut server = RelayServer::with_validator(net.endpoint(), validator, seed ^ 0x51ED);
        let server_dumps = SharedDumps::new();
        server.set_dump_sink(server_dumps.clone());
        let room = 1;
        match auth {
            Some((a, sim)) => server.create_authoritative_room(room, room_cfg, a, sim),
            None => server.create_room(room, room_cfg),
        }
        Self {
            net,
            server,
            clients: Vec::new(),
            server_dumps,
            now_us: 0,
            room,
            step_us: 1000,
            script,
            make_config: Rc::new(make_config),
        }
    }

    /// Connects a new client now; returns its index.
    pub fn add_client(&mut self, spec: &ClientSpec) -> usize {
        let idx = self.clients.len();
        let link = self.net.connect(spec.path);
        let conn = link.conn_id();
        let mut cfg = RelayClientConfig::new(self.room, spec.build_id);
        cfg.want_slot = spec.want_slot;
        cfg.token = spec.token;
        if let Some(r) = spec.input_redundancy {
            cfg.input_redundancy = r;
        }
        let dumps = DumpCollector::new();
        let make = self.make_config.clone();
        let client = RelayClient::<G, SimLink>::new(cfg, link, move |w| make(w), dumps.clone());
        self.clients.push(HarnessClient { client, link: conn, dumps, clock_ppm: spec.clock_ppm, started_at_us: self.now_us });
        idx
    }

    /// Changes the network path of client `i` from now on.
    pub fn set_path(&mut self, i: usize, params: PathParams) {
        self.net.set_params(self.clients[i].link, params);
    }

    /// Cuts client `i`'s connection as a network failure would.
    pub fn cut(&mut self, i: usize) {
        self.net.cut_conn(self.clients[i].link);
    }

    /// Advances virtual time by one step and runs the server and every
    /// client once.
    pub fn step(&mut self) {
        self.net.advance_us(self.step_us);
        self.now_us += self.step_us;
        self.server.update(self.now_us);
        let script = self.script.clone();
        for (i, c) in self.clients.iter_mut().enumerate() {
            CURRENT_CLIENT.with(|cur| cur.set(i));
            let local = c.local_now(self.now_us);
            c.client.update(local, &mut |tick| script(i, tick));
        }
        CURRENT_CLIENT.with(|cur| cur.set(usize::MAX));
    }

    /// Runs until the server has finalized `tick` (or `limit_us` of virtual
    /// time pass). Returns whether it got there.
    pub fn run_until_tick(&mut self, tick: u64, limit_us: u64) -> bool {
        let end = self.now_us + limit_us;
        while self.server.finalized_tick(self.room).unwrap_or(0) < tick {
            if self.now_us >= end {
                return false;
            }
            self.step();
        }
        true
    }

    pub fn run_for_us(&mut self, us: u64) {
        let end = self.now_us + us;
        while self.now_us < end {
            self.step();
        }
    }

    /// Runs until every client is playing (or the time limit passes).
    pub fn run_until_all_playing(&mut self, limit_us: u64) -> bool {
        let end = self.now_us + limit_us;
        while !self.clients.iter().all(|c| *c.client.state() == ClientState::Playing) {
            if self.now_us >= end {
                return false;
            }
            self.step();
        }
        true
    }

    /// Like [`run_until_all_playing`](Self::run_until_all_playing), ignoring
    /// the clients in `skip`.
    pub fn run_until_all_playing_except(&mut self, skip: &[usize], limit_us: u64) -> bool {
        let end = self.now_us + limit_us;
        loop {
            let ok = self
                .clients
                .iter()
                .enumerate()
                .all(|(i, c)| skip.contains(&i) || *c.client.state() == ClientState::Playing);
            if ok {
                return true;
            }
            if self.now_us >= end {
                return false;
            }
            self.step();
        }
    }

    /// The confirmed bundles the server recorded (needs `record_all`).
    pub fn recorded(&self) -> &[Bundle] {
        self.server.recorded(self.room)
    }
}

/// Replays confirmed bundles on a plain `Simulation` (no session, no
/// network) and returns its checksum at every multiple of `interval`.
pub fn headless_checksums<G: Game>(
    config: G::Config,
    tick_rate: u32,
    seed: u64,
    build_id: u64,
    player_count: u8,
    interval: u64,
    bundles: &[Bundle],
) -> BTreeMap<u64, u64> {
    let mut sim = Simulation::<G>::with_build_id(config, tick_rate, seed, build_id);
    let mut out = BTreeMap::new();
    for b in bundles {
        let mut ti = TickInputs::<G::Input, G::Command>::new(b.tick, player_count);
        let mut cmds = Vec::new();
        for (i, s) in b.slots.iter().enumerate() {
            let slot = PlayerSlot(i as u8);
            ti.set_input(slot, bytemuck::pod_read_unaligned::<G::Input>(&s.input));
            for raw in &s.commands {
                if let Some(c) = <G::Command as SimCommand>::decode(raw) {
                    cmds.push((slot, c));
                }
            }
        }
        ti.set_commands(cmds);
        sim.step(&ti);
        if b.tick % interval == 0 {
            out.insert(b.tick, sim.checksum());
        }
    }
    out
}
