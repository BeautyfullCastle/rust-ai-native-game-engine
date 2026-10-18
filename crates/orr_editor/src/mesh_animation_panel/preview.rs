#[path = "preview/transport.rs"]
mod transport;
use super::{Panel, text_field};
use crate::{editor::Editor, mesh_animation::Snapshot, model::Mode};
use egui::{Button, Ui};
use orr_remote::mesh_navigation::{MeshView, mesh_from_view};
use std::path::PathBuf;
use transport::{Speed, Transport};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    scene: PathBuf,
    epoch: u64,
    graph_revision: [u8; 32],
    source: SourceKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SourceKey {
    Applied(u64),
    Candidate(u64),
}

#[derive(Clone)]
struct Accepted {
    context: Context,
    snapshot: Snapshot,
    tick: u64,
}

#[derive(Default)]
pub(super) struct State {
    clip: String,
    tick: String,
    context: Option<Context>,
    accepted: Option<Accepted>,
    error: Option<String>,
    transport: Transport,
    looped: bool,
    accepted_fields: Option<(String, String)>,
}

impl State {
    pub(super) fn reset(&mut self) {
        self.clip = "0".into();
        self.tick = "0".into();
        self.context = None;
        self.accepted = None;
        self.error = None;
        self.transport = Transport::default();
        self.looped = false;
        self.accepted_fields = None;
    }

    fn clear_accepted(&mut self) {
        self.transport = Transport::default();
        self.accepted = None;
        self.error = None;
    }

