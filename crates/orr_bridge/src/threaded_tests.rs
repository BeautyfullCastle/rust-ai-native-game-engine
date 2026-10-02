use super::*;
use crate::{HostOutcome, PlayHost, PlaySession};
use orr_session::{AdvanceResult, PlayConfig, RollbackInfo};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};
use std::sync::mpsc::{Receiver, Sender};

struct BusyHost {
    inner: PlayHost<Arena>,
    started: bool,
    ready: Sender<()>,
    release: Receiver<()>,
    observed: Sender<bool>,
}

impl SimHost<Arena> for BusyHost {
    // One nanosecond deadlines make a tick due after even a short drain,
    // without relying on a sleep or the real simulation's tick duration.
    fn tick_rate(&self) -> u32 {
        1_000_000_000
    }
    fn local_slot(&self) -> PlayerSlot {
        PlayerSlot(0)
    }
    fn player_count(&self) -> u8 {
        2
    }
    fn advance(
        &mut self,
        input: ArenaInput,
        commands: Vec<SpawnBulletCmd>,
    ) -> AdvanceResult<Arena> {
        let _ = self.observed.send(true);
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
        0
    }
    fn last_rollback(&self) -> Option<RollbackInfo> {
        None
    }
    fn wants_tick(&self) -> bool {
        self.started
    }
    fn control(&mut self, _op: ControlOp) -> HostOutcome<Arena> {
        if !self.started {
            self.started = true;
            self.ready.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        let _ = self.observed.send(false);
        HostOutcome::default()
    }
}

#[test]
fn realtime_clock_gets_a_turn_before_a_full_control_queue_is_drained() {
    let (ready_tx, ready_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (observed_tx, observed_rx) = channel();
    let mut bridge = Threaded::spawn(
        move || BusyHost {
            inner: PlayHost::new(
                PlaySession::new(PlayConfig::new(2, 11, 50), ArenaConfig { player_count: 2 }),
                PlayerSlot(0),
            ),
            started: false,
            ready: ready_tx,
            release: release_rx,
            observed: observed_tx,
        },
        BridgeConfig::default().with_control_capacity(MAX_MESSAGES_PER_PASS * 2),
        ThreadedConfig::default(),
    )
    .unwrap();
    bridge.control(ControlOp::Play).unwrap();
    ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    for _ in 0..MAX_MESSAGES_PER_PASS * 2 {
        bridge.control(ControlOp::SetSpeed(Speed::NORMAL)).unwrap();
    }
    release_tx.send(()).unwrap();
    let mut controls_before_tick = 0;
    while !observed_rx.recv_timeout(Duration::from_secs(3)).unwrap() {
        controls_before_tick += 1;
    }
    assert!(controls_before_tick <= MAX_MESSAGES_PER_PASS);
    assert!(controls_before_tick > 0);
    drop(bridge);
}
