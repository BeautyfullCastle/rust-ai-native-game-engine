//! Reliable bridge admissions are bounded, ordered, and aware of replay mode.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use orr_bridge::{
    Bridge, BridgeConfig, BridgeError, ControlOp, InProc, Lifecycle, LoopbackPair, Pacing,
    PlayConfig, PlayHost, PlayMode, PlaySession, SimControl, SimHost, Threaded, ThreadedConfig,
};
use orr_fp::FP;
use orr_session::{AdvanceResult, SessionConfig};
use orr_sim::PlayerSlot;
use orr_testgame::{Arena, ArenaConfig, ArenaInput, Bullet, SpawnBulletCmd};

fn loopback() -> LoopbackPair<Arena> {
    LoopbackPair::new(
        || ArenaConfig { player_count: 2 },
        SessionConfig::new(2, PlayerSlot(0), 42, 50),
        SessionConfig::new(2, PlayerSlot(1), 42, 50),
        0,
        0,
        777,
        |_tick| (ArenaInput::default(), Vec::new()),
    )
}

fn paused_play_host() -> PlayHost<Arena> {
    let mut config = PlayConfig::new(2, 11, 50);
    config.start_paused = true;
    config.ring_capacity = 64;
    PlayHost::new(
        PlaySession::new(config, ArenaConfig { player_count: 2 }),
        PlayerSlot(0),
    )
}

fn manual_threaded(config: BridgeConfig<Arena>) -> Threaded<Arena> {
    Threaded::spawn(
        paused_play_host,
        config,
        ThreadedConfig {
            pacing: Pacing::Manual,
            max_catchup: 8,
        },
    )
    .unwrap()
}

fn bullet_owners<B: Bridge<Arena>>(bridge: &B) -> Vec<u32> {
    bridge
        .snapshot()
        .unwrap()
        .predicted()
        .dense::<Bullet>()
        .1
        .iter()
        .map(|bullet| bullet.owner_slot)
        .collect()
}

fn assert_command_budget<B: Bridge<Arena> + SimControl<Arena>>(bridge: &mut B) {
    bridge.send_command(SpawnBulletCmd { owner: 0 }).unwrap();
    bridge.send_command(SpawnBulletCmd { owner: 1 }).unwrap();
    assert_eq!(
        bridge.send_command(SpawnBulletCmd { owner: 0 }),
        Err(BridgeError::Backpressure)
    );

    // A full explicit-command budget does not prevent the reliable Step that
    // drains those commands into a paused host.
    bridge.control(ControlOp::Step(1)).unwrap();
    assert_eq!(
        bullet_owners(bridge),
        vec![0, 1],
        "accepted commands should be submitted once, in order"
    );

    // The host has submitted those commands, so a new one can be admitted.
    bridge.send_command(SpawnBulletCmd { owner: 0 }).unwrap();
    bridge.control(ControlOp::Step(1)).unwrap();
    assert_eq!(bullet_owners(bridge), vec![0, 1, 0]);
}

#[test]
fn paused_inproc_and_manual_threaded_bound_commands_and_release_budget_after_step() {
    let config = BridgeConfig::default().with_command_capacity(2);
    let mut inproc = InProc::new(paused_play_host(), config);
    assert_command_budget(&mut inproc);

    let mut threaded = manual_threaded(BridgeConfig::default().with_command_capacity(2));
    assert_command_budget(&mut threaded);
}

fn replay_viewer() -> PlaySession<Arena> {
    let mut config = PlayConfig::new(2, 11, 50);
    config.start_paused = true;
    config.ring_capacity = 64;
    let mut recording = PlaySession::<Arena>::new(config, ArenaConfig { player_count: 2 });
    for _ in 0..8 {
        recording.control(ControlOp::Step(1));
    }
    PlaySession::open_replay(&recording.save_replay(), ArenaConfig { player_count: 2 }, 0).unwrap()
}

fn player_zero_x<B: Bridge<Arena>>(bridge: &B) -> FP {
    let snap = bridge.snapshot().unwrap();
    let (_, positions) = snap.predicted().dense::<orr_testgame::Position>();
    positions[0].pos.x
}

