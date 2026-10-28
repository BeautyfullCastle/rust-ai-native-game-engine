//! Host-paired asynchronous semantic ERP requests use the human command owner.
use super::*;
use input::{CommandId, CommandStatus, Phase};
use orr_remote::editor_gameplay::{GameplayControl, GameplayOperation, GameplayRequest};

#[derive(Clone, PartialEq, Eq)]
struct Fence {
    mode: Mode,
    playing: bool,
    viewer: bool,
    session_epoch: u64,
    document_checksum: u64,
    frame_scope: Option<(
        orr_remote::frame_delta::FrameScope,
        orr_remote::FrameEvidenceOrigin,
    )>,
    phase: Phase,
    lifecycle_pending: bool,
    preview: bool,
}

#[derive(Default)]
pub(super) struct State {
    binding: Option<u64>,
    revision: u64,
    fence: Option<Fence>,
    source: Option<orr_remote::RemoteFrameEvidence>,
    controller: Option<u64>,
    exhausted: bool,
}
impl State {
    fn keeps_api_input_on_focus_loss(
        &self,
        binding: u64,
        phase: Phase,
        connection_alive: bool,
        force_release: bool,
    ) -> bool {
        self.binding == Some(binding)
            && self.controller.is_some()
            && connection_alive
            && matches!(phase, Phase::Claiming | Phase::Active)
            && !self.exhausted
            && !force_release
    }

    fn observe(
        &mut self,
        binding: u64,
        fence: Fence,
        source: Option<&orr_remote::RemoteFrameEvidence>,
    ) {
        if self.binding != Some(binding) {
            *self = Self {
                binding: Some(binding),
                ..Self::default()
            };
        }
        let same_source = match (&self.source, source) {
            (Some(old), Some(current)) => old.same_source(current),
            (None, None) => true,
            _ => false,
        };
        if self.fence.as_ref() != Some(&fence) || !same_source {
            if let Some(revision) = self.revision.checked_add(1) {
                self.revision = revision;
            } else {
                self.exhausted = true;
            }
            self.fence = Some(fence);
            self.source = source.cloned();
        }
    }
    fn token(&self, connection: u64) -> String {
        format!(
            "{}:{}:{}",
            self.binding.unwrap_or(0),
            connection,
            self.revision
        )
    }
}

impl Editor {
    /// Losing physical UI focus does not revoke a live API caller's lease.
    /// Use this only for focus/outside-click handling; explicit release, Escape,
    /// dialogs, preview and lifecycle/error paths keep their ordinary authority.
    pub(crate) fn release_for_ui_focus_loss(&mut self) {
        let force_release = self.down.is_some()
            || self.preview.is_some()
            || self.gameplay_error().is_some()
            || self.gameplay_lifecycle.is_some()
            || self.is_viewer()
            || self.sim.mode != Mode::Play
            || !self.sim.playing;
        let api_owned = self
            .backend
            .editor_gameplay_owner
            .as_ref()
            .is_some_and(|owner| {
                let alive = self
                    .erp_gameplay
                    .controller
                    .is_some_and(|connection| owner.connection_alive(connection));
                self.erp_gameplay.keeps_api_input_on_focus_loss(
                    owner.binding_id(),
                    self.input.phase,
                    alive,
                    force_release,
                )
            });
        if api_owned {
            return;
        }
        if self.gameplay.is_some() {
            self.gameplay_focus_lost();
        } else {
            self.release_control();
        }
    }

    /// An owned in-process SDK client with explicit capabilities. The caller
    /// must post/poll semantic requests while the editor's ordinary pump runs.
    /// A blocking call on the same UI thread cannot service its own request.
    pub fn gameplay_api_client(
        &self,
        name: &str,
        caps: orr_remote::Caps,
    ) -> Result<orr_remote::ErpClient, String> {
        self.backend.gameplay_api_client(name, caps)
    }

