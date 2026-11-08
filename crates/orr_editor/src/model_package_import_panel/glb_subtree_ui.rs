//! Explicit static GLB subtree choice; node names never serve as identifiers.
use super::*;
use crate::model_package_import::GlbSubtreeVariant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) node_index: Option<u32>,
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            enabled: false,
            node_index: None,
            asset_id: "variants/subtree.glb".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    root: PathBuf,
    project: u64,
    generation: u64,
    authoring_generation: u64,
    source: PathBuf,
    license: PathBuf,
    destination: PathBuf,
    name: String,
    version: String,
    kind: ModelKind,
    format: SourceFormat,
    source_sha256: String,
    spec: GlbSubtreeVariant,
}

impl Panel {
    pub(super) fn glb_subtree_request(&self) -> Result<Option<GlbSubtreeVariant>, String> {
        if !self.glb_subtree.enabled {
            return Ok(None);
        }
        if self.source_format != SourceFormat::RawGlb
            || self.kind != ModelKind::Static
            || self.static_uv.enabled
            || self.static_texture.enabled
            || self.static_assembly.enabled
            || self.static_pivot.enabled
            || self.static_sampler.enabled
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sampler.enabled
            || self.sequence.enabled
            || self.uv.enabled
        {
            return Err("Choose only Raw GLB Static subtree mode".into());
        }
        let catalog = self
            .inspection
            .current_glb_catalog(&self.source, self.kind, self.source_format)
            .ok_or("Inspect current GLB nodes first")?;
        let node_index = self
            .glb_subtree
            .node_index
            .ok_or("Select an inspected GLB node index explicitly")?;
        if self.inspected_hash.as_ref() != Some(&catalog.source_sha256)
            || !catalog
                .nodes
                .iter()
                .any(|node| node.index == node_index && node.primitive_count > 0)
        {
            return Err("Inspect and select a current GLB node index again".into());
        }
        let spec = GlbSubtreeVariant {
            node_index,
            asset_id: self.glb_subtree.asset_id.clone(),
        };
        model_package_import::validate_glb_subtree_variant(&spec)?;
        Ok(Some(spec))
    }

    pub(super) fn glb_subtree_snapshot(
        &self,
        root: &Path,
        project: u64,
        generation: u64,
    ) -> Result<Option<Snapshot>, String> {
        let Some(spec) = self.glb_subtree_request()? else {
            return Ok(None);
        };
        let request = self.request();
        Ok(Some(Snapshot {
            root: root.into(),
            project,
            generation,
            authoring_generation: self.authoring_generation,
            source: request.source,
            license: request.license,
            destination: request.destination,
            name: request.name,
            version: request.version,
            kind: request.kind,
            format: request.source_format,
            source_sha256: self
                .inspected_hash
                .clone()
                .ok_or("Inspect current GLB nodes first")?,
            spec,
        }))
    }

    pub(super) fn show_glb_subtree_controls(&mut self, ui: &mut Ui) {
        if self.source_format != SourceFormat::RawGlb || self.kind != ModelKind::Static {
            return;
        }
        let before = self.glb_subtree.clone();
        if ui
            .checkbox(&mut self.glb_subtree.enabled, "Create static GLB subtree")
            .changed()
        {
            self.glb_subtree.node_index = None;
            self.inspection.invalidate();
            self.inspected_hash = None;
            if self.glb_subtree.enabled {
                self.static_uv.enabled = false;
                self.static_texture.enabled = false;
                self.static_assembly.enabled = false;
                self.static_pivot.enabled = false;
                self.static_sampler.enabled = false;
                self.color_variant_enabled = false;
                self.texture_variant_enabled = false;
                self.trim.enabled = false;
                self.sampler.enabled = false;
                self.sequence.enabled = false;
                self.uv.enabled = false;
            }
        }
        if self.glb_subtree.enabled {
            ui.label(
                self.glb_subtree
                    .node_index
                    .map(|index| format!("Selected GLB node index: {index}"))
                    .unwrap_or_else(|| "Inspect GLB nodes, then choose an index".into()),
            );
            text_field(
                ui,
                "GLB subtree asset ID",
                &mut self.glb_subtree.asset_id,
                240,
            );
            ui.weak("Node names are advisory; choose a source node index. Import keeps that default-scene subtree and its source-world placement, then publishes a cooked static model. Use a new .glb logical asset ID. Preview uses the existing imported-package preview after import.");
        }
        if before != self.glb_subtree {
            self.source_inputs_changed();
        }
        if self.glb_subtree.enabled {
            if let Err(error) = self.glb_subtree_request() {
                ui.weak(error);
            }
        }
    }
}

#[cfg(test)]
#[path = "glb_subtree_ui_tests.rs"]
mod tests;
