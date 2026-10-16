//! Checked-v3 controls over a deterministic three-peer loopback mesh.
//! Verified senders and generation-scoped data links remain caller responsibilities.
use orr_fp::FP;
use orr_session::{
    checked_backlog_notice, import_checked_join_ticket, serve_checked_join, CheckedJoinContext,
    InputSource, JoinBootstrap, JoinBootstrapStatus as Status, JoinRoster, LocalInputSource,
    LoopbackClock, LoopbackEnd, LoopbackNetwork, PlayerSlot, RemoteInput, Session, SessionConfig,
};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

const JOINER: PlayerSlot = PlayerSlot(2);
fn cfg(slot: u8, log: u32) -> SessionConfig {
    let mut c = SessionConfig::new(3, PlayerSlot(slot), 42, 60);
    c.join_id = 7;
    c.input_log_ticks = log;
    c.checksum_interval = 1;
    c
}
fn roster() -> JoinRoster {
    JoinRoster::completed(
        3,
        JOINER,
        PlayerSlot(0),
        vec![PlayerSlot(1), PlayerSlot(0)],
        &[],
    )
    .unwrap()
}
fn game() -> ArenaConfig {
    ArenaConfig { player_count: 3 }
}
fn bootstrap<S: InputSource<Arena>>(budget: usize) -> JoinBootstrap<Arena, S> {
    JoinBootstrap::new(cfg(2, 512), roster(), budget, 1 << 20).unwrap()
}
#[derive(Default)]
struct Mesh {
    links: Vec<LoopbackEnd<Arena>>,
    sent: Vec<(u64, PlayerSlot, ArenaInput)>,
}
impl InputSource<Arena> for Mesh {
    fn send_local(
        &mut self,
        tick: u64,
        slot: PlayerSlot,
        input: ArenaInput,
        commands: Vec<SpawnBulletCmd>,
    ) {
        self.sent.push((tick, slot, input));
        for link in &mut self.links {
            link.send_local(tick, slot, input, commands.clone());
        }
    }
    fn poll_remote(&mut self) -> Vec<RemoteInput<Arena>> {
        self.links
            .iter_mut()
            .flat_map(InputSource::poll_remote)
            .collect()
    }
}
type Peer = Session<Arena, Mesh>;
struct Env {
    a: Peer,
    b: Peer,
    clock: LoopbackClock,
}
fn drive(peer: &mut Peer) {
    let t = peer.next_send_tick();
    let x = (t % 3) as i32 - 1;
    peer.advance(ArenaInput::new(FP::from_int(x), FP::ZERO, false), vec![]);
}
impl Env {
    fn new(log: u32) -> Self {
        let (a, b, clock) = LoopbackNetwork::new::<Arena>(2, 1, 91);
        let mut c = cfg(0, log);
        c.vacant_slots = vec![JOINER];
        let mut e = Self {
            a: Session::new(
                game(),
                c,
                Mesh {
                    links: vec![a],
                    ..Mesh::default()
                },
            ),
            b: Session::new(
                game(),
                cfg(1, log),
                Mesh {
                    links: vec![b],
                    ..Mesh::default()
                },
            ),
            clock,
        };
        for _ in 0..60 {
            e.round(None);
        }
        e
    }
    fn round(&mut self, join: Option<&mut JoinBootstrap<Arena, Mesh>>) {
        self.clock.tick();
        drive(&mut self.a);
        drive(&mut self.b);
        if let Some(j) = join {
            let t = j.session().unwrap().next_send_tick();
            j.advance(
                ArenaInput::new(FP::from_int((t % 3) as i32 - 1), FP::ZERO, false),
                vec![],
            );
        }
    }
    fn snapshot(
        &mut self,
        join: &mut JoinBootstrap<Arena, Mesh>,
        gap: bool,
    ) -> (Vec<u8>, [Vec<u8>; 2], Mesh) {
        let request = join.next_request().unwrap();
        let context = CheckedJoinContext::new(
            join.config().join_id,
            join.context().unwrap().attempt(),
            roster(),
        )
        .unwrap();
        let (snapshot, checked_ticket) =
            serve_checked_join(&mut self.a, &context, &request).unwrap();
        let ticket = checked_ticket.ticket();
        if gap {
            for _ in 0..40 {
                self.round(None);
            }
        }
        // The non-donor receives serialized control bytes, never an in-process
        // donor ticket. Its expected membership and verified donor are local inputs.
        let peer_ticket =
            import_checked_join_ticket(&self.b, &context, PlayerSlot(0), &snapshot, 1 << 20)
                .unwrap();
        let notices = [
            checked_backlog_notice(&mut self.a, &context, &checked_ticket).unwrap(),
            checked_backlog_notice(&mut self.b, &context, &peer_ticket).unwrap(),
        ];
        let mut source = Mesh::default();
        for (index, peer) in [&mut self.a, &mut self.b].into_iter().enumerate() {
            let (mut outgoing, incoming) =
                LoopbackNetwork::with_clock::<Arena>(&self.clock, 2, 1, 121 + index as u64);
            for r in peer.authored_since(ticket.snapshot_tick) {
                outgoing.send_local(r.tick, r.slot, r.input, r.commands);
            }
            peer.source_mut().links.push(outgoing);
            source.links.push(incoming);
        }
        (snapshot, notices, source)
    }
    fn assert_converged(&mut self, join: &mut JoinBootstrap<Arena, Mesh>) {
        let target = self.a.verified_tick() + 80;
        for _ in 0..180 {
            self.round(Some(join));
        }
        assert!(join.caught_up_to(target).unwrap());
        let j = join.session().unwrap();
        let common = self
            .a
            .verified_tick()
            .min(self.b.verified_tick())
            .min(j.verified_tick());
        let start = j.checksums()[0].0;
        for tick in start..=common {
            let sum = |p: &Peer| p.checksums().iter().find(|(t, _)| *t == tick).unwrap().1;
            assert_eq!(sum(&self.a), sum(&self.b), "existing peers tick {tick}");
            assert_eq!(sum(&self.a), sum(j), "joiner tick {tick}");
        }
        assert!(common > start + 80);
    }
}