    fn refresh_erp_gameplay_fence(&mut self, binding: u64) {
        let source = self.gameplay_frame_evidence().cloned();
        let document_checksum = if self.sim.mode == Mode::Edit {
            self.sim.checksum
        } else {
            orr_remote::wire::parse_checksum(&self.gameplay_state["doc_checksum"]).unwrap_or(0)
        };
        let fence = Fence {
            mode: self.sim.mode,
            playing: self.sim.playing,
            viewer: self.is_viewer(),
            session_epoch: self.sim.epoch,
            document_checksum,
            frame_scope: source
                .as_ref()
                .map(|frame| (frame.stamp().scope, frame.origin())),
            phase: self.input.phase,
            lifecycle_pending: self.gameplay_lifecycle.is_some(),
            preview: self.preview.is_some(),
        };
        self.erp_gameplay.observe(binding, fence, source.as_ref());
    }

    pub(super) fn pump_erp_gameplay(&mut self) {
        let Some(binding) = self
            .backend
            .editor_gameplay_owner
            .as_ref()
            .map(|owner| owner.binding_id())
        else {
            return;
        };
        self.refresh_erp_gameplay_fence(binding);
        let disconnected = self.erp_gameplay.controller.is_some_and(|connection| {
            self.backend
                .editor_gameplay_owner
                .as_ref()
                .is_none_or(|owner| !owner.connection_alive(connection))
        });
        if disconnected {
            self.release_gameplay();
            self.erp_gameplay.controller = None;
        }
        // Bounded service queues and the existing core retain terminal results.
        // A disconnect only cancels its still-pending command, never a UI action.
        for _ in 0..64 {
            let cancelled = self
                .backend
                .editor_gameplay_owner
                .as_mut()
                .and_then(|owner| owner.take_cancellation());
            let Some(id) = cancelled else { break };
            if matches!(
                self.gameplay_command_status(CommandId(id)),
                Some(CommandStatus::Pending)
            ) {
                self.release_gameplay();
            }
        }
        if self.input.phase == Phase::Off {
            self.erp_gameplay.controller = None;
        }
        let request = self
            .backend
            .editor_gameplay_owner
            .as_mut()
            .and_then(|owner| owner.take_request());
        let Some(request) = request else { return };
        if self
            .backend
            .editor_gameplay_owner
            .as_ref()
            .is_none_or(|owner| !owner.is_active(request.serial))
        {
            return;
        }
        let result = self.handle_erp_gameplay(&request);
        if let Some(owner) = &mut self.backend.editor_gameplay_owner {
            owner.complete(request.serial, result);
        }
    }

    fn erp_gameplay_base(&self, connection: u64) -> J {
        let phase = match self.input.phase {
            Phase::Off => "off",
            Phase::Claiming => "claiming",
            Phase::Active => "active",
            Phase::Releasing => "releasing",
        };
        json!({"binding":self.erp_gameplay.binding.unwrap_or(0).to_string(),
            "scope":self.erp_gameplay.token(connection),
            "mode":if self.sim.mode == Mode::Play {"play"} else {"edit"},
            "playing":self.sim.playing,"phase":phase,
            "can_submit":self.can_submit_gameplay() && self.preview.is_none()
                && self.erp_gameplay.controller == Some(connection),
            "can_claim":self.erp_gameplay_can_claim(),
            "cleanup_pending":self.input_cleanup_pending() || self.input.phase == Phase::Releasing
                || self.gameplay_lifecycle.is_some() || self.gameplay_cleanup_notice().is_some(),
            "error":self.gameplay_error().is_some(),"tick":self.sim.head_tick.to_string()})
    }

    fn erp_gameplay_can_claim(&self) -> bool {
        self.down.is_none()
            && self.preview.is_none()
            && self.input.phase == Phase::Off
            && self.erp_gameplay.controller.is_none()
            && self.gameplay_lifecycle.is_none()
            && !self
                .pending
                .values()
                .any(|pending| matches!(pending.kind, Pend::Input(..)))
            && self.gameplay.as_ref().is_some_and(|core| core.live())
    }

