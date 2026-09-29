//! Late join that survives a slow or lossy hand-over: a joiner that finds a
//! hole in the peers' input backlogs says so (instead of stalling) and joins
//! again; peers hold their inputs while a snapshot is in transfer; a player
//! that left can rejoin from a fresh snapshot. All on the loopback network
//! with latency, jitter and rollbacks, and checked against a headless run.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use orr_fp::{FrameRng, FP};
use orr_session::{
    compare_checksums, join_request, AdvanceResult, InputSource, JoinAttempts, JoinError, JoinStatus, JoinTicket,
    LocalInputSource, LoopbackClock, LoopbackEnd, LoopbackNetwork, PlayerSlot, RemoteInput, Session, SessionConfig,
};
use orr_sim::{Simulation, TickInputs};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd, FIRE};
use xxhash_rust::xxh3::xxh3_64;

const SEED: u64 = 42;
const SLOT2: PlayerSlot = PlayerSlot(2);
/// One-way delay of every join message and link, in rounds.
const LAT: u64 = 3;

fn scripted_input(rng: &mut FrameRng, slot: u8, tick: u64) -> ArenaInput {
    let ax = rng.range_i32(-1, 2);
    let ay = rng.range_i32(-1, 2);
    let fire = rng.next_u32() % 5 == 0 && tick % 3 == (slot as u64 % 3);
    ArenaInput::new(FP::from_int(ax), FP::from_int(ay), fire)
}

fn commands_for(input: ArenaInput, slot: u8) -> Vec<SpawnBulletCmd> {
    if input.buttons & FIRE != 0 {
        vec![SpawnBulletCmd { owner: slot as u32 }]
    } else {
        Vec::new()
    }
}

/// A peer's connections to every other peer.
struct Mesh {
    links: Vec<LoopbackEnd<Arena>>,
}

impl InputSource<Arena> for Mesh {
    fn send_local(&mut self, tick: u64, slot: PlayerSlot, input: ArenaInput, commands: Vec<SpawnBulletCmd>) {
        for link in &mut self.links {
            link.send_local(tick, slot, input, commands.clone());
        }
    }
    fn poll_remote(&mut self) -> Vec<RemoteInput<Arena>> {
        self.links.iter_mut().flat_map(|l| l.poll_remote()).collect()
    }
}

type Peer = Session<Arena, Mesh>;
type Table = BTreeMap<u64, [ArenaInput; 3]>;

fn cfg3(local: u8, input_log_ticks: u32) -> SessionConfig {
    let mut cfg = SessionConfig::new(3, PlayerSlot(local), SEED, 60);
    cfg.checksum_interval = 10;
    cfg.build_id = 0xC0FFEE;
    cfg.input_log_ticks = input_log_ticks;
    cfg
}

fn arena3() -> ArenaConfig {
    ArenaConfig { player_count: 3 }
}

fn drive(peer: &mut Peer, slot: u8, rng: &mut FrameRng, table: &mut Table, idle: bool) -> AdvanceResult<Arena> {
    let stamp = peer.next_send_tick();
    let input = if idle { ArenaInput::default() } else { scripted_input(rng, slot, stamp) };
    table.entry(stamp).or_default()[slot as usize] = input;
    peer.advance(input, commands_for(input, slot))
}

fn headless_checksums(table: &Table, up_to: u64) -> BTreeMap<u64, u64> {
    let mut sim = Simulation::<Arena>::new(arena3(), 60, SEED);
    let mut out = BTreeMap::new();
    for tick in 1..=up_to {
        let inputs = table.get(&tick).copied().unwrap_or_default();
        let mut ti = TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 3);
        let mut cmds = Vec::new();
        for (slot, &input) in inputs.iter().enumerate() {
            ti.set_input(PlayerSlot(slot as u8), input);
            cmds.extend(commands_for(input, slot as u8).into_iter().map(|c| (PlayerSlot(slot as u8), c)));
        }
        ti.set_commands(cmds);
        sim.step(&ti);
        out.insert(tick, sim.checksum());
    }
    out
}

/// Sends what `peer` authored after the snapshot down `link`, then adds the
/// link to the peer's mesh.
fn attach(peer: &mut Peer, mut link: LoopbackEnd<Arena>, snapshot_tick: u64) {
    for r in peer.authored_since(snapshot_tick) {
        link.send_local(r.tick, r.slot, r.input, r.commands);
    }
    peer.source_mut().links.push(link);
}

