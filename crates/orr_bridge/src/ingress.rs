//! Bounded reliable admission plus a coalesced latest-input slot.
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use orr_session::ControlOp;
use orr_sim::{DebugCommand, Game};

use crate::bridge::BridgeError;
use crate::core::ToSim;

struct State<G: Game> {
    input: G::Input,
    queue: VecDeque<ToSim<G>>,
    data_queued: usize,
    controls_queued: usize,
    outstanding_commands: usize,
    open: bool,
    writable: bool,
    branch_enables_commands: bool,
    pending_branches: usize,
}

pub(crate) struct Ingress<G: Game> {
    state: Mutex<State<G>>,
    ready: Condvar,
    command_capacity: usize,
    control_capacity: usize,
}

impl<G: Game> Ingress<G> {
    pub(crate) fn new(command_capacity: usize, control_capacity: usize) -> Self {
        Self {
            state: Mutex::new(State {
                input: G::Input::default(),
                queue: VecDeque::new(),
                data_queued: 0,
                controls_queued: 0,
                outstanding_commands: 0,
                open: true,
                writable: false,
                branch_enables_commands: false,
                pending_branches: 0,
            }),
            ready: Condvar::new(),
            command_capacity: command_capacity.max(1),
            control_capacity: control_capacity.max(1),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State<G>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn initialize(&self, writable: bool, branch_enables_commands: bool, pending: usize) {
        let mut state = self.lock();
        state.writable = writable;
        state.branch_enables_commands = branch_enables_commands;
        state.outstanding_commands = pending;
    }

    pub(crate) fn reconcile_writable(&self, writable: bool, branch_finished: bool) {
        let mut state = self.lock();
        state.writable = writable;
        if branch_finished && state.branch_enables_commands {
            state.pending_branches -= 1;
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.lock().open
    }

    pub(crate) fn close(&self) {
        self.lock().open = false;
        self.ready.notify_all();
    }

    pub(crate) fn input(&self) -> G::Input {
        self.lock().input
    }

    pub(crate) fn set_input(&self, input: G::Input) -> Result<(), BridgeError> {
        let mut state = self.lock();
        Self::check_writable(&state)?;
        state.input = input;
        Ok(())
    }

    fn check_open(state: &State<G>) -> Result<(), BridgeError> {
        if state.open {
            Ok(())
        } else {
            Err(BridgeError::Disconnected)
        }
    }

    fn check_writable(state: &State<G>) -> Result<(), BridgeError> {
        Self::check_open(state)?;
        if state.writable || state.pending_branches > 0 {
            Ok(())
        } else {
            Err(BridgeError::ReplayReadOnly)
        }
    }

    pub(crate) fn command(&self, command: G::Command) -> Result<(), BridgeError> {
        let mut state = self.lock();
        Self::check_writable(&state)?;
        if state.outstanding_commands >= self.command_capacity
            || state.data_queued >= self.command_capacity
        {
            return Err(BridgeError::Backpressure);
        }
        state.outstanding_commands += 1;
        state.data_queued += 1;
        state.queue.push_back(ToSim::Command(command));
        self.ready.notify_one();
        Ok(())
    }

    pub(crate) fn debug(&self, command: DebugCommand) -> Result<(), BridgeError> {
        let mut state = self.lock();
        Self::check_open(&state)?;
        if state.data_queued >= self.command_capacity {
            return Err(BridgeError::Backpressure);
        }
        state.data_queued += 1;
        state.queue.push_back(ToSim::Debug(command));
        self.ready.notify_one();
        Ok(())
    }

    pub(crate) fn control(&self, op: ControlOp) -> Result<(), BridgeError> {
        let mut state = self.lock();
        Self::check_open(&state)?;
        if state.controls_queued >= self.control_capacity {
            return Err(BridgeError::Backpressure);
        }
        let message = match op {
            ControlOp::Step(n) => ToSim::ControlStep {
                n,
                input: state.input,
            },
            _ => ToSim::Control(op),
        };
        state.controls_queued += 1;
        state.queue.push_back(message);
        // Only hosts that guarantee Branch cannot fail permit optimistic
        // admission. The command and Branch share this same ordered FIFO.
        if op == ControlOp::Branch && state.branch_enables_commands {
            state.pending_branches += 1;
        }
        self.ready.notify_one();
        Ok(())
    }

    pub(crate) fn step(&self, n: u32) -> Result<(), BridgeError> {
        let mut state = self.lock();
        Self::check_open(&state)?;
        if state.controls_queued >= self.control_capacity {
            return Err(BridgeError::Backpressure);
        }
        let input = state.input;
        state.controls_queued += 1;
        state.queue.push_back(ToSim::Step { n, input });
        self.ready.notify_one();
        Ok(())
    }

    fn pop(state: &mut State<G>) -> Option<ToSim<G>> {
        if !state.open {
            return None;
        }
        let msg = state.queue.pop_front()?;
        match &msg {
            ToSim::Command(_) | ToSim::Debug(_) => state.data_queued -= 1,
            _ => state.controls_queued -= 1,
        }
        Some(msg)
    }

    pub(crate) fn try_recv(&self) -> Option<ToSim<G>> {
        Self::pop(&mut self.lock())
    }

    pub(crate) fn recv(&self) -> Option<ToSim<G>> {
        let mut state = self.lock();
        while state.open && state.queue.is_empty() {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        Self::pop(&mut state)
    }

    pub(crate) fn release_commands(&self, count: usize) {
        let mut state = self.lock();
        debug_assert!(state.outstanding_commands >= count);
        state.outstanding_commands -= count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_fp::FP;
    use orr_testgame::{Arena, ArenaInput, SpawnBulletCmd};

    fn input(x: i32) -> ArenaInput {
        ArenaInput::new(FP::from_int(x), FP::ZERO, false)
    }

    #[test]
    fn held_input_coalesces_but_both_explicit_step_kinds_capture_at_admission() {
        let ingress = Ingress::<Arena>::new(2, 2);
        ingress.initialize(true, false, 0);
        ingress.set_input(input(1)).unwrap();
        ingress.step(2).unwrap();
        ingress.control(ControlOp::Step(3)).unwrap();
        for _ in 0..10_000 {
            ingress.set_input(input(-1)).unwrap();
        }
        assert_eq!(ingress.lock().queue.len(), 2);
        for expected in [2, 3] {
            let (n, captured) = match ingress.try_recv().unwrap() {
                ToSim::Step { n, input } | ToSim::ControlStep { n, input } => (n, input),
                _ => panic!("expected an explicit step"),
            };
            assert_eq!(n, expected);
            assert_eq!(input_fields(captured), input_fields(input(1)));
        }
        assert_eq!(input_fields(ingress.input()), input_fields(input(-1)));
    }

    // ArenaInput is Pod but this crate need not add bytemuck just for equality.
    fn input_fields(input: ArenaInput) -> (i64, i64, u32) {
        (input.axis_x.0, input.axis_y.0, input.buttons)
    }

    #[test]
    fn command_credits_survive_dequeue_and_controls_have_a_separate_fifo_quota() {
        let ingress = Ingress::<Arena>::new(1, 1);
        ingress.initialize(true, true, 0);
        ingress.command(SpawnBulletCmd { owner: 1 }).unwrap();
        ingress.control(ControlOp::Branch).unwrap();
        assert_eq!(
            ingress.control(ControlOp::Play),
            Err(BridgeError::Backpressure)
        );
        assert!(matches!(ingress.try_recv(), Some(ToSim::Command(_))));
        assert_eq!(
            ingress.command(SpawnBulletCmd { owner: 2 }),
            Err(BridgeError::Backpressure)
        );
        assert!(matches!(
            ingress.try_recv(),
            Some(ToSim::Control(ControlOp::Branch))
        ));
        ingress.release_commands(1);
        ingress.command(SpawnBulletCmd { owner: 2 }).unwrap();
    }

    #[test]
    fn failed_branch_admission_does_not_enable_viewer_input_or_commands() {
        let ingress = Ingress::<Arena>::new(1, 1);
        ingress.initialize(false, true, 0);
        ingress.control(ControlOp::Pause).unwrap();
        assert_eq!(
            ingress.control(ControlOp::Branch),
            Err(BridgeError::Backpressure)
        );
        assert_eq!(
            ingress.set_input(input(1)),
            Err(BridgeError::ReplayReadOnly)
        );
        assert_eq!(
            ingress.command(SpawnBulletCmd { owner: 0 }),
            Err(BridgeError::ReplayReadOnly)
        );
        assert_eq!(
            input_fields(ingress.input()),
            input_fields(ArenaInput::default())
        );
        ingress.try_recv();
        ingress.control(ControlOp::Branch).unwrap();
        ingress.set_input(input(1)).unwrap();
        ingress.command(SpawnBulletCmd { owner: 0 }).unwrap();
        assert!(matches!(
            ingress.try_recv(),
            Some(ToSim::Control(ControlOp::Branch))
        ));
        assert!(matches!(ingress.try_recv(), Some(ToSim::Command(_))));
    }

    #[test]
    fn unsupported_branch_cannot_speculatively_enable_admission() {
        let ingress = Ingress::<Arena>::new(1, 1);
        ingress.initialize(false, false, 0);
        ingress.control(ControlOp::Branch).unwrap();
        assert_eq!(
            ingress.command(SpawnBulletCmd { owner: 0 }),
            Err(BridgeError::ReplayReadOnly)
        );
    }

    #[test]
    fn reconciliation_preserves_only_outstanding_guaranteed_branches() {
        let ingress = Ingress::<Arena>::new(2, 2);
        ingress.initialize(false, true, 0);
        ingress.control(ControlOp::Branch).unwrap();
        ingress.control(ControlOp::Branch).unwrap();
        ingress.reconcile_writable(false, false);
        ingress.set_input(input(1)).unwrap();
        ingress.reconcile_writable(false, true);
        ingress.set_input(input(1)).unwrap();
        ingress.reconcile_writable(false, true);
        assert_eq!(
            ingress.set_input(input(1)),
            Err(BridgeError::ReplayReadOnly)
        );
        ingress.reconcile_writable(true, false);
        ingress.set_input(input(1)).unwrap();
        ingress.reconcile_writable(false, false);
        assert_eq!(
            ingress.set_input(input(1)),
            Err(BridgeError::ReplayReadOnly)
        );
    }

    #[test]
    fn shutdown_is_out_of_band_even_when_both_quotas_are_full() {
        let ingress = Ingress::<Arena>::new(1, 1);
        ingress.initialize(true, false, 0);
        ingress.command(SpawnBulletCmd { owner: 0 }).unwrap();
        ingress.control(ControlOp::Play).unwrap();
        ingress.close();
        assert!(ingress.recv().is_none());
        assert_eq!(
            ingress.command(SpawnBulletCmd { owner: 0 }),
            Err(BridgeError::Disconnected)
        );
        assert_eq!(
            ingress.control(ControlOp::Play),
            Err(BridgeError::Disconnected)
        );
        assert_eq!(ingress.set_input(input(1)), Err(BridgeError::Disconnected));
    }
}
