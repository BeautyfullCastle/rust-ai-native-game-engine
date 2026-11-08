//! Read-only source discovery; its immutable identity is independent of import completion.
use super::*;
use crate::model_package_import::source_inspection::{inspect_animated_source, inspect_static_source, Catalog};
use crate::model_package_import::{inspect_glb_subtree_source, GlbSubtreeCatalog};
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
    pending_glb: Option<Receiver<(Identity, Result<GlbSubtreeCatalog, String>)>>,
    glb_catalog: Option<GlbSubtreeCatalog>,
    status: String,
    revision: u64,
}
impl State {
    pub(super) fn current_glb_catalog(&self, source: &str, kind: ModelKind, format: SourceFormat) -> Option<&GlbSubtreeCatalog> {
        let identity = self.identity.as_ref()?;
        (identity.source == source.trim() && identity.kind == kind && identity.format == format)
            .then_some(self.glb_catalog.as_ref()).flatten()
    }
    pub(super) fn catalog(&self) -> Option<&Catalog> { self.catalog.as_ref() }
    pub(super) fn current_catalog(&self, source: &str, kind: ModelKind, format: SourceFormat) -> Option<&Catalog> {
        let identity = self.identity.as_ref()?;
        (identity.source == source.trim() && identity.kind == kind && identity.format == format)
            .then_some(self.catalog.as_ref()).flatten()
    }
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some() || self.pending_glb.is_some()
    }
    pub(super) fn invalidate(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.identity = None;
        self.catalog = None;
        self.glb_catalog = None;
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
            || (self.source_format == SourceFormat::RawGlb && self.kind == ModelKind::Static && self.glb_subtree.enabled)
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
            if self.inspection.identity.as_ref().is_some_and(|id| id.format == SourceFormat::CookedStatic || id.format == SourceFormat::RawGlb) {
                self.source_inputs_changed();
            }
            self.inspection.identity = None;
            self.inspection.catalog = None;
            self.inspection.glb_catalog = None;
            self.inspection.status.clear();
            self.inspected_hash = None;
            self.static_assembly.source_sha256 = None;
            self.static_pivot.source_sha256 = None;
            self.static_sampler.material = None;
            self.glb_subtree.node_index = None;
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
        let result = match self.inspection.pending_glb.as_ref().map(|receiver| receiver.try_recv()) {
            Some(Ok(value)) => Some(value),
            Some(Err(TryRecvError::Disconnected)) => {
                self.inspection.pending_glb = None;
                self.inspection.status = "GLB node inspection worker disconnected; inspect again explicitly.".into();
                None
            }
            _ => None,
        };
        if let Some((identity, result)) = result {
            self.inspection.pending_glb = None;
            if current.as_ref() != Some(&identity) { return; }
            self.inspection.identity = Some(identity);
            match result {
                Ok(catalog) => {
                    self.inspection.glb_catalog = Some(catalog);
                    self.inspection.status = "GLB inspection is read-only. Select a node index explicitly; names are advisory. Rights acknowledgement is still required.".into();
                }
                Err(error) => {
                    self.inspection.glb_catalog = None;
                    self.inspection.status = format!("GLB node inspection failed: {error}");
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
        if identity.format == SourceFormat::RawGlb {
            self.start_glb_node_inspection(identity);
            return;
        }
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
                self.static_pivot.source_sha256 = None;
                self.static_sampler.material = None;
            }
            Err(error) => {
                self.inspection.status = format!("Cannot start source inspection: {error}")
            }
        }
    }
    fn start_glb_node_inspection(&mut self, identity: Identity) {
        let worker = identity.clone();
        let source = PathBuf::from(&identity.source);
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new().name("orr-glb-node-inspect".into()).spawn(move || {
            let result = inspect_glb_subtree_source(&source);
            let _ = sender.send((worker, result));
        }) {
            Ok(_) => {
                self.inspection.identity = Some(identity);
                self.inspection.catalog = None;
                self.inspection.glb_catalog = None;
                self.inspection.pending_glb = Some(receiver);
                self.inspection.status = "Inspecting GLB nodes on a read-only worker...".into();
                self.glb_subtree.node_index = None;
                self.inspected_hash = None;
            }
            Err(error) => self.inspection.status = format!("Cannot start GLB node inspection: {error}"),
        }
    }
    #[cfg(all(test, target_os = "linux"))]
    pub(super) fn hold_glb_inspection_for_test(&mut self, root: &Path, project: u64, generation: u64, near_exhaustion: bool) -> Box<dyn FnOnce(Result<GlbSubtreeCatalog, String>)> {
        assert!(!self.busy());
        if near_exhaustion { self.inspection.revision = u64::MAX - 1; }
        let identity = self.inspection_identity(Some(root), project, generation).expect("GLB test inspection context");
        let (sender, receiver) = mpsc::channel();
        self.inspection.identity = Some(identity.clone());
        self.inspection.glb_catalog = None;
        self.inspection.pending_glb = Some(receiver);
        self.inspected_hash = None;
        self.glb_subtree.node_index = None;
        Box::new(move |result| { sender.send((identity, result)).unwrap(); })
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
            || (self.source_format == SourceFormat::RawGlb && self.kind == ModelKind::Static && self.glb_subtree.enabled)
            || (cfg!(feature = "room-animated-models") && self.source_format == SourceFormat::CookedAnimated && self.kind == ModelKind::Animated)) {
            return;
        }
        if ui
            .add_enabled(
                !self.busy()
                    && self
                        .inspection_identity(root, project, generation)
                        .is_some(),
                egui::Button::new(match self.source_format { SourceFormat::RawGlb => "Inspect GLB nodes", SourceFormat::CookedStatic => "Inspect static source", SourceFormat::CookedAnimated => "Inspect animated source" }),
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
                            && (self.color_variant_enabled || self.texture_variant_enabled || self.sampler.enabled || self.uv.enabled || self.static_uv.enabled || self.static_texture.enabled || self.static_sampler.enabled),
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
            if self.static_sampler.enabled { self.static_sampler.material=Some(index); }
            self.inspected_hash = Some(hash);
            self.source_inputs_changed();
        }
        let mut selected_node = None;
        if let Some(catalog) = &self.inspection.glb_catalog {
            ui.label(format!("Source SHA-256: {}", catalog.source_sha256));
            for node in &catalog.nodes {
                ui.label(format!("Node {}: {}; {} subtree primitives (name advisory)", node.index, node.name.as_deref().unwrap_or("unnamed"), node.primitive_count));
                if ui.add_enabled(!self.busy() && self.glb_subtree.enabled && node.primitive_count > 0, egui::Button::new(format!("Use node {}", node.index))).clicked() {
                    selected_node = Some((node.index, catalog.source_sha256.clone()));
                }
            }
        }
        if let Some((index, hash)) = selected_node {
            self.glb_subtree.node_index = Some(index);
            self.inspected_hash = Some(hash);
            self.source_inputs_changed();
        }
    }
}
#[cfg(test)]
#[path = "source_inspection_tests.rs"]
mod tests;