#[test]
fn notice_snapshot_permutations_preserve_exact_histories_and_checksums() {
    let mut baseline = None;
    let mut previous_env = Env::new(512);
    let mut previous_config = cfg(2, 512);
    previous_config.join_id = 6;
    let mut previous =
        JoinBootstrap::<Arena, Mesh>::new(previous_config, roster(), 4096, 1 << 20).unwrap();
    let (stale_snapshot, stale_notices, _) = previous_env.snapshot(&mut previous, false);
    // Every ordering of two member notices and the snapshot.
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut env = Env::new(512);
        let mut join = bootstrap::<Mesh>(4096);
        let (snapshot, notices, source) = env.snapshot(&mut join, false);
        let mut source = Some(source);
        for event in order {
            // Interleave stale traffic in every real loopback permutation.
            // The old generation has identical attempt, slot, and membership.
            if join.session().is_none() {
                assert_snapshot_rejected(&mut join, 0, &stale_snapshot);
            }
            for (sender, stale) in stale_notices.iter().enumerate() {
                assert_notice_rejected(&mut join, sender as u8, stale);
            }
            if event == 2 {
                join.receive_snapshot(PlayerSlot(0), game(), source.take().unwrap(), &snapshot)
                    .unwrap();
            } else {
                join.receive_notice(PlayerSlot(event), &notices[event as usize])
                    .unwrap();
                join.receive_notice(PlayerSlot(event), &notices[event as usize])
                    .unwrap();
            }
        }
        for (sender, stale) in stale_notices.iter().enumerate() {
            assert_notice_rejected(&mut join, sender as u8, stale);
        }
        assert_eq!(join.status().unwrap(), Status::Ready);
        // Notices alone did not deliver/poll the queued inputs or catch up.
        let snapshot_tick = join.session().unwrap().verified_tick();
        assert!(!join.caught_up_to(snapshot_tick + 1).unwrap());
        env.assert_converged(&mut join);
        let j = join.session().unwrap();
        let result = (
            j.checksums().to_vec(),
            j.source().sent.clone(),
            env.a.source().sent.clone(),
            env.b.source().sent.clone(),
        );
        if let Some(ref expected) = baseline {
            assert_eq!(&result, expected);
        } else {
            baseline = Some(result);
        }
    }
}

