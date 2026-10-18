use crate::{editor::Editor, game::EditorGame, model::Mode};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

struct Request {
    source: PathBuf,
    checksum: u64,
    epoch: u64,
    destination: PathBuf,
}

struct Completion {
    request: Request,
    result: Result<super::copy::Plan, String>,
}

#[derive(Default)]
pub(crate) struct Panel {
    pub(crate) open: bool,
    pub(crate) destination: String,
    pub(crate) status: String,
    pending: Option<Receiver<Completion>>,
    cancel_pending: bool,
}

impl Panel {
    pub(crate) fn open(&mut self) -> bool {
        if self.busy() || self.cancel_pending {
            return false;
        }
        self.open = true;
        true
    }

    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn visible(&self) -> bool {
        self.open
    }

    pub(crate) fn show(
        &mut self,
        context: &egui::Context,
        editor: &mut Editor,
        blocker: Option<String>,
    ) -> Option<PathBuf> {
        if !self.open {
            let published = self.poll(editor, blocker.as_deref());
            if self.busy() {
                context.request_repaint_after(std::time::Duration::from_millis(40));
            }
            return published;
        }
        let mut window_open = self.open;
        egui::Window::new("Duplicate mesh workspace")
            .open(&mut window_open)
            .default_width(520.0)
            .default_pos(egui::pos2(250.0, 80.0))
            .show(context, |ui| {
                ui.label("Copy the saved mesh scene and validated embedded presentation sidecars into a new directory.");
                ui.label("Package sources and unrelated workspace files are not copied.");
                if let Some(reason) = blocker.as_deref() {
                    ui.label(reason);
                }
                ui.add_enabled_ui(!self.busy(), |ui| {
                    let label = ui.label("Mesh workspace destination directory");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.destination)
                            .desired_width(f32::INFINITY)
                            .char_limit(4096),
                    )
                    .labelled_by(label.id);
                    let can_duplicate = blocker.is_none()
                        && !self.busy()
                        && editor.game() == EditorGame::MeshNavigation
                        && editor.mode() == Mode::Edit
                        && editor.spec().is_local();
                    if ui
                        .add_enabled(can_duplicate, egui::Button::new("Duplicate mesh workspace"))
                        .clicked()
                    {
                        self.start(editor, blocker.as_deref());
                    }
                });
                if self.busy() {
                    ui.spinner();
                    context.request_repaint_after(std::time::Duration::from_millis(40));
                }
                if ui.button("Close duplicate workspace").clicked() {
                    self.open = false;
                    self.cancel_pending = self.busy();
                }
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
            });
        self.open = window_open && self.open;
        self.poll(editor, blocker.as_deref())
    }

    fn start(&mut self, editor: &mut Editor, blocker: Option<&str>) -> bool {
        if self.busy() {
            return false;
        }
        if let Some(reason) = blocker {
            self.status = format!("Mesh workspace duplicate blocked: {reason}");
            return false;
        }
        if editor.game() != EditorGame::MeshNavigation || editor.mode() != Mode::Edit {
            self.status = "Duplicate requires a MeshNavigation scene in Edit mode.".into();
            return false;
        }
        let (game, source, checksum) = match editor.mesh_workspace_source_stamp() {
            Ok(stamp) => stamp,
            Err(error) => {
                self.status = format!("Mesh workspace duplicate blocked: {error}");
                return false;
            }
        };
        if game != EditorGame::MeshNavigation {
            self.status = "Duplicate requires a MeshNavigation scene.".into();
            return false;
        }
        let destination = PathBuf::from(self.destination.trim());
        if destination.as_os_str().is_empty()
            || destination.as_os_str().as_encoded_bytes().len() > 4096
        {
            self.status = "Destination directory must contain 1..=4096 native bytes.".into();
            return false;
        }
        let request = Request {
            source,
            checksum,
            epoch: editor.scene_load_epoch(),
            destination,
        };
        let worker_request = Request {
            source: request.source.clone(),
            checksum: request.checksum,
            epoch: request.epoch,
            destination: request.destination.clone(),
        };
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("orr-mesh-workspace-copy".into())
            .spawn(move || {
                let result =
                    super::copy::Plan::prepare(&worker_request.source, &worker_request.destination);
                let _ = sender.send(Completion {
                    request: worker_request,
                    result,
                });
            });
        match worker {
            Ok(_) => {
                self.pending = Some(receiver);
                self.cancel_pending = false;
                self.status = "Validating saved mesh scene and preparing workspace copy…".into();
                true
            }
            Err(error) => {
                self.status = format!("Cannot start mesh workspace copy worker: {error}");
                false
            }
        }
    }

    fn poll(&mut self, editor: &mut Editor, blocker: Option<&str>) -> Option<PathBuf> {
        let receiver = self.pending.as_ref()?;
        let completion = match receiver.try_recv() {
            Ok(completion) => completion,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.cancel_pending = false;
                self.status =
                    "Mesh workspace copy worker disconnected; no destination was published.".into();
                return None;
            }
        };
        self.pending = None;
        if !self.open || self.cancel_pending {
            self.cancel_pending = false;
            self.status = "Mesh workspace copy canceled because its window was closed.".into();
            return None;
        }
        if let Some(reason) = blocker {
            self.status =
                format!("Mesh workspace copy discarded because the editor became busy: {reason}");
            return None;
        }
        let current = match editor.mesh_workspace_source_stamp() {
            Ok(current) => current,
            Err(error) => {
                self.status = format!(
                    "Mesh workspace copy discarded because the source is no longer ready: {error}"
                );
                return None;
            }
        };
        if !request_matches(
            &completion.request,
            &current,
            editor.scene_load_epoch(),
            Path::new(self.destination.trim()),
        ) {
            self.status =
                "Mesh workspace copy discarded because the source, destination or editor changed."
                    .into();
            return None;
        }
        let plan = match completion.result {
            Ok(plan) => plan,
            Err(error) => {
                self.status = format!("Mesh workspace duplicate failed: {error}");
                return None;
            }
        };
        if plan.source_checksum() != completion.request.checksum {
            self.status = "Mesh workspace duplicate discarded because saved scene bytes differ from the admitted host checksum.".into();
            return None;
        }
        match plan.publish() {
            Ok(scene) => {
                self.status = format!("Mesh workspace duplicate ready: {}", scene.display());
                Some(scene)
            }
            Err(error) => {
                self.status = format!("Mesh workspace duplicate failed: {error}");
                None
            }
        }
    }
}

fn request_matches(
    request: &Request,
    current: &(EditorGame, PathBuf, u64),
    current_epoch: u64,
    destination: &Path,
) -> bool {
    current.0 == EditorGame::MeshNavigation
        && current.1 == request.source
        && current.2 == request.checksum
        && current_epoch == request.epoch
        && destination == request.destination
}

#[cfg(test)]
#[path = "copy_panel/tests.rs"]
mod tests;