enum Ev {
    /// A join request reaching the host.
    ToHost(Vec<u8>),
    /// The snapshot (with the joiner's links to the peers) reaching the joiner.
    Snapshot { bytes: Vec<u8>, links: Vec<LoopbackEnd<Arena>> },
    /// A backlog notice reaching the joiner.
    Notice(Vec<u8>),
    /// Peer B learning of the join: it holds its inputs and links the joiner.
    Notify { ticket: JoinTicket, link: LoopbackEnd<Arena> },
}

/// The running session: host A (slot 0), peer B (slot 1), and slot 2 either
/// open (A authors it) or played by C. Slot 2 is joined through `Joiner`.
struct Env {
    clock: LoopbackClock,
    round: u64,
    a: Peer,
    b: Peer,
    c: Option<Peer>,
    rng: [FrameRng; 3],
    table: Table,
    pending: Vec<(u64, Ev)>,
    tickets: Vec<JoinTicket>,
    host_errors: Vec<JoinError>,
    /// Rounds until B learns of the join, per attempt (the last one repeats).
    notify_delays: Vec<u64>,
    snapshot_delay: u64,
}

impl Env {
    /// `with_c`: slot 2 is played by C. Otherwise A covers it.
    fn new(with_c: bool, input_log_ticks: u32) -> Self {
        let (ab_a, ab_b, clock) = LoopbackNetwork::new::<Arena>(4, 1, 777);
        let (mut links_a, mut links_b, mut c) = (vec![ab_a], vec![ab_b], None);
        let mut cfg_a = cfg3(0, input_log_ticks);
        if with_c {
            let (ac_a, ac_c) = LoopbackNetwork::with_clock::<Arena>(&clock, 4, 1, 778);
            let (bc_b, bc_c) = LoopbackNetwork::with_clock::<Arena>(&clock, 4, 1, 779);
            links_a.push(ac_a);
            links_b.push(bc_b);
            c = Some(Session::<Arena, _>::new(arena3(), cfg3(2, input_log_ticks), Mesh { links: vec![ac_c, bc_c] }));
        } else {
            cfg_a.vacant_slots = vec![SLOT2];
        }
        Self {
            a: Session::new(arena3(), cfg_a, Mesh { links: links_a }),
            b: Session::new(arena3(), cfg3(1, input_log_ticks), Mesh { links: links_b }),
            c,
            clock,
            round: 0,
            rng: [FrameRng::new(SEED), FrameRng::new(SEED ^ 0xABCD), FrameRng::new(SEED ^ 0x1234)],
            table: Table::new(),
            pending: Vec::new(),
            tickets: Vec::new(),
            host_errors: Vec::new(),
            notify_delays: vec![2],
            snapshot_delay: LAT,
        }
    }

    fn request(&mut self, joiner: &mut Joiner) {
        match joiner.attempts.next_request() {
            Ok(request) => self.pending.push((self.round + LAT, Ev::ToHost(request))),
            Err(e) => joiner.exhausted = Some(e),
        }
    }

