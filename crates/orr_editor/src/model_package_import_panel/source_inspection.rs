//! Read-only source discovery; its immutable identity is independent of import completion.
use super::*;
use crate::model_package_import::source_inspection::{inspect_animated_source, Catalog};
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    root: PathBuf,
    source: String,
    project: u64,
    generation: u64,
    kind: ModelKind,
    format: SourceFormat,
}
#[derive(Default)]
pub(super) struct State {
    identity: Option<Identity>,
    pending: Option<Receiver<(Identity, Result<Catalog, String>)>>,
    catalog: Option<Catalog>,
    status: String,
}
impl State {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub(super) fn invalidate(&mut self) {
        self.identity = None;
        self.catalog = None;
        self.status.clear();
    }
}
impl Panel {
    fn inspection_identity(
        &self,
        root: Option<&Path>,
        project: u64,
        generation: u64,
    ) -> Option<Identity> {
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
            || self.source.trim().is_empty()
        {
            return None;
        }
        Some(Identity {
            root: root?.into(),
            source: self.source.trim().into(),
            project,
            generation,
            kind: self.kind,
            format: self.source_format,
        })
    }
    pub(super) fn sync_inspection(&mut self, root: Option<&Path>, project: u64, generation: u64) {
        let current = self.inspection_identity(root, project, generation);
        if self.inspection.identity != current {
            self.inspection.identity = None;
            self.inspection.catalog = None;
            self.inspection.status.clear();
            self.inspected_hash = None;
        }
        let result = match self
            .inspection
            .pending
            .as_ref()
            .map(|receiver| receiver.try_recv())
        {
            Some(Ok(value)) => Some(value),
            Some(Err(TryRecvError::Disconnected)) => {
                self.inspection.pending = None;
                self.inspection.status =
                    "Animated source inspection worker disconnected; inspect again explicitly."
                        .into();
                None
            }
            _ => None,
        };
        if let Some((identity, result)) = result {
            self.inspection.pending = None;
            if current.as_ref() != Some(&identity) {
                return;
            }
            self.inspection.identity = Some(identity);
            match result {
                Ok(catalog) => {
                    self.inspection.catalog = Some(catalog);
                    self.inspection.status="Inspection is read-only; choose an authoring operation and a used material. License acknowledgement is still required.".into();
                }
                Err(error) => {
                    self.inspection.catalog = None;
                    self.inspection.status = format!("Animated source inspection failed: {error}");
                }
            }
        }
    }
    fn start_source_inspection(&mut self, root: Option<&Path>, project: u64, generation: u64) {
        if self.busy() {
            return;
        }
        let Some(identity) = self.inspection_identity(root, project, generation) else {
            return;
        };
        let worker_identity = identity.clone();
        let source = PathBuf::from(&identity.source);
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-animated-source-inspect".into())
            .spawn(move || {
                let result = inspect_animated_source(&source);
                let _ = sender.send((worker_identity, result));
            }) {
            Ok(_) => {
                self.inspection.identity = Some(identity);
                self.inspection.catalog = None;
                self.inspection.pending = Some(receiver);
                self.inspection.status =
                    "Inspecting animated source on a read-only worker...".into();
                self.inspected_hash = None;
            }
            Err(error) => {
                self.inspection.status = format!("Cannot start animated source inspection: {error}")
            }
        }
    }
    pub(super) fn show_source_inspection(
        &mut self,
        ui: &mut Ui,
        root: Option<&Path>,
        project: u64,
        generation: u64,
    ) {
        self.sync_inspection(root, project, generation);
        if !cfg!(feature = "room-animated-models")
            || self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
        {
            return;
        }
        if ui
            .add_enabled(
                !self.busy()
                    && self
                        .inspection_identity(root, project, generation)
                        .is_some(),
                egui::Button::new("Inspect animated source"),
            )
            .clicked()
        {
            self.start_source_inspection(root, project, generation);
        }
        if !self.inspection.status.is_empty() {
            ui.label(&self.inspection.status);
        }
        let mut selected = None;
        if let Some(catalog) = &self.inspection.catalog {
            ui.label(format!("Source SHA-256: {}", catalog.source_sha256));
            for material in &catalog.materials {
                ui.label(format!("Material {}: linear RGBA {:?}; image {} ({}x{}); {:?}/{:?}, linear filter {}; used by {} primitives",material.index,material.base_color,material.image,material.width,material.height,material.wrap_s,material.wrap_t,material.linear_filter,material.used_primitive_count));
                if ui
                    .add_enabled(
                        !self.busy()
                            && material.used_primitive_count > 0
                            && (self.color_variant_enabled || self.texture_variant_enabled),
                        egui::Button::new(format!("Use material {}", material.index)),
                    )
                    .clicked()
                {
                    selected = Some((material.index, catalog.source_sha256.clone()));
                }
            }
            for clip in &catalog.clips {
                ui.label(format!(
                    "Clip {}: {} ({:.3}s)",
                    clip.index, clip.name, clip.duration
                ));
            }
        }
        if let Some((index, hash)) = selected {
            if self.color_variant_enabled {
                self.color_variant.material = index;
            } else if self.texture_variant_enabled {
                self.texture_variant.material = index;
            }
            self.inspected_hash = Some(hash);
            self.source_inputs_changed();
        }
    }
}
#[cfg(test)]
#[path = "source_inspection_tests.rs"]
mod tests;
