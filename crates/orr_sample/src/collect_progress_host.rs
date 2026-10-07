//! Local-window completion observer, before the bridge's lossy view mailbox.
//!
//! This owns only bounded presentation-side state. It never mutates a frame or
//! performs filesystem I/O. Timeline edits permanently retire award eligibility;
//! the standalone window does not expose those editor/replay operations.
use crate::collect_game::{self as game, CollectDodgeV1, CollectInput, CollectRun};
use orr_bridge::{
    ControlOp, EventStatus, HostOutcome, Lifecycle, PlayHost, PlayMode, SimHost, Speed, Timeline,
};
use orr_ecs::Frame;
use orr_session::{AdvanceResult, RollbackInfo};
use orr_sim::{DebugCommand, Game, PlayerSlot};

pub(crate) struct ProgressHost {
    inner: PlayHost<CollectDodgeV1>,
    completed_best: Option<u32>,
    eligible: bool,
}
impl ProgressHost {
    pub(crate) fn new(inner: PlayHost<CollectDodgeV1>) -> Self {
        let session = inner.session();
        let eligible = session.mode() == PlayMode::Record
            && session.head_tick() == session.last_tick()
            && session.epoch() == 0;
        Self {
            inner,
            completed_best: None,
            eligible,
        }
    }

    pub(crate) fn completed_best(&self) -> Option<u32> {
        self.completed_best
    }
}
impl SimHost<CollectDodgeV1> for ProgressHost {
    fn advance(
        &mut self,
        input: CollectInput,
        commands: Vec<<CollectDodgeV1 as Game>::Command>,
    ) -> AdvanceResult<CollectDodgeV1> {
        let before = *self.inner.session().frame().singleton::<CollectRun>();
        let before_tick = self.inner.head_tick();
        let before_epoch = self.inner.epoch();
        let recording = self.inner.session().mode() == PlayMode::Record;
        let result = self.inner.advance(input, commands);
        if !recording || self.inner.epoch() != before_epoch {
            self.eligible = false;
        }
        if let AdvanceResult::Advanced {
            tick,
            events,
            rollback: None,
        } = &result
        {
            let after = self.inner.session().frame().singleton::<CollectRun>();
            let terminal = matches!(
                after.phase,
                game::WON | game::LOST_HAZARD | game::LOST_TIMEOUT
            );
            let finished = events.iter().any(|(key, status)| {
                key.tick == *tick
                    && matches!(status, EventStatus::Verified(event)
                        if event.kind == game::EVENT_FINISHED && event.value == after.phase)
            });
            if self.eligible
                && before_tick.checked_add(1) == Some(*tick)
                && before.phase == game::PLAYING
                && terminal
                && finished
            {
                self.completed_best = Some(self.completed_best.unwrap_or(0).max(after.score));
            }
        }
        result
    }
    fn tick_rate(&self) -> u32 {
        self.inner.tick_rate()
    }
    fn local_slot(&self) -> PlayerSlot {
        self.inner.local_slot()
    }
    fn player_count(&self) -> u8 {
        self.inner.player_count()
    }
    fn head_tick(&self) -> u64 {
        self.inner.head_tick()
    }
    fn verified_tick(&self) -> u64 {
        self.inner.verified_tick()
    }
    fn predicted_frame(&self) -> &Frame {
        self.inner.predicted_frame()
    }
    fn verified_frame(&self) -> Option<&Frame> {
        self.inner.verified_frame()
    }
    fn frame_at(&self, tick: u64) -> Option<&Frame> {
        self.inner.frame_at(tick)
    }
    fn rollback_count(&self) -> u64 {
        self.inner.rollback_count()
    }
    fn last_rollback(&self) -> Option<RollbackInfo> {
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
    fn speed(&self) -> Speed {
        self.inner.speed()
    }
    fn timeline(&self) -> Option<Timeline> {
        self.inner.timeline()
    }
    fn control(&mut self, op: ControlOp) -> HostOutcome<CollectDodgeV1> {
        // Bridge Step is handled as individual advance calls, never here.
        if matches!(
            op,
            ControlOp::Seek(_) | ControlOp::Branch | ControlOp::Step(_)
        ) {
            self.eligible = false;
        }
        self.inner.control(op)
    }
    fn debug_command(&mut self, cmd: DebugCommand) -> HostOutcome<CollectDodgeV1> {
        self.eligible = false;
        self.inner.debug_command(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_bridge::{Bridge, BridgeConfig, InProc, PlayConfig, PlaySession};
    use orr_fp::{FPVec2, FP};
    use std::time::Duration;

    fn level() -> game::CollectLevel {
        game::CollectLevel::new(
            FPVec2::ZERO,
            vec![FPVec2::ZERO, FPVec2::new(FP::from_int(20), FP::ZERO)],
            vec![],
            2,
        )
        .unwrap()
    }
    fn session() -> PlaySession<CollectDodgeV1> {
        let simulation = orr_sim::Simulation::<CollectDodgeV1>::new(level(), 60, 42);
        let mut cfg = PlayConfig::new(1, 42, 60);
        cfg.start_paused = false;
        PlaySession::from_frame(cfg, simulation.frame()).unwrap()
    }
    fn host() -> ProgressHost {
        ProgressHost::new(PlayHost::new(session(), PlayerSlot(0)))
    }
    fn step(host: &mut ProgressHost, buttons: u32) {
        let _ = host.advance(
            CollectInput {
                buttons,
                ..Default::default()
            },
            vec![],
        );
    }

    #[test]
    fn awards_completed_loss_only_and_retains_max_after_restart() {
        let mut host = host();
        step(&mut host, 0);
        assert_eq!(host.predicted_frame().singleton::<CollectRun>().score, 1);
        assert_eq!(host.completed_best(), None);
        step(&mut host, game::RESTART); // Aborted score does not count.
        assert_eq!(host.completed_best(), None);
        step(&mut host, 0);
        step(&mut host, 0);
        assert_eq!(host.completed_best(), Some(1));
        for _ in 0..20 {
            step(&mut host, 0);
        }
        assert_eq!(host.completed_best(), Some(1));
        step(&mut host, game::RESTART);
        assert_eq!(host.predicted_frame().singleton::<CollectRun>().score, 0);
        assert_eq!(host.completed_best(), Some(1));
    }

    #[test]
    fn win_and_zero_score_hazard_loss_are_completed() {
        for hazard in [false, true] {
            let level = game::CollectLevel::new(
                FPVec2::ZERO,
                vec![FPVec2::ZERO],
                if hazard {
                    vec![game::HazardSpec {
                        position: FPVec2::ZERO,
                        velocity: FPVec2::ZERO,
                    }]
                } else {
                    vec![]
                },
                60,
            )
            .unwrap();
            let simulation = orr_sim::Simulation::<CollectDodgeV1>::new(level, 60, 42);
            let session =
                PlaySession::from_frame(PlayConfig::new(1, 42, 60), simulation.frame()).unwrap();
            let mut host = ProgressHost::new(PlayHost::new(session, PlayerSlot(0)));
            step(&mut host, 0);
            assert_eq!(host.completed_best(), Some(if hazard { 0 } else { 1 }));
            assert_eq!(
                host.predicted_frame().singleton::<CollectRun>().phase,
                if hazard { game::LOST_HAZARD } else { game::WON }
            );
        }
    }

    #[test]
    fn observer_does_not_change_simulation_checksums() {
        let mut observed = host();
        let mut plain = PlayHost::new(session(), PlayerSlot(0));
        for buttons in [0, 0, 0, game::RESTART, 0, 0, 0] {
            step(&mut observed, buttons);
            let _ = plain.advance(
                CollectInput {
                    buttons,
                    ..Default::default()
                },
                vec![],
            );
            assert_eq!(
                observed.predicted_frame().checksum(),
                plain.predicted_frame().checksum()
            );
        }
    }

    #[test]
    fn initial_terminal_is_not_a_completion() {
        let mut inner = PlayHost::new(session(), PlayerSlot(0));
        for _ in 0..2 {
            let _ = inner.advance(CollectInput::default(), vec![]);
        }
        let mut host = ProgressHost::new(inner);
        step(&mut host, 0);
        assert_eq!(host.completed_best(), None);
        step(&mut host, game::RESTART);
        step(&mut host, 0);
        step(&mut host, 0);
        assert_eq!(host.completed_best(), Some(1));
    }

    #[test]
    fn observes_every_catchup_tick_despite_mailbox_overflow() {
        let mut bridge = InProc::new(
            host(),
            BridgeConfig {
                view_event_capacity: 1,
                ..Default::default()
            },
        );
        bridge.update(Duration::from_secs(1));
        assert_eq!(bridge.host().completed_best(), Some(1));
        // Do not poll between completion and restart: only the restarted frame
        // is visible, but the host-side maximum survives.
        bridge
            .set_input(
                PlayerSlot(0),
                CollectInput {
                    buttons: game::RESTART,
                    ..Default::default()
                },
            )
            .unwrap();
        bridge.update(Duration::from_millis(17));
        let update = bridge.poll_view();
        assert_eq!(
            update
                .snapshot
                .unwrap()
                .predicted()
                .singleton::<CollectRun>()
                .score,
            0
        );
        assert_eq!(bridge.host().completed_best(), Some(1));
    }

    #[test]
    fn seek_branch_and_debug_cannot_create_awards() {
        for op in [ControlOp::Seek(0), ControlOp::Branch, ControlOp::Step(1)] {
            let mut host = host();
            let _ = host.control(op);
            step(&mut host, 0);
            step(&mut host, 0);
            assert_eq!(host.completed_best(), None);
        }
        let mut host = host();
        let _ = host.debug_command(DebugCommand::Spawn { components: vec![] });
        step(&mut host, 0);
        step(&mut host, 0);
        assert_eq!(host.completed_best(), None);
    }

    #[test]
    fn replay_and_replay_branch_are_ineligible() {
        let mut original = session();
        for _ in 0..2 {
            let _ = original.step_now();
        }
        let bytes = original.save_replay();
        let replay = PlaySession::<CollectDodgeV1>::open_replay(&bytes, level(), 0).unwrap();
        let mut host = ProgressHost::new(PlayHost::new(replay, PlayerSlot(0)));
        step(&mut host, 0);
        step(&mut host, 0);
        assert_eq!(host.completed_best(), None);
        let _ = host.control(ControlOp::Branch);
        step(&mut host, game::RESTART);
        step(&mut host, 0);
        step(&mut host, 0);
        assert_eq!(host.completed_best(), None);
    }
}