    fn deliver(&mut self, joiner: &mut Joiner) {
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending).into_iter().partition(|(at, _)| *at <= self.round);
        self.pending = later;
        for (_, ev) in due {
            match ev {
                Ev::ToHost(request) => match self.a.serve_join(&request) {
                    Ok(bytes) => {
                        let ticket = self.a.pending_join(SLOT2).expect("a served join is pending");
                        let n = self.tickets.len() as u64;
                        self.tickets.push(ticket);
                        let (a_end, j_a) = LoopbackNetwork::with_clock::<Arena>(&self.clock, LAT, 1, 11 + n);
                        let (link_b, j_b) = LoopbackNetwork::with_clock::<Arena>(&self.clock, LAT, 1, 51 + n);
                        attach(&mut self.a, a_end, ticket.snapshot_tick);
                        let notice = self.a.backlog_notice(SLOT2).expect("host holds the join");
                        self.pending.push((self.round + LAT, Ev::Notice(notice)));
                        let delay = self.notify_delays[(n as usize).min(self.notify_delays.len() - 1)];
                        self.pending.push((self.round + delay, Ev::Notify { ticket, link: link_b }));
                        let snapshot = Ev::Snapshot { bytes, links: vec![j_a, j_b] };
                        self.pending.push((self.round + self.snapshot_delay, snapshot));
                    }
                    Err(e) => self.host_errors.push(e),
                },
                Ev::Notify { ticket, link } => {
                    self.b.hold_inputs_for_join(ticket);
                    attach(&mut self.b, link, ticket.snapshot_tick);
                    let notice = self.b.backlog_notice(SLOT2).expect("peer holds the join");
                    self.pending.push((self.round + LAT, Ev::Notice(notice)));
                }
                Ev::Notice(bytes) => joiner.inbox.push(bytes),
                Ev::Snapshot { bytes, links } => {
                    let cfg = joiner.attempts.config().clone();
                    match Session::<Arena, _>::from_join_snapshot(arena3(), cfg, Mesh { links }, &bytes) {
                        Ok(s) => joiner.session = Some(s),
                        // A snapshot of an attempt the joiner has given up.
                        Err(JoinError::StaleAttempt { .. }) => {}
                        Err(e) => panic!("joiner rejects the snapshot: {e}"),
                    }
                }
            }
        }
    }

    /// One round: deliver due messages, then every peer advances once.
    fn round(&mut self, joiner: &mut Joiner, auto_retry: bool) {
        self.clock.tick();
        self.round += 1;
        self.deliver(joiner);
        drive(&mut self.a, 0, &mut self.rng[0], &mut self.table, false);
        drive(&mut self.b, 1, &mut self.rng[1], &mut self.table, false);
        if let Some(c) = self.c.as_mut() {
            drive(c, 2, &mut self.rng[2], &mut self.table, false);
        }
        joiner.step(self.a.head_tick());
        if let Some(e) = joiner.failed.take() {
            joiner.gaps.push(e);
            joiner.session = None;
            joiner.table.clear();
            joiner.caught_up = false;
            if auto_retry {
                self.request(joiner);
            }
        }
    }

    fn run(&mut self, joiner: &mut Joiner, rounds: u64, auto_retry: bool) {
        for _ in 0..rounds {
            self.round(joiner, auto_retry);
        }
    }
}

/// The player of slot 2 while joining: numbers its requests, checks the
/// backlogs, and plays idle until it has caught up with the others.
struct Joiner {
    attempts: JoinAttempts,
    session: Option<Peer>,
    inbox: Vec<Vec<u8>>,
    /// What the current attempt authored (slot 2 only).
    table: Table,
    rng: FrameRng,
    caught_up: bool,
    failed: Option<JoinError>,
    gaps: Vec<JoinError>,
    exhausted: Option<JoinError>,
    /// Saw a state with some but not all notices in.
    partial_seen: bool,
}

impl Joiner {
    fn new(input_log_ticks: u32, max_attempts: u32) -> Self {
        let mut cfg = cfg3(2, input_log_ticks);
        cfg.join_id = 0xABCD_EF01;
        cfg.join_backlog_peers = 2;
        cfg.max_join_attempts = max_attempts;
        Self {
            attempts: JoinAttempts::new(cfg).unwrap(),
            session: None,
            inbox: Vec::new(),
            table: Table::new(),
            rng: FrameRng::new(SEED ^ 0x5555),
            caught_up: false,
            failed: None,
            gaps: Vec::new(),
            exhausted: None,
            partial_seen: false,
        }
    }

    fn step(&mut self, a_head: u64) {
        let Some(s) = self.session.as_mut() else { return };
        for message in self.inbox.drain(..) {
            match s.receive_backlog(&message) {
                Ok(JoinStatus::Syncing { received, .. }) if received > 0 => self.partial_seen = true,
                Ok(_) => {}
                // A notice of an attempt this session is not.
                Err(JoinError::StaleAttempt { .. }) => {}
                Err(e @ JoinError::InputGap { .. }) => self.failed = Some(e),
                Err(e) => panic!("bad backlog notice: {e}"),
            }
        }
        if self.failed.is_some() {
            return;
        }
        if self.caught_up {
            drive(s, 2, &mut self.rng, &mut self.table, false);
        } else {
            // Fast-forward to the others' head, authoring idle input.
            loop {
                let r = drive(s, 2, &mut self.rng, &mut self.table, true);
                if matches!(r, AdvanceResult::Stalled { .. }) || s.head_tick() >= a_head {
                    break;
                }
            }
            self.caught_up = s.head_tick() >= a_head;
        }
    }
}