#[test]
fn viewer_refuses_live_data_without_retaining_it_and_branch_reopens_admission() {
    let derived = Arc::new(AtomicUsize::new(0));
    let derived_in_sim = derived.clone();
    let config = BridgeConfig::default().with_commands_from_input(move |input: &ArenaInput| {
        derived_in_sim.fetch_add(1, Ordering::SeqCst);
        if input.buttons & orr_testgame::FIRE != 0 {
            vec![SpawnBulletCmd { owner: 0 }]
        } else {
            Vec::new()
        }
    });
    let viewer = PlayHost::new(replay_viewer(), PlayerSlot(0));
    let mut bridge = InProc::new(viewer, config);

    let rejected_input = ArenaInput::new(FP::ONE, FP::ZERO, true);
    assert_eq!(
        bridge.set_input(PlayerSlot(0), rejected_input),
        Err(BridgeError::ReplayReadOnly)
    );
    assert_eq!(
        bridge.send_command(SpawnBulletCmd { owner: 1 }),
        Err(BridgeError::ReplayReadOnly)
    );

    // Replay playback does not derive commands from live input.
    bridge.control(ControlOp::Step(1)).unwrap();
    assert_eq!(
        derived.load(Ordering::SeqCst),
        0,
        "Viewer playback must not call commands_from_input"
    );
    assert!(bullet_owners(&bridge).is_empty());

    // Branch first, then try another step before providing new input. Neither
    // refused pre-branch write may have survived the read-only period.
    bridge.control(ControlOp::Branch).unwrap();
    assert_eq!(bridge.timeline().unwrap().mode, PlayMode::Record);
    let x_before = player_zero_x(&bridge);
    bridge.control(ControlOp::Step(1)).unwrap();
    assert_eq!(
        player_zero_x(&bridge),
        x_before,
        "refused viewer input must not become held input after Branch"
    );
    assert!(
        bullet_owners(&bridge).is_empty(),
        "a refused command must not be staged for after Branch"
    );
    assert_eq!(
        derived.load(Ordering::SeqCst),
        1,
        "Record mode derives even the default input"
    );

    let post_branch = ArenaInput::new(FP::ONE, FP::ZERO, true);
    bridge.set_input(PlayerSlot(0), post_branch).unwrap();
    bridge.send_command(SpawnBulletCmd { owner: 1 }).unwrap();
    bridge.control(ControlOp::Step(1)).unwrap();
    assert!(player_zero_x(&bridge) > x_before);
    assert_eq!(
        bullet_owners(&bridge),
        vec![1, 0],
        "explicit then input-derived command should each run once"
    );
    assert_eq!(derived.load(Ordering::SeqCst), 2);
}

struct RetainingHost {
    inner: LoopbackPair<Arena>,
    staged: Vec<SpawnBulletCmd>,
    release_staged: Arc<AtomicBool>,
    disconnect_next: Arc<AtomicBool>,
    writable: Arc<AtomicBool>,
    delivered: Arc<Mutex<Vec<u32>>>,
    lifecycle: Vec<Lifecycle>,
}

impl SimHost<Arena> for RetainingHost {
    fn tick_rate(&self) -> u32 {
        self.inner.tick_rate()
    }
    fn local_slot(&self) -> PlayerSlot {
        self.inner.local_slot()
    }
    fn player_count(&self) -> u8 {
        self.inner.player_count()
    }
    fn advance(
        &mut self,
        input: ArenaInput,
        commands: Vec<SpawnBulletCmd>,
    ) -> AdvanceResult<Arena> {
        self.staged.extend(commands);
        let deliver = if self.release_staged.load(Ordering::SeqCst) {
            std::mem::take(&mut self.staged)
        } else {
            Vec::new()
        };
        self.delivered
            .lock()
            .unwrap()
            .extend(deliver.iter().map(|command| command.owner));
        let result = self.inner.advance(input, deliver);
        if self.disconnect_next.swap(false, Ordering::SeqCst) {
            self.lifecycle.push(Lifecycle::Disconnected);
        }
        result
    }
    fn head_tick(&self) -> u64 {
        self.inner.head_tick()
    }
    fn verified_tick(&self) -> u64 {
        self.inner.verified_tick()
    }
    fn predicted_frame(&self) -> &orr_ecs::Frame {
        self.inner.predicted_frame()
    }
    fn verified_frame(&self) -> Option<&orr_ecs::Frame> {
        self.inner.verified_frame()
    }
    fn frame_at(&self, tick: u64) -> Option<&orr_ecs::Frame> {
        self.inner.frame_at(tick)
    }
    fn rollback_count(&self) -> u64 {
        self.inner.rollback_count()
    }
    fn last_rollback(&self) -> Option<orr_session::RollbackInfo> {
        self.inner.last_rollback()
    }
    fn take_lifecycle(&mut self) -> Vec<Lifecycle> {
        std::mem::take(&mut self.lifecycle)
    }
    fn pending_command_count(&self) -> usize {
        self.staged.len()
    }
    fn accepts_commands(&self) -> bool {
        self.writable.load(Ordering::SeqCst)
    }
    fn control(&mut self, op: ControlOp) -> orr_bridge::HostOutcome<Arena> {
        match op {
            ControlOp::Pause => self.writable.store(false, Ordering::SeqCst),
            ControlOp::Branch => self.writable.store(true, Ordering::SeqCst),
            _ => {}
        }
        orr_bridge::HostOutcome::default()
    }
}

