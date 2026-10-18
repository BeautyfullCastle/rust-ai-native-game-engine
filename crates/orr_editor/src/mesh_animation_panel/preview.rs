use super::{text_field, Panel};
use crate::{editor::Editor, mesh_animation::Snapshot, model::Mode};
use egui::{Button, Ui};
use orr_remote::mesh_navigation::{mesh_from_view, MeshView};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    scene: PathBuf,
    epoch: u64,
    graph_revision: [u8; 32],
    history_id: u64,
}

#[derive(Clone)]
struct Accepted {
    context: Context,
    snapshot: Snapshot,
    tick: u32,
}

#[derive(Default)]
pub(super) struct State {
    clip: String,
    tick: String,
    context: Option<Context>,
    accepted: Option<Accepted>,
    error: Option<String>,
}

impl State {
    pub(super) fn reset(&mut self) {
        self.clip = "0".into();
        self.tick = "0".into();
        self.context = None;
        self.accepted = None;
        self.error = None;
    }

    fn clear_accepted(&mut self) {
        self.accepted = None;
        self.error = None;
    }

    fn request(&mut self, panel: &Panel, editor: &Editor, clip: &str, tick: &str) {
        match accept(panel, editor, clip, tick) {
            Ok(accepted) => {
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
        ui.separator();
        ui.label("Animation pose preview");
        let enabled = preview_allowed(panel, editor) && self.context.is_some();
        ui.add_enabled_ui(enabled, |ui| {
            text_field(ui, "Animation preview clip", &mut self.clip);
            text_field(ui, "Animation preview tick", &mut self.tick);
            if ui.button("Preview animation pose").clicked() {
                let clip = self.clip.clone();
                let tick = self.tick.clone();
                self.request(panel, editor, &clip, &tick);
            }
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
            .placement_for_view(view, u64::from(accepted.tick))
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

fn preview_allowed(panel: &Panel, editor: &Editor) -> bool {
    panel.can_edit(editor)
        && panel.current_snapshot().is_some()
        && !panel.form_dirty
        && !panel.source_dirty
        && panel.candidate.is_none()
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
        history_id: panel.current_entry()?.id,
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
        return Err("Animation preview requires a clean applied animation in its matching editable mesh scene".into());
    }
    let snapshot = panel
        .current_snapshot()
        .ok_or("Apply a mesh animation before previewing a pose")?;
    let (preview_snapshot, tick) = prepare_snapshot(snapshot, clip_text, tick_text)?;
    let (scene, graph_revision) = panel
        .current_identity(editor)
        .ok_or("Wait for a coherent local Mesh Navigation frame before previewing")?;
    let context = Context {
        scene,
        epoch: editor.scene_load_epoch(),
        graph_revision,
        history_id: panel
            .current_entry()
            .ok_or("No applied animation history entry")?
            .id,
    };
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
        tick,
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
mod tests {
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
