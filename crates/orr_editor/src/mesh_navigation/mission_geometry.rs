use super::Panel as NavigationPanel;
use crate::{editor::Editor, game::EditorGame, model::Mode};
use egui::Ui;
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, SearchBudget};
use orr_reflect::bytemuck::Zeroable;
use orr_remote::mesh_navigation::{
    mesh_from_view, mesh_navigation_types,
    mission::{self, MeshMission, MeshMissionRun},
    AgentSpec, MeshScenePin, PIN_NAME,
};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    scene: PathBuf,
    epoch: u64,
    tick: u64,
    checksum: u64,
    old_revision: [u8; 32],
    candidate_revision: [u8; 32],
    projection_max_vertical: FP,
}

#[derive(Clone)]
pub(crate) struct Context {
    identity: Identity,
    source_pin: MeshScenePin,
    source_mission: MeshMission,
    source_run: MeshMissionRun,
    candidate_graph: StaticMeshGraph,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Preview {
    identity: Identity,
    pin: MeshScenePin,
    mission: MeshMission,
    old_start: [FP; 3],
    new_start: [FP; 3],
    old_goal: [FP; 3],
    new_goal: [FP; 3],
    point_changes: Vec<([FP; 3], [FP; 3])>,
    old_exit: [FP; 3],
    new_exit: [FP; 3],
}

#[derive(Default)]
pub(crate) struct State {
    opted_in: bool,
    confirmation: bool,
    preview: Option<Preview>,
    status: Option<String>,
    error: Option<String>,
}

pub(crate) struct Applied {
    pub spec: AgentSpec,
    pub graph: StaticMeshGraph,
}

impl Context {
    pub(crate) fn capture(panel: &NavigationPanel, editor: &Editor) -> Result<Self, String> {
        if editor.game() != EditorGame::MeshNavigation
            || !editor.spec().is_local()
            || !editor.can_mutate()
            || editor.mode() != Mode::Edit
            || editor.in_gesture()
            || editor.previewing().is_some()
            || !editor.proposals().is_empty()
        {
            return Err(
                "Mission-preserving geometry requires an idle, editable local MeshNavigation scene"
                    .into(),
            );
        }
        if !panel.geometry_staged || panel.geometry.dirty() || panel.geometry_vertex_busy() {
            return Err("Stage valid mesh geometry before previewing mission reprojection".into());
        }
        if panel.agent_pick_busy() || panel.prop_gesture_busy() {
            return Err("Finish or cancel active viewport picking or prop movement before mission reprojection".into());
        }
        if panel.geometry_presentation_pending || panel.art_import_busy() || panel.art_dirty() {
            return Err(
                "Save or discard scene-bound presentations before changing the mesh graph".into(),
            );
        }
        let snapshot = editor.snapshot().ok_or("Wait for the current mesh Frame")?;
        let frame = snapshot.predicted();
        if editor.checksum() != frame.checksum() {
            return Err("Wait for the matching current mesh Frame before reprojection".into());
        }
        if panel.geometry_host_context != Some((frame.tick(), frame.checksum())) {
            return Err("The staged geometry is based on a stale host Frame".into());
        }
        panel.geometry_bindings_blocker(editor, true)?;
        let scene = editor
            .path()
            .ok_or("Save the mesh scene before geometry edits")?;
        if panel.loaded_scene.as_ref() != Some(&scene)
            || panel.loaded_scene_epoch != Some(editor.scene_load_epoch())
        {
            return Err("The staged geometry does not belong to the current scene".into());
        }
        let source_view = mesh_from_view(frame)?;
        if panel.loaded_graph_revision != Some(source_view.graph.revision())
            || source_view.pin.graph_revision != source_view.graph.revision()
        {
            return Err(
                "The staged geometry was not derived from the currently admitted mesh".into(),
            );
        }
        if panel
            .start
            .iter()
            .chain(&panel.goal)
            .chain([&panel.max_vertical, &panel.distance_per_tick])
            .any(|value| value.len() > 48)
        {
            return Err("Agent route and tolerance fields must each be at most 48 bytes".into());
        }
        let authored_spec = panel.parse_spec()?;
        let source_spec = source_view.pin.spec();
        validate_pending_route(&authored_spec, &source_spec)?;
        let mission_view = mission::from_view(frame)?
            .ok_or("Enable a valid mesh mission before opting into mission-preserving geometry")?;
        if mission_view.run != MeshMissionRun::zeroed() {
            return Err("Reset mission progress before reprojecting mesh geometry".into());
        }
        let projection_max_vertical = authored_spec.max_vertical;
        if projection_max_vertical.raw() < 0 {
            return Err("Maximum vertical tolerance must be nonnegative".into());
        }
        let candidate_graph = panel.graph.as_ref().ok_or("Staged mesh graph is missing")?;
        let history_graph = panel
            .geometry_history
            .current()
            .flatten()
            .ok_or("Staged mesh graph history is missing")?;
        if history_graph.revision() != candidate_graph.revision()
            || history_graph.to_bytes() != candidate_graph.to_bytes()
            || candidate_graph.profile() != source_view.graph.profile()
        {
            return Err("Staged mesh graph and local geometry history do not match".into());
        }
        Ok(Self {
            identity: Identity {
                scene,
                epoch: editor.scene_load_epoch(),
                tick: frame.tick(),
                checksum: frame.checksum(),
                old_revision: source_view.graph.revision(),
                candidate_revision: candidate_graph.revision(),
                projection_max_vertical,
            },
            source_pin: source_view.pin,
            source_mission: mission_view.config,
            source_run: mission_view.run,
            candidate_graph: candidate_graph.clone(),
        })
    }
}

fn validate_pending_route(authored: &AgentSpec, source: &AgentSpec) -> Result<(), String> {
    if authored.start != source.start
        || authored.goal != source.goal
        || authored.distance_per_tick != source.distance_per_tick
    {
        return Err("Apply or discard pending agent start, goal, or speed changes before geometry reprojection".into());
    }
    Ok(())
}

impl State {
    pub(crate) fn show(&mut self, ui: &mut Ui, context: Result<Context, String>) -> bool {
        ui.checkbox(
            &mut self.opted_in,
            "Preserve and reproject the active mesh mission",
        );
        if !self.opted_in {
            self.preview = None;
            self.confirmation = false;
            ui.weak("Geometry remains blocked by an active mission unless this explicit mode is enabled.");
            return false;
        }
        if let Some(status) = self.status.as_deref() {
            ui.label(status);
        }
        let current = match context {
            Ok(current) => Some(current),
            Err(error) => {
                self.cancel_preview();
                ui.weak(error);
                None
            }
        };
        if self
            .preview
            .as_ref()
            .zip(current.as_ref())
            .is_some_and(|(preview, current)| preview.identity != current.identity)
        {
            self.cancel_preview();
            self.error = Some("Mission geometry preview canceled because the scene, host Frame, or staged graph changed.".into());
        }
        if let Some(reason) = self.error.as_deref() {
            ui.colored_label(egui::Color32::RED, reason);
        }
        if ui
            .add_enabled(
                current.is_some(),
                egui::Button::new("Preview mission-preserving geometry"),
            )
            .clicked()
        {
            let preview_result = current
                .as_ref()
                .ok_or_else(|| "Current mesh context is unavailable".to_owned())
                .and_then(build_preview);
            match preview_result {
                Ok(preview) => {
                    self.preview = Some(preview);
                    self.confirmation = false;
                    self.error = None;
                    self.status = Some(
                        "Mission reprojection preview is ready; the host is unchanged.".into(),
                    );
                }
                Err(error) => {
                    self.cancel_preview();
                    self.error = Some(format!("Mission geometry preview failed: {error}"));
                }
            }
        }
        if let Some(preview) = self.preview.as_ref() {
            show_preview(ui, preview);
            ui.checkbox(
                &mut self.confirmation,
                "I confirm reprojecting the agent and active mission onto this staged graph",
            );
            let apply = ui
                .add_enabled(
                    self.confirmation && current.is_some(),
                    egui::Button::new("Apply geometry and reproject mission"),
                )
                .clicked();
            if ui.button("Cancel mission geometry preview").clicked() {
                self.cancel_preview();
                self.status = Some("Mission geometry preview canceled; staged geometry and host history are unchanged.".into());
                return false;
            }
            return apply;
        }
        false
    }