    fn handle_erp_gameplay(&mut self, request: &GameplayRequest) -> Result<J, String> {
        if self.down.is_some() {
            return Err("control_disconnected".into());
        }
        let handle = self
            .game()
            .gameplay()
            .ok_or("command_capability_unavailable")?;
        if !handle.has_commands() {
            return Err("command_capability_unavailable".into());
        }
        if self.erp_gameplay.exhausted {
            return Err("scope_exhausted".into());
        }
        let writes = matches!(
            request.operation,
            GameplayOperation::Control { .. }
                | GameplayOperation::Submit { .. }
                | GameplayOperation::Cancel { .. }
        );
        if writes && self.preview.is_some() {
            return Err("control_not_ready".into());
        }
        if writes
            && request.scope.as_deref()
                != Some(self.erp_gameplay.token(request.connection).as_str())
        {
            return Err("stale_scope".into());
        }
        let mut detail = match &request.operation {
            GameplayOperation::Describe { intent } => {
                let contract = handle.validated();
                let mut value = json!({"game":handle.name(),"build_id":contract.identity().build_id,
                    "max_players":contract.expected_input().max_players(),
                    "max_sequence":contract.command_max_sequence().map(|n|n.to_string()),
                    "intents":contract.intent_catalog().map(|entry|entry.id).collect::<Vec<_>>()});
                if let Some(id) = intent {
                    let metadata = contract
                        .intent_catalog()
                        .find(|entry| entry.id == id)
                        .ok_or("unknown_intent")?;
                    value["intent"] = json!(id);
                    value["payload_schema"] = metadata.payload_schema.clone();
                }
                value
            }
            GameplayOperation::Control { action } => {
                if self.gameplay_lifecycle.is_some() {
                    return Err("lifecycle_pending".into());
                }
                let before_tick = if self.sim.mode == Mode::Play {
                    self.sim.head_tick
                } else {
                    0
                };
                match action {
                    GameplayControl::Claim(player) => {
                        if self.erp_gameplay.controller.is_some() || self.input.phase != Phase::Off
                        {
                            return Err("control_owned".into());
                        }
                        if !self.erp_gameplay_can_claim() {
                            return Err("control_not_ready".into());
                        }
                        if *player >= handle.expected_input().max_players() {
                            return Err("player_out_of_bounds".into());
                        }
                        self.claim_gameplay(*player)
                            .map_err(|_| "control_not_ready")?;
                        self.erp_gameplay.controller = Some(request.connection);
                    }
                    GameplayControl::Release => {
                        if self.erp_gameplay.controller != Some(request.connection) {
                            return Err("command_not_owned".into());
                        }
                        self.release_gameplay();
                        self.erp_gameplay.controller = None;
                    }
                    GameplayControl::Play => self.play(),
                    GameplayControl::Pause => {
                        if self.sim.mode != Mode::Play {
                            return Err("control_not_ready".into());
                        }
                        self.pause();
                    }
                    GameplayControl::Step(n) => self.step(*n),
                    GameplayControl::Stop => {
                        self.stop();
                    }
                }
                if self.gameplay_lifecycle.is_none() {
                    let applied = match action {
                        GameplayControl::Play => self.sim.mode == Mode::Play && self.sim.playing,
                        GameplayControl::Pause => self.sim.mode == Mode::Play && !self.sim.playing,
                        GameplayControl::Step(n) => {
                            self.sim.mode == Mode::Play
                                && !self.sim.playing
                                && self.sim.head_tick >= before_tick.saturating_add(u64::from(*n))
                        }
                        GameplayControl::Stop => self.sim.mode == Mode::Edit,
                        GameplayControl::Claim(_) | GameplayControl::Release => true,
                    };
                    if !applied {
                        return Err("control_failed".into());
                    }
                }
                json!({"status":"queued"})
            }
            GameplayOperation::Submit { intent, payload } => {
                if self.erp_gameplay.controller != Some(request.connection) {
                    return Err("command_not_owned".into());
                }
                if !self.can_submit_gameplay() {
                    return Err("command_not_ready".into());
                }
                let intent =
                    crate::static_adapter::IntentRequest::new(intent.clone(), payload.clone())
                        .map_err(|_| "invalid_intent")?;
                let id = self.submit_gameplay_intent(intent).map_err(|error| {
                    if error == "command_sequence_exhausted" {
                        "command_sequence_exhausted"
                    } else {
                        "invalid_intent"
                    }
                })?;
                command_status_json(
                    id,
                    self.gameplay_command_status(id).ok_or("command_unknown")?,
                )
            }
            GameplayOperation::Status { id } => command_status_json(
                CommandId(*id),
                self.gameplay_command_status(CommandId(*id))
                    .ok_or("command_unknown")?,
            ),
            GameplayOperation::Cancel { id } => {
                match self.gameplay_command_status(CommandId(*id)) {
                    Some(CommandStatus::Pending) => self.release_gameplay(),
                    Some(CommandStatus::Consumed { .. }) => {
                        return Err("command_not_cancellable".into())
                    }
                    Some(_) => {}
                    None => return Err("command_unknown".into()),
                }
                command_status_json(
                    CommandId(*id),
                    self.gameplay_command_status(CommandId(*id))
                        .ok_or("command_unknown")?,
                )
            }
        };
        self.refresh_erp_gameplay_fence(request.binding);
        let mut result = self.erp_gameplay_base(request.connection);
        result
            .as_object_mut()
            .expect("base object")
            .append(detail.as_object_mut().expect("operation object"));
        Ok(result)
    }
}