/// Every peer's checksums agree with each other and with a headless run of
/// all authored inputs. `joined` is the last snapshot's tick.
fn assert_converged(env: &mut Env, joiner: &Joiner, joined: u64) {
    let j = joiner.session.as_ref().expect("joiner joined");
    assert_eq!(j.join_status().unwrap(), JoinStatus::Ready);
    assert!(joiner.caught_up, "joiner never caught up");
    for (&tick, inputs) in &joiner.table {
        env.table.entry(tick).or_default()[2] = inputs[2];
    }
    let (cs_a, cs_b, cs_j) = (env.a.checksums(), env.b.checksums(), j.checksums());
    assert!(cs_j.len() >= 10, "joiner recorded too few checksums: {}", cs_j.len());
    assert!(cs_j.iter().all(|&(t, _)| t > joined));
    assert!(cs_j.last().unwrap().0 + 60 >= cs_a.last().unwrap().0, "joiner fell behind");
    for other in [cs_a, cs_b] {
        assert!(compare_checksums(cs_j, other).is_empty(), "joiner desynced");
        // The joiner may be ahead of a peer by the link latency.
        let peer_last = other.last().unwrap().0;
        for &(tick, _) in cs_j.iter().filter(|&&(t, _)| t <= peer_last) {
            assert!(other.iter().any(|&(t, _)| t == tick), "no peer checksum at tick {tick}");
        }
    }
    let last = [cs_a, cs_b, cs_j].iter().map(|c| c.last().unwrap().0).max().unwrap();
    let headless = headless_checksums(&env.table, last);
    for (name, cs) in [("A", cs_a), ("B", cs_b), ("joiner", cs_j)] {
        for &(tick, checksum) in cs {
            assert_eq!(headless[&tick], checksum, "peer {name} differs from the headless run at tick {tick}");
        }
    }
}

/// B hears of the join far too late: it has pruned the inputs the joiner
/// needs. The joiner sees that from B's notice (and not before, while the
/// notice is only late), asks again, and the second snapshot works.
#[test]
fn input_gap_is_detected_and_a_retry_converges() {
    const LOG: u32 = 2;
    let mut env = Env::new(false, LOG);
    // Attempt 1: B is told 40 rounds after the host. Attempt 2: promptly.
    env.notify_delays = vec![40, 2];
    let mut joiner = Joiner::new(LOG, 3);
    env.run(&mut joiner, 60, true);
    env.request(&mut joiner);
    env.run(&mut joiner, 400, true);

    assert_eq!(joiner.gaps.len(), 1, "expected exactly one gap: {:?}", joiner.gaps);
    assert!(joiner.partial_seen, "the missing notice should read as late, not as a gap");
    let (t1, t2) = (env.tickets[0], env.tickets[1]);
    assert_eq!((t1.attempt, t2.attempt), (1, 2));
    assert!(t2.snapshot_tick > t1.snapshot_tick && t2.first_input_tick > t1.first_input_tick);
    match &joiner.gaps[0] {
        JoinError::InputGap { slot, needed_from, available_from } => {
            assert_eq!(*slot, PlayerSlot(1), "B is the peer that pruned");
            assert!(*needed_from <= t1.snapshot_tick + 1 + u64::from(LOG) + 12 && *needed_from > t1.snapshot_tick);
            assert!(available_from > needed_from);
        }
        e => panic!("wrong error {e}"),
    }
    assert!(env.host_errors.is_empty(), "{:?}", env.host_errors);
    assert!(env.a.pending_join(SLOT2).is_none(), "join confirmed");
    assert!(env.a.backlog_notice(SLOT2).is_none() && env.b.backlog_notice(SLOT2).is_none(), "holds released");
    assert_converged(&mut env, &joiner, t2.snapshot_tick);
}

/// The snapshot takes far longer than `input_log_ticks` to arrive. The hold
/// keeps the inputs, so the first attempt works.
#[test]
fn slow_snapshot_transfer_converges_with_the_hold() {
    const LOG: u32 = 4;
    let mut env = Env::new(false, LOG);
    env.snapshot_delay = 80;
    let mut joiner = Joiner::new(LOG, 3);
    env.run(&mut joiner, 60, true);
    env.request(&mut joiner);
    env.run(&mut joiner, 500, true);

    assert!(joiner.gaps.is_empty(), "no retry needed: {:?}", joiner.gaps);
    assert_eq!(env.tickets.len(), 1);
    assert!(env.a.head_tick() > env.tickets[0].snapshot_tick + 2 * u64::from(LOG));
    // Held while in transfer: A's log reaches back to the snapshot, and the
    // hold is over once the joiner is confirmed.
    assert!(env.a.pending_join(SLOT2).is_none());
    assert!(env.a.backlog_notice(SLOT2).is_none() && env.b.backlog_notice(SLOT2).is_none());
    let joined = env.tickets[0].snapshot_tick;
    assert_converged(&mut env, &joiner, joined);
}

