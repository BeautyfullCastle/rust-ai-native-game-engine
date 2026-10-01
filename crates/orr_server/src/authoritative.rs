//! The simulation side of the authoritative mode (design §6.1,
//! "Authoritative"): the server runs the game's `orr_sim` headless on the
//! confirmed bundles.
//!
//! The room (see `room`) is byte oriented and knows no game. It talks to the
//! game through the object-safe [`ServerSim`]; [`GameSim<G>`] is the
//! implementation for any `orr_sim::Game`.
//!
//! What the sim does for the room, in the order a tick finalizes:
//!
//! 1. [`ServerSim::drive`]: the inputs of the server-driven slots (AI
//!    players, scripted events) for tick `T`, decided from the state after
//!    tick `T - 1`. They become part of the bundle everybody gets, so the
//!    clients stay deterministic without knowing who decided them.
//! 2. [`ServerSim::step`]: the finished bundle advances the sim to `T`.
//!    Returns the violations of the game's state audit, if it has one.
//! 3. [`ServerSim::checksum`] at checkpoint ticks, [`ServerSim::snapshot`]
//!    for corrections and late joiners, and for `.orrd` dumps.
//!
//! The sim never predicts and never rolls back: it sees only confirmed
//! input. Its state when tick `T + 1` is decided is the state after the
//! confirmed tick `T`, so a server-driven slot reacts to the world one tick
//! behind the confirm point.
use orr_ecs::Frame;
use orr_proto::{Bundle, FLAG_ABSENT};
use orr_sim::{Game, PlayerFlags, PlayerSlot, SimCommand, Simulation, TickInputs};

/// Where the server's `.orrd` desync dumps go. No path is hardcoded: the
/// caller decides (a directory, memory).
pub trait DumpWriter: Send {
    fn write_dump(&mut self, name: &str, bytes: Vec<u8>);
}

/// Dumps go nowhere (the default).
pub struct NoDumps;

impl DumpWriter for NoDumps {
    fn write_dump(&mut self, _name: &str, _bytes: Vec<u8>) {}
}

/// Writes each dump as a file in a directory (created on first use).
pub struct DirDumps(pub std::path::PathBuf);

impl DumpWriter for DirDumps {
    fn write_dump(&mut self, name: &str, bytes: Vec<u8>) {
        let _ = std::fs::create_dir_all(&self.0);
        if let Err(e) = std::fs::write(self.0.join(name), bytes) {
            eprintln!("orr_server: cannot write dump {name}: {e}");
        }
    }
}

/// Keeps dumps in memory; clones share the storage (for tests and tools).
#[derive(Clone, Default)]
pub struct SharedDumps(std::sync::Arc<std::sync::Mutex<Vec<Dump>>>);

/// A written dump: name and bytes.
pub type Dump = (String, Vec<u8>);

impl SharedDumps {
    pub fn new() -> Self {
        Self::default()
    }
    /// Takes every dump written so far.
    pub fn take(&self) -> Vec<Dump> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
    pub fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl DumpWriter for SharedDumps {
    fn write_dump(&mut self, name: &str, bytes: Vec<u8>) {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((name.to_string(), bytes));
    }
}

/// One thing the state audit found wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub slot: u8,
    pub reason: String,
}

impl Violation {
    pub fn new(slot: u8, reason: impl Into<String>) -> Self {
        Self { slot, reason: reason.into() }
    }
}

/// What the room asks of the server-side simulation.
pub trait ServerSim: Send {
    /// Byte size of one input of the game.
    fn input_size(&self) -> u32;
    fn player_count(&self) -> u8;
    /// The build hash the clients' simulations have (see
    /// `Simulation::build_hash`), for dumps.
    fn build_hash(&self) -> u64;
    /// The tick the sim is at.
    fn tick(&self) -> u64;
    /// The input and encoded commands of each server-driven slot in `slots`
    /// for `tick` (`self.tick() + 1`). Called once per tick, before the
    /// bundle is built.
    fn drive(&mut self, tick: u64, slots: &[u8]) -> Vec<(u8, Vec<u8>, Vec<Vec<u8>>)>;
    /// Advances the sim by the confirmed bundle of tick `self.tick() + 1`.
    fn step(&mut self, bundle: &Bundle) -> Vec<Violation>;
    /// Checksum of the current frame (what a client reports for this tick).
    fn checksum(&self) -> u64;
    /// `Frame::to_bytes` of the current frame.
    fn frame_bytes(&self) -> Vec<u8>;
}

/// Decides what a server-driven slot does at `tick`, from the frame after
/// `tick - 1`.
pub type Brain<G> = Box<dyn FnMut(&Frame, u64, u8) -> (<G as Game>::Input, Vec<<G as Game>::Command>) + Send>;

/// What a state audit sees of one tick.
pub struct Audit<'a, G: Game> {
    pub tick: u64,
    /// The frame before the tick (a copy; only kept when the sim has an audit).
    pub before: &'a Frame,
    /// The frame after the tick.
    pub after: &'a Frame,
    pub inputs: &'a TickInputs<G::Input, G::Command>,
}