#[test]
fn host_retention_keeps_explicit_command_permits_until_zero_and_disconnect_is_terminal() {
    let release_staged = Arc::new(AtomicBool::new(false));
    let disconnect_next = Arc::new(AtomicBool::new(false));
    let writable = Arc::new(AtomicBool::new(true));
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let host = RetainingHost {
        inner: loopback(),
        staged: Vec::new(),
        release_staged: release_staged.clone(),
        disconnect_next: disconnect_next.clone(),
        writable: writable.clone(),
        delivered: delivered.clone(),
        lifecycle: Vec::new(),
    };
    let mut bridge = InProc::new(host, BridgeConfig::default().with_command_capacity(2));

    bridge.send_command(SpawnBulletCmd { owner: 0 }).unwrap();
    bridge.send_command(SpawnBulletCmd { owner: 1 }).unwrap();
    bridge.step(1);
    assert_eq!(bridge.host().pending_command_count(), 2);
    assert_eq!(
        bridge.send_command(SpawnBulletCmd { owner: 0 }),
        Err(BridgeError::Backpressure)
    );

    // The custom host now submits both staged commands. Admission is released
    // only after a later host observation reports zero pending commands.
    release_staged.store(true, Ordering::SeqCst);
    bridge.step(1);
    assert_eq!(bridge.host().pending_command_count(), 0);
    assert_eq!(*delivered.lock().unwrap(), vec![0, 1]);
    bridge.send_command(SpawnBulletCmd { owner: 0 }).unwrap();

    disconnect_next.store(true, Ordering::SeqCst);
    bridge.step(1);
    assert_eq!(*delivered.lock().unwrap(), vec![0, 1, 0]);
    assert_eq!(
        bridge.set_input(PlayerSlot(0), ArenaInput::default()),
        Err(BridgeError::Disconnected)
    );
    assert_eq!(
        bridge.send_command(SpawnBulletCmd { owner: 1 }),
        Err(BridgeError::Disconnected)
    );
}

#[derive(Debug)]
struct AdvanceObservation {
    input: ArenaInput,
    command_owners: Vec<u32>,
}

struct GatedPlayHost {
    inner: PlayHost<Arena>,
    advances: Sender<AdvanceObservation>,
    release: Receiver<()>,
    controls: Sender<ControlOp>,
}

impl SimHost<Arena> for GatedPlayHost {
    fn tick_rate(&self) -> u32 {
        self.inner.tick_rate()
    }
    fn local_slot(&self) -> PlayerSlot {
        self.inner.local_slot()
    }
    fn player_count(&self) -> u8 {
        self.inner.player_count()
    }
    fn advance(
        &mut self,
        input: ArenaInput,
        commands: Vec<SpawnBulletCmd>,
    ) -> AdvanceResult<Arena> {
        self.advances
            .send(AdvanceObservation {
                input,
                command_owners: commands.iter().map(|command| command.owner).collect(),
            })
            .expect("the test is still observing gated ticks");
        self.release
            .recv_timeout(Duration::from_secs(3))
            .expect("the test releases every gated tick");
        self.inner.advance(input, commands)
    }
    fn head_tick(&self) -> u64 {
        self.inner.head_tick()
    }
    fn verified_tick(&self) -> u64 {
        self.inner.verified_tick()
    }
    fn predicted_frame(&self) -> &orr_ecs::Frame {
        self.inner.predicted_frame()
    }
    fn verified_frame(&self) -> Option<&orr_ecs::Frame> {
        self.inner.verified_frame()
    }
    fn frame_at(&self, tick: u64) -> Option<&orr_ecs::Frame> {
        self.inner.frame_at(tick)
    }
    fn rollback_count(&self) -> u64 {
        self.inner.rollback_count()
    }
    fn last_rollback(&self) -> Option<orr_session::RollbackInfo> {
        self.inner.last_rollback()
    }
    fn take_lifecycle(&mut self) -> Vec<Lifecycle> {
        self.inner.take_lifecycle()
    }
    fn pending_command_count(&self) -> usize {
        self.inner.pending_command_count()
    }
    fn accepts_commands(&self) -> bool {
        self.inner.accepts_commands()
    }
    fn branch_enables_commands(&self) -> bool {
        self.inner.branch_enables_commands()
    }
    fn epoch(&self) -> u64 {
        self.inner.epoch()
    }
    fn wants_tick(&self) -> bool {
        self.inner.wants_tick()
    }
    fn speed(&self) -> orr_bridge::Speed {
        self.inner.speed()
    }
    fn timeline(&self) -> Option<orr_bridge::Timeline> {
        self.inner.timeline()
    }
    fn control(&mut self, op: ControlOp) -> orr_bridge::HostOutcome<Arena> {
        let outcome = self.inner.control(op);
        self.controls
            .send(op)
            .expect("the test observes accepted controls");
        outcome
    }
    fn debug_command(
        &mut self,
        command: orr_bridge::DebugCommand,
    ) -> orr_bridge::HostOutcome<Arena> {
        self.inner.debug_command(command)
    }
}

