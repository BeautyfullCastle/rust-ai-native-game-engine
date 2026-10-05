//! Explicit legacy-v2 P2P bootstrap, using the unchanged v2 wire messages and loopback inputs.
use orr_fp::FP;
use orr_session::{
    InputSource, JoinBootstrap, JoinBootstrapError as Error, JoinBootstrapStatus as Status,
    JoinError, JoinRoster, LocalInputSource, LoopbackClock, LoopbackEnd, LoopbackNetwork,
    PlayerSlot, RemoteInput, Session, SessionConfig,
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
    JoinBootstrap::new_legacy_v2(cfg(2, 512), roster(), budget).unwrap()
}
fn notice(attempt: u32, sender: u8, spans: &[(u8, u64, u64)]) -> Vec<u8> {
    let mut bytes = b"ORRB".to_vec();
    bytes.extend(2u32.to_le_bytes());
    bytes.extend(attempt.to_le_bytes());
    bytes.push(sender);
    bytes.extend((spans.len() as u32).to_le_bytes());
    for &(slot, from, until) in spans {
        bytes.push(slot);
        bytes.extend(from.to_le_bytes());
        bytes.extend(until.to_le_bytes());
    }
    bytes.extend(xxhash_rust::xxh3::xxh3_64(&bytes).to_le_bytes());
    bytes
}

#[test]
fn roster_requires_completed_distinct_membership_and_never_uses_zero() {
    for (active, vacant) in [
        (vec![], vec![PlayerSlot(0), PlayerSlot(1)]),
        (vec![PlayerSlot(0)], vec![]),
        (vec![PlayerSlot(0), PlayerSlot(0)], vec![]),
        (vec![PlayerSlot(0), JOINER], vec![PlayerSlot(1)]),
        (vec![PlayerSlot(0), PlayerSlot(1)], vec![PlayerSlot(1)]),
        (vec![PlayerSlot(0), PlayerSlot(3)], vec![]),
        (vec![PlayerSlot(1)], vec![PlayerSlot(0)]),
    ] {
        assert!(JoinRoster::completed(3, JOINER, PlayerSlot(0), active, &vacant).is_err());
    }
    let vacant = JoinRoster::completed(
        4,
        PlayerSlot(3),
        PlayerSlot(0),
        vec![PlayerSlot(0), PlayerSlot(2)],
        &[PlayerSlot(1)],
    )
    .unwrap();
    assert_eq!(vacant.peers(), &[PlayerSlot(0), PlayerSlot(2)]);
    let mut c = cfg(2, 512);
    c.join_backlog_peers = 999;
    let b = JoinBootstrap::<Arena, LocalInputSource>::new_legacy_v2(c, roster(), 1024).unwrap();
    assert_eq!(b.config().join_backlog_peers, 2);
    assert!(JoinBootstrap::<Arena, LocalInputSource>::new_legacy_v2(cfg(2, 512), roster(), 0).is_err());
    let mut c = cfg(2, 512);
    c.relay = true;
    assert!(JoinBootstrap::<Arena, LocalInputSource>::new_legacy_v2(c, roster(), 1024).is_err());
}

