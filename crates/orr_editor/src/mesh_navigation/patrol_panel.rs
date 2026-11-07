//! Optional route configuration. Draft fields never mutate a Frame; Apply uses
//! the existing atomic singleton patch and ordinary scene save/history path.
use super::*;
use orr_remote::mesh_navigation::patrol::{self, MeshPatrol};
const MAX_POINTS: usize = 8;
fn supported_profile(game: EditorGame) -> bool {
    game.is_mesh_navigation() || game.is_mesh_hazards()
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    scene: PathBuf,
    epoch: u64,
    checksum: u64,
    revision: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Form {
    enabled: bool,
    points: Vec<[String; 3]>,
    looped: bool,
    wait: String,
}
impl Default for Form {
    fn default() -> Self {
        Self {
            enabled: false,
            points: vec![["1".into(), "0".into(), "1".into()]],
            looped: false,
            wait: "0".into(),
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    form: Form,
    context: Option<Context>,
    dirty: bool,
    stale: bool,
    status: String,
}
fn parse(form: &Form, revision: [u8; 32]) -> Result<MeshPatrol, String> {
    let mut config = MeshPatrol {
        enabled: 0,
        count: 0,
        looped: 0,
        wait_ticks: 0,
        points: [[FP::ZERO; 3]; 8],
        graph_revision: [0; 32],
    };
    if !form.enabled {
        return Ok(config);
    }
    if !(1..=MAX_POINTS).contains(&form.points.len()) {
        return Err("Patrol requires 1..8 waypoints".into());
    }
    let wait = form
        .wait
        .trim()
        .parse::<u32>()
        .map_err(|_| "Patrol wait ticks must be an integer in 0..3600")?;
    if wait > 3600 {
        return Err("Patrol wait ticks must be in 0..3600".into());
    }
    config.enabled = 1;
    config.count = form.points.len() as u32;
    config.looped = u32::from(form.looped);
    config.wait_ticks = wait;
    config.graph_revision = revision;
    for (index, point) in form.points.iter().enumerate() {
        for (axis, text) in point.iter().enumerate() {
            let value = parse_fixed(text).map_err(|e| {
                format!(
                    "Patrol waypoint {} {}: {e}",
                    index + 1,
                    ["X", "Y", "Z"][axis]
                )
            })?;
            if value < FP::from_int(-256) || value > FP::from_int(256) {
                return Err("Patrol coordinates must be within -256..256".into());
            }
            config.points[index][axis] = value;
        }
    }
    Ok(config)
}
impl State {
    fn load(&mut self, context: Context, config: Option<MeshPatrol>, goal: [FP; 3]) {
        self.form = match config {
            Some(config) if config.enabled == 1 => Form {
                enabled: true,
                points: config
                    .points
                    .iter()
                    .take(config.count as usize)
                    .map(|p| p.map(format_fixed))
                    .collect(),
                looped: config.looped == 1,
                wait: config.wait_ticks.to_string(),
            },
            _ => Form {
                points: vec![goal.map(format_fixed)],
                ..Default::default()
            },
        };
        self.context = Some(context);
        self.dirty = false;
        self.stale = false;
        self.status =
            "Patrol fields match the current scene. Apply changes, then save the scene explicitly."
                .into();
    }
    fn sync_context(&mut self, current: Option<&Context>) {
        if self.context.as_ref() != current && self.dirty {
            self.stale = true;
            self.status="Patrol draft is stale after a scene or graph change; reload fields before applying.".into();
        }
    }
    fn controls(&mut self, ui: &mut Ui, editable: bool) -> bool {
        let before = self.form.clone();
        let mut apply = false;
        ui.add_enabled_ui(editable && !self.stale, |ui| {
            ui.checkbox(&mut self.form.enabled, "Enable waypoint patrol");
            if self.form.enabled {
                let mut remove = None;
                let can_remove = self.form.points.len() > 1;
                for (index, point) in self.form.points.iter_mut().enumerate() {
                    for (axis, value) in ["X", "Y", "Z"].into_iter().zip(point) {
                        field(ui, &format!("Patrol waypoint {} {axis}", index + 1), value);
                    }
                    if ui
                        .add_enabled(
                            can_remove,
                            egui::Button::new(format!("Remove patrol waypoint {}", index + 1)),
                        )
                        .clicked()
                    {
                        remove = Some(index);
                    }
                }
                if let Some(index) = remove {
                    self.form.points.remove(index);
                }
                if ui
                    .add_enabled(
                        self.form.points.len() < MAX_POINTS,
                        egui::Button::new("Add patrol waypoint"),
                    )
                    .clicked()
                {
                    let point = self
                        .form
                        .points
                        .last()
                        .cloned()
                        .unwrap_or_else(|| ["0".into(), "0".into(), "0".into()]);
                    self.form.points.push(point);
                }
                ui.checkbox(&mut self.form.looped, "Loop patrol");
                field(ui, "Patrol wait ticks", &mut self.form.wait);
            }
            if self.form != before {
                self.dirty = true;
            }
            apply = ui
                .add_enabled(self.dirty, egui::Button::new("Apply waypoint patrol"))
                .clicked();
        });
        apply
    }
}
impl Panel {
    fn patrol_context(
        &self,
        editor: &Editor,
    ) -> Result<(Context, orr_remote::mesh_navigation::MeshView), String> {
        if !supported_profile(editor.game())
            || !editor.spec().is_local()
            || editor.mode() != Mode::Edit
            || !editor.can_mutate()
            || editor.previewing().is_some()
            || editor.in_gesture()
            || !editor.proposals().is_empty()
        {
            return Err(
                "Patrol requires a coherent writable local MeshNavigation or MeshHazards Edit host"
                    .into(),
            );
        }
        if self.geometry.dirty()
            || self.geometry_staged
            || self.geometry_vertex_busy()
            || self.agent_pick_busy()
            || self.props.gesture_busy()
            || self.draft_dirty
        {
            return Err(
                "Finish or discard mesh geometry, source and agent drafts before editing patrol"
                    .into(),
            );
        }
        if self.asset_path.trim() != self.loaded_asset_source.as_deref().unwrap_or_default()
            || self.model_path.trim() != self.loaded_model_source.as_deref().unwrap_or_default()
        {
            return Err("Open the selected mesh/model source before editing patrol".into());
        }
        let scene = editor.path().ok_or("Patrol scene is unavailable")?;
        if !crate::mesh_context::reported_scene_matches_request(
            editor.spec(),
            editor.game(),
            &scene,
        ) {
            return Err("Patrol scene source changed".into());
        }
        let snapshot = editor.snapshot().ok_or("Wait for a current mesh Frame")?;
        let frame = snapshot.predicted();
        let view = mesh_from_view(frame)?;
        patrol::from_view(frame)?;
        if self.loaded_scene.as_ref() != Some(&scene)
            || self.loaded_scene_epoch != Some(editor.scene_load_epoch())
            || self.graph.as_ref().map(StaticMeshGraph::revision) != Some(view.graph.revision())
        {
            return Err("Place or reload the current mesh agent before configuring patrol".into());
        }
        Ok((
            Context {
                scene,
                epoch: editor.scene_load_epoch(),
                checksum: frame.checksum(),
                revision: view.graph.revision(),
            },
            view,
        ))
    }
    pub(super) fn sync_patrol(&mut self, editor: &Editor) {
        let current = self.patrol_context(editor).ok();
        self.patrol.sync_context(current.as_ref().map(|v| &v.0));
        if !self.patrol.dirty {
            if let Some((context, view)) = current {
                if self.patrol.context.as_ref() != Some(&context) {
                    let config = editor
                        .snapshot()
                        .and_then(|s| patrol::from_view(s.predicted()).ok().flatten())
                        .map(|p| p.config);
                    self.patrol.load(context, config, view.pin.goal);
                }
            }
        }
    }
    fn apply_patrol(&mut self, editor: &mut Editor) -> Result<(), String> {
        let (context, view) = self.patrol_context(editor)?;
        if self.patrol.stale || self.patrol.context.as_ref() != Some(&context) {
            return Err("Patrol draft context changed; reload before applying".into());
        }
        let config = parse(&self.patrol.form, context.revision)?;
        let request = patrol::patch_request(&config, &view)?;
        editor.host_call("world.singleton.patch", request)?;
        editor.mesh_navigation_mark_edited();
        editor.sync();
        let snapshot = editor
            .snapshot()
            .ok_or("Applied patrol Frame is unavailable")?;
        let actual = patrol::from_view(snapshot.predicted())?;
        if config.enabled == 1 && actual.as_ref().map(|p| p.config) != Some(config)
            || config.enabled == 0 && actual.is_some()
        {
            return Err("Applied patrol readback differs from requested configuration".into());
        }
        self.patrol.dirty = false;
        self.patrol.context = None;
        self.sync(editor);
        self.patrol.status = "Patrol applied as one scene edit; save the scene explicitly.".into();
        Ok(())
    }
    pub(super) fn show_patrol(&mut self, ui: &mut Ui, editor: &mut Editor) {
        self.sync_patrol(editor);
        ui.collapsing("Waypoint patrol",|ui| {
            ui.weak("Point-agent waypoints; radius, headroom and clearance are not checked. Staging does not change game state.");
            let current=self.patrol_context(editor);let editable=current.is_ok();
            if let Err(reason)=&current {ui.label(reason);}
            if self.patrol.controls(ui,editable) {if let Err(error)=self.apply_patrol(editor){self.error=Some(format!("Apply waypoint patrol failed: {error}"));}}
            if ui.add_enabled(editable,egui::Button::new("Reload patrol fields")).clicked(){if let Ok((context,view))=current {let config=editor.snapshot().and_then(|s|patrol::from_view(s.predicted()).ok().flatten()).map(|p|p.config);self.patrol.load(context,config,view.pin.goal);}}
            if !self.patrol.status.is_empty(){ui.label(&self.patrol.status);}
        });
    }
}
#[cfg(test)]
#[path = "patrol_panel_tests.rs"]
mod tests;