/// Checks the result of a tick against the rules of the game ("a paddle
/// cannot move faster than X") and names the slots that broke them.
pub type AuditFn<G> = Box<dyn FnMut(&Audit<'_, G>) -> Vec<Violation> + Send>;

/// [`ServerSim`] for any [`Game`]: a plain `Simulation<G>` plus the optional
/// brain of the server-driven slots and the optional state audit.
pub struct GameSim<G: Game> {
    sim: Simulation<G>,
    player_count: u8,
    brain: Option<Brain<G>>,
    audit: Option<AuditFn<G>>,
    before: Option<Frame>,
}

impl<G: Game> GameSim<G> {
    /// A sim built exactly like a client's (`Session::new`): `config`, tick
    /// rate, seed and `build_id` must equal the clients'.
    pub fn new(config: G::Config, tick_rate: u32, seed: u64, build_id: u64, player_count: u8) -> Self {
        Self {
            sim: Simulation::<G>::with_build_id(config, tick_rate, seed, build_id),
            player_count,
            brain: None,
            audit: None,
            before: None,
        }
    }

    /// The inputs of server-driven slots come from `brain(frame, tick, slot)`.
    pub fn with_brain(mut self, brain: impl FnMut(&Frame, u64, u8) -> (G::Input, Vec<G::Command>) + Send + 'static) -> Self {
        self.brain = Some(Box::new(brain));
        self
    }

    /// Every stepped tick goes through `audit`. Costs one frame copy per tick.
    pub fn with_audit(mut self, audit: impl FnMut(&Audit<'_, G>) -> Vec<Violation> + Send + 'static) -> Self {
        self.before = Some(Frame::new(self.sim.registry().clone()));
        self.audit = Some(Box::new(audit));
        self
    }

    pub fn simulation(&self) -> &Simulation<G> {
        &self.sim
    }

    /// Builds the tick's inputs from a bundle, as a client's session does for
    /// a confirmed tick: absent slots are flagged disconnected, commands that
    /// do not decode are skipped, commands keep slot order.
    fn inputs_of(&self, bundle: &Bundle) -> TickInputs<G::Input, G::Command> {
        let mut ti = TickInputs::<G::Input, G::Command>::new(bundle.tick, self.player_count);
        let mut cmds = Vec::new();
        for (i, s) in bundle.slots.iter().enumerate() {
            let slot = PlayerSlot(i as u8);
            if let Ok(input) = bytemuck::try_pod_read_unaligned::<G::Input>(&s.input) {
                ti.set_input(slot, input);
            }
            if s.flags & FLAG_ABSENT != 0 {
                ti.set_flags(slot, PlayerFlags { predicted: false, disconnected: true });
            }
            for raw in &s.commands {
                if let Some(c) = <G::Command as SimCommand>::decode(raw) {
                    cmds.push((slot, c));
                }
            }
        }
        ti.set_commands(cmds);
        ti
    }
}

impl<G: Game> ServerSim for GameSim<G> {
    fn input_size(&self) -> u32 {
        std::mem::size_of::<G::Input>() as u32
    }

    fn player_count(&self) -> u8 {
        self.player_count
    }

    fn build_hash(&self) -> u64 {
        self.sim.build_hash()
    }

    fn tick(&self) -> u64 {
        self.sim.tick()
    }

    fn drive(&mut self, tick: u64, slots: &[u8]) -> Vec<(u8, Vec<u8>, Vec<Vec<u8>>)> {
        let Some(brain) = self.brain.as_mut() else {
            return slots.iter().map(|&s| (s, vec![0; std::mem::size_of::<G::Input>()], Vec::new())).collect();
        };
        slots
            .iter()
            .map(|&slot| {
                let (input, commands) = brain(self.sim.frame(), tick, slot);
                let encoded = commands
                    .iter()
                    .map(|c| {
                        let mut b = Vec::new();
                        c.encode(&mut b);
                        b
                    })
                    .collect();
                (slot, bytemuck::bytes_of(&input).to_vec(), encoded)
            })
            .collect()
    }

    fn step(&mut self, bundle: &Bundle) -> Vec<Violation> {
        debug_assert_eq!(self.sim.tick() + 1, bundle.tick, "the sim steps one confirmed tick at a time");
        let inputs = self.inputs_of(bundle);
        if let Some(before) = self.before.as_mut() {
            before.copy_from(self.sim.frame());
        }
        let _events = self.sim.step(&inputs);
        match (self.audit.as_mut(), self.before.as_ref()) {
            (Some(audit), Some(before)) => {
                audit(&Audit { tick: bundle.tick, before, after: self.sim.frame(), inputs: &inputs })
            }
            _ => Vec::new(),
        }
    }

    fn checksum(&self) -> u64 {
        self.sim.checksum()
    }

    fn frame_bytes(&self) -> Vec<u8> {
        self.sim.frame().to_bytes()
    }
}