    fn request(&mut self, panel: &Panel, editor: &Editor, clip: &str, tick: &str) {
        self.sync(panel, editor);
        self.transport.pause();
        match accept(panel, editor, clip, tick) {
            Ok(accepted) => {
                self.accepted_fields = Some((clip.to_owned(), tick.to_owned()));
                self.transport.seek(accepted.tick);
                self.looped = accepted.snapshot.settings().looped;
                self.accepted = Some(accepted);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn show(&mut self, ui: &mut Ui, panel: &Panel, editor: &Editor) {
        let current_context = context(panel, editor);
        if current_context != self.context {
            self.reset();
            self.context = current_context;
        }
        let now = ui.input(|input| input.time);
        let frame = ui.ctx().cumulative_frame_nr();
        let focused = ui.input(|input| input.focused);
        self.advance(now, frame, focused);
        ui.separator();
        ui.label("Animation pose preview");
        let enabled = preview_allowed(panel, editor) && self.context.is_some();
        ui.add_enabled_ui(enabled, |ui| {
            let previous_fields = (self.clip.clone(), self.tick.clone());
            if let Some(snapshot) = source_snapshot(panel) {
                super::clip_choices::show(ui, &snapshot.clips(), &mut self.clip);
            }
            text_field(ui, "Animation preview clip", &mut self.clip);
            text_field(ui, "Animation preview tick", &mut self.tick);
            if previous_fields != (self.clip.clone(), self.tick.clone()) {
                self.transport.pause();
            }
            if ui.button("Preview animation pose").clicked() {
                let clip = self.clip.clone();
                let tick = self.tick.clone();
                self.request(panel, editor, &clip, &tick);
            }
            if ui
                .button(if self.transport.running {
                    "Pause animation preview"
                } else {
                    "Play animation preview"
                })
                .clicked()
            {
                if self.transport.running {
                    self.transport.pause();
                } else {
                    self.play(panel, editor, now, frame);
                }
            }
            if ui
                .add_enabled(
                    self.accepted.is_some(),
                    Button::new("Restart animation preview"),
                )
                .clicked()
            {
                if let Some(end) = self.end_tick() {
                    self.transport.restart(now, frame, end, self.looped);
                    if let Some(accepted) = &mut self.accepted {
                        accepted.tick = self.transport.tick;
                    }
                }
            }
            if ui
                .add_enabled(
                    self.accepted.is_some(),
                    egui::Checkbox::new(&mut self.looped, "Loop animation preview"),
                )
                .changed()
            {
                self.transport.pause();
                if let Some(accepted) = &mut self.accepted {
                    let mut settings = accepted.snapshot.settings();
                    settings.looped = self.looped;
                    match accepted.snapshot.clone().with_settings(settings) {
                        Ok(snapshot) => accepted.snapshot = snapshot,
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            egui::ComboBox::from_label("Animation preview speed")
                .selected_text(self.transport.speed.label())
                .show_ui(ui, |ui| {
                    for speed in Speed::ALL {
                        ui.selectable_value(&mut self.transport.speed, speed, speed.label());
                    }
                });
            if ui
                .add_enabled(
                    self.accepted.is_some(),
                    Button::new("Clear animation preview"),
                )
                .clicked()
            {
                self.clear_accepted();
            }
        });
        if self.transport.running {
            ui.ctx().request_repaint();
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::RED, error);
        }
        if let Some(accepted) = &self.accepted {
            ui.label(format!(
                "Previewing clip {} at tick {} (transient; not saved)",
                accepted.snapshot.settings().clip,
                accepted.tick
            ));
        }
    }

    fn end_tick(&self) -> Option<u64> {
        let accepted = self.accepted.as_ref()?;
        let settings = accepted.snapshot.settings();
        let duration = accepted
            .snapshot
            .clips()
            .into_iter()
            .find(|(index, _, _)| *index == settings.clip)?
            .2;
        // Match the sampler's f32 rate-adjusted duration, then ceil to its first
        // endpoint tick. Transport never changes the authored playback rate.
        let ticks =
            f64::from(duration / settings.rate) * f64::from(orr_sample::yard3d_game::TICK_RATE);
        (ticks.is_finite() && ticks >= 0.0 && ticks < u64::MAX as f64).then(|| ticks.ceil() as u64)
    }
    fn advance(&mut self, now: f64, frame: u64, focused: bool) {
        let Some(end) = self.end_tick() else {
            self.transport.pause();
            return;
        };
        self.transport.advance(
            now,
            frame,
            focused,
            orr_sample::yard3d_game::TICK_RATE,
            end,
            self.looped,
        );
        if let Some(accepted) = &mut self.accepted {
            accepted.tick = self.transport.tick;
        }
    }
    fn play(&mut self, panel: &Panel, editor: &Editor, now: f64, frame: u64) {
        // Revalidate edited fields before resuming; invalid input keeps the last pose.
        let accepted_matches = self.accepted.is_some()
            && self.accepted_fields.as_ref() == Some(&(self.clip.clone(), self.tick.clone()));
        if !accepted_matches || self.error.is_some() {
            let clip = self.clip.clone();
            let tick = self.tick.clone();
            self.request(panel, editor, &clip, &tick);
            if self.error.is_some() {
                return;
            }
        }
        if let Some(end) = self.end_tick() {
            self.transport.play(now, frame, end, self.looped);
        }
    }

    fn sync(&mut self, panel: &Panel, editor: &Editor) {
        if editor.mode() != Mode::Edit {
            self.reset();
            return;
        }
        let current_context = context(panel, editor);
        if current_context != self.context {
            self.reset();
            self.context = current_context;
        }
    }

    pub(super) fn placement(
        &self,
        panel: &Panel,
        editor: &Editor,
        view: &MeshView,
    ) -> Result<Option<crate::viewport3d::AnimatedPlacement>, String> {
        if !preview_allowed(panel, editor) {
            return Ok(None);
        }
        let Some(accepted) = &self.accepted else {
            return Ok(None);
        };
        if Some(&accepted.context) != self.context.as_ref()
            || context(panel, editor).as_ref() != Some(&accepted.context)
            || view.graph.revision() != accepted.context.graph_revision
        {
            return Ok(None);
        }
        accepted
            .snapshot
            .placement_for_view(view, accepted.tick)
            .map(Some)
    }
}

pub(super) fn sync(panel: &mut Panel, editor: &Editor) {
    let mut state = std::mem::take(&mut panel.preview);
    state.sync(panel, editor);
    panel.preview = state;
}

pub(super) fn show(ui: &mut Ui, panel: &mut Panel, editor: &Editor) {
    let mut state = std::mem::take(&mut panel.preview);
    state.show(ui, panel, editor);
    panel.preview = state;
}

#[cfg(test)]
pub(super) fn request(panel: &mut Panel, editor: &Editor, clip: &str, tick: &str) {
    let mut state = std::mem::take(&mut panel.preview);
    state.request(panel, editor, clip, tick);
    panel.preview = state;
}

#[cfg(test)]
pub(super) fn has_preview(panel: &Panel) -> bool {
    panel.preview.accepted.is_some()
}

#[cfg(test)]
pub(super) fn clear(panel: &mut Panel) {
    panel.preview.clear_accepted();
}

fn source_snapshot(panel: &Panel) -> Option<&Snapshot> {
    panel
        .candidate
        .as_ref()
        .or_else(|| panel.current_snapshot())
}

fn preview_allowed(panel: &Panel, editor: &Editor) -> bool {
    panel.can_edit(editor)
        && panel.scene_epoch == Some(editor.scene_load_epoch())
        && source_snapshot(panel).is_some()
        && !panel.form_dirty
        && !panel.source_dirty
        && !panel.document_pending
        && !panel.context_stale
        && panel.sidecar_error.is_none()
}

fn context(panel: &Panel, editor: &Editor) -> Option<Context> {
    if !preview_allowed(panel, editor) {
        return None;
    }
    let (scene, graph_revision) = panel.current_identity(editor)?;
    Some(Context {
        scene,
        epoch: editor.scene_load_epoch(),
        graph_revision,
        source: if panel.candidate.is_some() {
            SourceKey::Candidate(panel.candidate_generation)
        } else {
            SourceKey::Applied(panel.current_entry()?.id)
        },
    })
}

pub(super) fn prepare_snapshot(
    snapshot: &Snapshot,
    clip_text: &str,
    tick_text: &str,
) -> Result<(Snapshot, u32), String> {
    let clip = parse_decimal(clip_text, "Animation preview clip")?;
    let tick = parse_decimal(tick_text, "Animation preview tick")?;
    if tick > 3600 {
        return Err("Animation preview tick must be between 0 and 3600".into());
    }
    if !snapshot.clips().iter().any(|(index, _, _)| *index == clip) {
        return Err(format!(
            "Animation preview clip {clip} is not present in this package"
        ));
    }
    let mut settings = snapshot.settings();
    settings.clip = clip;
    settings.idle_clip = None;
    let preview_snapshot = snapshot
        .clone()
        .with_settings(settings)
        .map_err(|error| format!("Animation preview settings rejected: {error}"))?;
    Ok((preview_snapshot, tick))
}

fn accept(
    panel: &Panel,
    editor: &Editor,
    clip_text: &str,
    tick_text: &str,
) -> Result<Accepted, String> {
    if !preview_allowed(panel, editor) {
        return Err("Animation preview requires a clean admitted candidate or applied animation in its matching editable mesh scene".into());
    }
    let snapshot =
        source_snapshot(panel).ok_or("Load an admitted animation before previewing a pose")?;
    let (preview_snapshot, tick) = prepare_snapshot(snapshot, clip_text, tick_text)?;
    let context = context(panel, editor)
        .ok_or("Wait for a coherent local Mesh Navigation frame before previewing")?;
    let frame = editor
        .snapshot()
        .ok_or("Animation preview requires a coherent Frame")?
        .predicted();
    let view = mesh_from_view(frame)?;
    preview_snapshot
        .placement_for_view(&view, u64::from(tick))
        .map_err(|error| format!("Animation preview pose rejected: {error}"))?;
    Ok(Accepted {
        context,
        snapshot: preview_snapshot,
        tick: u64::from(tick),
    })
}

fn parse_decimal(value: &str, label: &str) -> Result<u32, String> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{label} must be a decimal unsigned integer"));
    }
    value
        .parse::<u32>()
        .map_err(|_| format!("{label} is outside the supported integer range"))
}

#[cfg(test)]
mod decimal_tests {
    use super::parse_decimal;

    #[test]
    fn preview_tick_and_clip_use_strict_unsigned_decimal_bounds() {
        assert_eq!(parse_decimal("0", "tick").unwrap(), 0);
        assert_eq!(parse_decimal("3600", "tick").unwrap(), 3600);
        for invalid in ["", "-1", "+1", "1.0", "1e2", "  "] {
            assert!(parse_decimal(invalid, "tick").is_err(), "{invalid:?}");
        }
    }
}

#[cfg(all(test, feature = "mesh-mission"))]
#[path = "preview/tests.rs"]
mod tests;
