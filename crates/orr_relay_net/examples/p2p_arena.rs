//! Finite two-peer Arena over pinned direct QUIC. No relay, TUI, discovery or mesh.
//! `p2p_arena host [bind] [generation]` prints the exact join command.
//! `p2p_arena join <address> <sha256> <generation>` joins slot 1.
//! `p2p_arena smoke` runs both peers over real loopback sockets and checks every
//! available verified checksum against an independently stepped simulation.
//! The host admits the first connected client for this local-development demo;
//! a generation is a fence, not a credential or production client authentication.

#![allow(clippy::disallowed_types)] // Wall clock belongs to this transport driver, never the simulation.

use std::error::Error;
use std::net::SocketAddr;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use orr_net::SendError;
use orr_proto::{Channel, Endpoint, Link, LinkEvent, ServerEvent};
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
const WINDOW_END: u64 = 4097;
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
        end_tick: WINDOW_END,
        ..P2pInputLimits::default()
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
}
fn reference(target: u64, first_input: u64) -> Vec<u64> {
    let mut sim = Simulation::<Arena>::with_build_id(game(), 60, 42, BUILD_ID);
    let mut out = vec![sim.frame().checksum()];
    for tick in 1..=target {
        let mut inputs = TickInputs::new(tick, 2);
        for slot in [HOST, JOINER] {
            if tick > 2 && (slot == HOST || tick >= first_input) {
                inputs.set_input(slot, input(slot, tick));
                for command in commands(slot) {
                    inputs.push_command(slot, command);
                }
            }
        }
        sim.step(&inputs);
        out.push(sim.frame().checksum());
    }
    out
}
fn verified_report(peer: &Peer, snapshot: u64, first_input: u64, target: u64) -> Result<Report> {
    if peer.verified_tick() < target {
        return Err("target is not verified".into());
    }
    let reference = reference(target, first_input);
    let mut checked = 0;
    for &(tick, checksum) in peer.checksums() {
        if tick <= target {
            if checksum != reference[tick as usize] {
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
fn run_host(
    bind: SocketAddr,
    generation: u64,
    announce: Option<mpsc::Sender<(SocketAddr, [u8; 32])>>,
) -> Result<Report> {
    let mut opts = ListenOptions::new(bind, TransportKind::Quic);
    opts.net = net_config();
    let mut endpoint = listen(&opts)?;
    let ctx = context(generation)?;
    let (driver, source) = P2pInputDriver::<Arena, ArenaCodec>::new(HOST, ctx, limits())?;
    let mut peer = Peer::new(game(), config(HOST, generation), source);
    let mut members = P2pMembership::new(2, HOST, CONTROL_LIMIT * 2)?;
    assert!(members.admit_local()?.is_empty());
    assert!(members.commit_vacant(JOINER)?.is_empty());
    for _ in 0..24 {
        drive(&mut peer, &driver)?;
    }
    peer.poll_confirmed();
    driver.check()?;
    let addr = endpoint.local_addr();
    let fp = endpoint
        .cert_sha256()
        .ok_or("missing generated fingerprint")?;
    println!(
        "HOST_RUNNING verified={} vacant_slot=1 address={addr} generation={generation}",
        peer.verified_tick()
    );
    println!(
        "JOIN_COMMAND p2p_arena join {addr} {} {generation}",
        format_fingerprint(&fp)
    );
    if let Some(announce) = announce {
        announce.send((addr, fp))?;
    }
    let mut attempt: Option<P2pAttempt> = None;
    let mut connection = None;
    let mut grant = None;
    let mut own_report: Option<Report> = None;
    let mut remote_report = None;
    let mut ack_sent = false;
    let start = Instant::now();
    let mut next_tick = Instant::now();
    let result = (|| -> Result<Report> {
        loop {
            if start.elapsed() > Duration::from_secs(60) {
                return Err("host timed out waiting for bounded join/completion".into());
            }
            driver.check()?;
            for _ in 0..256 {
                let Some(event) = endpoint.poll() else {
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
                    continue;
                }
                if let ServerEvent::Disconnected(conn) = &event {
                    if Some(*conn) != connection {
                        continue;
                    }
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
                            .checked_add(LIVE_TICKS)
                            .ok_or("tick overflow")?;
                        if target + 64 >= WINDOW_END {
                            return Err("join arrived too late for finite tick window".into());
                        }
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
                        grant = Some((raw.snapshot_tick, raw.first_input_tick, target));
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
                    P2pRoutedEvent::Disconnected { cleanup, .. } => {
                        for clean in cleanup {
                            let _ = clean.release_local_hold(&mut peer);
                            clean.retire_exclusive_connections(&mut endpoint);
                        }
                        attempt = None;
                        driver.cancel();
                        if ack_sent {
                            return own_report
                                .clone()
                                .ok_or_else(|| "missing host report".into());
                        }
                        return Err(
                            "joiner disconnected; owned holds, link and input buffers retired"
                                .into(),
                        );
                    }
                    other => return Err(format!("unexpected host event: {other:?}").into()),
                }
            }
            let controls_done = check_flush(
                members.flush(|conn, _, bytes| endpoint.try_send_reliable(conn, bytes)),
            )?;
            if grant.is_some() && controls_done {
                driver.flush(|conn, bytes| endpoint.try_send_reliable(conn, bytes))?;
            }
            peer.poll_confirmed();
            driver.check()?;
            if Instant::now() >= next_tick {
                if grant.is_none_or(|(_, _, target)| peer.head_tick() < target) {
                    drive(&mut peer, &driver)?;
                }
                next_tick = Instant::now() + Duration::from_millis(16);
            }
            if let Some((snapshot, first_input, target)) = grant {
                if peer.verified_tick() >= target && own_report.is_none() {
                    let report = verified_report(&peer, snapshot, first_input, target)?;
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

fn run_join(addr: SocketAddr, fingerprint: [u8; 32], generation: u64) -> Result<Report> {
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
    let (driver, source) =
        P2pInputDriver::<Arena, ArenaCodec>::new(JOINER, context(generation)?, limits())?;
    let mut source = Some(source);
    let mut attempt = None;
    let mut grant = None;
    let mut ready = false;
    let mut own_report = None;
    let mut report_sent = false;
    let start = Instant::now();
    let mut next_tick = Instant::now();
    let result = (|| -> Result<Report> {
        loop {
            if start.elapsed() > Duration::from_secs(60) {
                return Err("join timed out waiting for bootstrap/completion".into());
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
                                first_input.checked_add(LIVE_TICKS).ok_or("tick overflow")?;
                            if target + 64 >= WINDOW_END {
                                return Err("snapshot outside finite tick window".into());
                            }
                            driver.bind(conn, a.context(), snapshot, first_input)?;
                            grant = Some((snapshot, first_input, target));
                        } else if data.starts_with(b"ORRB") {
                            members.receive_notice(a, conn, &mut bootstrap, &data)?;
                        } else {
                            return Err("unexpected join control".into());
                        }
                        if bootstrap.status()? == JoinBootstrapStatus::Ready && !ready {
                            ready = true;
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
                        snapshot,
                        first_input,
                        target,
                    )?;
                    println!("JOIN_CAUGHT_UP verified={target} checksum={:016x} reference_checkpoints={}", report.checksum, report.checked);
                    own_report = Some(report);
                }
            }
            if let Some(ours) = &own_report {
                if !report_sent && driver.pending().0 == 0 {
                    match link.try_send_reliable(&report_bytes(generation, 1, ours)) {
                        Ok(()) => report_sent = true,
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
fn smoke() -> Result<()> {
    let (send, recv) = mpsc::channel();
    let host = thread::spawn(move || run_host("127.0.0.1:0".parse().unwrap(), 77, Some(send)));
    let (addr, fingerprint) = recv.recv_timeout(Duration::from_secs(10))?;
    let joined = run_join(addr, fingerprint, 77);
    let hosted = host.join().map_err(|_| "host thread panicked")?;
    let join = joined?;
    let host = hosted?;
    if (join.tick, join.checksum, join.snapshot, join.first_input)
        != (host.tick, host.checksum, host.snapshot, host.first_input)
    {
        return Err("loopback reports disagree".into());
    }
    println!("SMOKE_OK real_quic=true backlog_commands=nonempty live_commands=ordered_and_repeated verified_tick={} checksum={:016x}", host.tick, host.checksum);
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("host") if args.len() <= 3 => { run_host(args.get(1).map_or("127.0.0.1:7000", String::as_str).parse()?, args.get(2).map(|s| s.parse()).transpose()?.unwrap_or_else(|| fresh_seed().max(1)), None)?; }
        Some("join") if args.len() == 4 => { run_join(args[1].parse()?, parse_fingerprint(&args[2])?, args[3].parse()?)?; }
        Some("smoke") if args.len() == 1 => smoke()?,
        _ => return Err("usage: p2p_arena host [bind] [generation] | join <addr> <fingerprint> <generation> | smoke".into()),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn real_quic_late_join_ordered_commands() {
        super::smoke().unwrap();
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