/// While a join is pending, an ordinary log of `input_log_ticks` would drop
/// inputs the joiner needs; the ticket's hold keeps them, and only that.
#[test]
fn hold_keeps_authored_inputs_until_the_join_is_confirmed() {
    let new_host = || {
        let mut cfg = SessionConfig::new(2, PlayerSlot(0), SEED, 60);
        cfg.input_log_ticks = 4;
        cfg.vacant_slots = vec![PlayerSlot(1)];
        Session::<Arena, _>::new(ArenaConfig { player_count: 2 }, cfg, LocalInputSource)
    };
    let run = |host: &mut Session<Arena, LocalInputSource>, ticks: u32| {
        for _ in 0..ticks {
            host.advance(ArenaInput::default(), Vec::new());
        }
    };
    let oldest = |host: &Session<Arena, LocalInputSource>| host.authored_since(0).first().unwrap().tick;

    let mut plain = new_host();
    run(&mut plain, 60);
    assert!(oldest(&plain) >= plain.verified_tick() - 4, "the log is short without a hold");

    // A hold on tick 10, as a peer sets it when a join starts there.
    let mut host = new_host();
    host.hold_inputs_for_join(JoinTicket { slot: PlayerSlot(1), attempt: 1, snapshot_tick: 10, first_input_tick: 400 });
    run(&mut host, 60);
    assert_eq!(oldest(&host), 11, "everything after the snapshot tick is kept");
    // A newer ticket of the same slot (a retry) replaces the old one.
    host.hold_inputs_for_join(JoinTicket { slot: PlayerSlot(1), attempt: 2, snapshot_tick: 50, first_input_tick: 400 });
    run(&mut host, 2);
    assert_eq!(oldest(&host), 51);
    host.release_join_hold(PlayerSlot(1));
    run(&mut host, 2);
    assert!(oldest(&host) >= host.verified_tick() - 4, "a released hold lets the log shrink");
}

/// A player leaves, its slot is marked vacant, and a new player joins the
/// slot from a fresh snapshot.
#[test]
fn rejoin_after_disconnect_matches_peers_and_headless() {
    const LOG: u32 = 256;
    let mut env = Env::new(true, LOG);
    let mut joiner = Joiner::new(LOG, 3);
    env.run(&mut joiner, 100, false);

    // C is gone: its session and links are dropped. Both peers stall.
    env.c = None;
    env.run(&mut joiner, 30, false);
    let last_c = env.a.last_remote_tick(SLOT2).unwrap();
    assert_eq!(env.b.last_remote_tick(SLOT2), Some(last_c), "peers hold the same inputs of C");
    assert!(env.a.verified_tick() <= last_c, "the game is waiting for C");

    // The slot cannot be handed out while its player is not vacated.
    assert!(matches!(env.a.serve_join(&join_request(joiner.attempts.config())), Err(JoinError::SlotNotVacant(_))));
    // Vacating needs a tick after C's last input and after the verified one.
    for bad in [last_c, env.a.verified_tick(), env.a.next_send_tick() + 1] {
        assert!(matches!(env.a.mark_slot_vacant(SLOT2, bad), Err(JoinError::InvalidVacate(_))), "tick {bad}");
    }
    assert!(matches!(env.a.mark_slot_vacant(PlayerSlot(0), last_c + 1), Err(JoinError::InvalidVacate(_))));
    env.a.mark_slot_vacant(SLOT2, last_c + 1).expect("vacate after C's last input");
    assert!(matches!(env.a.mark_slot_vacant(SLOT2, last_c + 1), Err(JoinError::InvalidVacate(_))));

    // A now authors slot 2: exactly one input per tick from last_c + 1 on.
    let authored = env.a.authored_since(last_c);
    let mut slot2_ticks: Vec<u64> = authored.iter().filter(|r| r.slot == SLOT2).map(|r| r.tick).collect();
    assert_eq!(slot2_ticks.first(), Some(&(last_c + 1)));
    let count = slot2_ticks.len();
    slot2_ticks.dedup();
    assert_eq!(slot2_ticks.len(), count, "two inputs for one tick");

    // The game runs on with A covering slot 2, then C rejoins.
    let verified_before = env.a.verified_tick();
    env.run(&mut joiner, 40, false);
    assert!(env.a.verified_tick() > verified_before + 20, "vacated slot lets the game go on");
    env.request(&mut joiner);
    env.run(&mut joiner, 400, true);

    assert!(joiner.gaps.is_empty() && env.host_errors.is_empty());
    assert_eq!(env.tickets.len(), 1);
    assert!(env.tickets[0].snapshot_tick > last_c, "fresh snapshot, not C's old state");
    let joined = env.tickets[0].snapshot_tick;
    assert_converged(&mut env, &joiner, joined);
}

