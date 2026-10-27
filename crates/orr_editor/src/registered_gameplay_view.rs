//! One view adapter for registered gameplay capabilities. It owns no host or router.
use crate::editor::input::{CommandId, Phase};
use crate::editor::{Editor, Mode};
use crate::static_adapter::{
    EmbeddedGameplayView, GameplayViewContext, GameplayViewPhase, IntentRequest, RegisteredGameplay,
};
use orr_remote::{frame_delta::FrameStamp, FrameEvidenceOrigin};

#[derive(Default)]
pub(crate) struct State {
    game: Option<RegisteredGameplay>,
    view: Option<Box<dyn EmbeddedGameplayView>>,
    frame: Option<(FrameStamp, FrameEvidenceOrigin)>,
    display: Option<(u64, u64)>,
    source: Option<orr_remote::RemoteFrameEvidence>,
    phase: Option<GameplayViewPhase>,
    control: Option<Phase>,
    blocked: bool,
    catalog: Vec<(String, String)>,
    selected: usize,
    payload: String,
    last_command: Option<CommandId>,
    error: Option<String>,
}

fn phase(editor: &Editor) -> GameplayViewPhase {
    if editor.is_viewer() {
        GameplayViewPhase::Viewer
    } else if editor.mode() == Mode::Edit {
        GameplayViewPhase::Edit
    } else if editor.timeline().is_some_and(|timeline| timeline.playing) {
        GameplayViewPhase::Live
    } else {
        GameplayViewPhase::Paused
    }
}

fn scope_changed(
    previous: Option<(FrameStamp, FrameEvidenceOrigin)>,
    next: Option<(FrameStamp, FrameEvidenceOrigin)>,
) -> bool {
    match (previous, next) {
        (Some((a, origin_a)), Some((b, origin_b))) => {
            origin_a != origin_b
                || a.scope != b.scope
                || b.tick < a.tick
                || (a.tick == b.tick && a.frame_checksum != b.frame_checksum)
        }
        (None, None) => false,
        _ => true,
    }
}

impl State {
    fn bind(&mut self, game: RegisteredGameplay) {
        if self.game == Some(game) {
            return;
        }
        if let Some(view) = &mut self.view {
            view.reset();
        }
        *self = Self {
            game: Some(game),
            view: game.create_view(),
            catalog: game
                .validated()
                .intent_catalog()
                .map(|intent| {
                    (
                        intent.id.to_owned(),
                        serde_json::to_string_pretty(intent.payload_schema)
                            .expect("validated JSON schema"),
                    )
                })
                .collect(),
            ..Self::default()
        };
    }

    fn observe_cut(
        &mut self,
        current_phase: GameplayViewPhase,
        control: Phase,
        blocked: bool,
        stamp: Option<(FrameStamp, FrameEvidenceOrigin)>,
        display_marker: Option<(u64, u64)>,
        source_changed: bool,
    ) {
        let mode_reset = self.phase.is_some_and(|old| old != current_phase);
        let disarmed =
            self.control == Some(Phase::Active) && matches!(control, Phase::Off | Phase::Releasing);
        let focus_reset = blocked && !self.blocked;
        let display_reset = self
            .display
            .zip(display_marker)
            .is_some_and(|(old, new)| new.0 < old.0 || (new.0 == old.0 && new.1 != old.1));
        if source_changed
            || mode_reset
            || disarmed
            || focus_reset
            || display_reset
            || scope_changed(self.frame, stamp)
        {
            if let Some(view) = &mut self.view {
                view.reset();
            }
            self.last_command = None;
            self.error = None;
        }
        self.phase = Some(current_phase);
        self.control = Some(control);
        self.blocked = blocked;
        self.frame = stamp;
        self.display = display_marker;
    }