fn stamp_json(stamp: orr_remote::frame_delta::FrameStamp) -> J {
    json!({"stream_generation":stamp.scope.stream_generation.to_string(),
        "play_epoch":stamp.scope.play_epoch.to_string(),"timeline_epoch":stamp.scope.timeline_epoch.to_string(),
        "tick":stamp.tick.to_string(),"checksum":orr_remote::wire::checksum_text(stamp.frame_checksum)})
}
fn command_status_json(id: CommandId, status: &CommandStatus) -> J {
    let mut value = json!({"command_id":id.0.to_string()});
    match status {
        CommandStatus::Pending => value["status"] = json!("pending"),
        CommandStatus::Consumed { stamp } => {
            value["status"] = json!("consumed");
            value["consumed"] = stamp_json(*stamp);
        }
        CommandStatus::Completed { consumed, neutral } => {
            value["status"] = json!("completed");
            value["consumed"] = stamp_json(*consumed);
            value["neutral"] = stamp_json(*neutral);
        }
        CommandStatus::Rejected { code, .. } => {
            value["status"] = json!("rejected");
            value["reason"] = json!(code);
        }
        CommandStatus::Cancelled { code, .. } => {
            value["status"] = json!("cancelled");
            value["reason"] = json!(code);
        }
        CommandStatus::Uncertain { code, .. } => {
            value["status"] = json!("uncertain");
            value["reason"] = json!(code);
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fence() -> Fence {
        Fence {
            mode: Mode::Play,
            playing: true,
            viewer: false,
            session_epoch: 7,
            document_checksum: 42,
            frame_scope: None,
            phase: Phase::Off,
            lifecycle_pending: false,
            preview: false,
        }
    }

    #[test]
    fn only_current_live_api_ownership_is_exempt_from_focus_only_release() {
        let mut state = State {
            binding: Some(3),
            controller: Some(11),
            ..State::default()
        };
        for phase in [Phase::Claiming, Phase::Active] {
            assert!(state.keeps_api_input_on_focus_loss(3, phase, true, false));
            assert!(
                !state.keeps_api_input_on_focus_loss(4, phase, true, false),
                "new binding cannot inherit an old controller"
            );
            assert!(
                !state.keeps_api_input_on_focus_loss(3, phase, false, false),
                "disconnected caller cannot retain input"
            );
            assert!(
                !state.keeps_api_input_on_focus_loss(3, phase, true, true),
                "explicit blocking state must force release"
            );
        }
        for phase in [Phase::Off, Phase::Releasing] {
            assert!(!state.keeps_api_input_on_focus_loss(3, phase, true, false));
        }
        state.controller = None;
        assert!(
            !state.keeps_api_input_on_focus_loss(3, Phase::Active, true, false),
            "human control retains its focus-loss behavior"
        );
        state.controller = Some(11);
        state.exhausted = true;
        assert!(!state.keeps_api_input_on_focus_loss(3, Phase::Active, true, false));
    }

    #[test]
    fn connection_specific_scope_stays_stable_until_a_lifecycle_or_authority_boundary() {
        let mut state = State::default();
        state.observe(3, fence(), None);
        let initial = state.token(11);
        assert_ne!(
            initial,
            state.token(12),
            "another connection cannot replay a mutation scope"
        );
        state.observe(3, fence(), None);
        assert_eq!(
            initial,
            state.token(11),
            "ordinary live polling must not stale realtime commands"
        );
        let mut changed = fence();
        changed.session_epoch += 1;
        state.observe(3, changed, None);
        assert_ne!(initial, state.token(11));
        let old_binding = state.token(11);
        state.observe(4, fence(), None);
        assert_ne!(old_binding, state.token(11));
        state.revision = u64::MAX;
        let mut changed = fence();
        changed.preview = true;
        state.observe(4, changed, None);
        assert!(
            state.exhausted,
            "owner scope must fail closed rather than wrap"
        );
    }

    #[test]
    fn status_preserves_consumption_and_neutral_evidence_without_private_error_text() {
        let stamp = orr_remote::frame_delta::FrameStamp {
            scope: orr_remote::frame_delta::FrameScope {
                stream_generation: 2,
                play_epoch: 3,
                timeline_epoch: 4,
            },
            tick: 10,
            frame_checksum: 9,
        };
        let consumed = command_status_json(CommandId(5), &CommandStatus::Consumed { stamp });
        assert_eq!(consumed["status"], "consumed");
        assert!(consumed.get("neutral").is_none());
        let completed = command_status_json(
            CommandId(5),
            &CommandStatus::Completed {
                consumed: stamp,
                neutral: orr_remote::frame_delta::FrameStamp { tick: 11, ..stamp },
            },
        );
        assert_eq!(completed["status"], "completed");
        assert_eq!(completed["consumed"]["tick"], "10");
        assert_eq!(completed["neutral"]["tick"], "11");
        let rejected = command_status_json(
            CommandId(5),
            &CommandStatus::Rejected {
                code: "invalid_payload",
                message: "SECRET_TOKEN_AND_LOCAL_PATH".into(),
            },
        );
        assert_eq!(rejected["reason"], "invalid_payload");
        assert!(!rejected.to_string().contains("SECRET_TOKEN"));
    }

    #[test]
    fn preview_guard_precedes_command_dispatch_and_preserves_host_and_input() {
        let mut editor = Editor::start(&HostSpec::local(default_scene_path())).unwrap();
        editor.sync();
        let before = editor.host_call("sim.state", J::Null).unwrap();
        let stream = editor.backend.frame_stream("view").unwrap();
        editor.preview = Some(Preview {
            id: "readonly-fixture".into(),
            stream,
            seq: 0,
            generation: 0,
            rows_dirty: true,
            bodies: Vec::new(),
            rows: Vec::new(),
        });
        assert!(!editor.can_submit_gameplay());
        assert_eq!(editor.claim_gameplay(0).unwrap_err(), "preview_active");
        let request = crate::static_adapter::IntentRequest::new("set", json!({})).unwrap();
        assert_eq!(
            editor.submit_gameplay_intent(request).unwrap_err(),
            "preview_active"
        );
        assert_eq!(editor.input.phase, Phase::Off);
        assert_eq!(editor.host_call("sim.state", J::Null).unwrap(), before);
        editor.preview = None;
        assert_eq!(
            editor.claim_gameplay(0).unwrap_err(),
            "command_capability_unavailable"
        );
    }
}