fn receive<T>(rx: &Receiver<T>, what: &str) -> T {
    rx.recv_timeout(Duration::from_secs(3))
        .unwrap_or_else(|error| panic!("timed out waiting for {what}: {error}"))
}

#[test]
fn realtime_branch_admits_following_writes_and_step_captures_input_at_admission() {
    let (advance_tx, advance_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (control_tx, control_rx) = mpsc::channel();
    let derived = Arc::new(AtomicUsize::new(0));
    let derived_in_sim = derived.clone();
    let config = BridgeConfig::default()
        .with_command_capacity(1)
        .with_control_capacity(2)
        .with_commands_from_input(move |input: &ArenaInput| {
            derived_in_sim.fetch_add(1, Ordering::SeqCst);
            if input.buttons & orr_testgame::FIRE != 0 {
                vec![SpawnBulletCmd { owner: 0 }]
            } else {
                Vec::new()
            }
        });
    let replay = replay_viewer();
    let mut bridge = Threaded::spawn(
        move || GatedPlayHost {
            inner: PlayHost::new(replay, PlayerSlot(0)),
            advances: advance_tx,
            release: release_rx,
            controls: control_tx,
        },
        config,
        ThreadedConfig {
            pacing: Pacing::Realtime,
            max_catchup: 1,
        },
    )
    .unwrap();

    bridge.control(ControlOp::Play).unwrap();
    assert_eq!(receive(&control_rx, "Play control"), ControlOp::Play);
    let first = receive(&advance_rx, "first gated replay tick");
    assert!(first.command_owners.is_empty());
    assert_eq!(
        derived.load(Ordering::SeqCst),
        0,
        "Viewer must not invoke the derivation callback"
    );
    assert_eq!(bridge.timeline().unwrap().mode, PlayMode::Viewer);

    // The view snapshot is still in Viewer mode because the sim thread is
    // blocked in its first tick. Branch's guaranteed FIFO effect must open
    // admission immediately, before that new mode can be published.
    bridge.control(ControlOp::Branch).unwrap();
    assert_eq!(bridge.timeline().unwrap().mode, PlayMode::Viewer);
    bridge.send_command(SpawnBulletCmd { owner: 1 }).unwrap();
    assert_eq!(
        bridge.send_command(SpawnBulletCmd { owner: 0 }),
        Err(BridgeError::Backpressure),
        "explicit command capacity stays independent from the control quota"
    );

    let input_a = ArenaInput::new(FP::ONE, FP::ZERO, true);
    let input_b = ArenaInput::new(FP::ZERO, FP::ONE, false);
    bridge.set_input(PlayerSlot(0), input_a).unwrap();
    assert_eq!(bridge.timeline().unwrap().mode, PlayMode::Viewer);
    bridge.control(ControlOp::Step(1)).unwrap(); // captures input_a now
    bridge.set_input(PlayerSlot(0), input_b).unwrap(); // latest held input for the following real-time tick
    assert_eq!(
        bridge.control(ControlOp::Pause),
        Err(BridgeError::Backpressure)
    );

    release_tx.send(()).unwrap();
    assert_eq!(
        receive(&control_rx, "queued Branch control"),
        ControlOp::Branch
    );
    let stepped = receive(&advance_rx, "queued Step");
    assert_eq!(
        stepped.input, input_a,
        "Step must retain the input value captured when admitted"
    );
    assert_eq!(
        stepped.command_owners,
        vec![1, 0],
        "explicit command precedes the command derived from input_a"
    );

    release_tx.send(()).unwrap();
    let realtime = receive(&advance_rx, "tick after queued Step");
    assert_eq!(
        realtime.input, input_b,
        "held input coalesces to the latest accepted value"
    );
    assert!(realtime.command_owners.is_empty());

    bridge.control(ControlOp::Pause).unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(receive(&control_rx, "Pause control"), ControlOp::Pause);
    let pause_deadline = Instant::now() + Duration::from_secs(3);
    while bridge.timeline().is_none_or(|timeline| timeline.playing) {
        assert!(
            Instant::now() < pause_deadline,
            "timed out waiting for the published pause"
        );
        std::thread::yield_now();
    }
    let timeline = bridge.timeline().unwrap();
    assert_eq!(timeline.mode, PlayMode::Record);
    assert!(!timeline.playing);
    assert_eq!(
        derived.load(Ordering::SeqCst),
        2,
        "only the Record Step and following real-time tick derive commands"
    );
    assert_eq!(bullet_owners(&bridge), vec![1, 0]);
}

#[test]
fn a_custom_host_read_only_transition_refuses_new_writes_but_preserves_accepted_commands() {
    let release_staged = Arc::new(AtomicBool::new(true));
    let disconnect_next = Arc::new(AtomicBool::new(false));
    let writable = Arc::new(AtomicBool::new(true));
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let host = RetainingHost {
        inner: loopback(),
        staged: Vec::new(),
        release_staged,
        disconnect_next,
        writable,
        delivered: delivered.clone(),
        lifecycle: Vec::new(),
    };
    let mut bridge = InProc::new(host, BridgeConfig::default().with_command_capacity(2));

    bridge.send_command(SpawnBulletCmd { owner: 1 }).unwrap();
    bridge.control(ControlOp::Pause).unwrap(); // this custom host uses Pause to become read-only
    assert_eq!(
        bridge.set_input(PlayerSlot(0), ArenaInput::new(FP::ONE, FP::ZERO, false)),
        Err(BridgeError::ReplayReadOnly)
    );
    assert_eq!(
        bridge.send_command(SpawnBulletCmd { owner: 0 }),
        Err(BridgeError::ReplayReadOnly)
    );

    // Run a tick while the host refuses writes. The previously accepted
    // command must remain in bridge staging instead of being discarded.
    bridge.control(ControlOp::Step(1)).unwrap();
    assert!(delivered.lock().unwrap().is_empty());

    // Reopening admission delivers that earlier command once, followed by a
    // new command accepted after the transition.
    bridge.control(ControlOp::Branch).unwrap();
    bridge
        .set_input(PlayerSlot(0), ArenaInput::new(FP::ONE, FP::ZERO, false))
        .unwrap();
    bridge.send_command(SpawnBulletCmd { owner: 0 }).unwrap();
    bridge.control(ControlOp::Step(1)).unwrap();
    assert_eq!(*delivered.lock().unwrap(), vec![1, 0]);
}

#[test]
fn dropping_realtime_bridge_interrupts_a_large_explicit_step_out_of_band() {
    let (first_tick_tx, first_tick_rx) = mpsc::channel();
    let sent = Arc::new(AtomicBool::new(false));
    let sent_once = sent.clone();
    let config = BridgeConfig::default().with_step_observer(move |_| {
        if !sent_once.swap(true, Ordering::SeqCst) {
            let _ = first_tick_tx.send(());
        }
    });
    let mut bridge = Threaded::spawn(
        paused_play_host,
        config,
        ThreadedConfig {
            pacing: Pacing::Realtime,
            max_catchup: 8,
        },
    )
    .unwrap();

    bridge.control(ControlOp::Step(u32::MAX)).unwrap();
    receive(&first_tick_rx, "first tick of the large Step");

    let (stopped_tx, stopped_rx) = mpsc::channel();
    let dropper = std::thread::spawn(move || {
        drop(bridge);
        let _ = stopped_tx.send(());
    });
    receive(&stopped_rx, "realtime thread shutdown after a large Step");
    dropper.join().unwrap();
}

#[test]
fn manual_control_reports_disconnect_when_a_multi_step_request_ends_early() {
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let mut bridge = Threaded::spawn(
        move || RetainingHost {
            inner: loopback(),
            staged: Vec::new(),
            release_staged: Arc::new(AtomicBool::new(true)),
            disconnect_next: Arc::new(AtomicBool::new(true)),
            writable: Arc::new(AtomicBool::new(true)),
            delivered,
            lifecycle: Vec::new(),
        },
        BridgeConfig::default(),
        ThreadedConfig {
            pacing: Pacing::Manual,
            max_catchup: 8,
        },
    )
    .unwrap();
    assert_eq!(
        bridge.control(ControlOp::Step(3)),
        Err(BridgeError::Disconnected)
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while bridge.snapshot().unwrap().tick() == 0 {
        assert!(Instant::now() < deadline, "the completed first tick must be published");
        std::thread::yield_now();
    }
    assert_eq!(bridge.snapshot().unwrap().tick(), 1);
}
