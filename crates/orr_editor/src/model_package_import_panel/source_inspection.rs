//! Read-only source discovery; its immutable identity is independent of import completion.
use super::*;
use crate::model_package_import::source_inspection::{inspect_animated_source, inspect_static_source, Catalog};
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    root: PathBuf,
    source: String,
    project: u64,
    generation: u64,
    kind: ModelKind,
    format: SourceFormat,
    revision: u64,
}
#[derive(Default)]
pub(super) struct State {
    identity: Option<Identity>,
    pending: Option<Receiver<(Identity, Result<Catalog, String>)>>,
    catalog: Option<Catalog>,
    status: String,
    revision: u64,
}
impl State {
    pub(super) fn catalog(&self) -> Option<&Catalog> { self.catalog.as_ref() }
    pub(super) fn current_catalog(&self, source: &str, kind: ModelKind, format: SourceFormat) -> Option<&Catalog> {
        let identity = self.identity.as_ref()?;
        (identity.source == source.trim() && identity.kind == kind && identity.format == format)
            .then_some(self.catalog.as_ref()).flatten()
    }
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub(super) fn invalidate(&mut self) {
        self.revision = self.revision.saturating_add(1);
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
        let supported = (self.source_format == SourceFormat::CookedStatic && self.kind == ModelKind::Static)
            || (cfg!(feature = "room-animated-models") && self.source_format == SourceFormat::CookedAnimated && self.kind == ModelKind::Animated);
        if !supported || self.source.trim().is_empty() || self.inspection.revision == u64::MAX
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
            revision: self.inspection.revision,
        })
    }
    pub(super) fn sync_inspection(&mut self, root: Option<&Path>, project: u64, generation: u64) {
        let mut current = self.inspection_identity(root, project, generation);
        if self.inspection.identity != current {
            if self.inspection.identity.is_some() {
                self.inspection.revision = self.inspection.revision.saturating_add(1);
                current = self.inspection_identity(root, project, generation);
            }
            if self.inspection.identity.as_ref().is_some_and(|id| id.format == SourceFormat::CookedStatic) {
                self.source_inputs_changed();
            }
            self.inspection.identity = None;
            self.inspection.catalog = None;
            self.inspection.status.clear();
            self.inspected_hash = None;
            self.static_assembly.source_sha256 = None;
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
                    "Source inspection worker disconnected; inspect again explicitly."
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
                    self.inspection.status="Inspection is read-only; choose an authoring operation and its source or used material. License acknowledgement is still required.".into();
                }
                Err(error) => {
                    self.inspection.catalog = None;
                    self.inspection.status = format!("Source inspection failed: {error}");
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
            .name("orr-model-source-inspect".into())
            .spawn(move || {
                let result = match worker_identity.format {
                    SourceFormat::CookedStatic => inspect_static_source(&source),
                    _ => inspect_animated_source(&source),
                };
                let _ = sender.send((worker_identity, result));
            }) {
            Ok(_) => {
                self.inspection.identity = Some(identity);
                self.inspection.catalog = None;
                self.inspection.pending = Some(receiver);
                self.inspection.status =
                    "Inspecting source on a read-only worker...".into();
                self.inspected_hash = None;
                self.static_assembly.source_sha256 = None;
            }
            Err(error) => {
                self.inspection.status = format!("Cannot start source inspection: {error}")
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
        if !((self.source_format == SourceFormat::CookedStatic && self.kind == ModelKind::Static)
            || (cfg!(feature = "room-animated-models") && self.source_format == SourceFormat::CookedAnimated && self.kind == ModelKind::Animated)) {
            return;
        }
        if ui
            .add_enabled(
                !self.busy()
                    && self
                        .inspection_identity(root, project, generation)
                        .is_some(),
                egui::Button::new(if self.source_format == SourceFormat::CookedStatic { "Inspect static source" } else { "Inspect animated source" }),
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
                            && (self.color_variant_enabled || self.texture_variant_enabled || self.sampler.enabled || self.uv.enabled || self.static_uv.enabled || self.static_texture.enabled),
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
            if self.sampler.enabled { self.sampler.material=Some(index); }
            if self.uv.enabled { self.uv.material=Some(index); }
            if self.static_uv.enabled { self.static_uv.material=Some(index); }
            if self.static_texture.enabled { self.static_texture.material=Some(index); }
            self.inspected_hash = Some(hash);
            self.source_inputs_changed();
        }
    }
}
#[cfg(test)]
#[path = "source_inspection_tests.rs"]
mod tests;
