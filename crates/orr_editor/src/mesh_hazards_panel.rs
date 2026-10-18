//! Authoring for the explicit hazards profile. Runtime loss state is read-only.
use crate::{
    editor::Editor,
    mesh_context::{self, Context},
    model::Mode,
};
use egui::Ui;
use orr_fp::FP;
use orr_remote::mesh_navigation::hazards::{self, HazardSphere, HazardsView, MeshHazards};
use std::path::PathBuf;

const MAX_HAZARDS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    scene: PathBuf,
    epoch: u64,
    revision: [u8; 32],
    checksum: u64,
}

impl Identity {
    fn from_context(context: &Context) -> Self {
        Self {
            scene: context.scene.clone(),
            epoch: context.scene_epoch,
            revision: context.graph_revision,
            checksum: context.checksum,
        }
    }
    fn same_document(&self, other: &Self) -> bool {
        self.scene == other.scene && self.epoch == other.epoch && self.revision == other.revision
    }
}

pub(crate) struct Panel {
    enabled: bool,
    count: String,
    spheres: [[String; 4]; MAX_HAZARDS],
    form_dirty: bool,
    context_stale: bool,
    document_pending: bool,
    identity: Option<Identity>,
    status: String,
    error: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            enabled: false,
            count: "1".into(),
            spheres: default_spheres(),
            form_dirty: false,
            context_stale: false,
            document_pending: true,
            identity: None,
            status: String::new(),
            error: None,
        }
    }
}

impl Panel {
    pub(crate) fn dirty(&self) -> bool {
        self.form_dirty || self.context_stale
    }

    pub(crate) fn sync(&mut self, editor: &Editor) {
        let Ok((context, view)) = current(editor) else {
            self.document_pending = true;
            return;
        };
        self.document_pending = false;
        let identity = Identity::from_context(&context);
        let same_document = self
            .identity
            .as_ref()
            .is_some_and(|old| old.same_document(&identity));
        let changed = !same_document
            || (editor.mode() == Mode::Edit && self.identity.as_ref() != Some(&identity));
        if !changed {
            return;
        }
        if self.dirty() {
            self.context_stale = true;
            self.status = "Hazard draft is stale after a scene, graph, or document edit; reload before applying.".into();
            return;
        }
        // Never adopt runtime progress as the editable baseline.
        if editor.mode() != Mode::Edit {
            return;
        }
        self.load(identity, view.as_ref());
        self.status = "Loaded hazards from the authoritative scene.".into();
    }

    pub(crate) fn play_blocker(&self, editor: &Editor) -> Option<String> {
        if !editor.game().is_mesh_hazards() {
            return None;
        }
        if self.context_stale {
            return Some("Reload the stale hazard draft before Play".into());
        }
        if self.form_dirty {
            return Some("Apply or reload hazard fields before Play".into());
        }
        if self.document_pending || !self.identity_matches(editor) {
            return Some("Wait for the current local hazard scene and admitted graph".into());
        }
        None
    }

    fn editable(editor: &Editor) -> bool {
        editor.game().is_mesh_hazards()
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.in_gesture()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
    }

    fn identity_matches(&self, editor: &Editor) -> bool {
        current(editor).is_ok_and(|(context, _)| {
            self.identity.as_ref() == Some(&Identity::from_context(&context))
        })
    }

    fn can_edit(&self, editor: &Editor) -> bool {
        Self::editable(editor)
            && !self.document_pending
            && !self.context_stale
            && self.identity_matches(editor)
    }

    fn can_reload(&self, editor: &Editor) -> bool {
        Self::editable(editor) && current(editor).is_ok()
    }

    fn load(&mut self, identity: Identity, view: Option<&HazardsView>) {
        if let Some(view) = view {
            self.enabled = true;
            self.count = view.config.count.to_string();
            self.spheres = view.config.spheres.map(format_sphere);
        } else {
            self.enabled = false;
            self.count = "1".into();
            self.spheres = default_spheres();
        }
        self.identity = Some(identity);
        self.form_dirty = false;
        self.context_stale = false;
        self.document_pending = false;
        self.error = None;
    }

    fn parse_form(&self, context: &Context) -> Result<MeshHazards, String> {
        let mut config = MeshHazards::default();
        if self.enabled {
            let count = parse_count(&self.count)?;
            config.enabled = 1;
            config.count = count as u32;
            config.graph_revision = context.graph_revision;
            for index in 0..count {
                config.spheres[index] = parse_sphere(&self.spheres[index])?;
            }
        }
        // Use the same graph-pinned validation as the remote host. Unused slots and
        // every byte of a disabled configuration remain canonical zero.
        hazards::patch_request(&config, &context.mesh)?;
        Ok(config)
    }

    fn apply(&mut self, editor: &mut Editor) {
        if !self.can_edit(editor) {
            self.error = Some(
                "Apply requires the matching editable local hazard scene; reload stale fields."
                    .into(),
            );
            return;
        }
        let result: Result<(), String> = (|| {
            let (context, _) = current(editor)?;
            let config = self.parse_form(&context)?;
            let request = hazards::patch_request(&config, &context.mesh)?;
            editor.host_call("world.singleton.patch", request)?;
            editor.mesh_navigation_mark_edited();
            editor.sync();
            // Keep the draft until the exact configuration is observed on the
            // same scene-load epoch and graph, even when the RPC succeeded.
            self.refresh_after_apply(editor, config)?;
            self.status = "Hazards applied; save the scene explicitly.".into();
            Ok(())
        })();
        if let Err(error) = result {
            self.error = Some(format!("Apply hazards failed: {error}"));
        }
    }

