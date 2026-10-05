//! Rolling two-peer Arena over pinned direct QUIC. No relay, TUI, discovery or mesh.
//! `p2p_arena host [bind] [generation] [live_ticks] [pre_activation_retries] [host_continuations]` prints the exact join command.
//! `p2p_arena join <address> <sha256> <generation> [live_ticks]` joins slot 1.
//! `p2p_arena smoke [live_ticks]` runs both peers over real loopback sockets and checks every
//! available verified checksum against an independently stepped simulation.
//! The host admits the first connected client for this local-development demo;
//! a generation is a fence, not a credential or production client authentication.

#![allow(clippy::disallowed_types)] // Wall clock belongs to this transport driver, never the simulation.

use std::collections::BTreeMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use orr_net::SendError;
use orr_proto::{Channel, ConnId, Endpoint, Link, LinkEvent, ServerEvent};
use orr_relay_net::p2p_input::P2pRollingWindow;
use orr_relay_net::{
    connect, format_fingerprint, fresh_seed, listen, parse_fingerprint, ConnectOptions,
    ListenOptions, NetConfig, P2pInputCodec, P2pInputDriver, P2pInputLimits, P2pInputSource,
    TransportKind, Trust,
};
use orr_session::{
    CheckedJoinContext, JoinBootstrap, JoinBootstrapStatus, JoinRoster, P2pAttempt, P2pMembership,
    P2pRoutedEvent, PlayerSlot, Session, SessionConfig,
};
use orr_sim::{Simulation, TickInputs, FP};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
type Source = P2pInputSource<Arena, ArenaCodec>;
type Peer = Session<Arena, Source>;
type Bootstrap = JoinBootstrap<Arena, Source>;
const HOST: PlayerSlot = PlayerSlot(0);
const JOINER: PlayerSlot = PlayerSlot(1);
const CONTROL_LIMIT: usize = 256 * 1024;
const LIVE_TICKS: u64 = 120;
const RECENT_TICKS: u64 = 8;
const FUTURE_TICKS: u64 = 64;
const PROGRESS_TIMEOUT: Duration = Duration::from_secs(30);
const BUILD_ID: u64 = 0x5032504152454e31; // P2P Arena v1 demo build contract

