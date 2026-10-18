use crate::{editor::Editor, game::EditorGame, model::Mode};
use egui::Ui;
use orr_fp::FP;
use orr_remote::mesh_navigation::{
    mesh_from_view,
    mission::{self, MeshMission, MissionPoint, MissionView},
};
use std::path::{Path, PathBuf};

const MAX_MISSION_POINTS: usize = 8;
const MAX_COORDINATE_RAW: i64 = 256 * FP::SCALE;
const MAX_RADIUS_RAW: i64 = 4 * FP::SCALE;

#[path = "mesh_mission_panel/viewport_picking.rs"]
mod viewport_picking;
pub(crate) use viewport_picking::ViewInput;

pub(crate) struct Panel {
    enabled: bool,
    mission_present: bool,
    count: String,
    points: [[String; 4]; MAX_MISSION_POINTS],
    exit: [String; 4],
    form_dirty: bool,
    context_stale: bool,
    document_pending: bool,
    confirm_remove: bool,
    scene: Option<PathBuf>,
    scene_epoch: Option<u64>,
    graph_revision: Option<[u8; 32]>,
    observed_checksum: Option<u64>,
    status: String,
    error: Option<String>,
    viewport_picking: viewport_picking::State,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            enabled: false,
            mission_present: false,
            count: "2".into(),
            points: default_points(),
            exit: ["3".into(), "2".into(), "1".into(), "0.5".into()],
            form_dirty: false,
            context_stale: false,
            document_pending: true,
            confirm_remove: false,
            scene: None,
            scene_epoch: None,
            graph_revision: None,
            observed_checksum: None,
            status: String::new(),
            error: None,
            viewport_picking: viewport_picking::State::default(),
        }
    }
}

impl Panel {
    pub(crate) fn sync(&mut self, editor: &Editor) {
        self.sync_mission_picking(editor);
        if editor.game() != EditorGame::MeshNavigation || !editor.spec().is_local() {
            self.document_pending = true;
            return;
        }
        let Some((scene, graph_revision, checksum, mission_view)) = current_identity(editor) else {
            self.document_pending = true;
            self.status = "Waiting for the requested local mesh scene and admitted graph.".into();
            return;
        };
        self.document_pending = false;
        let scene_epoch = editor.scene_load_epoch();
        let same_context = self.scene.as_ref() == Some(&scene)
            && self.scene_epoch == Some(scene_epoch)
            && self.graph_revision == Some(graph_revision);
        if same_context {
            if editor.mode() == Mode::Edit
                && self
                    .observed_checksum
                    .is_some_and(|value| value != checksum)
            {
                if self.dirty() {
                    self.context_stale = true;
                    self.status = "Mesh mission draft is stale after another scene edit; reload or discard it before applying.".into();
                } else {
                    self.load_fields(mission_view.as_ref());
                    self.observed_checksum = Some(checksum);
                    self.status = "Reloaded mesh mission after the scene changed.".into();
                }
            }
            self.update_progress(mission_view.as_ref());
            return;
        }
        if self.dirty() {
            self.context_stale = true;
            self.status = "Unsaved mesh mission belongs to another scene or graph; reload or discard it before continuing.".into();
            return;
        }
        self.scene = Some(scene);
        self.scene_epoch = Some(scene_epoch);
        self.graph_revision = Some(graph_revision);
        self.observed_checksum = Some(checksum);
        self.context_stale = false;
        self.error = None;
        self.confirm_remove = false;
        self.load_fields(mission_view.as_ref());
        self.status = if mission_view.is_some() {
            "Loaded the saved mesh mission from the current scene.".into()
        } else {
            "No mesh mission is configured; apply one explicitly.".into()
        };
    }

