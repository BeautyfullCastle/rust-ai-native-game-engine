//! Source-less capped-cylinder creation with a dedicated backend request.
use super::*;
use crate::model_package_import::{CylinderRequest, CylinderSpec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Segments {
    Eight,
    Sixteen,
    ThirtyTwo,
    SixtyFour,
    OneTwentyEight,
}
impl Segments {
    fn count(self) -> u32 {
        match self {
            Self::Eight => 8,
            Self::Sixteen => 16,
            Self::ThirtyTwo => 32,
            Self::SixtyFour => 64,
            Self::OneTwentyEight => 128,
        }
    }
    fn label(self) -> String {
        format!("{} cylinder segments", self.count())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) radius: String,
    pub(super) height: String,
    pub(super) segments: Segments,
    pub(super) asset_id: String,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            enabled: false,
            radius: "0.5".into(),
            height: "1".into(),
            segments: Segments::ThirtyTwo,
            asset_id: "primitives/prop.cylinder.json".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Context {
    root: PathBuf,
    project: u64,
    source: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Snapshot {
    context: Context,
    authoring_generation: u64,
    enabled: bool,
    kind: ModelKind,
    format: SourceFormat,
    license: PathBuf,
    destination: PathBuf,
    name: String,
    version: String,
    spec: CylinderSpec,
}
fn number(raw: &str) -> Result<f64, String> {
    let value = raw
        .trim()
        .parse::<f64>()
        .map_err(|_| "Cylinder dimensions must be finite decimal numbers")?;
    if !value.is_finite() || !(0.001..=1_000_000.0).contains(&value) {
        return Err("Cylinder radius and height must be finite f64 values in 0.001..=1000000 before conversion".into());
    }
    Ok(value)
}
impl Panel {
    pub(super) fn set_cylinder_mode(&mut self, enabled: bool) {
        if self.cylinder.enabled == enabled {
            return;
        }
        self.reset_color_variant();
        self.cylinder.enabled = enabled;
        self.cylinder_context = None;
        if enabled {
            self.source_format = SourceFormat::CookedStatic;
            self.kind = ModelKind::Static;
        }
        self.source_inputs_changed();
    }
    pub(super) fn sync_cylinder_context(&mut self, root: Option<&Path>, project: u64, source: u64) {
        if !self.cylinder.enabled {
            self.cylinder_context = None;
            return;
        }
        let context = root.map(|root| Context {
            root: root.into(),
            project,
            source,
        });
        if self.cylinder_context != context {
            self.cylinder_context = context;
            self.source_inputs_changed();
        }
    }
    pub(super) fn cylinder_spec(&self) -> Result<Option<CylinderSpec>, String> {
        if !self.cylinder.enabled {
            return Ok(None);
        }
        if self.kind != ModelKind::Static
            || self.source_format != SourceFormat::CookedStatic
            || self.static_uv.enabled
            || self.static_texture.enabled
            || self.static_assembly.enabled
            || self.static_pivot.enabled
            || self.static_sampler.enabled
            || self.static_recolor.enabled
            || self.static_planar.enabled
            || self.glb_subtree.enabled
            || self.pose_bake.enabled
            || self.color_variant_enabled
            || self.texture_variant_enabled
            || self.trim.enabled
            || self.sampler.enabled
            || self.sequence.enabled
            || self.uv.enabled
        {
            return Err("Choose only source-less Static cylinder creation mode".into());
        }
        let spec = CylinderSpec {
            radius: number(&self.cylinder.radius)?,
            height: number(&self.cylinder.height)?,
            segments: self.cylinder.segments.count(),
            asset_id: self.cylinder.asset_id.clone(),
        };
        model_package_import::validate_cylinder_spec(&spec)?;
        Ok(Some(spec))
    }
    pub(super) fn cylinder_request(&self) -> Result<CylinderRequest, String> {
        let spec = self
            .cylinder_spec()?
            .ok_or("Enable cylinder creation first")?;
        if !self.license_acknowledged {
            return Err(
                "Explicit rights acknowledgement is required before cylinder creation".into(),
            );
        }
        if self.license.trim().is_empty()
            || self.destination.trim().is_empty()
            || self.name.trim().is_empty()
            || self.version.trim().is_empty()
        {
            return Err(
                "Provide a cylinder license, new destination, package name and version".into(),
            );
        }
        Ok(CylinderRequest {
            license: self.license.trim().into(),
            destination: self.destination.trim().into(),
            name: self.name.trim().into(),
            version: self.version.trim().into(),
            license_acknowledged: self.license_acknowledged,
            spec,
        })
    }
    pub(super) fn cylinder_snapshot(
        &self,
        root: &Path,
        project: u64,
        source: u64,
    ) -> Result<Option<Snapshot>, String> {
        let Some(spec) = self.cylinder_spec()? else {
            return Ok(None);
        };
        if self.authoring_generation == u64::MAX {
            return Err("Authoring generation exhausted; reopen the panel".into());
        }
        Ok(Some(Snapshot {
            context: Context {
                root: root.into(),
                project,
                source,
            },
            authoring_generation: self.authoring_generation,
            enabled: self.cylinder.enabled,
            kind: self.kind,
            format: self.source_format,
            license: self.license.trim().into(),
            destination: self.destination.trim().into(),
            name: self.name.trim().into(),
            version: self.version.trim().into(),
            spec,
        }))
    }
    pub(super) fn show_cylinder_controls(&mut self, ui: &mut Ui) {
        let before = self.cylinder.clone();
        text_field(ui, "Cylinder radius", &mut self.cylinder.radius, 64);
        text_field(ui, "Cylinder height", &mut self.cylinder.height, 64);
        egui::ComboBox::from_label("Cylinder segments")
            .selected_text(self.cylinder.segments.label())
            .show_ui(ui, |ui| {
                for segments in [
                    Segments::Eight,
                    Segments::Sixteen,
                    Segments::ThirtyTwo,
                    Segments::SixtyFour,
                    Segments::OneTwentyEight,
                ] {
                    if ui
                        .selectable_value(&mut self.cylinder.segments, segments, segments.label())
                        .clicked()
                    {
                        ui.close();
                    }
                }
            });
        text_field(ui, "Cylinder asset ID", &mut self.cylinder.asset_id, 240);
        ui.weak("Create a capped Y-up cylinder as a new static model package without an external model source. Dimensions use original decimal bounds 0.001..=1000000. Side, top and bottom have separate material slots. Rights acknowledgement and a license file remain required. Preview uses the existing imported-package preview after creation.");
        if before != self.cylinder {
            self.source_inputs_changed();
        }
        if let Err(error) = self.cylinder_spec() {
            ui.weak(error);
        }
    }
    pub(super) fn start_cylinder_creation(&mut self, root: PathBuf, project: u64, source: u64) {
        if self.busy() {
            return;
        }
        self.sync_cylinder_context(Some(&root), project, source);
        if self.authoring_generation == u64::MAX {
            self.status = "Authoring generation exhausted; reopen the panel".into();
            return;
        }
        let request = match self.cylinder_request() {
            Ok(request) => request,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let snapshot = match self.cylinder_snapshot(&root, project, source) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-cylinder-package-create".into())
            .spawn(move || {
                let expected_destination = request_destination(&request.destination);
                let result = model_package_import::create_cylinder_package(request);
                let _ = sender.send(Completion {
                    source_format: SourceFormat::CookedStatic,
                    project_root: root,
                    expected_destination,
                    project_generation: project,
                    source_generation: source,
                    result,
                });
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.submitted_cylinder = Some(snapshot);
                self.destination_identity = None;
                self.status =
                    "Creating and validating a capped cylinder package on a worker...".into();
            }
            Err(error) => self.status = format!("Cannot start cylinder creation: {error}"),
        }
    }
}
#[cfg(test)]
#[path = "cylinder_ui_tests.rs"]
mod tests;