/// Every attempt ends in a gap: after `max_join_attempts` the joiner gets a
/// definite failure instead of asking forever.
#[test]
fn retry_limit_gives_a_definite_failure() {
    let mut attempts = {
        let mut cfg = cfg3(2, 256);
        cfg.join_id = 7;
        cfg.join_backlog_peers = 2;
        cfg.max_join_attempts = 2;
        JoinAttempts::new(cfg).unwrap()
    };
    attempts.next_request().unwrap();
    assert_eq!(attempts.config().join_attempt, 1);
    attempts.next_request().unwrap();
    assert_eq!(attempts.config().join_attempt, 2);
    assert!(matches!(attempts.next_request(), Err(JoinError::TooManyAttempts { max: 2 })));
    assert!(matches!(attempts.next_request(), Err(JoinError::TooManyAttempts { max: 2 })));
    assert_eq!(attempts.attempts_made(), 2);

    // A join that cannot be retried safely is refused up front.
    let mut anonymous = cfg3(2, 256);
    anonymous.join_backlog_peers = 2;
    assert!(JoinAttempts::new(anonymous).is_err());
    assert!(JoinAttempts::new(cfg3(2, 256)).is_err());

    // In a run: every snapshot arrives with peers that pruned, so each
    // attempt fails; the joiner stops after its limit.
    const LOG: u32 = 2;
    let mut env = Env::new(false, LOG);
    env.notify_delays = vec![40];
    let mut joiner = Joiner::new(LOG, 1);
    env.run(&mut joiner, 60, true);
    env.request(&mut joiner);
    env.run(&mut joiner, 200, true);
    assert_eq!(joiner.gaps.len(), 1);
    assert!(matches!(joiner.exhausted, Some(JoinError::TooManyAttempts { max: 1 })));
    assert!(joiner.session.is_none());
}

fn lone_host() -> Session<Arena, LocalInputSource> {
    let mut cfg = SessionConfig::new(2, PlayerSlot(0), SEED, 60);
    cfg.build_id = 0xC0FFEE;
    cfg.vacant_slots = vec![PlayerSlot(1)];
    let mut host = Session::<Arena, _>::new(ArenaConfig { player_count: 2 }, cfg, LocalInputSource);
    for _ in 0..40 {
        host.advance(ArenaInput::default(), Vec::new());
    }
    host
}

fn joiner_cfg(id: u64, attempt: u32, peers: u32) -> SessionConfig {
    let mut cfg = SessionConfig::new(2, PlayerSlot(1), SEED, 60);
    cfg.build_id = 0xC0FFEE;
    cfg.join_id = id;
    cfg.join_attempt = attempt;
    cfg.join_backlog_peers = peers;
    cfg
}

fn join2(cfg: SessionConfig, msg: &[u8]) -> Result<Session<Arena, LocalInputSource>, JoinError> {
    Session::<Arena, _>::from_join_snapshot(ArenaConfig { player_count: 2 }, cfg, LocalInputSource, msg)
}