// Wire edits below are adversarial control fixtures, never authentication claims.
fn rehash(bytes: &mut [u8]) {
    let end = bytes.len() - 8;
    let hash = xxhash_rust::xxh3::xxh3_64(&bytes[..end]);
    bytes[end..].copy_from_slice(&hash.to_le_bytes());
}
fn payload_range(bytes: &[u8]) -> std::ops::Range<usize> {
    let peers = u32::from_le_bytes(bytes[23..27].try_into().unwrap()) as usize;
    let start = 31 + peers;
    start..bytes.len() - 8
}
fn raw_payload(bytes: &[u8]) -> Vec<u8> {
    bytes[payload_range(bytes)].to_vec()
}
fn edit_inner(bytes: &[u8], edit: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let mut changed = bytes.to_vec();
    let range = payload_range(bytes);
    edit(&mut changed[range.clone()]);
    rehash(&mut changed[range]);
    rehash(&mut changed);
    changed
}
fn assert_notice_rejected(join: &mut JoinBootstrap<Arena, Mesh>, sender: u8, bytes: &[u8]) {
    let usage = join.notice_usage();
    let status = join.status().unwrap();
    let state = join.session().map(|s| {
        (
            s.verified_tick(),
            s.next_send_tick(),
            s.checksums().to_vec(),
            s.source().sent.clone(),
        )
    });
    assert!(join.receive_notice(PlayerSlot(sender), bytes).is_err());
    assert_eq!(join.notice_usage(), usage);
    assert_eq!(join.status().unwrap(), status);
    assert_eq!(
        join.session().map(|s| (
            s.verified_tick(),
            s.next_send_tick(),
            s.checksums().to_vec(),
            s.source().sent.clone()
        )),
        state
    );
}
fn assert_snapshot_rejected(join: &mut JoinBootstrap<Arena, Mesh>, sender: u8, bytes: &[u8]) {
    let usage = join.notice_usage();
    let status = join.status().unwrap();
    let state = join.session().map(|s| {
        (
            s.verified_tick(),
            s.next_send_tick(),
            s.checksums().to_vec(),
            s.source().sent.clone(),
        )
    });
    assert!(join
        .receive_snapshot(PlayerSlot(sender), game(), Mesh::default(), bytes)
        .is_err());
    assert_eq!(join.notice_usage(), usage);
    assert_eq!(join.status().unwrap(), status);
    assert_eq!(
        join.session().map(|s| (
            s.verified_tick(),
            s.next_send_tick(),
            s.checksums().to_vec(),
            s.source().sent.clone()
        )),
        state
    );
}

#[test]
fn previous_join_same_attempt_slot_and_roster_is_rejected_before_and_after_snapshot() {
    let mut old_env = Env::new(512);
    let mut old = bootstrap::<Mesh>(4096);
    let (old_snapshot, old_notices, _) = old_env.snapshot(&mut old, false);
    let mut env = Env::new(512);
    let mut config = cfg(2, 512);
    config.join_id = 8;
    let mut join = JoinBootstrap::<Arena, Mesh>::new(config, roster(), 4096, 1 << 20).unwrap();
    let (snapshot, notices, source) = env.snapshot(&mut join, false);
    // Both independent generations used attempt 1 and exactly the same roster.
    assert_eq!(&snapshot[16..20], &old_snapshot[16..20]);
    assert_ne!(&snapshot[8..16], &old_snapshot[8..16]);
    join.receive_notice(PlayerSlot(0), &notices[0]).unwrap();
    assert_snapshot_rejected(&mut join, 0, &old_snapshot);
    assert_notice_rejected(&mut join, 1, &old_notices[1]);
    join.receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    assert_notice_rejected(&mut join, 1, &old_notices[1]);
    assert!(join.session().unwrap().source().sent.is_empty());
    join.receive_notice(PlayerSlot(1), &notices[1]).unwrap();
    env.assert_converged(&mut join);
}