/// Arena wire schema v1: i64 LE raw Q48.16 x, i64 LE y, u32 LE buttons.
/// `_pad` is omitted and reconstructed as zero. Axes are bounded [-1,1], only
/// FIRE bit 0 is accepted. Each command is u32 LE owner, restricted to slots 0/1.
/// This encoding is independent of native Rust layout/endianness.
struct ArenaCodec;
impl P2pInputCodec<Arena> for ArenaCodec {
    const SCHEMA: u64 = 0x4152454e41000001;
    fn encode_input(input: &ArenaInput, out: &mut Vec<u8>) {
        out.extend_from_slice(&input.axis_x.0.to_le_bytes());
        out.extend_from_slice(&input.axis_y.0.to_le_bytes());
        out.extend_from_slice(&input.buttons.to_le_bytes());
    }
    fn decode_input(bytes: &[u8]) -> Option<ArenaInput> {
        if bytes.len() != 20 {
            return None;
        }
        let x = i64::from_le_bytes(bytes[..8].try_into().ok()?);
        let y = i64::from_le_bytes(bytes[8..16].try_into().ok()?);
        let buttons = u32::from_le_bytes(bytes[16..].try_into().ok()?);
        if !(-65536..=65536).contains(&x) || !(-65536..=65536).contains(&y) || buttons > 1 {
            return None;
        }
        Some(ArenaInput {
            axis_x: FP::from_raw(x),
            axis_y: FP::from_raw(y),
            buttons,
            _pad: 0,
        })
    }
    fn encode_command(command: &SpawnBulletCmd, out: &mut Vec<u8>) {
        out.extend_from_slice(&command.owner.to_le_bytes());
    }
    fn decode_command(bytes: &[u8]) -> Option<SpawnBulletCmd> {
        let owner = u32::from_le_bytes(bytes.try_into().ok()?);
        (owner < 2).then_some(SpawnBulletCmd { owner })
    }
}
fn roster() -> JoinRoster {
    JoinRoster::completed(2, JOINER, HOST, vec![HOST], &[]).unwrap()
}
fn context(generation: u64) -> Result<CheckedJoinContext> {
    Ok(CheckedJoinContext::new(generation, 1, roster())?)
}
fn game() -> ArenaConfig {
    ArenaConfig { player_count: 2 }
}
fn config(slot: PlayerSlot, generation: u64) -> SessionConfig {
    let mut cfg = SessionConfig::new(2, slot, 42, 60);
    cfg.join_id = generation;
    cfg.build_id = BUILD_ID;
    cfg.checksum_interval = 1;
    cfg.max_prediction = 32;
    cfg.input_log_ticks = 256;
    if slot == HOST {
        cfg.vacant_slots.push(JOINER);
    }
    cfg
}
fn limits() -> P2pInputLimits {
    P2pInputLimits {
        max_packet_bytes: 256,
        max_input_bytes: 20,
        max_commands: 3,
        max_command_bytes: 4,
        max_records: 192,
        max_retained_bytes: 192 * 256,
        ..P2pInputLimits::default()
    }
}
fn window() -> P2pRollingWindow {
    P2pRollingWindow {
        recent_ticks: RECENT_TICKS,
        future_ticks: FUTURE_TICKS,
    }
}
fn net_config() -> NetConfig {
    NetConfig {
        max_message_size: CONTROL_LIMIT,
        max_connections: 1,
        event_queue: 256,
        max_queued_send_bytes: 512 * 1024,
        worker_threads: 2,
        ..NetConfig::default()
    }
}
fn input(slot: PlayerSlot, tick: u64) -> ArenaInput {
    ArenaInput::new(
        FP::from_int((tick % 3) as i32 - 1),
        FP::from_int(i32::from(slot.0) * 2 - 1),
        true,
    )
}
fn commands(slot: PlayerSlot) -> Vec<SpawnBulletCmd> {
    // Repeated commands and distinct owners make both multiplicity and order observable.
    [slot.0, 1 - slot.0, slot.0]
        .map(|owner| SpawnBulletCmd {
            owner: u32::from(owner),
        })
        .to_vec()
}
fn check_flush(result: orr_session::P2pFlush<SendError>) -> Result<bool> {
    for (_, error) in result.blocked {
        if error != SendError::Backpressure {
            return Err(error.into());
        }
    }
    Ok(result.remaining_bytes == 0)
}
fn drive(peer: &mut Peer, driver: &P2pInputDriver<Arena, ArenaCodec>) -> Result<()> {
    driver.check()?;
    let tick = peer.next_send_tick();
    peer.advance(
        input(peer.config().local_slot, tick),
        commands(peer.config().local_slot),
    );
    driver.check()?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Report {
    tick: u64,
    checksum: u64,
    snapshot: u64,
    first_input: u64,
    checked: usize,
    retired_through: u64,
    evidence_records: usize,
}
// Host reference covers every gameplay interval, including preserved departed
// input and the default gap. Returners instead start from their validated fresh
// snapshot: their abandoned speculative timeline is never used as a baseline.
struct Reference {
    baseline: Simulation<Arena>,
    gameplay: Vec<(u64, Option<u64>)>,
}
impl Reference {
    fn initial() -> Self {
        Self {
            baseline: Simulation::with_build_id(game(), 60, 42, BUILD_ID),
            gameplay: Vec::new(),
        }
    }
    fn from_snapshot(peer: &Peer, first_input: u64) -> Result<Self> {
        Ok(Self {
            baseline: Simulation::from_frame(
                peer.verified_frame().ok_or("missing snapshot")?,
                60,
                BUILD_ID,
            )?,
            gameplay: vec![(first_input, None)],
        })
    }
    fn depart(&mut self, target: u64) {
        if let Some((first, end)) = self.gameplay.last_mut() {
            if target < *first {
                self.gameplay.pop();
            } else {
                *end = Some(target);
            }
        }
    }
    fn checksums(&self, target: u64) -> Result<BTreeMap<u64, u64>> {
        let mut sim = Simulation::<Arena>::from_frame(self.baseline.frame(), 60, BUILD_ID)?;
        let mut out = BTreeMap::from([(sim.tick(), sim.frame().checksum())]);
        for tick in sim.tick() + 1..=target {
            let mut inputs = TickInputs::new(tick, 2);
            for slot in [HOST, JOINER] {
                let active = slot == HOST
                    || self
                        .gameplay
                        .iter()
                        .any(|&(from, until)| tick >= from && until.is_none_or(|end| tick <= end));
                if tick > 2 && active {
                    inputs.set_input(slot, input(slot, tick));
                    for command in commands(slot) {
                        inputs.push_command(slot, command);
                    }
                }
            }
            sim.step(&inputs);
            out.insert(tick, sim.frame().checksum());
        }
        Ok(out)
    }
}
fn verified_report(
    peer: &Peer,
    driver: &P2pInputDriver<Arena, ArenaCodec>,
    snapshot: u64,
    first_input: u64,
    target: u64,
    reference: &Reference,
) -> Result<Report> {
    if peer.verified_tick() < target {
        return Err("target is not verified".into());
    }
    let reference = reference.checksums(target)?;
    let mut checked = 0;
    for &(tick, checksum) in peer.checksums() {
        if let Some(expected) = reference.get(&tick) {
            if checksum != *expected {
                return Err(format!("reference checksum mismatch at tick {tick}").into());
            }
            checked += 1;
        }
    }
    let checksum = peer
        .checksums()
        .iter()
        .find(|(tick, _)| *tick == target)
        .ok_or("missing target checksum")?
        .1;
    Ok(Report {
        tick: target,
        checksum,
        snapshot,
        first_input,
        checked,
        retired_through: driver
            .rolling_progress()
            .ok_or("expected rolling source")?
            .1,
        evidence_records: driver.retained_evidence().0,
    })
}
// Example-only application completion handshake, not part of ORRI or checked controls:
// ORRA, v1 u8, kind u8 (1 verified report / 2 matching ack), generation u64 LE,
// attempt u32 LE, verified tick u64 LE, checksum u64 LE. Exactly 34 bytes.
fn report_bytes(generation: u64, kind: u8, report: &Report) -> Vec<u8> {
    let mut bytes = b"ORRA\x01".to_vec();
    bytes.push(kind);
    bytes.extend_from_slice(&generation.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&report.tick.to_le_bytes());
    bytes.extend_from_slice(&report.checksum.to_le_bytes());
    bytes
}
fn read_report(bytes: &[u8], generation: u64, kind: u8) -> Result<(u64, u64)> {
    if bytes.len() != 34
        || &bytes[..4] != b"ORRA"
        || bytes[4] != 1
        || bytes[5] != kind
        || u64::from_le_bytes(bytes[6..14].try_into()?) != generation
        || u32::from_le_bytes(bytes[14..18].try_into()?) != 1
    {
        return Err("invalid application report context".into());
    }
    Ok((
        u64::from_le_bytes(bytes[18..26].try_into()?),
        u64::from_le_bytes(bytes[26..34].try_into()?),
    ))
}
/// Retrying means explicitly allowing another first-connected development client.
/// It does not authenticate that client as the previous user.
struct HostOptions {
    pre_activation_retries: u32,
    /// Explicitly accept only driver-admitted departed input and fresh return snapshots.
    host_continuations: u32,
    join_wait: Duration,
    initial_ticks: u64,
    #[cfg(test)]
    hold_first_notice: bool,
    #[cfg(test)]
    held_notice_progress: Option<mpsc::Sender<()>>,
}
impl Default for HostOptions {
    fn default() -> Self {
        Self {
            pre_activation_retries: 0,
            host_continuations: 0,
            join_wait: PROGRESS_TIMEOUT,
            initial_ticks: 24,
            #[cfg(test)]
            hold_first_notice: false,
            #[cfg(test)]
            held_notice_progress: None,
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct HostAnnouncement {
    address: SocketAddr,
    fingerprint: [u8; 32],
    generation: u64,
    verified_tick: u64,
}
fn announce_host(
    info: HostAnnouncement,
    live_ticks: u64,
    announce: &Option<mpsc::Sender<HostAnnouncement>>,
) -> Result<()> {
    println!(
        "HOST_RUNNING verified={} vacant_slot=1 address={} generation={}",
        info.verified_tick, info.address, info.generation
    );
    println!(
        "JOIN_COMMAND p2p_arena join {} {} {} {live_ticks}",
        info.address,
        format_fingerprint(&info.fingerprint),
        info.generation
    );
    if let Some(announce) = announce {
        announce.send(info)?;
    }
    Ok(())
}
// Fence every old-link event, including non-control input which membership forwards.
fn current_host_event(event: &ServerEvent, connection: Option<ConnId>) -> bool {
    match event {
        ServerEvent::Disconnected(conn) | ServerEvent::Message { conn, .. } => {
            Some(*conn) == connection
        }
        ServerEvent::Connected(_) => true,
    }
}
fn run_host(
    bind: SocketAddr,
    generation: u64,
    live_ticks: u64,
    announce: Option<mpsc::Sender<HostAnnouncement>>,
) -> Result<Report> {
    run_host_with_options(
        bind,
        generation,
        live_ticks,
        announce,
        HostOptions::default(),
    )
}
fn run_host_with_options(
    bind: SocketAddr,
    mut generation: u64,
    live_ticks: u64,
    announce: Option<mpsc::Sender<HostAnnouncement>>,
    options: HostOptions,
) -> Result<Report> {
    let mut opts = ListenOptions::new(bind, TransportKind::Quic);
    opts.net = net_config();
    let mut endpoint = listen(&opts)?;
    let ctx = context(generation)?;
    let (mut driver, source) =
        P2pInputDriver::<Arena, ArenaCodec>::new_rolling(HOST, ctx, limits(), window())?;
    let mut peer = Peer::new(game(), config(HOST, generation), source);
    let mut members = P2pMembership::new(2, HOST, CONTROL_LIMIT * 2)?;
    assert!(members.admit_local()?.is_empty());
    assert!(members.commit_vacant(JOINER)?.is_empty());
    for _ in 0..options.initial_ticks {
        drive(&mut peer, &driver)?;
    }
    peer.poll_confirmed();
    driver.check()?;
    let addr = endpoint.local_addr();
    let fp = endpoint
        .cert_sha256()
        .ok_or("missing generated fingerprint")?;
    announce_host(
        HostAnnouncement {
            address: addr,
            fingerprint: fp,
            generation,
            verified_tick: peer.verified_tick(),
        },
        live_ticks,
        &announce,
    )?;
    let mut retry_count = 0;
    let mut continuation_count = 0;
    let mut reference = Reference::initial();
    #[cfg(test)]
    let mut held_notice_progress = options.held_notice_progress;
    let mut waiting_since = Instant::now();
    // Readiness is possible after successful transport enqueue, even if no ORRI arrives.
    let mut final_notice_queued = false;
    let mut pending_disconnect = None;
    let mut attempt: Option<P2pAttempt> = None;
    let mut connection = None;
    let mut grant = None;
    let mut own_report: Option<Report> = None;
    let mut remote_report = None;
    let mut ack_sent = false;
    let mut last_progress = Instant::now();
    let mut verified_progress = 0;
    let mut next_tick = Instant::now();
    let result = (|| -> Result<Report> {
        loop {
            if !final_notice_queued && waiting_since.elapsed() > options.join_wait {
                return Err("host pre-activation wait expired; no further clients admitted".into());
            }
            if last_progress.elapsed() > PROGRESS_TIMEOUT {
                return Err(
                    "host stalled waiting for verified progress/bootstrap/completion".into(),
                );
            }
            driver.check()?;
            for _ in 0..256 {
                let Some(event) = pending_disconnect
                    .take()
                    .map(ServerEvent::Disconnected)
                    .or_else(|| endpoint.poll())
                else {
                    break;
                };
                if let ServerEvent::Connected(conn) = event {
                    if connection.is_some() {
                        endpoint.disconnect(conn);
                        continue;
                    }
                    // Demo admission policy: first actual connected client, expected slot 1.
                    assert!(members.admit_joiner(JOINER, conn)?.is_empty());
                    let a = members.begin_attempt(JOINER, HOST, generation, 1)?;
                    let lease = endpoint
                        .claim_exclusive_connection(conn)
                        .ok_or("cannot own joining connection")?;
                    members.register_exclusive_connection(&a, lease)?;
                    connection = Some(conn);
                    attempt = Some(a);
                    last_progress = Instant::now();
                    continue;
                }
                if !current_host_event(&event, connection) {
                    continue;
                }
                match members.route_event(event) {
                    P2pRoutedEvent::Control { conn, data } => {
                        if data.len() > CONTROL_LIMIT
                            || grant.is_some()
                            || !data.starts_with(b"ORRQ")
                        {
                            return Err("unexpected host control".into());
                        }
                        let a = attempt.as_ref().ok_or("unadmitted request")?;
                        let (snapshot, ticket) =
                            members.serve_request(a, conn, &mut peer, &data)?;
                        let raw = ticket.ticket();
                        let target = raw
                            .first_input_tick
                            .checked_add(live_ticks)
                            .ok_or("tick overflow")?;
                        target
                            .checked_add(FUTURE_TICKS)
                            .ok_or("tick horizon overflow")?;
                        driver.bind(
                            conn,
                            ticket.context(),
                            raw.snapshot_tick,
                            raw.first_input_tick,
                        )?;
                        let notice = members.backlog_notice(a, &mut peer, &ticket)?;
                        members.queue_control(a, JOINER, &snapshot)?;
                        members.queue_control(a, JOINER, &notice)?;
                        println!("HOST_SNAPSHOT tick={} first_input={} backlog_records={} target={target}", raw.snapshot_tick, raw.first_input_tick, driver.pending().0);
                        reference.gameplay.push((raw.first_input_tick, None));
                        grant = Some((raw.snapshot_tick, raw.first_input_tick, target));
                        last_progress = Instant::now();
                    }
                    P2pRoutedEvent::Forward(ServerEvent::Message {
                        conn,
                        channel,
                        data,
                    }) => {
                        if Some(conn) != connection
                            || members.sender(conn)? != JOINER
                            || channel != Channel::Reliable
                        {
                            return Err("wrong host input connection/channel".into());
                        }
                        if data.starts_with(b"ORRI") {
                            driver.receive(conn, channel, &data)?;
                        } else {
                            let report = read_report(&data, generation, 1)?;
                            if remote_report.is_some_and(|old| old != report) {
                                return Err("conflicting verified report".into());
                            }
                            remote_report = Some(report);
                        }
                    }
                    P2pRoutedEvent::Disconnected { event, cleanup } => {
                        // route_event already removed the old route and cancelled its
                        // attempt. Fence application traffic before releasing anything.
                        connection = None;
                        attempt = None;
                        for clean in cleanup {
                            let _ = clean.release_local_hold(&mut peer);
                            clean.retire_exclusive_connections(&mut endpoint);
                        }
                        if ack_sent {
                            driver.cancel();
                            return own_report
                                .clone()
                                .ok_or_else(|| "missing host report".into());
                        }
                        let fresh_generation =
                            generation.checked_add(1).ok_or("generation exhausted")?;
                        let replacement = if final_notice_queued {
                            if continuation_count >= options.host_continuations {
                                driver.cancel();
                                return Err("joiner disconnected after final readiness notice was queued; explicit host-continuation budget exhausted or disabled".into());
                            }
                            let conn = match event {
                                ServerEvent::Disconnected(conn) => conn,
                                _ => return Err("invalid disconnect event".into()),
                            };
                            // Route and transport are fenced, but the healthy driver still
                            // owns accepted input, including records not polled by Session.
                            let target = driver.fence_host_continuation(&peer, conn)?;
                            driver.verify_host_continuation(&mut peer)?;
                            let replacement = driver.replace_fenced_host_rolling(
                                &mut peer,
                                context(fresh_generation)?,
                            )?;
                            reference.depart(target);
                            continuation_count += 1;
                            println!("HOST_CONTINUED target={target} cutoff={} accepted_inputs_only=true continuations_remaining={}", target + 1, options.host_continuations - continuation_count);
                            replacement
                        } else {
                            driver.cancel();
                            if retry_count >= options.pre_activation_retries {
                                return Err("pre-activation retry budget exhausted; owned holds, link and input buffers retired".into());
                            }
                            // The stricter no-final-notice/no-admitted-input path is unchanged.
                            let replacement = driver.replace_cancelled_host_rolling(
                                &mut peer,
                                context(fresh_generation)?,
                            )?;
                            if let Some((_, first_input, _)) = grant {
                                peer.mark_slot_vacant(JOINER, first_input)?;
                                reference.gameplay.pop();
                            }
                            retry_count += 1;
                            replacement
                        };
                        replacement.check()?;
                        driver = replacement;
                        assert!(members.commit_vacant(JOINER)?.is_empty());
                        peer.poll_confirmed();
                        driver.check()?;
                        generation = fresh_generation;
                        final_notice_queued = false;
                        grant = None;
                        own_report = None;
                        remote_report = None;
                        last_progress = Instant::now();
                        waiting_since = Instant::now();
                        println!("HOST_RETRY generation={generation} retries_remaining={} head={} verified={}", options.pre_activation_retries - retry_count, peer.head_tick(), peer.verified_tick());
                        announce_host(
                            HostAnnouncement {
                                address: addr,
                                fingerprint: fp,
                                generation,
                                verified_tick: peer.verified_tick(),
                            },
                            live_ticks,
                            &announce,
                        )?;
                    }
                    other => return Err(format!("unexpected host event: {other:?}").into()),
                }
            }
            let controls_done = check_flush(members.flush(|conn, _, bytes| {
                #[cfg(test)]
                if options.hold_first_notice && retry_count == 0 && bytes.starts_with(b"ORRB") {
                    return Err(SendError::Backpressure);
                }
                match endpoint.try_send_reliable(conn, bytes) {
                    Ok(()) => {
                        if bytes.starts_with(b"ORRB") {
                            final_notice_queued = true;
                        }
                        Ok(())
                    }
                    Err(SendError::UnknownConnection) => {
                        // A transport may report loss before its queued disconnect event.
                        // Retain controls until the ordinary fenced cleanup path runs.
                        pending_disconnect = Some(conn);
                        Err(SendError::Backpressure)
                    }
                    Err(error) => Err(error),
                }
            }))?;
            if pending_disconnect.is_some() {
                continue;
            }
            if grant.is_some() && controls_done {
                driver.flush(
                    |conn, bytes| match endpoint.try_send_reliable(conn, bytes) {
                        Err(SendError::UnknownConnection) => {
                            pending_disconnect = Some(conn);
                            Err(SendError::Backpressure)
                        }
                        result => result,
                    },
                )?;
            }
            if pending_disconnect.is_some() {
                continue;
            }
            peer.poll_confirmed();
            driver.check()?;
            if Instant::now() >= next_tick {
                if grant.is_none_or(|(_, _, target)| peer.head_tick() < target) {
                    drive(&mut peer, &driver)?;
                }
                next_tick = Instant::now() + Duration::from_millis(16);
            }
            #[cfg(test)]
            if options.hold_first_notice
                && retry_count == 0
                && grant.is_some_and(|(_, first, _)| peer.head_tick() >= first + 4)
            {
                if let Some(progress) = held_notice_progress.take() {
                    progress.send(())?;
                }
            }
            if peer.verified_tick() > verified_progress {
                verified_progress = peer.verified_tick();
                last_progress = Instant::now();
            }
            if let Some((snapshot, first_input, target)) = grant {
                if final_notice_queued && peer.verified_tick() >= target && own_report.is_none() {
                    let report =
                        verified_report(&peer, &driver, snapshot, first_input, target, &reference)?;
                    println!(
                        "HOST_VERIFIED tick={target} checksum={:016x} reference_checkpoints={}",
                        report.checksum, report.checked
                    );
                    own_report = Some(report);
                }
            }
            if let (Some(ours), Some(theirs)) = (&own_report, remote_report) {
                if (ours.tick, ours.checksum) != theirs {
                    return Err("peer verified checksum mismatch".into());
                }
                if !ack_sent && driver.pending().0 == 0 {
                    let conn = connection.ok_or("missing connection")?;
                    match endpoint.try_send_reliable(conn, &report_bytes(generation, 2, ours)) {
                        Ok(()) => {
                            ack_sent = true;
                            last_progress = Instant::now();
                            println!(
                                "HOST_AGREED tick={} checksum={:016x}",
                                ours.tick, ours.checksum
                            );
                        }
                        Err(SendError::Backpressure) => {}
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
    })();
    if let Some(a) = attempt {
        if let Ok(clean) = members.cancel_attempt(&a) {
            let _ = clean.release_local_hold(&mut peer);
            clean.retire_exclusive_connections(&mut endpoint);
        }
    }
    driver.cancel();
    result
}

fn run_join(
    addr: SocketAddr,
    fingerprint: [u8; 32],
    generation: u64,
    live_ticks: u64,
) -> Result<Report> {
    let mut opts = ConnectOptions::new(
        addr.to_string(),
        TransportKind::Quic,
        Trust::Fingerprint(fingerprint),
    );
    opts.net = net_config();
    let mut link = connect(&opts)?;
    let conn = link.connection_id();
    let mut members = P2pMembership::new(2, JOINER, CONTROL_LIMIT * 2)?;
    assert!(members.commit_vacant(JOINER)?.is_empty());
    let mut bootstrap = Bootstrap::new(config(JOINER, generation), roster(), 4096, CONTROL_LIMIT)?;
    let (driver, source) = P2pInputDriver::<Arena, ArenaCodec>::new_rolling(
        JOINER,
        context(generation)?,
        limits(),
        window(),
    )?;
    let mut source = Some(source);
    let mut attempt = None;
    let mut grant = None;
    let mut ready = false;
    let mut reference = None;
    let mut own_report = None;
    let mut report_sent = false;
    let mut last_progress = Instant::now();
    let mut verified_progress = 0;
    let mut next_tick = Instant::now();
    let result = (|| -> Result<Report> {
        loop {
            if last_progress.elapsed() > PROGRESS_TIMEOUT {
                return Err(
                    "join stalled waiting for verified progress/bootstrap/completion".into(),
                );
            }
            driver.check()?;
            for _ in 0..256 {
                let Some(event) = link.poll() else {
                    break;
                };
                let event = match event {
                    LinkEvent::Connected => {
                        assert!(members.admit_active(HOST, conn)?.is_empty());
                        let a = members.begin_attempt(JOINER, HOST, generation, 1)?;
                        let lease = link
                            .claim_exclusive_connection()
                            .ok_or("cannot own donor link")?;
                        members.register_exclusive_connection(&a, lease)?;
                        let request = bootstrap.next_request()?;
                        members.queue_control(&a, HOST, &request)?;
                        attempt = Some(a);
                        continue;
                    }
                    LinkEvent::Disconnected => ServerEvent::Disconnected(conn),
                    LinkEvent::Message { channel, data } => ServerEvent::Message {
                        conn,
                        channel,
                        data,
                    },
                };
                match members.route_event(event) {
                    P2pRoutedEvent::Control { conn, data } => {
                        let a = attempt.as_ref().ok_or("unadmitted donor control")?;
                        if data.starts_with(b"ORRJ") {
                            members.receive_snapshot(
                                a,
                                conn,
                                &mut bootstrap,
                                game(),
                                source.take().ok_or("duplicate snapshot")?,
                                &data,
                            )?;
                            let peer = bootstrap
                                .session()
                                .ok_or("missing accepted snapshot session")?;
                            let snapshot = peer.verified_tick();
                            let first_input = peer.next_send_tick();
                            let target =
                                first_input.checked_add(live_ticks).ok_or("tick overflow")?;
                            target
                                .checked_add(FUTURE_TICKS)
                                .ok_or("tick horizon overflow")?;
                            driver.bind(conn, a.context(), snapshot, first_input)?;
                            reference = Some(Reference::from_snapshot(peer, first_input)?);
                            grant = Some((snapshot, first_input, target));
                            last_progress = Instant::now();
                        } else if data.starts_with(b"ORRB") {
                            members.receive_notice(a, conn, &mut bootstrap, &data)?;
                        } else {
                            return Err("unexpected join control".into());
                        }
                        if bootstrap.status()? == JoinBootstrapStatus::Ready && !ready {
                            ready = true;
                            last_progress = Instant::now();
                            let (_, _, target) = grant.ok_or("Ready without accepted snapshot")?;
                            println!(
                                "JOIN_READY caught_up={} target={target}",
                                bootstrap.caught_up_to(target)?
                            );
                        }
                    }
                    P2pRoutedEvent::Forward(ServerEvent::Message {
                        conn,
                        channel,
                        data,
                    }) => {
                        if conn != link.connection_id()
                            || members.sender(conn)? != HOST
                            || channel != Channel::Reliable
                        {
                            return Err("wrong donor input connection/channel".into());
                        }
                        if data.starts_with(b"ORRI") {
                            driver.receive(conn, channel, &data)?;
                        } else {
                            let ack = read_report(&data, generation, 2)?;
                            let ours: &Report =
                                own_report.as_ref().ok_or("premature completion ack")?;
                            if !report_sent || (ours.tick, ours.checksum) != ack {
                                return Err("incorrect completion ack".into());
                            }
                            println!(
                                "JOIN_AGREED tick={} checksum={:016x}",
                                ours.tick, ours.checksum
                            );
                            link.close();
                            // Let the explicit close reach the transport; the host waits for it.
                            let close_deadline = Instant::now() + Duration::from_secs(2);
                            while Instant::now() < close_deadline {
                                if matches!(link.poll(), Some(LinkEvent::Disconnected)) {
                                    break;
                                }
                                thread::sleep(Duration::from_millis(1));
                            }
                            return Ok(ours.clone());
                        }
                    }
                    P2pRoutedEvent::Disconnected { cleanup, .. } => {
                        bootstrap.invalidate_membership();
                        for clean in cleanup {
                            clean.retire_exclusive_connections(&mut link);
                        }
                        attempt = None;
                        driver.cancel();
                        return Err(
                            "donor disconnected; bootstrap, owned link and input buffers retired"
                                .into(),
                        );
                    }
                    other => return Err(format!("unexpected join event: {other:?}").into()),
                }
            }
            check_flush(members.flush(|id, _, bytes| {
                if id == conn {
                    link.try_send_reliable(bytes)
                } else {
                    Err(SendError::UnknownConnection)
                }
            }))?;
            if grant.is_some() {
                driver.flush(|id, bytes| {
                    if id == conn {
                        link.try_send_reliable(bytes)
                    } else {
                        Err(SendError::UnknownConnection)
                    }
                })?;
            }
            bootstrap.poll_confirmed();
            driver.check()?;
            if let Some(peer) = bootstrap.session() {
                if peer.verified_tick() > verified_progress {
                    verified_progress = peer.verified_tick();
                    last_progress = Instant::now();
                }
            }
            if let Some((snapshot, first_input, target)) = grant {
                if ready
                    && Instant::now() >= next_tick
                    && bootstrap.session().unwrap().head_tick() < target
                {
                    let tick = bootstrap.session().unwrap().next_send_tick();
                    bootstrap
                        .advance(input(JOINER, tick), commands(JOINER))
                        .ok_or("bootstrap no longer active")?;
                    driver.check()?;
                    next_tick = Instant::now() + Duration::from_millis(16);
                }
                if bootstrap.caught_up_to(target)? && own_report.is_none() {
                    let report = verified_report(
                        bootstrap.session().unwrap(),
                        &driver,
                        snapshot,
                        first_input,
                        target,
                        reference.as_ref().ok_or("missing snapshot baseline")?,
                    )?;
                    println!("JOIN_CAUGHT_UP verified={target} checksum={:016x} reference_checkpoints={}", report.checksum, report.checked);
                    own_report = Some(report);
                }
            }
            if let Some(ours) = &own_report {
                if !report_sent && driver.pending().0 == 0 {
                    match link.try_send_reliable(&report_bytes(generation, 1, ours)) {
                        Ok(()) => {
                            report_sent = true;
                            last_progress = Instant::now();
                        }
                        Err(SendError::Backpressure) => {}
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
    })();
    bootstrap.cancel();
    if let Some(a) = attempt {
        if let Ok(clean) = members.cancel_attempt(&a) {
            clean.retire_exclusive_connections(&mut link);
        }
    }
    driver.cancel();
    result
}
fn smoke(live_ticks: u64) -> Result<()> {
    let (send, recv) = mpsc::channel();
    let host =
        thread::spawn(move || run_host("127.0.0.1:0".parse().unwrap(), 77, live_ticks, Some(send)));
    let info = recv.recv_timeout(Duration::from_secs(10))?;
    let joined = run_join(info.address, info.fingerprint, info.generation, live_ticks);
    let hosted = host.join().map_err(|_| "host thread panicked")?;
    let join = joined?;
    let host = hosted?;
    if (join.tick, join.checksum, join.snapshot, join.first_input)
        != (host.tick, host.checksum, host.snapshot, host.first_input)
    {
        return Err("loopback reports disagree".into());
    }
    if live_ticks > 2 * RECENT_TICKS
        && (host.retired_through <= host.snapshot + RECENT_TICKS
            || join.retired_through <= join.snapshot + RECENT_TICKS)
    {
        return Err("rolling source did not retire multiple evidence windows".into());
    }
    println!("SMOKE_OK real_quic=true backlog_commands=nonempty live_commands=ordered_and_repeated verified_tick={} checksum={:016x} recent_ticks={RECENT_TICKS} host_floor={} join_floor={} evidence_records={}/{}", host.tick, host.checksum, host.retired_through, join.retired_through, host.evidence_records, join.evidence_records);
    Ok(())
}
fn live_ticks(value: Option<&String>) -> Result<u64> {
    let ticks = value.map(|s| s.parse()).transpose()?.unwrap_or(LIVE_TICKS);
    if ticks == 0 {
        return Err("live_ticks must be positive".into());
    }
    Ok(ticks)
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("host") if args.len() <= 6 => {
            run_host_with_options(
                args.get(1).map_or("127.0.0.1:7000", String::as_str).parse()?,
                args.get(2).map(|s| s.parse()).transpose()?.unwrap_or_else(|| fresh_seed().max(1)),
                live_ticks(args.get(3))?,
                None,
                HostOptions {
                    pre_activation_retries: args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(0),
                    host_continuations: args.get(5).map(|s| s.parse()).transpose()?.unwrap_or(0),
                    ..HostOptions::default()
                },
            )?;
        }
        Some("join") if (4..=5).contains(&args.len()) => {
            run_join(args[1].parse()?, parse_fingerprint(&args[2])?, args[3].parse()?, live_ticks(args.get(4))?)?;
        }
        Some("smoke") if args.len() <= 2 => smoke(live_ticks(args.get(1))?)?,
        _ => return Err("usage: p2p_arena host [bind] [generation] [live_ticks] [pre_activation_retries] [host_continuations] | join <addr> <fingerprint> <generation> [live_ticks] | smoke [live_ticks]".into()),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn real_quic_late_join_ordered_commands() {
        super::smoke(super::LIVE_TICKS).unwrap();
    }
    // A real checked bootstrap consumes the snapshot (and optionally final notice),
    // but deliberately never advances or emits joiner input before closing its link.
    fn disconnect_during_bootstrap(
        info: super::HostAnnouncement,
        after_notice: bool,
        host_progress: Option<super::mpsc::Receiver<()>>,
    ) -> super::Result<(u64, u64)> {
        use super::*;
        let mut opts = ConnectOptions::new(
            info.address.to_string(),
            TransportKind::Quic,
            Trust::Fingerprint(info.fingerprint),
        );
        opts.net = net_config();
        let mut link = connect(&opts)?;
        let conn = link.connection_id();
        let mut members = P2pMembership::new(2, JOINER, CONTROL_LIMIT * 2)?;
        members.commit_vacant(JOINER)?;
        let mut bootstrap = Bootstrap::new(
            config(JOINER, info.generation),
            roster(),
            4096,
            CONTROL_LIMIT,
        )?;
        let (driver, source) = P2pInputDriver::<Arena, ArenaCodec>::new_rolling(
            JOINER,
            context(info.generation)?,
            limits(),
            window(),
        )?;
        let mut source = Some(source);
        let mut attempt = None;
        let mut grant = None;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            while let Some(event) = link.poll() {
                let event = match event {
                    LinkEvent::Connected => {
                        members.admit_active(HOST, conn)?;
                        let a = members.begin_attempt(JOINER, HOST, info.generation, 1)?;
                        members.register_exclusive_connection(
                            &a,
                            link.claim_exclusive_connection().ok_or("missing lease")?,
                        )?;
                        members.queue_control(&a, HOST, &bootstrap.next_request()?)?;
                        attempt = Some(a);
                        continue;
                    }
                    LinkEvent::Message { channel, data } => ServerEvent::Message {
                        conn,
                        channel,
                        data,
                    },
                    LinkEvent::Disconnected => {
                        return Err("donor disconnected before test cutoff".into())
                    }
                };
                match members.route_event(event) {
                    P2pRoutedEvent::Control { conn, data } => {
                        let a = attempt.as_ref().ok_or("missing attempt")?;
                        if data.starts_with(b"ORRJ") {
                            members.receive_snapshot(
                                a,
                                conn,
                                &mut bootstrap,
                                game(),
                                source.take().ok_or("duplicate snapshot")?,
                                &data,
                            )?;
                            let peer = bootstrap.session().ok_or("snapshot not accepted")?;
                            grant = Some((peer.verified_tick(), peer.next_send_tick()));
                            driver.bind(
                                conn,
                                a.context(),
                                peer.verified_tick(),
                                peer.next_send_tick(),
                            )?;
                        } else if data.starts_with(b"ORRB") {
                            members.receive_notice(a, conn, &mut bootstrap, &data)?;
                        } else {
                            return Err("unexpected test control".into());
                        }
                        if grant.is_some()
                            && (!after_notice || bootstrap.status()? == JoinBootstrapStatus::Ready)
                        {
                            if !after_notice {
                                assert_ne!(bootstrap.status()?, JoinBootstrapStatus::Ready);
                            }
                            if let Some(progress) = &host_progress {
                                progress.recv_timeout(Duration::from_secs(10))?;
                            }
                            bootstrap.cancel();
                            let clean = members.cancel_attempt(a)?;
                            clean.retire_exclusive_connections(&mut link);
                            driver.cancel();
                            link.close();
                            let close_deadline = Instant::now() + Duration::from_secs(2);
                            while Instant::now() < close_deadline {
                                if matches!(link.poll(), Some(LinkEvent::Disconnected)) {
                                    break;
                                }
                                thread::sleep(Duration::from_millis(1));
                            }
                            return Ok(grant.unwrap());
                        }
                    }
                    P2pRoutedEvent::Forward(ServerEvent::Message {
                        conn,
                        channel,
                        data,
                    }) => {
                        driver.receive(conn, channel, &data)?;
                    }
                    other => return Err(format!("unexpected test event: {other:?}").into()),
                }
            }
            check_flush(members.flush(|_, _, bytes| link.try_send_reliable(bytes)))?;
            thread::sleep(Duration::from_millis(1));
        }
        Err("bootstrap test timed out".into())
    }
    #[test]
    fn real_quic_pre_activation_retry_preserves_running_world() {
        use super::*;
        let (send, recv) = mpsc::channel();
        let (progress_send, progress_recv) = mpsc::channel();
        let host = thread::spawn(move || {
            run_host_with_options(
                "127.0.0.1:0".parse().unwrap(),
                91,
                LIVE_TICKS,
                Some(send),
                HostOptions {
                    pre_activation_retries: 1,
                    initial_ticks: 96,
                    hold_first_notice: true,
                    held_notice_progress: Some(progress_send),
                    ..HostOptions::default()
                },
            )
        });
        let initial = recv.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(initial.verified_tick > FUTURE_TICKS);
        let (abandoned_snapshot, abandoned_first) =
            disconnect_during_bootstrap(initial, false, Some(progress_recv)).unwrap();
        let retry = recv.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            (retry.address, retry.fingerprint),
            (initial.address, initial.fingerprint)
        );
        assert_eq!(retry.generation, initial.generation + 1);
        assert!(
            retry.verified_tick > abandoned_first,
            "defaults must verify the formerly abandoned suffix"
        );
        let joined = run_join(
            retry.address,
            retry.fingerprint,
            retry.generation,
            LIVE_TICKS,
        );
        let hosted = host.join().unwrap();
        let joined = joined.unwrap();
        let hosted = hosted.unwrap();
        assert_eq!(
            (
                hosted.tick,
                hosted.checksum,
                hosted.snapshot,
                hosted.first_input
            ),
            (
                joined.tick,
                joined.checksum,
                joined.snapshot,
                joined.first_input
            )
        );
        assert!(hosted.snapshot >= abandoned_snapshot);
        assert!(
            hosted.first_input > abandoned_first,
            "host must keep authoring after vacancy recovery"
        );
        // Both reports were independently compared at every available checkpoint,
        // using only the successful grant as the joiner's activation boundary.
        assert!(hosted.checked > LIVE_TICKS as usize);
        assert!(joined.checked >= LIVE_TICKS as usize);
    }
    // Real pinned QUIC moves requests, snapshots, notices and every input. These
    // helpers deliberately control polling so a nonempty admitted tail stays in
    // the driver's queue at disconnect, rather than merely in the transport.
    fn server_message(
        endpoint: &mut orr_relay_net::NetEndpoint,
        expected: super::ConnId,
    ) -> Vec<u8> {
        use super::*;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match endpoint.poll() {
                Some(ServerEvent::Message {
                    conn,
                    channel: Channel::Reliable,
                    data,
                }) => {
                    assert_eq!(conn, expected);
                    return data;
                }
                Some(other) => panic!("unexpected server event: {other:?}"),
                None => thread::sleep(Duration::from_millis(1)),
            }
        }
        panic!("server receive timed out");
    }
    fn client_message(link: &mut orr_relay_net::NetLink) -> Vec<u8> {
        use super::*;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match link.poll() {
                Some(LinkEvent::Message {
                    channel: Channel::Reliable,
                    data,
                }) => return data,
                Some(other) => panic!("unexpected client event: {other:?}"),
                None => thread::sleep(Duration::from_millis(1)),
            }
        }
        panic!("client receive timed out");
    }
    fn socket_link(
        endpoint: &mut orr_relay_net::NetEndpoint,
    ) -> (orr_relay_net::NetLink, super::ConnId) {
        use super::*;
        let mut options = ConnectOptions::new(
            endpoint.local_addr().to_string(),
            TransportKind::Quic,
            Trust::Fingerprint(endpoint.cert_sha256().unwrap()),
        );
        options.net = net_config();
        let mut link = connect(&options).unwrap();
        let mut server = None;
        let mut client = false;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(event) = endpoint.poll() {
                match event {
                    ServerEvent::Connected(conn) => server = Some(conn),
                    other => panic!("{other:?}"),
                }
            }
            if let Some(event) = link.poll() {
                match event {
                    LinkEvent::Connected => client = true,
                    other => panic!("{other:?}"),
                }
            }
            if client && server.is_some() {
                return (link, server.unwrap());
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("connect timed out");
    }
    fn socket_bootstrap(
        endpoint: &mut orr_relay_net::NetEndpoint,
        link: &mut orr_relay_net::NetLink,
        conn: super::ConnId,
        host: &mut super::Peer,
        driver: &super::P2pInputDriver<super::Arena, super::ArenaCodec>,
        generation: u64,
    ) -> (
        super::Bootstrap,
        super::P2pInputDriver<super::Arena, super::ArenaCodec>,
        super::Reference,
        u64,
    ) {
        use super::*;
        let ctx = context(generation).unwrap();
        let mut bootstrap =
            Bootstrap::new(config(JOINER, generation), roster(), 4096, CONTROL_LIMIT).unwrap();
        let (joined, source) =
            P2pInputDriver::new_rolling(JOINER, ctx.clone(), limits(), window()).unwrap();
        link.try_send_reliable(&bootstrap.next_request().unwrap())
            .unwrap();
        let request = server_message(endpoint, conn);
        let (snapshot, ticket) = orr_session::serve_checked_join(host, &ctx, &request).unwrap();
        let raw = ticket.ticket();
        driver
            .bind(conn, &ctx, raw.snapshot_tick, raw.first_input_tick)
            .unwrap();
        let notice = orr_session::checked_backlog_notice(host, &ctx, &ticket).unwrap();
        endpoint.try_send_reliable(conn, &snapshot).unwrap();
        bootstrap
            .receive_snapshot(HOST, game(), source, &client_message(link))
            .unwrap();
        joined
            .bind(
                link.connection_id(),
                &ctx,
                raw.snapshot_tick,
                raw.first_input_tick,
            )
            .unwrap();
        let baseline =
            Reference::from_snapshot(bootstrap.session().unwrap(), raw.first_input_tick).unwrap();
        let sent = driver
            .flush(|id, bytes| endpoint.try_send_reliable(id, bytes))
            .unwrap()
            .sent;
        for _ in 0..sent {
            let bytes = client_message(link);
            joined
                .receive(link.connection_id(), Channel::Reliable, &bytes)
                .unwrap();
        }
        bootstrap.poll_confirmed();
        endpoint.try_send_reliable(conn, &notice).unwrap();
        bootstrap
            .receive_notice(HOST, &client_message(link))
            .unwrap();
        assert_eq!(bootstrap.status().unwrap(), JoinBootstrapStatus::Ready);
        (bootstrap, joined, baseline, raw.first_input_tick)
    }
    #[test]
    fn real_quic_admitted_unpolled_commands_continue_and_fresh_snapshot_reconnects() {
        use super::*;
        use orr_relay_net::{P2pInputAccepted, P2pInputError};
        let mut options = ListenOptions::new("127.0.0.1:0".parse().unwrap(), TransportKind::Quic);
        options.net = net_config();
        let mut endpoint = listen(&options).unwrap();
        let (old, source) =
            P2pInputDriver::new_rolling(HOST, context(301).unwrap(), limits(), window()).unwrap();
        let mut host = Peer::new(game(), config(HOST, 301), source);
        let mut reference = Reference::initial();
        for _ in 0..96 {
            drive(&mut host, &old).unwrap();
        }
        host.poll_confirmed();
        let (mut link, conn) = socket_link(&mut endpoint);
        let (mut abandoned, departed, _, first) =
            socket_bootstrap(&mut endpoint, &mut link, conn, &mut host, &old, 301);
        reference.gameplay.push((first, None));
        let target = first + 5;
        while host.next_send_tick() < target + 5 {
            drive(&mut host, &old).unwrap();
        }
        for tick in first..=target {
            abandoned
                .advance(input(JOINER, tick), commands(JOINER))
                .unwrap();
        }
        let sent = departed
            .flush(|_, bytes| link.try_send_reliable(bytes))
            .unwrap()
            .sent;
        assert_eq!(sent, 6);
        let mut retained_packet = Vec::new();
        for n in 0..sent {
            let bytes = server_message(&mut endpoint, conn);
            assert_eq!(
                old.receive(conn, Channel::Reliable, &bytes),
                Ok(P2pInputAccepted::New)
            );
            if n < 2 {
                host.poll_confirmed();
            }
            retained_packet = bytes;
        }
        assert_eq!(host.last_remote_tick(JOINER), Some(first + 1));
        assert_eq!(host.verified_tick(), first + 1);
        // No speculative returning state survives: close the actual old socket
        // before fencing, but do not call old.cancel/disconnected on healthy input.
        abandoned.cancel();
        departed.cancel();
        link.close();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(event) = endpoint.poll() {
                assert!(matches!(event, ServerEvent::Disconnected(id) if id == conn));
                break;
            }
            assert!(Instant::now() < deadline, "disconnect timed out");
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(old.fence_host_continuation(&host, conn), Ok(target));
        assert_eq!(
            old.receive(conn, Channel::Reliable, &retained_packet),
            Err(P2pInputError::Fenced)
        );
        assert_eq!(old.disconnected(conn), Err(P2pInputError::Fenced));
        assert_eq!(old.verify_host_continuation(&mut host), Ok(target));
        reference.depart(target);
        assert_eq!(
            host.verified_frame().unwrap().checksum(),
            reference.checksums(target).unwrap()[&target]
        );
        let world = host.predicted_frame().to_bytes();
        let head = host.head_tick();
        let fresh = old
            .replace_fenced_host_rolling(&mut host, context(302).unwrap())
            .unwrap();
        assert_eq!(host.head_tick(), head);
        assert_eq!(host.predicted_frame().to_bytes(), world);
        assert_eq!(old.check(), Err(P2pInputError::Cancelled));
        assert_eq!(
            old.receive(conn, Channel::Reliable, &retained_packet),
            Err(P2pInputError::Cancelled)
        );
        for _ in 0..8 {
            drive(&mut host, &fresh).unwrap();
        }
        host.poll_confirmed();
        let checkpoint = host.verified_tick();
        assert!(checkpoint > target);
        assert_eq!(
            host.verified_frame().unwrap().checksum(),
            reference.checksums(checkpoint).unwrap()[&checkpoint]
        );
        let (mut returned_link, returned_conn) = socket_link(&mut endpoint);
        assert_ne!(returned_conn, conn);
        let (mut returned, returned_driver, baseline, returned_first) = socket_bootstrap(
            &mut endpoint,
            &mut returned_link,
            returned_conn,
            &mut host,
            &fresh,
            302,
        );
        assert_eq!(returned.session().unwrap().verified_tick(), checkpoint);
        reference.gameplay.push((returned_first, None));
        for _ in 0..24 {
            drive(&mut host, &fresh).unwrap();
            let tick = returned.session().unwrap().next_send_tick();
            returned
                .advance(input(JOINER, tick), commands(JOINER))
                .unwrap();
            let sent = fresh
                .flush(|id, bytes| endpoint.try_send_reliable(id, bytes))
                .unwrap()
                .sent;
            for _ in 0..sent {
                let bytes = client_message(&mut returned_link);
                returned_driver
                    .receive(returned_link.connection_id(), Channel::Reliable, &bytes)
                    .unwrap();
            }
            let sent = returned_driver
                .flush(|_, bytes| returned_link.try_send_reliable(bytes))
                .unwrap()
                .sent;
            for _ in 0..sent {
                let bytes = server_message(&mut endpoint, returned_conn);
                fresh
                    .receive(returned_conn, Channel::Reliable, &bytes)
                    .unwrap();
            }
            host.poll_confirmed();
            returned.poll_confirmed();
            fresh.check().unwrap();
            returned_driver.check().unwrap();
        }
        let target = host
            .verified_tick()
            .min(returned.session().unwrap().verified_tick());
        let hosted = verified_report(
            &host,
            &fresh,
            checkpoint,
            returned_first,
            target,
            &reference,
        )
        .unwrap();
        let joined = verified_report(
            returned.session().unwrap(),
            &returned_driver,
            checkpoint,
            returned_first,
            target,
            &baseline,
        )
        .unwrap();
        assert_eq!(hosted.checksum, joined.checksum);
        assert!(hosted.checked > joined.checked);
        assert!(joined.checked >= 20);
    }
    #[test]
    fn real_quic_explicit_continuation_policy_reopens_same_running_host() {
        use super::*;
        let (send, recv) = mpsc::channel();
        let host = thread::spawn(move || {
            run_host_with_options(
                "127.0.0.1:0".parse().unwrap(),
                401,
                24,
                Some(send),
                HostOptions {
                    host_continuations: 1,
                    ..HostOptions::default()
                },
            )
        });
        let initial = recv.recv_timeout(Duration::from_secs(10)).unwrap();
        let (_, old_first) = disconnect_during_bootstrap(initial, true, None).unwrap();
        let next = recv.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(next.generation, initial.generation + 1);
        assert!(next.verified_tick >= old_first - 1);
        let joined = run_join(next.address, next.fingerprint, next.generation, 24).unwrap();
        let hosted = host.join().unwrap().unwrap();
        assert_eq!(joined.checksum, hosted.checksum);
        assert!(hosted.snapshot >= old_first - 1);
    }
    #[test]
    fn real_quic_final_notice_disconnect_without_input_is_not_retryable() {
        use super::*;
        let (send, recv) = mpsc::channel();
        let host = thread::spawn(move || {
            run_host_with_options(
                "127.0.0.1:0".parse().unwrap(),
                101,
                LIVE_TICKS,
                Some(send),
                HostOptions {
                    pre_activation_retries: 2,
                    initial_ticks: 96,
                    ..HostOptions::default()
                },
            )
        });
        let info = recv.recv_timeout(Duration::from_secs(10)).unwrap();
        disconnect_during_bootstrap(info, true, None).unwrap();
        let error = host.join().unwrap().unwrap_err().to_string();
        assert!(
            error.contains("after final readiness notice was queued"),
            "{error}"
        );
        assert!(
            recv.try_recv().is_err(),
            "must not announce a replacement identity after activation became possible"
        );
    }
    #[test]
    fn real_quic_pre_activation_retry_requires_explicit_budget() {
        use super::*;
        let (send, recv) = mpsc::channel();
        let host = thread::spawn(move || {
            run_host_with_options(
                "127.0.0.1:0".parse().unwrap(),
                111,
                LIVE_TICKS,
                Some(send),
                HostOptions {
                    hold_first_notice: true,
                    ..HostOptions::default()
                },
            )
        });
        let info = recv.recv_timeout(Duration::from_secs(10)).unwrap();
        disconnect_during_bootstrap(info, false, None).unwrap();
        let error = host.join().unwrap().unwrap_err().to_string();
        assert!(error.contains("retry budget exhausted"), "{error}");
        assert!(recv.try_recv().is_err());
    }
    #[test]
    fn pre_activation_wait_is_bounded_despite_continuing_verified_progress() {
        use super::*;
        let error = run_host_with_options(
            "127.0.0.1:0".parse().unwrap(),
            121,
            LIVE_TICKS,
            None,
            HostOptions {
                pre_activation_retries: 2,
                join_wait: Duration::from_millis(50),
                ..HostOptions::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("pre-activation wait expired"), "{error}");
    }
    #[test]
    fn retired_host_route_ignores_late_input_control_and_disconnect() {
        use super::*;
        let old = ConnId(7);
        let fresh = ConnId(8);
        for bytes in [b"ORRI".to_vec(), b"ORRQ".to_vec(), b"ORRB".to_vec()] {
            let event = ServerEvent::Message {
                conn: old,
                channel: Channel::Reliable,
                data: bytes,
            };
            assert!(!current_host_event(&event, None));
            assert!(!current_host_event(&event, Some(fresh)));
        }
        assert!(!current_host_event(
            &ServerEvent::Disconnected(old),
            Some(fresh)
        ));
        assert!(current_host_event(
            &ServerEvent::Disconnected(fresh),
            Some(fresh)
        ));
    }
    #[test]
    fn arena_wire_is_explicit_little_endian_and_rejects_invalid_values() {
        use super::*;
        let input = ArenaInput::new(FP::from_raw(-65536), FP::ONE, true);
        let mut bytes = Vec::new();
        ArenaCodec::encode_input(&input, &mut bytes);
        assert_eq!(bytes.len(), 20);
        assert_eq!(&bytes[..8], &(-65536i64).to_le_bytes());
        assert_eq!(ArenaCodec::decode_input(&bytes), Some(input));
        bytes[16] = 2;
        assert!(ArenaCodec::decode_input(&bytes).is_none());
        assert!(ArenaCodec::decode_command(&2u32.to_le_bytes()).is_none());
        assert!(ArenaCodec::decode_command(&[0; 5]).is_none());
    }
}