    pub(crate) fn show(&mut self, ui: &mut Ui, editor: &mut Editor) {
        if editor.game() != EditorGame::MeshNavigation {
            return;
        }
        self.sync(editor);
        ui.collapsing("Mesh mission", |ui| {
            ui.weak("Collectibles and an exit are scene gameplay state. Reaching the route endpoint alone never declares a win.");
            self.show_mission_picking(ui, editor);
            if self.document_pending {
                ui.label("Waiting for the requested local mesh scene and admitted graph.");
            }
            let mut changed = false;
            ui.add_enabled_ui(self.can_edit(editor) && !self.mission_pick_busy(), |ui| {
                changed |= ui.checkbox(&mut self.enabled, "Enable mesh mission").changed();
                changed |= text_field(ui, "Mission collectible count", &mut self.count);
                let point_count = parse_count(&self.count).unwrap_or(0);
                for index in 0..point_count {
                    ui.label(format!("Collectible {}", index + 1));
                    for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                        changed |= text_field(
                            ui,
                            &format!("Collectible {} {label}", index + 1),
                            &mut self.points[index][axis],
                        );
                    }
                    changed |= text_field(
                        ui,
                        &format!("Collectible {} radius", index + 1),
                        &mut self.points[index][3],
                    );
                }
                ui.label("Mission exit");
                for (axis, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                    changed |= text_field(ui, &format!("Mission exit {label}"), &mut self.exit[axis]);
                }
                changed |= text_field(ui, "Mission exit radius", &mut self.exit[3]);
                if changed {
                    self.form_dirty = true;
                    self.error = None;
                    self.status = "Mesh mission fields changed; apply them before Play.".into();
                }
                if ui
                    .add_enabled(
                        self.form_dirty && !self.mission_pick_busy(),
                        egui::Button::new("Apply mesh mission"),
                    )
                    .clicked()
                {
                    self.apply(editor);
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.can_reload(editor) && !self.mission_pick_busy(),
                        egui::Button::new("Reload mission fields"),
                    )
                    .clicked()
                {
                    self.reload(editor);
                }
                self.confirm_remove = self.confirm_remove && self.current_mission_enabled();
                ui.checkbox(&mut self.confirm_remove, "I confirm removing the mesh mission");
                if ui
                    .add_enabled(
                        self.can_edit(editor)
                            && !self.mission_pick_busy()
                            && self.current_mission_enabled()
                            && self.confirm_remove,
                        egui::Button::new("Remove mesh mission"),
                    )
                    .clicked()
                {
                    self.remove(editor);
                }
            });
            self.show_run_state(editor, ui);
            if self.dirty() {
                ui.colored_label(egui::Color32::YELLOW, "Unsaved mesh mission fields block Play and workspace changes.");
            }
            if self.context_stale {
                ui.weak("Reload mission fields to discard this stale draft.");
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            } else if !self.status.is_empty() {
                ui.label(&self.status);
            }
        });
    }

    pub(crate) fn dirty(&self) -> bool {
        self.form_dirty || self.context_stale
    }

    pub(crate) fn play_blocker(&self, editor: &Editor) -> Option<String> {
        if editor.game() != EditorGame::MeshNavigation {
            return None;
        }
        if self.mission_pick_busy() {
            return Some("Finish or cancel mission point picking before Play".into());
        }
        if self.document_pending || current_identity(editor).is_none() {
            return Some("Wait for the requested local mesh scene and admitted graph".into());
        }
        if self.context_stale {
            return Some("Reload or discard the stale mesh mission draft before Play".into());
        }
        if self.form_dirty {
            return Some("Apply or reload mesh mission fields before Play".into());
        }
        None
    }

    fn current_mission_enabled(&self) -> bool {
        self.mission_present
    }

    fn can_edit(&self, editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.in_gesture()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && !self.document_pending
            && !self.context_stale
            && self.scene.is_some()
            && self.identity_matches(editor)
    }

    fn can_reload(&self, editor: &Editor) -> bool {
        editor.game() == EditorGame::MeshNavigation
            && editor.spec().is_local()
            && editor.mode() == Mode::Edit
            && editor.can_mutate()
            && !editor.in_gesture()
            && editor.previewing().is_none()
            && editor.proposals().is_empty()
            && !self.document_pending
            && current_identity(editor).is_some()
    }

    fn identity_matches(&self, editor: &Editor) -> bool {
        current_identity(editor).is_some_and(|(scene, revision, _, _)| {
            self.scene.as_ref() == Some(&scene)
                && self.scene_epoch == Some(editor.scene_load_epoch())
                && self.graph_revision == Some(revision)
        })
    }

    fn load_fields(&mut self, view: Option<&MissionView>) {
        if let Some(view) = view {
            let config = view.config;
            self.mission_present = true;
            self.enabled = config.enabled == 1;
            self.count = config.count.to_string();
            for index in 0..MAX_MISSION_POINTS {
                self.points[index] = format_point(config.points[index]);
            }
            self.exit = format_point(config.exit);
        } else {
            self.mission_present = false;
            self.enabled = false;
            self.count = "2".into();
            self.points = default_points();
            self.exit = ["3".into(), "2".into(), "1".into(), "0.5".into()];
        }
        self.form_dirty = false;
        self.context_stale = false;
        self.confirm_remove = false;
    }

    fn update_progress(&mut self, view: Option<&MissionView>) {
        if let Some(view) = view {
            let required = view.config.count;
            let collected = (view.run.collected & ((1u32 << required) - 1)).count_ones();
            self.status = if view.run.won == 1 {
                format!("Mission complete · collected {collected}/{required}")
            } else {
                format!("Mission in progress · collected {collected}/{required}")
            };
        } else {
            self.status = "Mission disabled".into();
        }
    }

    fn parse_form(&self, editor: &Editor) -> Result<MeshMission, String> {
        let Some((_, graph_revision, _, _)) = current_identity(editor) else {
            return Err("The current mesh graph is not admitted".into());
        };
        if !self.enabled {
            return Ok(disabled_mission());
        }
        let count = parse_count(&self.count)?;
        let snapshot = editor.snapshot().ok_or("Mesh frame is unavailable")?;
        let frame = snapshot.predicted();
        let view = mesh_from_view(frame)?;
        let mut config = MeshMission {
            enabled: 1,
            count: count as u32,
            points: [MissionPoint::default(); MAX_MISSION_POINTS],
            exit: MissionPoint::default(),
            graph_revision,
        };
        for index in 0..count {
            config.points[index] =
                parse_point(&self.points[index], &format!("collectible {}", index + 1))?;
        }
        config.exit = parse_point(&self.exit, "mission exit")?;
        mission::validate_config(&config, &view.graph, view.pin.max_vertical)?;
        Ok(config)
    }

    fn apply(&mut self, editor: &mut Editor) {
        if !self.can_edit(editor) {
            self.error = Some("Apply requires the matching editable local mesh scene".into());
            return;
        }
        let result: Result<(), String> = (|| {
            let config = self.parse_form(editor)?;
            let snapshot = editor.snapshot().ok_or("Mesh frame is unavailable")?;
            let mesh_view = mesh_from_view(snapshot.predicted())?;
            let request = mission::patch_request(&config, &mesh_view)?;
            editor.host_call("world.singleton.patch", request)?;
            editor.mesh_navigation_mark_edited();
            editor.sync();
            self.form_dirty = false;
            self.context_stale = false;
            self.error = None;
            self.observed_checksum = Some(editor.checksum());
            self.status = if config.enabled == 1 {
                "Mesh mission applied to the scene; save the scene explicitly.".into()
            } else {
                "Mesh mission disabled with the canonical all-zero value; save the scene explicitly.".into()
            };
            self.sync(editor);
            Ok(())
        })();
        if let Err(error) = result {
            self.error = Some(format!("Apply mesh mission failed: {error}"));
        }
    }

    fn reload(&mut self, editor: &Editor) {
        if !self.can_reload(editor) {
            self.error = Some("Reload requires a matching editable local mesh scene".into());
            return;
        }
        if let Some((scene, revision, checksum, view)) = current_identity(editor) {
            self.scene = Some(scene);
            self.scene_epoch = Some(editor.scene_load_epoch());
            self.graph_revision = Some(revision);
            self.observed_checksum = Some(checksum);
            self.load_fields(view.as_ref());
            self.error = None;
            self.status = "Reloaded mission fields from the current scene.".into();
        }
    }

    fn remove(&mut self, editor: &mut Editor) {
        if !self.can_edit(editor) || !self.confirm_remove || !self.current_mission_enabled() {
            self.error = Some("Remove requires confirmation and an enabled mesh mission".into());
            return;
        }
        let result: Result<(), String> = (|| {
            let config = disabled_mission();
            let snapshot = editor.snapshot().ok_or("Mesh frame is unavailable")?;
            let mesh_view = mesh_from_view(snapshot.predicted())?;
            let request = mission::patch_request(&config, &mesh_view)?;
            editor.host_call("world.singleton.patch", request)?;
            editor.mesh_navigation_mark_edited();
            editor.sync();
            self.load_fields(None);
            self.observed_checksum = Some(editor.checksum());
            self.error = None;
            self.status = "Mesh mission removed using the canonical all-zero configuration; save the scene explicitly.".into();
            Ok(())
        })();
        if let Err(error) = result {
            self.error = Some(format!("Remove mesh mission failed: {error}"));
        }
    }

    fn show_run_state(&self, editor: &Editor, ui: &mut Ui) {
        let Some(snapshot) = editor.snapshot() else {
            return;
        };
        match mission::from_view(snapshot.predicted()) {
            Ok(Some(view)) => {
                let required = view.config.count;
                let collected = (view.run.collected & ((1u32 << required) - 1)).count_ones();
                ui.label(format!("Mission collected: {collected}/{required}"));
                if view.run.won == 1 {
                    ui.strong("Mission complete");
                } else {
                    ui.label("Mission in progress");
                }
            }
            Ok(None) => {
                ui.label("Mission disabled");
            }
            Err(error) => {
                ui.colored_label(
                    egui::Color32::RED,
                    format!("Mission state unavailable: {error}"),
                );
            }
        };
    }
}