    fn refresh_after_apply(
        &mut self,
        editor: &Editor,
        expected: MeshHazards,
    ) -> Result<(), String> {
        self.context_stale = true;
        let (context, view) = current(editor)?;
        let identity = Identity::from_context(&context);
        if !self
            .identity
            .as_ref()
            .is_some_and(|old| old.same_document(&identity))
        {
            return Err("Authoritative hazard readback belongs to another scene or graph".into());
        }
        let actual = view.as_ref().map(|view| view.config).unwrap_or_default();
        if actual != expected {
            return Err(
                "Authoritative hazard readback differs from the applied configuration".into(),
            );
        }
        self.load(identity, view.as_ref());
        Ok(())
    }

    fn reload(&mut self, editor: &Editor) {
        if !self.can_reload(editor) {
            self.error =
                Some("Reload requires an editable local hazard scene and current graph.".into());
            return;
        }
        match current(editor) {
            Ok((context, view)) => {
                self.load(Identity::from_context(&context), view.as_ref());
                self.status = "Reloaded authoritative hazards; discarded draft fields.".into();
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(crate) fn show(&mut self, ui: &mut Ui, editor: &mut Editor) {
        if !editor.game().is_mesh_hazards() {
            return;
        }
        self.sync(editor);
        ui.collapsing("Mesh hazards", |ui| {
            ui.weak("Spherical hazards use exact fixed-point coordinates. Runtime loss state is read-only.");
            #[cfg(feature="mesh-hazards-export")]
            ui.weak("Create a scene with File > New hazard game. File > Export hazard game supports scene-only bundles with enabled hazards; workspace copying and composed presentations remain unsupported.");
            #[cfg(not(feature="mesh-hazards-export"))]
            ui.weak("Create a scene with File > New hazard game. Enable mesh-hazards-export for scene-only standalone export; workspace copying remains unsupported.");
            ui.add_enabled_ui(self.can_edit(editor), |ui| {
                let mut changed = ui.checkbox(&mut self.enabled, "Enable mesh hazards").changed();
                changed |= text_field(ui, "Hazard count (1–8)", &mut self.count);
                for index in 0..parse_count(&self.count).unwrap_or(0) {
                    for (axis, label) in ["X", "Y", "Z", "radius"].into_iter().enumerate() {
                        changed |= text_field(ui, &format!("Hazard {} {label}", index + 1), &mut self.spheres[index][axis]);
                    }
                }
                if changed { self.form_dirty = true; self.error = None; }
                if ui.add_enabled(self.form_dirty, egui::Button::new("Apply mesh hazards")).clicked() { self.apply(editor); }
            });
            if ui.add_enabled(self.can_reload(editor), egui::Button::new("Reload hazard fields")).clicked() { self.reload(editor); }
            if self.document_pending { ui.label("Waiting for the requested local hazard scene and admitted graph."); }
            if self.dirty() { ui.colored_label(egui::Color32::YELLOW, "Unsaved or stale hazard fields block Play and workspace changes."); }
            if let Ok((_, view)) = current(editor) {
                match view {
                    Some(view) if view.run.lost == 1 => { ui.label(format!("Lost · hazard {} · tick {}", view.run.hit_index + 1, view.run.lost_tick)); }
                    Some(_) => { ui.label("Hazards enabled · no loss"); }
                    None => { ui.label("Hazards disabled"); }
                }
            }
            if let Some(error) = &self.error { ui.colored_label(egui::Color32::RED, error); }
            else if !self.status.is_empty() { ui.label(&self.status); }
        });
    }
}

fn current(editor: &Editor) -> Result<(Context, Option<HazardsView>), String> {
    // Typed singleton access is safe only after the explicit registry check.
    if !editor.game().is_mesh_hazards() {
        return Err("Select the explicit MeshHazardsV1 profile".into());
    }
    let context = mesh_context::current(editor)?;
    if !context.game.is_mesh_hazards() {
        return Err("Hazard context profile changed".into());
    }
    let frame = editor
        .snapshot()
        .ok_or("Hazard frame unavailable")?
        .predicted();
    Ok((context, hazards::from_view(frame)?))
}

fn default_spheres() -> [[String; 4]; MAX_HAZARDS] {
    std::array::from_fn(|_| ["1".into(), "0".into(), "1".into(), "0.25".into()])
}
fn parse_count(text: &str) -> Result<usize, String> {
    let text = text.trim();
    let count = text
        .parse::<usize>()
        .ok()
        .filter(|count| text.len() == 1 && (1..=MAX_HAZARDS).contains(count));
    count.ok_or_else(|| "Enabled hazards require an integer count from 1 to 8".into())
}
fn parse_sphere(values: &[String; 4]) -> Result<HazardSphere, String> {
    let mut parsed = [FP::ZERO; 4];
    for (index, value) in values.iter().enumerate() {
        if value.len() > 48 {
            return Err("Hazard decimal is too long".into());
        }
        parsed[index] = orr_reflect::decimal::parse_fp(value.trim())
            .map_err(|error| format!("Invalid hazard fixed-point decimal: {error:?}"))?;
    }
    Ok(HazardSphere {
        center: [parsed[0], parsed[1], parsed[2]],
        radius: parsed[3],
    })
}
fn format_sphere(sphere: HazardSphere) -> [String; 4] {
    [
        sphere.center[0],
        sphere.center[1],
        sphere.center[2],
        sphere.radius,
    ]
    .map(orr_reflect::decimal::fp_to_decimal)
}
fn text_field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let response = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(ui.available_width())
            .char_limit(48),
    )
    .labelled_by(response.id)
    .changed()
}

#[cfg(test)]
#[path = "mesh_hazards_panel/tests.rs"]
mod tests;