#[test]
fn notices_are_bounded_validated_and_transactional() {
    let a = notice(1, 0, &[(0, 1, u64::MAX)]);
    let b = notice(1, 1, &[(1, 1, u64::MAX)]);
    let mut join = bootstrap::<LocalInputSource>(a.len() + b.len() - 1);
    assert!(matches!(
        join.receive_notice(PlayerSlot(0), &a),
        Err(Error::Inactive)
    ));
    join.next_request().unwrap();
    assert_eq!(
        join.receive_notice(PlayerSlot(0), &a).unwrap(),
        Status::AwaitingSnapshot {
            received: 1,
            expected: 2
        }
    );
    let used = join.notice_usage();
    assert_eq!(
        join.receive_notice(PlayerSlot(0), &a).unwrap(),
        Status::AwaitingSnapshot {
            received: 1,
            expected: 2
        }
    );
    assert!(matches!(
        join.receive_notice(PlayerSlot(1), &b),
        Err(Error::NoticeBudget { .. })
    ));
    assert!(matches!(
        join.receive_notice(JOINER, &b),
        Err(Error::NonMember(_))
    ));
    assert!(matches!(
        join.receive_notice(PlayerSlot(0), &b),
        Err(Error::SenderMismatch)
    ));
    let mut bad = a.clone();
    bad[0] ^= 1;
    assert!(matches!(
        join.receive_notice(PlayerSlot(0), &bad),
        Err(Error::Join(JoinError::BadMessageChecksum))
    ));
    assert!(matches!(
        join.receive_notice(PlayerSlot(0), &notice(1, 0, &[(3, 1, 2)])),
        Err(Error::Join(JoinError::Corrupt(_)))
    ));
    assert!(matches!(
        join.receive_notice(PlayerSlot(0), &notice(1, 0, &[(0, 2, 2)])),
        Err(Error::Join(JoinError::Corrupt(_)))
    ));
    assert!(matches!(
        join.receive_notice(PlayerSlot(0), &notice(2, 0, &[])),
        Err(Error::Join(JoinError::StaleAttempt { .. }))
    ));
    assert!(matches!(
        join.receive_notice(PlayerSlot(0), &notice(1, 0, &[])),
        Err(Error::ConflictingNotice(_))
    ));
    assert_eq!(join.notice_usage(), used);
    join.next_request().unwrap();
    assert_eq!(join.notice_usage(), (0, 0));
    assert!(matches!(
        join.receive_notice(PlayerSlot(0), &a),
        Err(Error::Join(JoinError::StaleAttempt { .. }))
    ));
    join.receive_notice(PlayerSlot(0), &notice(2, 0, &[]))
        .unwrap();
    assert!(join.cancel().is_none());
    assert_eq!(join.notice_usage(), (0, 0));
    assert_eq!(join.status().unwrap(), Status::Cancelled);
    join.next_request().unwrap();
    join.receive_notice(PlayerSlot(0), &notice(3, 0, &[]))
        .unwrap();
    join.invalidate_membership();
    assert_eq!(join.notice_usage(), (0, 0));
    assert_eq!(join.status().unwrap(), Status::Invalidated);
    assert!(matches!(join.next_request(), Err(Error::Invalidated)));
    assert!(matches!(
        join.receive_notice(PlayerSlot(1), &b),
        Err(Error::Invalidated)
    ));
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
        let snapshot = self.a.serve_join(&request).unwrap();
        let ticket = self.a.pending_join(JOINER).unwrap();
        if gap {
            for _ in 0..40 {
                self.round(None);
            }
        }
        self.b.hold_inputs_for_join(ticket);
        let notices = [
            self.a.backlog_notice(JOINER).unwrap(),
            self.b.backlog_notice(JOINER).unwrap(),
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

#[test]
fn delayed_peer_never_unlocks_and_handoff_flushes_exact_local_history() {
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
    assert_eq!(
        join.receive_notice(PlayerSlot(1), &notices[1]).unwrap(),
        Status::Ready
    );
    assert_eq!(join.session().unwrap().source().sent, authored);
    env.assert_converged(&mut join);
    assert!(join.cancel().is_some());
    assert_eq!(join.notice_usage(), (0, 0));
}

#[test]
fn natural_pruning_gap_then_fresh_retry_converges_and_rejects_stale_attempt() {
    let mut env = Env::new(0);
    let mut join = bootstrap::<Mesh>(4096);
    let (old_snapshot, old_notices, source) = env.snapshot(&mut join, true);
    for (i, n) in old_notices.iter().enumerate() {
        join.receive_notice(PlayerSlot(i as u8), n).unwrap();
    }
    let gap = join
        .receive_snapshot(PlayerSlot(0), game(), source, &old_snapshot)
        .unwrap();
    assert!(
        matches!(
            gap,
            Status::InputGap {
                slot: PlayerSlot(1),
                ..
            }
        ),
        "{gap:?}"
    );
    assert!(join.session().unwrap().source().sent.is_empty());
    assert!(join.cancel().is_some());
    assert_eq!(join.notice_usage(), (0, 0));
    // Remove old join links; the A/B mesh remains. No helper remote-cleanup promise.
    env.a.source_mut().links.truncate(1);
    env.b.source_mut().links.truncate(1);
    let (snapshot, notices, source) = env.snapshot(&mut join, false);
    assert!(matches!(
        join.receive_snapshot(PlayerSlot(0), game(), Mesh::default(), &old_snapshot),
        Err(Error::Join(JoinError::StaleAttempt { current: 2, got: 1 }))
    ));
    assert!(matches!(
        join.receive_notice(PlayerSlot(0), &old_notices[0]),
        Err(Error::Join(JoinError::StaleAttempt { current: 2, got: 1 }))
    ));
    assert_eq!(join.notice_usage(), (0, 0));
    join.receive_notice(PlayerSlot(1), &notices[1]).unwrap();
    join.receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    join.receive_notice(PlayerSlot(0), &notices[0]).unwrap();
    env.assert_converged(&mut join);
    assert!(join.invalidate_membership().is_some());
    assert_eq!(join.notice_usage(), (0, 0));
    assert_eq!(join.status().unwrap(), Status::Invalidated);
}

#[test]
fn snapshot_rejections_preserve_staging_and_live_notice_rejections_are_transactional() {
    let mut env = Env::new(512);
    let mut join = bootstrap::<Mesh>(4096);
    let (snapshot, notices, source) = env.snapshot(&mut join, false);
    join.receive_notice(PlayerSlot(0), &notices[0]).unwrap();
    let used = join.notice_usage();
    assert!(matches!(
        join.receive_snapshot(PlayerSlot(1), game(), Mesh::default(), &snapshot),
        Err(Error::WrongDonor(PlayerSlot(1)))
    ));
    let mut bad = snapshot.clone();
    bad[0] ^= 1;
    assert!(matches!(
        join.receive_snapshot(PlayerSlot(0), game(), Mesh::default(), &bad),
        Err(Error::Join(JoinError::BadMessageChecksum))
    ));
    assert!(join.session().is_none());
    assert_eq!(join.notice_usage(), used);
    join.receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    let status = join.status().unwrap();
    assert!(matches!(
        join.receive_notice(PlayerSlot(0), &notice(1, 0, &[])),
        Err(Error::ConflictingNotice(_))
    ));
    let mut bad = notices[1].clone();
    bad[0] ^= 1;
    assert!(matches!(
        join.receive_notice(PlayerSlot(1), &bad),
        Err(Error::Join(JoinError::BadMessageChecksum))
    ));
    assert_eq!(join.notice_usage(), used);
    assert_eq!(join.status().unwrap(), status);
    assert!(join.session().unwrap().source().sent.is_empty());
    assert!(matches!(join.next_request(), Err(Error::SessionExists)));
    join.receive_notice(PlayerSlot(1), &notices[1]).unwrap();
    assert_eq!(join.status().unwrap(), Status::Ready);
}

#[test]
fn vacant_sender_cannot_count_and_exhaustion_clears_staging() {
    let roster = JoinRoster::completed(
        3,
        JOINER,
        PlayerSlot(0),
        vec![PlayerSlot(0)],
        &[PlayerSlot(1)],
    )
    .unwrap();
    let mut config = cfg(2, 512);
    config.max_join_attempts = 1;
    let mut join = JoinBootstrap::<Arena, LocalInputSource>::new_legacy_v2(config, roster, 4096).unwrap();
    join.next_request().unwrap();
    assert_eq!(join.config().join_backlog_peers, 1);
    assert!(matches!(
        join.receive_notice(PlayerSlot(1), &notice(1, 1, &[])),
        Err(Error::NonMember(PlayerSlot(1)))
    ));
    assert_eq!(join.notice_usage(), (0, 0));
    join.receive_notice(PlayerSlot(0), &notice(1, 0, &[]))
        .unwrap();
    assert!(matches!(
        join.next_request(),
        Err(Error::Join(JoinError::TooManyAttempts { max: 1 }))
    ));
    assert_eq!(join.notice_usage(), (0, 0));
    assert_eq!(join.status().unwrap(), Status::Idle);
    assert!(matches!(
        join.receive_notice(PlayerSlot(0), &notice(1, 0, &[])),
        Err(Error::Inactive)
    ));
}

#[test]
fn accepted_gap_is_a_terminal_outcome_not_a_transactional_rejection() {
    let mut env = Env::new(512);
    let mut join = bootstrap::<Mesh>(4096);
    let (snapshot, notices, source) = env.snapshot(&mut join, false);
    join.receive_snapshot(PlayerSlot(0), game(), source, &snapshot)
        .unwrap();
    join.receive_notice(PlayerSlot(0), &notices[0]).unwrap();
    let needed = join.session().unwrap().verified_tick() + 1;
    let bad_coverage = notice(1, 1, &[(1, needed + 2, u64::MAX)]);
    let status = join.receive_notice(PlayerSlot(1), &bad_coverage).unwrap();
    assert_eq!(
        status,
        Status::InputGap {
            slot: PlayerSlot(1),
            needed_from: needed,
            available_from: needed + 2
        }
    );
    assert_eq!(
        join.notice_usage(),
        (2, notices[0].len() + bad_coverage.len())
    );
    assert!(!join.caught_up_to(0).unwrap());
    assert!(join.advance(ArenaInput::default(), vec![]).is_none());
    assert!(matches!(
        join.receive_notice(PlayerSlot(1), &notices[1]),
        Err(Error::FailedAttempt)
    ));
    assert_eq!(join.status().unwrap(), status);
    assert!(join.session().unwrap().source().sent.is_empty());
    join.cancel();
    join.next_request().unwrap();
    assert_eq!(join.notice_usage(), (0, 0));
}