fn current_identity(editor: &Editor) -> Option<(PathBuf, [u8; 32], u64, Option<MissionView>)> {
    if editor.game() != EditorGame::MeshNavigation
        || !editor.spec().is_local()
        || editor.down().is_some()
    {
        return None;
    }
    let scene = editor.path()?;
    if !reported_scene_matches_request(editor.spec(), &scene) {
        return None;
    }
    let frame = editor.snapshot()?.predicted();
    if editor.mode() != Mode::Play && frame.checksum() != editor.sim().checksum {
        return None;
    }
    let view = mesh_from_view(frame).ok()?;
    let mission_view = mission::from_view(frame).ok()?;
    Some((scene, view.graph.revision(), frame.checksum(), mission_view))
}

fn reported_scene_matches_request(spec: &crate::backend::HostSpec, reported: &Path) -> bool {
    let requested = match spec {
        crate::backend::HostSpec::Local { scene, .. } => scene.as_path(),
        crate::backend::HostSpec::LocalGame {
            scene,
            game: EditorGame::MeshNavigation,
            ..
        } => scene.as_path(),
        _ => return false,
    };
    if requested == reported {
        return true;
    }
    matches!((std::fs::canonicalize(requested), std::fs::canonicalize(reported)), (Ok(requested), Ok(reported)) if requested == reported)
}