    pub(crate) fn apply(
        &mut self,
        editor: &mut Editor,
        current: Context,
    ) -> Result<Applied, String> {
        if !self.opted_in || !self.confirmation {
            return Err(
                "Explicitly enable and confirm mission-preserving geometry before applying".into(),
            );
        }
        let preview = self
            .preview
            .as_ref()
            .ok_or("Preview mission reprojection before applying")?;
        if preview.identity != current.identity {
            self.cancel_preview();
            return Err("Mission geometry preview is stale; create a new preview".into());
        }
        verify_editor_context(editor, &current)?;
        let request = reproject_request(&current, preview)?;
        let response = editor.host_call("mesh.geometry.reproject", request)?;
        verify_response(&response, preview)?;
        editor.sync();
        let snapshot = editor
            .snapshot()
            .ok_or("Mesh host did not publish the reprojected Frame")?;
        let frame = snapshot.predicted();
        if frame.checksum() != parse_checksum(&response["checksum"])? {
            return Err("Mesh host response checksum differs from its published Frame".into());
        }
        let view = mesh_from_view(frame)?;
        let mission_view =
            mission::from_view(frame)?.ok_or("Mesh host did not preserve the active mission")?;
        if view.pin != preview.pin
            || view.graph.to_bytes() != current.candidate_graph.to_bytes()
            || mission_view.config != preview.mission
            || mission_view.run != current.source_run
        {
            return Err(
                "Mesh host published a graph or mission different from the reviewed preview".into(),
            );
        }
        let applied = Applied {
            spec: preview.pin.spec(),
            graph: current.candidate_graph,
        };
        editor.mesh_navigation_mark_edited();
        self.preview = None;
        self.confirmation = false;
        self.opted_in = false;
        self.status = Some("Staged geometry and mission points were applied atomically; save the scene explicitly.".into());
        self.error = None;
        Ok(applied)
    }