/// A duplicate or stale request never hands the slot out twice; a newer
/// request of the same joiner supersedes; the slot has one author per tick.
#[test]
fn stale_and_duplicate_requests_do_not_assign_the_slot_twice() {
    let mut host = lone_host();
    let slot = PlayerSlot(1);
    let (r1, r2) = (join_request(&joiner_cfg(7, 1, 1)), join_request(&joiner_cfg(7, 2, 1)));

    let snap1 = host.serve_join(&r1).expect("first request");
    let t1 = host.pending_join(slot).unwrap();
    // The same request again, a different joiner, and an anonymous one.
    assert!(matches!(host.serve_join(&r1), Err(JoinError::StaleAttempt { current: 1, got: 1 })));
    let other = join_request(&joiner_cfg(8, 2, 1));
    assert!(matches!(host.serve_join(&other), Err(JoinError::SlotNotVacant(_))));
    assert!(matches!(host.serve_join(&join_request(&joiner_cfg(0, 0, 0))), Err(JoinError::SlotNotVacant(_))));
    // A joiner that does not check its backlogs cannot supersede.
    let unsafe_retry = join_request(&joiner_cfg(7, 3, 0));
    assert!(matches!(host.serve_join(&unsafe_retry), Err(JoinError::SlotNotVacant(_))));
    assert_eq!(host.pending_join(slot), Some(t1), "refused requests change nothing");

    // The game moves on while the first joiner never shows up.
    for _ in 0..20 {
        host.advance(ArenaInput::default(), Vec::new());
    }
    let snap2 = host.serve_join(&r2).expect("retry supersedes");
    let t2 = host.pending_join(slot).unwrap();
    assert_eq!(t2.attempt, 2);
    assert!(t2.first_input_tick >= t1.first_input_tick + 20 && t2.snapshot_tick >= t1.snapshot_tick);
    // A late copy of either request is stale now.
    assert!(matches!(host.serve_join(&r1), Err(JoinError::StaleAttempt { current: 2, got: 1 })));
    assert!(matches!(host.serve_join(&r2), Err(JoinError::StaleAttempt { current: 2, got: 2 })));

    // One author: the host authored slot 1 for every tick before the second
    // joiner's first input tick, once each, and none after.
    let mut ticks: Vec<u64> =
        host.authored_since(0).iter().filter(|r| r.slot == slot).map(|r| r.tick).collect();
    assert_eq!(ticks.last(), Some(&(t2.first_input_tick - 1)));
    let count = ticks.len();
    ticks.dedup();
    assert_eq!(ticks.len(), count, "two inputs for one tick");
    assert!(ticks.windows(2).all(|w| w[1] == w[0] + 1), "no tick left without an author");

    // Each snapshot only fits its own attempt.
    assert!(matches!(join2(joiner_cfg(7, 2, 1), &snap1), Err(JoinError::StaleAttempt { current: 2, got: 1 })));
    assert!(matches!(join2(joiner_cfg(7, 1, 1), &snap2), Err(JoinError::StaleAttempt { current: 1, got: 2 })));
    let joined = join2(joiner_cfg(7, 2, 1), &snap2).expect("current attempt is accepted");
    assert_eq!(joined.verified_tick(), t2.snapshot_tick);
    assert_eq!(joined.next_send_tick(), t2.first_input_tick);
    assert_eq!(joined.join_status().unwrap(), JoinStatus::Syncing { received: 0, expected: 1 });
}

/// A notice that leaves out ticks the joiner needs is a proven gap; one
/// that has not arrived is only "syncing".
#[test]
fn joiner_tells_late_notice_from_missing_input() {
    let mut host = lone_host();
    let slot = PlayerSlot(1);
    let snap = host.serve_join(&join_request(&joiner_cfg(7, 1, 1))).unwrap();
    let ticket = host.pending_join(slot).unwrap();

    // The host's honest notice covers everything: ready.
    let notice = host.backlog_notice(slot).unwrap();
    let mut joiner = join2(joiner_cfg(7, 1, 1), &snap).unwrap();
    assert_eq!(joiner.join_status().unwrap(), JoinStatus::Syncing { received: 0, expected: 1 });
    assert_eq!(joiner.receive_backlog(&notice).unwrap(), JoinStatus::Ready);

    // Two notices expected and one arrived: the rest may only be late.
    let mut joiner = join2(joiner_cfg(7, 1, 2), &snap).unwrap();
    assert_eq!(joiner.receive_backlog(&notice).unwrap(), JoinStatus::Syncing { received: 1, expected: 2 });
    // A repeat of the same peer's notice adds nothing.
    assert_eq!(joiner.receive_backlog(&notice).unwrap(), JoinStatus::Syncing { received: 1, expected: 2 });

    // A peer whose backlog starts two ticks after the first one needed.
    let late_start = ticket.snapshot_tick + 3;
    let honest_other = notice_bytes(1, PlayerSlot(0), &[(0, late_start, u64::MAX), (1, 3, ticket.first_input_tick)]);
    let mut joiner = join2(joiner_cfg(7, 1, 1), &snap).unwrap();
    match joiner.receive_backlog(&honest_other) {
        Err(JoinError::InputGap { slot, needed_from, available_from }) => {
            assert_eq!(slot, PlayerSlot(0));
            assert_eq!(needed_from, ticket.snapshot_tick + 1);
            assert_eq!(available_from, late_start);
        }
        other => panic!("expected a gap, got {other:?}"),
    }
    assert!(matches!(joiner.join_status(), Err(JoinError::InputGap { .. })), "the gap stays");

    // A notice of another attempt, or with a bad seal, changes nothing.
    let mut joiner = join2(joiner_cfg(7, 1, 1), &snap).unwrap();
    let wrong = notice_bytes(2, PlayerSlot(0), &[]);
    assert!(matches!(joiner.receive_backlog(&wrong), Err(JoinError::StaleAttempt { current: 1, got: 2 })));
    let mut torn = notice.clone();
    torn[6] ^= 1;
    assert!(matches!(joiner.receive_backlog(&torn), Err(JoinError::BadMessageChecksum)));
    assert_eq!(joiner.join_status().unwrap(), JoinStatus::Syncing { received: 0, expected: 1 });
}