fn default_points() -> [[String; 4]; MAX_MISSION_POINTS] {
    std::array::from_fn(|index| {
        if index == 0 {
            ["1.5".into(), "0".into(), "1".into(), "0.4".into()]
        } else {
            ["2".into(), "0".into(), "1".into(), "0.4".into()]
        }
    })
}

fn disabled_mission() -> MeshMission {
    MeshMission {
        enabled: 0,
        count: 0,
        points: [MissionPoint::default(); MAX_MISSION_POINTS],
        exit: MissionPoint::default(),
        graph_revision: [0; 32],
    }
}

fn parse_count(text: &str) -> Result<usize, String> {
    let text = text.trim();
    if text.len() > 1 {
        return Err("Mission collectible count must be an integer from 1 to 8".into());
    }
    let count = text
        .parse::<usize>()
        .map_err(|_| "Mission collectible count must be an integer from 1 to 8")?;
    if !(1..=MAX_MISSION_POINTS).contains(&count) {
        return Err("Mission collectible count must be an integer from 1 to 8".into());
    }
    Ok(count)
}

fn parse_point(values: &[String; 4], label: &str) -> Result<MissionPoint, String> {
    let point = MissionPoint {
        position: [
            parse_fixed(&values[0], &format!("{label} X"))?,
            parse_fixed(&values[1], &format!("{label} Y"))?,
            parse_fixed(&values[2], &format!("{label} Z"))?,
        ],
        radius: parse_fixed(&values[3], &format!("{label} radius"))?,
    };
    if point
        .position
        .iter()
        .any(|coordinate| !(-MAX_COORDINATE_RAW..=MAX_COORDINATE_RAW).contains(&coordinate.raw()))
    {
        return Err(format!("{label} coordinates must be within +/-256 units"));
    }
    if point.radius.raw() <= 0 || point.radius.raw() > MAX_RADIUS_RAW {
        return Err(format!(
            "{label} radius must be positive and at most 4 units"
        ));
    }
    Ok(point)
}

fn format_point(point: MissionPoint) -> [String; 4] {
    [
        format_fixed(point.position[0]),
        format_fixed(point.position[1]),
        format_fixed(point.position[2]),
        format_fixed(point.radius),
    ]
}

fn parse_fixed(text: &str, label: &str) -> Result<FP, String> {
    if text.len() > 48 {
        return Err(format!("{label} is too long"));
    }
    orr_reflect::decimal::parse_fp(text.trim())
        .map_err(|error| format!("invalid {label} fixed-point decimal: {error:?}"))
}

fn format_fixed(value: FP) -> String {
    orr_reflect::decimal::fp_to_decimal(value)
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
#[path = "mesh_mission_panel/tests.rs"]
mod tests;