    fn submit(&mut self, editor: &mut Editor, intent: IntentRequest, enabled: bool) {
        if !enabled {
            self.error = Some(
                "Input is disabled; take control at the live Play head before submitting".into(),
            );
            return;
        }
        match editor.submit_gameplay_intent(intent) {
            Ok(id) => {
                self.last_command = Some(id);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// True means the embedded presenter consumed the viewport. Spatial views
    /// return false so the existing spatial renderer can draw below the controls.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        take_control: &mut bool,
        player: u8,
        blocked: bool,
    ) -> bool {
        let Some(game) = editor.game().gameplay() else {
            if let Some(view) = &mut self.view {
                view.reset();
            }
            *self = Self::default();
            return false;
        };
        self.bind(game);
        let current_phase = phase(editor);
        let evidence = editor.gameplay_frame_evidence().cloned();
        // These typed preview/viewer snapshots are presentation only. They
        // never become command scope or consumption evidence.
        let display = if evidence.is_none()
            && matches!(
                current_phase,
                GameplayViewPhase::Edit | GameplayViewPhase::Viewer
            ) {
            editor.snapshot().cloned()
        } else {
            None
        };
        let display_marker = display
            .as_ref()
            .map(|snapshot| (snapshot.tick(), snapshot.predicted().checksum()));
        let stamp = evidence
            .as_ref()
            .map(|frame| (frame.stamp(), frame.origin()));
        let source_changed = self
            .source
            .as_ref()
            .zip(evidence.as_ref())
            .is_some_and(|(previous, current)| !previous.same_source(current));
        if blocked {
            editor.gameplay_focus_lost();
        }
        let control = editor.input_phase();
        self.observe_cut(
            current_phase,
            control,
            blocked,
            stamp,
            display_marker,
            source_changed,
        );
        self.source = evidence.clone();
        let live = current_phase == GameplayViewPhase::Live && !blocked;
        if game.has_commands() && *take_control {
            *take_control = false;
            if live {
                if let Err(error) = editor.claim_gameplay(player) {
                    self.error = Some(error);
                }
            }
        }
        if game.has_commands() {
            ui.horizontal_wrapped(|ui| {
                ui.strong(game.name());
                ui.label(format!("{current_phase:?}"));
                if ui
                    .add_enabled(
                        live && control == Phase::Off,
                        egui::Button::new("Take control"),
                    )
                    .clicked()
                {
                    match editor.claim_gameplay(player) {
                        Ok(()) => self.error = None,
                        Err(error) => self.error = Some(error),
                    }
                }
                if ui
                    .add_enabled(
                        matches!(control, Phase::Claiming | Phase::Active),
                        egui::Button::new("Release control"),
                    )
                    .clicked()
                {
                    editor.release_gameplay();
                }
                ui.label(if editor.can_submit_gameplay() {
                    "Commands ready"
                } else {
                    "Commands unavailable or awaiting host"
                });
            });
            if let Some(id) = self.last_command {
                if let Some(status) = editor.gameplay_command_status(id) {
                    ui.label(format!("Last command: {status:?}"));
                }
            }
            if let Some(error) = editor.gameplay_error() {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            if let Some(notice) = editor.gameplay_cleanup_notice() {
                ui.colored_label(egui::Color32::YELLOW, notice);
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let enabled = live && editor.can_submit_gameplay();
        if game.spatial().is_some() {
            if game.has_commands() {
                self.intent_form(ui, editor, enabled);
            }
            return false;
        }
        let frame = if let Some(evidence) = &evidence {
            evidence.snapshot().predicted()
        } else if let Some(display) = &display {
            display.predicted()
        } else {
            ui.label("Waiting for a validated gameplay frame");
            return true;
        };
        let Some(view) = &mut self.view else {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "Registered embedded presenter is unavailable",
            );
            return true;
        };
        let rect = ui.available_rect_before_wrap();
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("registered_gameplay_presenter")
                .max_rect(rect),
        );
        child.set_clip_rect(ui.clip_rect().intersect(rect));
        let requested = view.show(
            &mut child,
            frame,
            GameplayViewContext {
                phase: current_phase,
                input_ready: enabled,
            },
        );
        match requested {
            Ok(Some(intent)) => self.submit(editor, intent, enabled),
            Ok(None) => {}
            Err(error) => {
                self.error = Some(error);
                editor.gameplay_focus_lost();
            }
        }
        true
    }

    fn intent_form(&mut self, ui: &mut egui::Ui, editor: &mut Editor, enabled: bool) {
        if self.catalog.is_empty() {
            return;
        }
        self.selected = self.selected.min(self.catalog.len() - 1);
        ui.collapsing("Command intent", |ui| {
            egui::ComboBox::from_id_salt("registered_gameplay_intent")
                .selected_text(&self.catalog[self.selected].0)
                .show_ui(ui, |ui| {
                    for (index, (name, _)) in self.catalog.iter().enumerate() {
                        ui.selectable_value(&mut self.selected, index, name);
                    }
                });
            ui.collapsing("Payload schema", |ui| {
                ui.monospace(&self.catalog[self.selected].1);
            });
            ui.label("Explicit JSON payload");
            ui.add(
                egui::TextEdit::multiline(&mut self.payload)
                    .desired_rows(2)
                    .char_limit(16 * 1024),
            );
            if ui
                .add_enabled(enabled, egui::Button::new("Submit intent"))
                .clicked()
            {
                let request = serde_json::from_str(&self.payload)
                    .map_err(|error| error.to_string())
                    .and_then(|value| {
                        IntentRequest::new(self.catalog[self.selected].0.clone(), value)
                    });
                match request {
                    Ok(intent) => self.submit(editor, intent, enabled),
                    Err(error) => self.error = Some(error),
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_remote::frame_delta::FrameScope;
    use std::{cell::Cell, rc::Rc};

    struct ResetCounter(Rc<Cell<u32>>);
    impl EmbeddedGameplayView for ResetCounter {
        fn show(
            &mut self,
            _: &mut egui::Ui,
            _: orr_bridge::FrameView<'_>,
            _: GameplayViewContext,
        ) -> Result<Option<IntentRequest>, String> {
            Ok(None)
        }
        fn reset(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn presentation_lifecycle_resets_once_per_edge_and_preserves_busy_ticks() {
        let count = Rc::new(Cell::new(0));
        let mut state = State {
            view: Some(Box::new(ResetCounter(count.clone()))),
            ..State::default()
        };
        let mut stamp = FrameStamp {
            scope: FrameScope {
                stream_generation: 1,
                play_epoch: 1,
                timeline_epoch: 1,
            },
            tick: 1,
            frame_checksum: 1,
        };
        let origin = FrameEvidenceOrigin::FencedFull;
        state.observe_cut(
            GameplayViewPhase::Live,
            Phase::Active,
            false,
            Some((stamp, origin)),
            None,
            false,
        );
        assert_eq!(count.get(), 1);
        for tick in 2..10 {
            stamp.tick = tick;
            stamp.frame_checksum = tick;
            state.observe_cut(
                GameplayViewPhase::Live,
                Phase::Active,
                false,
                Some((stamp, origin)),
                None,
                false,
            );
        }
        assert_eq!(
            count.get(),
            1,
            "pending command frames retain Active control and presentation state"
        );
        state.observe_cut(
            GameplayViewPhase::Paused,
            Phase::Active,
            false,
            Some((stamp, origin)),
            None,
            false,
        );
        state.observe_cut(
            GameplayViewPhase::Paused,
            Phase::Active,
            false,
            Some((stamp, origin)),
            None,
            false,
        );
        assert_eq!(count.get(), 2);
        state.observe_cut(
            GameplayViewPhase::Live,
            Phase::Active,
            false,
            Some((stamp, origin)),
            None,
            false,
        );
        assert_eq!(count.get(), 3);
        state.observe_cut(
            GameplayViewPhase::Live,
            Phase::Releasing,
            true,
            Some((stamp, origin)),
            None,
            false,
        );
        state.observe_cut(
            GameplayViewPhase::Live,
            Phase::Releasing,
            true,
            Some((stamp, origin)),
            None,
            false,
        );
        assert_eq!(
            count.get(),
            4,
            "disarm and focus edges reset once, not once per blocked frame"
        );
        state.observe_cut(
            GameplayViewPhase::Live,
            Phase::Off,
            false,
            Some((stamp, origin)),
            None,
            true,
        );
        assert_eq!(
            count.get(),
            5,
            "a new decoder source resets even when its stamp matches"
        );
    }

    #[test]
    fn scope_reset_rejects_origin_scope_rewind_and_same_tick_replacement() {
        let stamp = FrameStamp {
            scope: FrameScope {
                stream_generation: 1,
                play_epoch: 1,
                timeline_epoch: 1,
            },
            tick: 5,
            frame_checksum: 9,
        };
        let origin = FrameEvidenceOrigin::FencedFull;
        assert!(!scope_changed(
            Some((stamp, origin)),
            Some((FrameStamp { tick: 6, ..stamp }, origin))
        ));
        assert!(scope_changed(
            Some((stamp, origin)),
            Some((FrameStamp { tick: 4, ..stamp }, origin))
        ));
        assert!(scope_changed(
            Some((stamp, origin)),
            Some((
                FrameStamp {
                    frame_checksum: 10,
                    ..stamp
                },
                origin
            ))
        ));
        assert!(scope_changed(
            Some((stamp, origin)),
            Some((stamp, FrameEvidenceOrigin::FrameCodec))
        ));
        assert!(scope_changed(
            Some((stamp, origin)),
            Some((
                FrameStamp {
                    scope: FrameScope {
                        play_epoch: 2,
                        ..stamp.scope
                    },
                    ..stamp
                },
                origin
            ))
        ));
        assert!(scope_changed(Some((stamp, origin)), None));
    }
}
