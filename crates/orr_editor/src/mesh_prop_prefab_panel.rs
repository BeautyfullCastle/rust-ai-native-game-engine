use crate::{mesh_art::Snapshot, mesh_prop_prefab::Template};
use egui::Ui;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Context {
    pub scene: PathBuf,
    pub scene_epoch: u64,
    pub graph_revision: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    Save,
    Load,
}

struct RequestStamp {
    context: Context,
    path: PathBuf,
    input_generation: u64,
    operation: Operation,
}

struct Completion {
    stamp: RequestStamp,
    result: Result<(Template, String), String>,
}

#[derive(Default)]
pub(crate) struct Panel {
    path: String,
    rights_acknowledged: bool,
    input_generation: u64,
    pending: Option<Receiver<Completion>>,
    status: String,
}

impl Panel {
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn poll(&mut self, context: Option<Context>) -> Option<Template> {
        let receiver = self.pending.as_ref()?;
        let completion = match receiver.try_recv() {
            Ok(completion) => completion,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.status = "Prop prefab worker disconnected; no candidate was selected.".into();
                return None;
            }
        };
        self.pending = None;
        let current = context_matches(
            &completion.stamp,
            context.as_ref(),
            Path::new(self.path.trim()),
            self.input_generation,
            self.rights_acknowledged,
        );
        match completion.result {
            Ok((_, digest)) if current && completion.stamp.operation == Operation::Save => {
                self.status = format!("Saved prop prefab asset. SHA-256: {digest}");
                None
            }
            Ok((template, digest)) if current => {
                self.status =
                    format!("Loaded prop prefab candidate; no prop was added. SHA-256: {digest}");
                Some(template)
            }
            Ok((_, digest)) => {
                self.status = match completion.stamp.operation {
                    Operation::Save => format!(
                        "Saved prop prefab asset at {}; editor context or input changed. SHA-256: {digest}",
                        completion.stamp.path.display()
                    ),
                    Operation::Load => {
                        "Loaded prop prefab was discarded because the scene, graph, path, or rights acknowledgement changed.".into()
                    }
                };
                None
            }
            Err(error) if current => {
                self.status = format!(
                    "Prop prefab {:?} failed: {error}",
                    completion.stamp.operation
                );
                None
            }
            Err(error) => {
                self.status = format!(
                    "Prop prefab {:?} for a prior scene or input state failed: {error}",
                    completion.stamp.operation
                );
                None
            }
        }
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        context: Option<Context>,
        selected: Option<(&str, &Snapshot)>,
        editable: bool,
    ) -> Option<Template> {
        let candidate = self.poll(context.clone());
        egui::CollapsingHeader::new("Prop prefab")
            .default_open(false)
            .show(ui, |ui| {
                ui.label("Save an applied prop as a reusable local asset, or load a candidate for explicit Add.");
                ui.label("Loading never edits the current prop document. Prefabs do not create linked instances or exported-game dependencies.");
                if context.is_none() {
                    ui.label("Open an admitted local mesh scene in Edit mode before using prop prefabs.");
                } else if !editable {
                    ui.label("Prop prefab operations require a stable editable MeshNavigation scene.");
                }
                let enabled = editable && context.is_some() && !self.busy();
                let mut path_changed = false;
                ui.add_enabled_ui(!self.busy(), |ui| {
                    let label = ui.label("Prop prefab file");
                    path_changed = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.path)
                                .desired_width(f32::INFINITY)
                                .char_limit(4096),
                        )
                        .labelled_by(label.id)
                        .changed();
                    if path_changed {
                        self.rights_acknowledged = false;
                        self.advance_input_generation();
                    }
                    let acknowledgement_changed = ui
                        .checkbox(
                            &mut self.rights_acknowledged,
                            "I have reviewed rights to use this prop prefab",
                        )
                        .changed();
                    if acknowledgement_changed {
                        self.advance_input_generation();
                    }
                });
                let path_ready = !self.path.trim().is_empty();
                let can_save = enabled && selected.is_some() && path_ready;
                if ui
                    .add_enabled(can_save, egui::Button::new("Save selected prop prefab"))
                    .clicked()
                {
                    if let (Some(context), Some((name, snapshot))) = (context.as_ref(), selected) {
                        self.start_save(context.clone(), name, snapshot);
                    }
                }
                let can_load = enabled && self.rights_acknowledged && path_ready;
                if ui
                    .add_enabled(can_load, egui::Button::new("Load prop prefab"))
                    .clicked()
                {
                    if let Some(context) = context.as_ref() {
                        self.start_load(context.clone());
                    }
                }
                if self.busy() {
                    ui.spinner();
                }
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
            });
        candidate
    }

    fn advance_input_generation(&mut self) {
        self.input_generation = self.input_generation.saturating_add(1);
    }

    fn start_save(&mut self, context: Context, name: &str, snapshot: &Snapshot) -> bool {
        if self.busy() {
            return false;
        }
        let path = PathBuf::from(self.path.trim());
        if path.as_os_str().is_empty() {
            self.status = "Choose a `.mesh-prop-prefab.json` destination before saving.".into();
            return false;
        }
        let stamp = RequestStamp {
            context,
            path: path.clone(),
            input_generation: self.input_generation,
            operation: Operation::Save,
        };
        let prefab_name = name.to_owned();
        let applied_snapshot = snapshot.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("orr-mesh-prop-prefab-save".into())
            .spawn(move || {
                let result = Template::capture(&prefab_name, &applied_snapshot)
                    .and_then(|template| template.encode().map(|bytes| (template, bytes)))
                    .and_then(|(template, bytes)| {
                        crate::mesh_prop_prefab_store::save_new(&path, &bytes)
                            .map(|digest| (template, digest))
                    });
                let _ = sender.send(Completion { stamp, result });
            });
        match worker {
            Ok(_) => {
                self.pending = Some(receiver);
                self.status = "Saving selected applied prop prefab on a worker…".into();
                true
            }
            Err(error) => {
                self.status = format!("Cannot start prop prefab save worker: {error}");
                false
            }
        }
    }

    fn start_load(&mut self, context: Context) -> bool {
        if self.busy() {
            return false;
        }
        if !self.rights_acknowledged {
            self.status = "Review and acknowledge rights before loading a prop prefab.".into();
            return false;
        }
        let path = PathBuf::from(self.path.trim());
        if path.as_os_str().is_empty() {
            self.status = "Choose a `.mesh-prop-prefab.json` file before loading.".into();
            return false;
        }
        let stamp = RequestStamp {
            context: context.clone(),
            path: path.clone(),
            input_generation: self.input_generation,
            operation: Operation::Load,
        };
        let graph_revision = context.graph_revision;
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("orr-mesh-prop-prefab-load".into())
            .spawn(move || {
                let result =
                    crate::mesh_prop_prefab_store::load(&path).and_then(|(bytes, digest)| {
                        Template::decode(&bytes, graph_revision).map(|template| (template, digest))
                    });
                let _ = sender.send(Completion { stamp, result });
            });
        match worker {
            Ok(_) => {
                self.pending = Some(receiver);
                self.status = "Loading and validating prop prefab on a worker…".into();
                true
            }
            Err(error) => {
                self.status = format!("Cannot start prop prefab load worker: {error}");
                false
            }
        }
    }
}

fn context_matches(
    stamp: &RequestStamp,
    context: Option<&Context>,
    path: &Path,
    input_generation: u64,
    rights_acknowledged: bool,
) -> bool {
    context == Some(&stamp.context)
        && path == stamp.path
        && input_generation == stamp.input_generation
        && (stamp.operation != Operation::Load || rights_acknowledged)
}

#[cfg(test)]
#[path = "mesh_prop_prefab_panel/tests.rs"]
mod tests;