    pub(crate) fn cancel_preview(&mut self) {
        self.preview = None;
        self.confirmation = false;
    }

    pub(crate) fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.opted_in
    }
}

fn build_preview(context: &Context) -> Result<Preview, String> {
    if context.source_run != MeshMissionRun::zeroed() {
        return Err("Reset mission progress before reprojecting mesh geometry".into());
    }
    let old_spec = context.source_pin.spec();
    let new_start = project_exact(
        &context.candidate_graph,
        old_spec.start,
        context.identity.projection_max_vertical,
    )?;
    let new_goal = project_exact(
        &context.candidate_graph,
        old_spec.goal,
        context.identity.projection_max_vertical,
    )?;
    let spec = AgentSpec {
        start: new_start,
        goal: new_goal,
        max_vertical: context.identity.projection_max_vertical,
        ..old_spec
    };
    let pin = MeshScenePin::new(&context.candidate_graph, spec)?;
    let mut mission_config = context.source_mission;
    mission_config.graph_revision = context.candidate_graph.revision();
    let mut point_changes = Vec::with_capacity(mission_config.count as usize);
    for point in mission_config
        .points
        .iter_mut()
        .take(mission_config.count as usize)
    {
        let old_position = point.position;
        point.position = project_exact(&context.candidate_graph, old_position, spec.max_vertical)?;
        point_changes.push((old_position, point.position));
    }
    let old_exit = mission_config.exit.position;
    mission_config.exit.position =
        project_exact(&context.candidate_graph, old_exit, spec.max_vertical)?;
    let path_budget = u32::try_from(context.candidate_graph.triangles().len())
        .map_err(|_| "candidate graph exceeds the path expansion budget")?;
    for (index, point) in mission_config
        .points
        .iter()
        .take(mission_config.count as usize)
        .enumerate()
    {
        context
            .candidate_graph
            .find_path(
                new_start,
                point.position,
                spec.max_vertical,
                SearchBudget {
                    max_expansions: path_budget,
                },
            )
            .map_err(|error| {
                format!(
                    "candidate graph cannot route from the agent start to collectible {}: {error}",
                    index + 1
                )
            })?;
    }
    context
        .candidate_graph
        .find_path(
            new_start,
            mission_config.exit.position,
            spec.max_vertical,
            SearchBudget {
                max_expansions: path_budget,
            },
        )
        .map_err(|error| {
            format!(
                "candidate graph cannot route from the agent start to the mission exit: {error}"
            )
        })?;
    mission::validate_config(&mission_config, &context.candidate_graph, spec.max_vertical)?;
    let new_exit = mission_config.exit.position;
    Ok(Preview {
        identity: context.identity.clone(),
        pin,
        mission: mission_config,
        old_start: old_spec.start,
        new_start,
        old_goal: old_spec.goal,
        new_goal,
        point_changes,
        old_exit,
        new_exit,
    })
}

fn project_exact(
    graph: &StaticMeshGraph,
    point: [FP; 3],
    max_vertical: FP,
) -> Result<[FP; 3], String> {
    graph
        .project(point, max_vertical)
        .map(|projection| projection.position)
        .map_err(|error| format!("cannot reproject authored point: {error}"))
}

