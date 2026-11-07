//! Terrain patrol drafts are source-bound UI state. Apply is one existing
//! singleton transaction; scene Save/Undo/Redo remain explicit and shared.
use super::*;
use orr_remote::navigation_yard3d::{
    navigation_from_view,
    patrol::{self, TerrainPatrol},
    NavigationView,
};
use std::path::PathBuf;
const MAX_POINTS: usize = 8;
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    scene: PathBuf,
    epoch: u64,
    checksum: u64,
    pin: NavigationScenePin,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Form {
    enabled: bool,
    points: Vec<[String; 2]>,
    looped: bool,
    wait: String,
}
impl Default for Form {
    fn default() -> Self {
        Self {
            enabled: false,
            points: vec![["0".into(), "0".into()]],
            looped: false,
            wait: "0".into(),
        }
    }
}
#[derive(Default)]
pub(super) struct State {
    form: Form,
    context: Option<Context>,
    pub(super) dirty: bool,
    pub(super) stale: bool,
    status: String,
}
fn parse(form: &Form, revision: [u8; 32]) -> Result<TerrainPatrol, String> {
    let mut config = TerrainPatrol {
        enabled: 0,
        count: 0,
        looped: 0,
        wait_ticks: 0,
        points: [[FP::ZERO; 2]; 8],
        graph_revision: [0; 32],
    };
    if !form.enabled {
        return Ok(config);
    }
    if !(1..=MAX_POINTS).contains(&form.points.len()) {
        return Err("Terrain patrol requires 1..8 waypoints".into());
    }
    let wait = form
        .wait
        .trim()
        .parse::<u32>()
        .map_err(|_| "Terrain patrol wait ticks must be an integer in 0..3600")?;
    if wait > 3600 {
        return Err("Terrain patrol wait ticks must be within 0..3600".into());
    }
    config.enabled = 1;
    config.count = form.points.len() as u32;
    config.looped = u32::from(form.looped);
    config.wait_ticks = wait;
    config.graph_revision = revision;
    for (index, point) in form.points.iter().enumerate() {
        for (axis, text) in point.iter().enumerate() {
            config.points[index][axis] = parse_fixed(text).map_err(|e| {
                format!(
                    "Terrain patrol waypoint {} {}: {e}",
                    index + 1,
                    ["X", "Z"][axis]
                )
            })?;
        }
    }
    Ok(config)
}
impl State {
    fn load(&mut self, context: Context, config: Option<TerrainPatrol>) {
        self.form = match config {
            Some(c) if c.enabled == 1 => Form {
                enabled: true,
                points: c
                    .points
                    .iter()
                    .take(c.count as usize)
                    .map(|p| p.map(format_fixed))
                    .collect(),
                looped: c.looped == 1,
                wait: c.wait_ticks.to_string(),
            },
            _ => Form {
                points: vec![context.pin.agent.goal.map(format_fixed)],
                ..Default::default()
            },
        };
        self.context = Some(context);
        self.dirty = false;
        self.stale = false;
        self.status="Terrain patrol fields match the current admitted source. Apply, then save the scene explicitly.".into();
    }
    fn sync_context(&mut self, current: Option<&Context>) {
        if self.dirty && self.context.as_ref() != current {
            self.stale = true;
            self.status="Terrain patrol draft is stale after scene, epoch, terrain or route changes; reload before applying.".into();
        }
    }
    fn controls(&mut self, ui: &mut Ui, editable: bool) -> bool {
        let before = self.form.clone();
        let mut apply = false;
        let editable = editable
            && ui.ctx().input(|input| {
                input.focused
                    && !input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::WindowFocused(false)))
            });
        ui.add_enabled_ui(editable && !self.stale, |ui| {
            ui.checkbox(&mut self.form.enabled, "Enable terrain waypoint patrol");
            if self.form.enabled {
                let can_remove = self.form.points.len() > 1;
                let mut remove = None;
                for (index, point) in self.form.points.iter_mut().enumerate() {
                    for (axis, value) in ["X", "Z"].into_iter().zip(point) {
                        field(
                            ui,
                            &format!("Terrain patrol waypoint {} {axis}", index + 1),
                            value,
                        );
                    }
                    if ui
                        .add_enabled(
                            can_remove,
                            egui::Button::new(format!(
                                "Remove terrain patrol waypoint {}",
                                index + 1
                            )),
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
                        egui::Button::new("Add terrain patrol waypoint"),
                    )
                    .clicked()
                {
                    self.form.points.push(
                        self.form
                            .points
                            .last()
                            .cloned()
                            .unwrap_or_else(|| ["0".into(), "0".into()]),
                    );
                }
                ui.checkbox(&mut self.form.looped, "Loop terrain patrol");
                field(ui, "Terrain patrol wait ticks", &mut self.form.wait);
            }
            if self.form != before {
                self.dirty = true;
            }
            apply = ui
                .add_enabled(
                    self.dirty,
                    egui::Button::new("Apply terrain waypoint patrol"),
                )
                .clicked();
        });
        apply
    }
}
impl NavigationPanel {
    fn patrol_context(
        &self,
        editor: &Editor,
        terrain: &TerrainPanel,
    ) -> Result<(Context, NavigationView), String> {
        self.source(editor, terrain)?;
        if !editor.yard_rows_coherent()
            || editor.in_gesture()
            || !editor.proposals().is_empty()
            || editor.down().is_some()
        {
            return Err("Terrain patrol requires a coherent local Edit snapshot without active gestures or proposals".into());
        }
        if self.route_stale(editor, terrain) {
            return Err(
                "Save terrain and Build the current route before editing terrain patrol".into(),
            );
        }
        let scene = editor
            .path()
            .ok_or("Terrain patrol requires a saved scene")?;
        let snapshot = editor
            .snapshot()
            .ok_or("Wait for an admitted terrain navigation Frame")?;
        let frame = snapshot.predicted();
        let view = navigation_from_view(frame)?;
        patrol::from_view(frame)?;
        Ok((
            Context {
                scene,
                epoch: editor.scene_load_epoch(),
                checksum: frame.checksum(),
                pin: *frame.singleton::<NavigationScenePin>(),
            },
            view,
        ))
    }
    fn sync_patrol(&mut self, editor: &Editor, terrain: &TerrainPanel) {
        let current = self.patrol_context(editor, terrain).ok();
        self.patrol.sync_context(current.as_ref().map(|v| &v.0));
        if !self.patrol.dirty {
            if let Some((context, _)) = current {
                if self.patrol.context.as_ref() != Some(&context) {
                    let config = editor
                        .snapshot()
                        .and_then(|s| patrol::from_view(s.predicted()).ok().flatten())
                        .map(|v| v.config);
                    self.patrol.load(context, config);
                }
            }
        }
    }
    fn apply_patrol(&mut self, editor: &mut Editor, terrain: &TerrainPanel) -> Result<(), String> {
        let (context, view) = self.patrol_context(editor, terrain)?;
        if self.patrol.stale || self.patrol.context.as_ref() != Some(&context) {
            return Err("Terrain patrol source changed; reload fields before applying".into());
        }
        let config = parse(&self.patrol.form, context.pin.graph_revision)?;
        let request = patrol::patch_request(&config, &view)?;
        editor.host_call("world.singleton.patch", request)?;
        editor.navigation_mark_edited();
        editor.sync();
        let actual = patrol::from_view(
            editor
                .snapshot()
                .ok_or("Applied terrain patrol Frame unavailable")?
                .predicted(),
        )?;
        if config.enabled == 1 && actual.as_ref().map(|v| v.config) != Some(config)
            || config.enabled == 0 && actual.is_some()
        {
            return Err(
                "Applied terrain patrol readback differs from the requested configuration".into(),
            );
        }
        self.patrol.dirty = false;
        self.patrol.context = None;
        self.sync_from_editor(editor);
        self.sync_patrol(editor, terrain);
        self.patrol.status =
            "Terrain patrol applied as one scene edit; save the scene explicitly.".into();
        Ok(())
    }
    pub(super) fn show_patrol(&mut self, ui: &mut Ui, editor: &mut Editor, terrain: &TerrainPanel) {
        self.sync_patrol(editor, terrain);
        ui.collapsing("Terrain waypoint patrol",|ui|{
            ui.weak("Point routes on the admitted terrain; no radius, headroom or step-clearance guarantee. Staging never moves the agent.");
            let current=self.patrol_context(editor,terrain);let editable=current.is_ok();if let Err(reason)=&current {ui.label(reason);}
            if self.patrol.controls(ui,editable){if let Err(error)=self.apply_patrol(editor,terrain){self.error=Some(format!("Apply terrain patrol failed: {error}"));}}
            if ui.add_enabled(editable,egui::Button::new("Reload terrain patrol fields")).clicked(){if let Ok((context,_))=current {let config=editor.snapshot().and_then(|s|patrol::from_view(s.predicted()).ok().flatten()).map(|v|v.config);self.patrol.load(context,config);}}
            if !self.patrol.status.is_empty(){ui.label(&self.patrol.status);}
        });
    }
}
#[cfg(test)]
#[path = "patrol_panel_tests.rs"]
mod tests;