/// Records what a session sends.
#[derive(Clone, Default)]
struct Sent(Rc<RefCell<Vec<(u64, PlayerSlot)>>>);

impl InputSource<Arena> for Sent {
    fn send_local(&mut self, tick: u64, slot: PlayerSlot, _: ArenaInput, _: Vec<SpawnBulletCmd>) {
        self.0.borrow_mut().push((tick, slot));
    }
    fn poll_remote(&mut self) -> Vec<RemoteInput<Arena>> {
        Vec::new()
    }
}

/// A joiner sends no input before its backlogs are proven complete, and
/// none ever when they are not: the slot then still has one author.
#[test]
fn joiner_sends_nothing_until_ready() {
    let mut host = lone_host();
    let snap = host.serve_join(&join_request(&joiner_cfg(7, 1, 1))).unwrap();
    let ticket = host.pending_join(PlayerSlot(1)).unwrap();
    let make = |sent: &Sent| {
        Session::<Arena, _>::from_join_snapshot(
            ArenaConfig { player_count: 2 },
            joiner_cfg(7, 1, 1),
            sent.clone(),
            &snap,
        )
        .unwrap()
    };

    // Ready: the inputs authored meanwhile go out, in order, at that moment.
    let sent = Sent::default();
    let mut j = make(&sent);
    for _ in 0..5 {
        j.advance(ArenaInput::default(), Vec::new());
    }
    assert!(sent.0.borrow().is_empty());
    assert_eq!(j.receive_backlog(&host.backlog_notice(PlayerSlot(1)).unwrap()).unwrap(), JoinStatus::Ready);
    let first = ticket.first_input_tick;
    assert_eq!(*sent.0.borrow(), (first..first + 5).map(|t| (t, PlayerSlot(1))).collect::<Vec<_>>());
    j.advance(ArenaInput::default(), Vec::new());
    assert_eq!(sent.0.borrow().last(), Some(&(first + 5, PlayerSlot(1))), "sends directly once ready");

    // Gap: nothing is ever sent.
    let sent = Sent::default();
    let mut j = make(&sent);
    let gap = notice_bytes(1, PlayerSlot(0), &[(0, ticket.snapshot_tick + 3, u64::MAX)]);
    assert!(matches!(j.receive_backlog(&gap), Err(JoinError::InputGap { .. })));
    for _ in 0..5 {
        j.advance(ArenaInput::default(), Vec::new());
    }
    assert!(sent.0.borrow().is_empty());
}

/// A backlog notice as a peer would send it: `(slot, from, until)` spans.
fn notice_bytes(attempt: u32, sender: PlayerSlot, spans: &[(u8, u64, u64)]) -> Vec<u8> {
    let mut out = b"ORRB".to_vec();
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&attempt.to_le_bytes());
    out.push(sender.0);
    out.extend_from_slice(&(spans.len() as u32).to_le_bytes());
    for &(slot, from, until) in spans {
        out.push(slot);
        out.extend_from_slice(&from.to_le_bytes());
        out.extend_from_slice(&until.to_le_bytes());
    }
    let sum = xxh3_64(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    out
}