fn reproject_request(context: &Context, preview: &Preview) -> Result<Value, String> {
    if preview.identity != context.identity
        || preview.pin.graph_revision != context.identity.candidate_revision
        || preview.mission.graph_revision != context.identity.candidate_revision
    {
        return Err("mission reprojection request does not match its preview context".into());
    }
    let types = mesh_navigation_types();
    let descriptor = types
        .get(PIN_NAME)
        .ok_or("MeshScenePin reflection is missing")?;
    let reflected = descriptor.read(orr_reflect::bytemuck::bytes_of(&preview.pin));
    let pin_value = orr_remote::json::value_to_json(&reflected);
    if !pin_value.is_object() {
        return Err("MeshScenePin reflection did not produce an object".into());
    }
    Ok(json!({
        "expected_checksum": format_checksum(context.identity.checksum),
        "expected_graph_revision": hex_revision(context.identity.old_revision),
        "pin": pin_value,
    }))
}

fn verify_editor_context(editor: &Editor, context: &Context) -> Result<(), String> {
    if editor.game() != EditorGame::MeshNavigation
        || !editor.spec().is_local()
        || !editor.can_mutate()
        || editor.mode() != Mode::Edit
        || editor.in_gesture()
        || editor.previewing().is_some()
        || !editor.proposals().is_empty()
        || editor.path().as_ref() != Some(&context.identity.scene)
        || editor.scene_load_epoch() != context.identity.epoch
    {
        return Err("Mission geometry context is no longer editable".into());
    }
    let frame = editor
        .snapshot()
        .ok_or("Mesh host Frame is unavailable")?
        .predicted();
    if frame.tick() != context.identity.tick
        || frame.checksum() != context.identity.checksum
        || *frame.singleton::<MeshScenePin>() != context.source_pin
    {
        return Err("Mesh host changed after the mission geometry preview".into());
    }
    let source_mission =
        mission::from_view(frame)?.ok_or("The active mission changed after preview")?;
    if source_mission.config != context.source_mission || source_mission.run != context.source_run {
        return Err("Mission state changed after the geometry preview".into());
    }
    Ok(())
}

fn verify_response(response: &Value, preview: &Preview) -> Result<(), String> {
    if response["changed"] != Value::Bool(true)
        || response["graph_revision"]
            != Value::String(hex_revision(preview.identity.candidate_revision))
        || response["mission"] != reflected_mission(&preview.mission)?
    {
        return Err("mesh.geometry.reproject returned an incomplete or unexpected result".into());
    }
    let _ = parse_checksum(&response["checksum"])?;
    Ok(())
}

fn reflected_mission(mission_config: &MeshMission) -> Result<Value, String> {
    let types = mesh_navigation_types();
    let descriptor = types
        .get(orr_remote::mesh_navigation::mission::MISSION_NAME)
        .ok_or("MeshMission reflection is missing")?;
    Ok(orr_remote::json::value_to_json(
        &descriptor.read(orr_reflect::bytemuck::bytes_of(mission_config)),
    ))
}

fn parse_checksum(value: &Value) -> Result<u64, String> {
    let value = value
        .as_str()
        .ok_or("mesh.geometry.reproject checksum must be a string")?;
    let digits = value
        .strip_prefix("0x")
        .ok_or("mesh.geometry.reproject checksum is not canonical hexadecimal")?;
    if digits.len() != 16
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("mesh.geometry.reproject checksum is not canonical hexadecimal".into());
    }
    u64::from_str_radix(digits, 16)
        .map_err(|_| "mesh.geometry.reproject checksum is outside the supported range".into())
}

fn format_checksum(checksum: u64) -> String {
    format!("0x{checksum:016x}")
}

fn hex_revision(revision: [u8; 32]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in revision {
        use std::fmt::Write;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn show_preview(ui: &mut Ui, preview: &Preview) {
    ui.strong("Mission-preserving geometry preview · host unchanged");
    ui.label(format!(
        "Graph revision: {} → {}",
        hex_revision(preview.identity.old_revision),
        hex_revision(preview.identity.candidate_revision)
    ));
    show_point(ui, "Agent start", preview.old_start, preview.new_start);
    show_point(ui, "Agent goal", preview.old_goal, preview.new_goal);
    for (index, (before, after)) in preview.point_changes.iter().enumerate() {
        show_point(ui, &format!("Collectible {}", index + 1), *before, *after);
    }
    show_point(ui, "Mission exit", preview.old_exit, preview.new_exit);
    ui.label(format!(
        "Mission count and radii preserved · {} collectible(s)",
        preview.mission.count
    ));
}

fn show_point(ui: &mut Ui, label: &str, before: [FP; 3], after: [FP; 3]) {
    ui.label(format!(
        "{label}: {} → {}",
        format_point(before),
        format_point(after)
    ));
}

fn format_point(point: [FP; 3]) -> String {
    point.map(orr_reflect::decimal::fp_to_decimal).join(", ")
}

#[path = "mission_geometry/tests.rs"]
#[cfg(test)]
mod tests;
