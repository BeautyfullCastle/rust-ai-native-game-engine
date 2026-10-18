use crate::{
    editor::{Editor, Mode},
    game::EditorGame,
};
use egui::Ui;
use orr_fp::FP;
use orr_remote::mesh_navigation::{mesh_from_view, MeshView};
use orr_render::Camera3D;
use std::path::PathBuf;

const MAX_GOAL_RAW: i128 = 256 * FP::SCALE as i128;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    scene: PathBuf,
    epoch: u64,
    graph_revision: [u8; 32],
}

#[derive(Default)]
pub(crate) struct Panel {
    context: Option<Context>,
    status: String,
}

impl Panel {
    pub(crate) fn show(&mut self, ui: &mut Ui, editor: &mut Editor) {
        self.sync(editor);
        ui.group(|ui| {
            ui.strong("Mesh Play input");
            let enabled = editor.mesh_play_input_enabled();
            let active_play = live_play_ready(editor).is_ok();
            if enabled {
                if ui.button("Disable mesh goal input").clicked() {
                    editor.set_mesh_play_input_enabled(false);
                    self.context = None;
                    self.status = "Mesh goal input released.".into();
                }
            } else if ui
                .add_enabled(active_play, egui::Button::new("Enable mesh goal input"))
                .clicked()
            {
                if editor.set_mesh_play_input_enabled(true) {
                    match current_context(editor) {
                        Ok((context, _)) => {
                            self.context = Some(context);
                            self.status = "Mesh goal input requested; waiting for managed control acknowledgement.".into();
                        }
                        Err(error) => {
                            editor.set_mesh_play_input_enabled(false);
                            self.status = format!("Mesh goal input unavailable: {error}");
                        }
                    }
                } else {
                    self.status = "Mesh goal input was refused by the current Play session.".into();
                }
            }
            ui.label(editor.mesh_play_input_hint());
            ui.label(format!("Mesh input phase: {:?}", editor.mesh_play_input_phase()));
            if editor.mesh_play_input_enabled() {
                ui.label("Click an admitted walkable surface in the viewport to set the live goal.");
            }
            if !self.status.is_empty() {
                ui.label(&self.status);
            }
        });
    }

    pub(crate) fn viewport_click(
        &mut self,
        editor: &mut Editor,
        camera: &Camera3D,
        size: (u32, u32),
        pixel: [f32; 2],
        blocked: bool,
    ) -> bool {
        self.sync(editor);
        if !editor.mesh_play_input_enabled() {
            return false;
        }
        if blocked {
            return false;
        }
        let (context, view) = match current_context(editor) {
            Ok(value) => value,
            Err(error) => {
                editor.set_mesh_play_input_enabled(false);
                self.context = None;
                self.status = format!("Mesh goal input canceled: {error}");
                return true;
            }
        };
        if self.context.as_ref().is_some_and(|saved| saved != &context) {
            editor.set_mesh_play_input_enabled(false);
            self.context = None;
            self.status =
                "Mesh goal input canceled because the scene or walkable graph changed.".into();
            return true;
        }
        self.context = Some(context);
        if size.0 == 0 || size.1 == 0 || pixel.iter().any(|coordinate| !coordinate.is_finite()) {
            self.status =
                "Mesh goal click has invalid viewport coordinates; click a walkable surface."
                    .into();
            return true;
        }
        let (origin, direction) = camera.screen_ray(pixel, size);
        match crate::mesh_agent_view::pick(
            &view.graph,
            origin,
            direction,
            view.agent.spec.max_vertical,
        ) {
            Ok(Some(hit)) => match input_origin(hit.position) {
                Ok(_) => match editor.queue_mesh_play_goal(hit.position) {
                    Ok(()) => {
                        self.status = format!(
                            "Queued live mesh goal on triangle {}; it applies on the next simulation tick.",
                            hit.triangle
                        );
                    }
                    Err(error) => self.status = format!("Mesh goal was not queued: {error}"),
                },
                Err(error) => self.status = error,
            },
            Ok(None) => {
                self.status = "No walkable surface was hit; the current route is unchanged.".into()
            }
            Err(error) => {
                self.status =
                    format!("Mesh goal pick failed; the current route is unchanged: {error}")
            }
        }
        true
    }

    fn sync(&mut self, editor: &mut Editor) {
        if !editor.mesh_play_input_enabled() {
            self.context = None;
            return;
        }
        match current_context(editor) {
            Ok((context, _)) if self.context.as_ref().is_none_or(|saved| saved == &context) => {
                self.context = Some(context);
            }
            Ok(_) => {
                editor.set_mesh_play_input_enabled(false);
                self.context = None;
                self.status =
                    "Mesh goal input released because the scene or walkable graph changed.".into();
            }
            Err(error) => {
                editor.set_mesh_play_input_enabled(false);
                self.context = None;
                self.status = format!("Mesh goal input released: {error}");
            }
        }
    }
}

fn live_play_ready(editor: &Editor) -> Result<(), String> {
    if editor.game() != EditorGame::MeshNavigation
        || !editor.spec().is_local()
        || editor.mode() != Mode::Play
        || !editor.sim().playing
        || editor.sim().head_tick != editor.sim().last_tick
        || editor.previewing().is_some()
    {
        return Err("live local MeshNavigation Play is required".into());
    }
    Ok(())
}

fn current_context(editor: &Editor) -> Result<(Context, MeshView), String> {
    live_play_ready(editor)?;
    let scene = editor
        .path()
        .ok_or_else(|| "the running mesh scene has no local path".to_owned())?;
    let snapshot = editor
        .snapshot()
        .ok_or_else(|| "wait for the current MeshNavigation frame".to_owned())?;
    let frame = snapshot.predicted();
    if frame.checksum() != editor.checksum() {
        return Err("wait for the current live MeshNavigation frame".into());
    }
    let view = mesh_from_view(frame)?;
    let context = Context {
        scene,
        epoch: editor.scene_load_epoch(),
        graph_revision: view.graph.revision(),
    };
    Ok((context, view))
}

fn input_origin(position: [FP; 3]) -> Result<[i32; 3], String> {
    let mut origin = [0_i32; 3];
    for (slot, coordinate) in origin.iter_mut().zip(position) {
        let raw = i128::from(coordinate.raw());
        if raw.abs() > MAX_GOAL_RAW {
            return Err("Picked mesh goal exceeds the managed input range.".into());
        }
        *slot = i32::try_from(coordinate.raw())
            .map_err(|_| "Picked mesh goal exceeds the managed input range.".to_owned())?;
    }
    Ok(origin)
}

#[cfg(test)]
#[path = "mesh_play_controls/tests.rs"]
mod tests;
