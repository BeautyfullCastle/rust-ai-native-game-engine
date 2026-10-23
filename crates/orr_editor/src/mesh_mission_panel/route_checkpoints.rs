use super::{current_identity, Panel};
use crate::{editor::Editor, game::EditorGame, model::Mode};
use orr_navigation::SearchBudget;
use orr_remote::mesh_navigation::{
    mesh_from_view,
    mission::{self, MeshMission, MeshMissionRun, MissionView},
    MeshView,
};

const MAX_ROUTE_TRIANGLES: usize = 512;
const MAX_ROUTE_POINTS: usize = MAX_ROUTE_TRIANGLES + 2;

pub(crate) fn candidate(view: &MeshView, config: &MeshMission) -> Result<MeshMission, String> {
    if view.graph.revision() != view.pin.graph_revision {
        return Err("The mission graph does not match the admitted scene pin".into());
    }
    if view.graph.triangles().is_empty() || view.graph.triangles().len() > MAX_ROUTE_TRIANGLES {
        return Err("The admitted route exceeds the bounded triangle limit".into());
    }
    mission::validate_config(config, &view.graph, view.pin.max_vertical)?;
    if config.enabled != 1 || !(1..=8).contains(&config.count) {
        return Err("Route checkpoints require an enabled mission with 1..=8 collectibles".into());
    }

    let spec = view.pin.spec();
    let budget = SearchBudget {
        max_expansions: u32::try_from(view.graph.triangles().len())
            .map_err(|_| "The admitted route exceeds the bounded triangle limit")?,
    };
    let path = view
        .graph
        .find_path(spec.start, spec.goal, spec.max_vertical, budget)
        .map_err(|error| {
            format!("Cannot derive mission checkpoints from the canonical route: {error}")
        })?;
    if path.points.len() < 2 || path.points.len() > MAX_ROUTE_POINTS {
        return Err("The canonical route has an invalid bounded waypoint count".into());
    }

    let mut checkpoints = Vec::with_capacity(path.points.len().saturating_sub(2));
    for point in path.points.iter().skip(1).take(path.points.len() - 2) {
        if *point != spec.start && *point != spec.goal && !checkpoints.contains(point) {
            checkpoints.push(*point);
        }
    }
    let requested = config.count as usize;
    if checkpoints.len() < requested {
        return Err(format!(
            "The canonical route has {} distinct interior checkpoints; {requested} are required",
            checkpoints.len()
        ));
    }

    let mut staged = *config;
    for ordinal in 1..=requested {
        let checkpoint_index = ordinal
            .checked_mul(checkpoints.len() + 1)
            .ok_or("Mission checkpoint index overflow")?
            / (requested + 1)
            - 1;
        staged.points[ordinal - 1].position = checkpoints[checkpoint_index];
    }
    staged.exit.position = spec.goal;
    staged.graph_revision = view.graph.revision();
    mission::validate_config(&staged, &view.graph, spec.max_vertical)?;
    if staged == *config {
        return Err("The mission already uses the canonical route checkpoints".into());
    }
    Ok(staged)
}

impl Panel {
    pub(super) fn stage_route_checkpoints(&mut self, editor: &Editor) {
        if !self.can_edit(editor)
            || self.dirty()
            || self.mission_pick_busy()
            || editor.mode() != Mode::Edit
            || editor.game() != EditorGame::MeshNavigation
        {
            self.error = Some(
                "Route checkpoint staging requires a clean matching editable mesh mission".into(),
            );
            return;
        }
        let Some((scene, graph_revision, checksum, _)) = current_identity(editor) else {
            self.error = Some("The current admitted mesh mission is unavailable".into());
            return;
        };
        if self.scene.as_ref() != Some(&scene)
            || self.scene_epoch != Some(editor.scene_load_epoch())
            || self.graph_revision != Some(graph_revision)
            || self.observed_checksum != Some(checksum)
        {
            self.error =
                Some("The mission panel is stale; reload before staging route checkpoints".into());
            return;
        }
        let result = (|| {
            let frame = editor
                .snapshot()
                .ok_or("Mesh frame is unavailable")?
                .predicted();
            let view = mesh_from_view(frame)?;
            let mission_view = mission::from_view(frame)?
                .ok_or("Route checkpoints require an enabled mesh mission")?;
            if mission_view.run != MeshMissionRun::default() {
                return Err(
                    "Route checkpoints can only be staged before mission progress begins".into(),
                );
            }
            let staged = candidate(&view, &mission_view.config)?;
            Ok::<_, String>((staged, mission_view.run))
        })();
        match result {
            Ok((config, run)) => {
                self.load_fields(Some(&MissionView { config, run }));
                self.form_dirty = true;
                self.error = None;
                self.status =
                    "Mission checkpoints staged from the canonical route; apply them explicitly."
                        .into();
            }
            Err(error) => self.error = Some(error),
        }
    }
}

#[cfg(test)]
#[path = "route_checkpoints/tests.rs"]
mod tests;