#[test]
fn malformed_mismatched_and_raw_v2_controls_are_transactional() {
    let mut env = Env::new(512);
    let mut join = bootstrap::<Mesh>(4096);
    let (snapshot, notices, source) = env.snapshot(&mut join, false);
    join.receive_notice(PlayerSlot(0), &notices[0]).unwrap();
    let mut snapshots = vec![
        raw_payload(&snapshot),
        snapshot[..snapshot.len() - 1].to_vec(),
    ];
    let mut bad_hash = snapshot.clone();
    bad_hash[0] ^= 1;
    snapshots.push(bad_hash);
    snapshots.push(edit_inner(&snapshot, |inner| {
        let attempt = inner.len() - 12;
        inner[attempt..attempt + 4].copy_from_slice(&2u32.to_le_bytes());
    }));
    snapshots.push(edit_inner(&snapshot, |inner| inner[29] = 1));
    let mut excessive_length = snapshot.clone();
    let length = payload_range(&snapshot).start - 4;
    excessive_length[length..length + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    rehash(&mut excessive_length);
    snapshots.push(excessive_length);
    let mut trailing = snapshot.clone();
    trailing.insert(trailing.len() - 8, 0);
    rehash(&mut trailing);
    snapshots.push(trailing);
    let mut oversized_roster = snapshot.clone();
    oversized_roster[23..27].copy_from_slice(&u32::MAX.to_le_bytes());
    rehash(&mut oversized_roster);
    snapshots.push(oversized_roster);
    let mut unsorted = snapshot.clone();
    unsorted.swap(27, 28);
    rehash(&mut unsorted);
    snapshots.push(unsorted);
    let mut unknown_version = snapshot.clone();
    unknown_version[4..8].copy_from_slice(&99u32.to_le_bytes());
    rehash(&mut unknown_version);
    snapshots.push(unknown_version);
    snapshots.push(edit_inner(&snapshot, |inner| {
        inner[4..8].copy_from_slice(&99u32.to_le_bytes())
    }));
    snapshots.push(edit_inner(&snapshot, |inner| {
        inner[54..58].copy_from_slice(&u32::MAX.to_le_bytes())
    }));
    for bytes in &snapshots {
        assert_snapshot_rejected(&mut join, 0, bytes);
    }
    assert_snapshot_rejected(&mut join, 1, &snapshot);

    let mut rejected_notices = vec![raw_payload(&notices[1]), notices[1][..10].to_vec()];
    rejected_notices.push(edit_inner(&notices[1], |inner| {
        inner[8..12].copy_from_slice(&2u32.to_le_bytes())
    }));
    rejected_notices.push(edit_inner(&notices[1], |inner| inner[12] = 0));
    let mut bad_hash = notices[1].clone();
    bad_hash[0] ^= 1;
    rejected_notices.push(bad_hash);
    rejected_notices.push(edit_inner(&notices[1], |inner| {
        inner[4..8].copy_from_slice(&99u32.to_le_bytes())
    }));
    rejected_notices.push(edit_inner(&notices[1], |inner| {
        inner[13..17].copy_from_slice(&u32::MAX.to_le_bytes())
    }));
    rejected_notices.push(edit_inner(&notices[1], |inner| inner[17] = 255));
    rejected_notices.push(edit_inner(&notices[1], |inner| {
        inner[26..34].copy_from_slice(&0u64.to_le_bytes())
    }));
    for bytes in &rejected_notices {
        assert_notice_rejected(&mut join, 1, bytes);
    }
    join.receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    for bytes in &rejected_notices {
        assert_notice_rejected(&mut join, 1, bytes);
    }
    assert_notice_rejected(&mut join, 2, &notices[1]);
    join.receive_notice(PlayerSlot(1), &notices[1]).unwrap();
    assert_eq!(join.status().unwrap(), Status::Ready);
    env.assert_converged(&mut join);
}

#[test]
fn natural_gap_fresh_retry_rejects_previous_attempt_and_converges() {
    let mut env = Env::new(0);
    let mut join = bootstrap::<Mesh>(4096);
    let (old_snapshot, old_notices, source) = env.snapshot(&mut join, true);
    for (i, notice) in old_notices.iter().enumerate() {
        join.receive_notice(PlayerSlot(i as u8), notice).unwrap();
    }
    assert!(matches!(
        join.receive_snapshot(PlayerSlot(0), game(), source, &old_snapshot)
            .unwrap(),
        Status::InputGap {
            slot: PlayerSlot(1),
            ..
        }
    ));
    assert!(!join.caught_up_to(0).unwrap());
    assert!(join.session().unwrap().source().sent.is_empty());
    join.cancel().unwrap();
    assert_eq!(join.notice_usage(), (0, 0));
    // Caller retires the old data links. Control fencing does not scope input packets.
    env.a.source_mut().links.truncate(1);
    env.b.source_mut().links.truncate(1);
    let (snapshot, notices, source) = env.snapshot(&mut join, false);
    assert_snapshot_rejected(&mut join, 0, &old_snapshot);
    for (sender, notice) in old_notices.iter().enumerate() {
        assert_notice_rejected(&mut join, sender as u8, notice);
    }
    join.receive_notice(PlayerSlot(1), &notices[1]).unwrap();
    join.receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    assert_notice_rejected(&mut join, 0, &old_notices[0]);
    join.receive_notice(PlayerSlot(0), &notices[0]).unwrap();
    env.assert_converged(&mut join);
}

#[test]
fn explicit_legacy_bootstrap_accepts_raw_v2_without_claiming_generation_fencing() {
    let mut env = Env::new(512);
    let mut checked = bootstrap::<Mesh>(4096);
    let (snapshot, notices, source) = env.snapshot(&mut checked, false);
    let mut legacy =
        JoinBootstrap::<Arena, Mesh>::new_legacy_v2(cfg(2, 512), roster(), 4096).unwrap();
    let request = legacy.next_request().unwrap();
    assert_eq!(&request[4..8], &2u32.to_le_bytes());
    legacy
        .receive_notice(PlayerSlot(1), &raw_payload(&notices[1]))
        .unwrap();
    legacy
        .receive_snapshot(PlayerSlot(0), game(), source, &raw_payload(&snapshot))
        .unwrap();
    legacy
        .receive_notice(PlayerSlot(0), &raw_payload(&notices[0]))
        .unwrap();
    assert_eq!(legacy.status().unwrap(), Status::Ready);
    env.assert_converged(&mut legacy);
}

#[test]
fn delayed_member_fences_exact_authored_history_until_complete_coverage() {
    let mut env = Env::new(512);
    let mut join = bootstrap::<Mesh>(4096);
    let (snapshot, notices, source) = env.snapshot(&mut join, false);
    join.receive_notice(PlayerSlot(0), &notices[0]).unwrap();
    join.receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    for _ in 0..5 {
        env.round(Some(&mut join));
        assert_eq!(
            join.receive_notice(PlayerSlot(0), &notices[0]).unwrap(),
            Status::Syncing {
                received: 1,
                expected: 2
            }
        );
        assert!(join.session().unwrap().source().sent.is_empty());
        assert!(!join.caught_up_to(0).unwrap());
    }
    let authored: Vec<_> = join
        .session()
        .unwrap()
        .authored_since(0)
        .into_iter()
        .map(|r| (r.tick, r.slot, r.input))
        .collect();
    assert!(!authored.is_empty());
    assert_eq!(
        join.receive_notice(PlayerSlot(1), &notices[1]).unwrap(),
        Status::Ready
    );
    assert_eq!(join.session().unwrap().source().sent, authored);
    env.assert_converged(&mut join);
}

fn four_slot_roster(donor: u8, active: [u8; 2], vacant: u8) -> JoinRoster {
    JoinRoster::completed(
        4,
        PlayerSlot(3),
        PlayerSlot(donor),
        active.into_iter().map(PlayerSlot).collect(),
        &[PlayerSlot(vacant)],
    )
    .unwrap()
}
fn four_slot_controls(roster: JoinRoster) -> (Vec<u8>, Vec<u8>) {
    let mut config = SessionConfig::new(4, PlayerSlot(3), 42, 60);
    config.join_id = 7;
    let mut join =
        JoinBootstrap::<Arena, LocalInputSource>::new(config, roster.clone(), 4096, 1 << 20)
            .unwrap();
    let request = join.next_request().unwrap();
    let context = CheckedJoinContext::new(7, 1, roster.clone()).unwrap();
    let mut donor_config = SessionConfig::new(4, roster.donor(), 42, 60);
    donor_config.vacant_slots = vec![PlayerSlot(3)];
    let mut donor = Session::<Arena, LocalInputSource>::new(
        ArenaConfig { player_count: 4 },
        donor_config,
        LocalInputSource,
    );
    let (snapshot, ticket) = serve_checked_join(&mut donor, &context, &request).unwrap();
    let notice = checked_backlog_notice(&mut donor, &context, &ticket).unwrap();
    (snapshot, notice)
}

#[test]
fn same_count_changed_donor_and_active_vacant_identities_do_not_count() {
    let expected = four_slot_roster(0, [0, 1], 2);
    let (snapshot, good) = four_slot_controls(expected.clone());
    let changed = [
        four_slot_roster(1, [0, 1], 2),
        four_slot_roster(0, [0, 2], 1),
    ];
    let mut config = SessionConfig::new(4, PlayerSlot(3), 42, 60);
    config.join_id = 7;
    let mut join = JoinBootstrap::<Arena, Mesh>::new(config, expected, 4096, 1 << 20).unwrap();
    join.next_request().unwrap();
    join.receive_notice(PlayerSlot(0), &good).unwrap();
    let usage = join.notice_usage();
    let status = join.status().unwrap();
    let mut bad_notices = Vec::new();
    for alternative in changed {
        let sender = alternative.donor();
        let (bad_snapshot, bad_notice) = four_slot_controls(alternative);
        // Route under the expected donor too: reject the encoded manifest,
        // independently of caller-verified sender rejection.
        assert!(join
            .receive_snapshot(
                PlayerSlot(0),
                ArenaConfig { player_count: 4 },
                Mesh::default(),
                &bad_snapshot
            )
            .is_err());
        assert!(join.receive_notice(sender, &bad_notice).is_err());
        assert_eq!(join.notice_usage(), usage);
        assert_eq!(join.status().unwrap(), status);
        assert!(join.session().is_none());
        bad_notices.push((sender, bad_notice));
    }
    join.receive_snapshot(
        PlayerSlot(0),
        ArenaConfig { player_count: 4 },
        Mesh::default(),
        &snapshot,
    )
    .unwrap();
    for (sender, bytes) in bad_notices {
        assert_notice_rejected(&mut join, sender.0, &bytes);
    }
    assert_eq!(join.notice_usage(), usage);
    assert!(join.session().unwrap().source().sent.is_empty());
}

#[test]
fn stale_or_inner_mismatched_requests_do_not_grant_and_stale_tickets_do_not_hold() {
    let mut env = Env::new(512);
    let mut old = bootstrap::<Mesh>(4096);
    let old_request = old.next_request().unwrap();
    let old_context = CheckedJoinContext::new(7, 1, roster()).unwrap();
    let expected = CheckedJoinContext::new(8, 1, roster()).unwrap();
    let sent = env.a.source().sent.clone();
    assert!(serve_checked_join(&mut env.a, &expected, &old_request).is_err());
    let bad_request = edit_inner(&old_request, |inner| {
        inner[30..38].copy_from_slice(&8u64.to_le_bytes())
    });
    let mut rejected = vec![
        bad_request,
        raw_payload(&old_request),
        old_request[..10].to_vec(),
    ];
    let mut bad_checksum = old_request.clone();
    bad_checksum[8] ^= 1;
    rejected.push(bad_checksum);
    for (offset, value) in [(0, b'X'), (4, 4)] {
        let mut bad = old_request.clone();
        bad[offset] = value;
        rehash(&mut bad);
        rejected.push(bad);
    }
    for offset in [23, payload_range(&old_request).start - 4] {
        let mut bad = old_request.clone();
        bad[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        rehash(&mut bad);
        rejected.push(bad);
    }
    let mut trailing = old_request.clone();
    trailing.insert(trailing.len() - 8, 0);
    rehash(&mut trailing);
    rejected.push(trailing);
    let mut unsorted = old_request.clone();
    unsorted.swap(27, 28);
    rehash(&mut unsorted);
    rejected.push(unsorted);
    let before = (
        env.a.verified_tick(),
        env.a.next_send_tick(),
        env.a.checksums().to_vec(),
    );
    for request in rejected {
        assert!(serve_checked_join(&mut env.a, &old_context, &request).is_err());
        assert!(env.a.pending_join(JOINER).is_none());
        assert_eq!(env.a.source().sent, sent);
        assert_eq!(
            (
                env.a.verified_tick(),
                env.a.next_send_tick(),
                env.a.checksums().to_vec()
            ),
            before
        );
    }
    assert!(env.a.pending_join(JOINER).is_none());
    assert_eq!(env.a.source().sent, sent);
    let (_, ticket) = serve_checked_join(&mut env.a, &old_context, &old_request).unwrap();
    assert!(env.b.backlog_notice(JOINER).is_none());
    let sent = env.b.source().sent.clone();
    assert!(checked_backlog_notice(&mut env.b, &expected, &ticket).is_err());
    let retry_context = CheckedJoinContext::new(7, 2, roster()).unwrap();
    assert!(checked_backlog_notice(&mut env.b, &retry_context, &ticket).is_err());
    assert!(env.b.backlog_notice(JOINER).is_none());
    assert_eq!(env.b.source().sent, sent);
    checked_backlog_notice(&mut env.b, &old_context, &ticket).unwrap();
    assert!(env.b.backlog_notice(JOINER).is_some());
}

#[test]
fn outer_wire_budgets_bound_staging_and_snapshot_without_mutation() {
    let mut env = Env::new(512);
    let mut fixture = bootstrap::<Mesh>(4096);
    let (snapshot, notices, source) = env.snapshot(&mut fixture, false);
    let budget = notices[0].len() + notices[1].len() - 1;
    let mut join =
        JoinBootstrap::<Arena, Mesh>::new(cfg(2, 512), roster(), budget, snapshot.len() - 1)
            .unwrap();
    join.next_request().unwrap();
    join.receive_notice(PlayerSlot(0), &notices[0]).unwrap();
    assert_eq!(join.notice_usage(), (1, notices[0].len()));
    assert_notice_rejected(&mut join, 1, &notices[1]);
    assert_snapshot_rejected(&mut join, 0, &snapshot);
    // An exact duplicate consumes neither another slot nor more wire bytes.
    join.receive_notice(PlayerSlot(0), &notices[0]).unwrap();
    assert_eq!(join.notice_usage(), (1, notices[0].len()));
    join.cancel();
    assert_eq!(join.notice_usage(), (0, 0));
    let exact_budget = notices[0].len() + notices[1].len();
    let mut exact =
        JoinBootstrap::<Arena, Mesh>::new(cfg(2, 512), roster(), exact_budget, snapshot.len())
            .unwrap();
    exact.next_request().unwrap();
    exact.receive_notice(PlayerSlot(0), &notices[0]).unwrap();
    exact
        .receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    exact.receive_notice(PlayerSlot(1), &notices[1]).unwrap();
    assert_eq!(exact.notice_usage(), (2, exact_budget));
    env.assert_converged(&mut exact);
}

#[test]
fn byte_import_validates_context_donor_and_snapshot_before_installing_any_hold() {
    let mut env = Env::new(512);
    let mut join = bootstrap::<Mesh>(4096);
    let request = join.next_request().unwrap();
    let expected = CheckedJoinContext::new(7, 1, roster()).unwrap();
    let (snapshot, donor_ticket) = serve_checked_join(&mut env.a, &expected, &request).unwrap();
    let stale_generation = CheckedJoinContext::new(8, 1, roster()).unwrap();
    let stale_attempt = CheckedJoinContext::new(7, 2, roster()).unwrap();
    let before = (
        env.b.verified_tick(),
        env.b.next_send_tick(),
        env.b.checksums().to_vec(),
        env.b.source().sent.clone(),
    );
    let mut corrupt = snapshot.clone();
    corrupt[0] ^= 1;
    let malformed_length = edit_inner(&snapshot, |inner| {
        inner[54..58].copy_from_slice(&u32::MAX.to_le_bytes())
    });
    let wrong_seed = edit_inner(&snapshot, |inner| {
        inner[16..24].copy_from_slice(&43u64.to_le_bytes())
    });
    let mut failures = vec![
        (
            &stale_generation,
            PlayerSlot(0),
            snapshot.as_slice(),
            snapshot.len(),
        ),
        (
            &stale_attempt,
            PlayerSlot(0),
            snapshot.as_slice(),
            snapshot.len(),
        ),
        (
            &expected,
            PlayerSlot(1),
            snapshot.as_slice(),
            snapshot.len(),
        ),
        (
            &expected,
            PlayerSlot(0),
            snapshot.as_slice(),
            snapshot.len() - 1,
        ),
        (&expected, PlayerSlot(0), snapshot.as_slice(), 0),
    ];
    for bytes in [&corrupt, &malformed_length, &wrong_seed] {
        failures.push((&expected, PlayerSlot(0), bytes.as_slice(), 1 << 20));
    }
    for (index, (context, donor, bytes, limit)) in failures.into_iter().enumerate() {
        assert!(
            import_checked_join_ticket(&env.b, context, donor, bytes, limit).is_err(),
            "import rejection case {index}"
        );
        assert!(env.b.backlog_notice(JOINER).is_none());
        assert!(env.b.pending_join(JOINER).is_none());
        assert_eq!(
            (
                env.b.verified_tick(),
                env.b.next_send_tick(),
                env.b.checksums().to_vec(),
                env.b.source().sent.clone()
            ),
            before
        );
    }
    // A zero build id intentionally retains the legacy wildcard. Exercise a
    // real mismatch against a peer with a concrete nonzero build identity.
    let mut strict_config = cfg(1, 512);
    strict_config.build_id = 123;
    let strict_peer = Session::<Arena, Mesh>::new(game(), strict_config, Mesh::default());
    assert_ne!(strict_peer.build_hash(), 0);
    let mismatched_hash: u64 = if strict_peer.build_hash() == 1 { 2 } else { 1 };
    let wrong_build = edit_inner(&snapshot, |inner| {
        inner[8..16].copy_from_slice(&mismatched_hash.to_le_bytes())
    });
    assert!(import_checked_join_ticket(
        &strict_peer,
        &expected,
        PlayerSlot(0),
        &wrong_build,
        wrong_build.len()
    )
    .is_err());
    assert!(strict_peer.backlog_notice(JOINER).is_none());
    assert!(strict_peer.pending_join(JOINER).is_none());
    assert!(strict_peer.source().sent.is_empty());
    // Exact-limit import is read-only; installing the imported ticket is explicit.
    let imported =
        import_checked_join_ticket(&env.b, &expected, PlayerSlot(0), &snapshot, snapshot.len())
            .unwrap();
    assert_eq!(imported.context(), &expected);
    assert_eq!(imported.ticket(), donor_ticket.ticket());
    assert!(env.b.backlog_notice(JOINER).is_none());
    checked_backlog_notice(&mut env.b, &expected, &imported).unwrap();
    let held_notice = env.b.backlog_notice(JOINER).unwrap();
    assert!(import_checked_join_ticket(
        &env.b,
        &stale_attempt,
        PlayerSlot(0),
        &snapshot,
        snapshot.len()
    )
    .is_err());
    assert_eq!(env.b.backlog_notice(JOINER).unwrap(), held_notice);
}
