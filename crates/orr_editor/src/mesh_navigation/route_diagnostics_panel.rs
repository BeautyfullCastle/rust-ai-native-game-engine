//! On-demand, read-only diagnostics for the currently loaded mesh draft.
use super::{format_fixed, parse_fixed, route_diagnostics, Panel};
use crate::{editor::Editor, model::Mode};
use egui::Ui;
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, NavigationError, SearchBudget};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    revision: [u8; 32],
    scene: Option<PathBuf>,
    epoch: u64,
    fields: [String; 8],
}

#[derive(Default)]
pub(super) struct State {
    observation: Option<(Context, Result<route_diagnostics::RouteReport, String>)>,
}

impl State {
    fn sync(&mut self, context: Option<&Context>) {
        if self
            .observation
            .as_ref()
            .is_some_and(|(old, _)| Some(old) != context)
        {
            self.observation = None;
        }
    }

    fn show(
        &mut self,
        ui: &mut Ui,
        graph: Option<&StaticMeshGraph>,
        context: Result<Context, String>,
    ) {
        self.sync(context.as_ref().ok());
        ui.weak("Draft route only: point agent; radius, headroom and clearance are not checked.");
        if ui
            .add_enabled(context.is_ok(), egui::Button::new("Inspect draft route"))
            .clicked()
        {
            if let (Some(graph), Ok(context)) = (graph, context.as_ref()) {
                self.observation = Some((context.clone(), inspect_fields(graph, context)));
            }
        }
        if let Err(reason) = context {
            ui.weak(reason);
        }
        let Some((_, result)) = &self.observation else {
            return;
        };
        match result {
            Err(error) => {
                ui.label(format!("Route input error: {error}"));
            }
            Ok(report) => {
                for (name, endpoint) in [("Start", &report.start), ("Goal", &report.goal)] {
                    match endpoint {
                        Ok(projected) => {
                            ui.label(format!(
                                "{name} projects to triangle {} at [{}, {}, {}]",
                                projected.triangle,
                                format_fixed(projected.position[0]),
                                format_fixed(projected.position[1]),
                                format_fixed(projected.position[2])
                            ));
                        }
                        Err(error) => {
                            ui.label(format!("{name} projection failed: {}", explain(*error)));
                        }
                    }
                }
                match &report.route {
                    route_diagnostics::RouteOutcome::EndpointsInvalid => {
                        ui.label("Route not searched: fix endpoint projection first.");
                    }
                    route_diagnostics::RouteOutcome::Ready {
                        corridor_triangles,
                        length,
                    } => {
                        ui.label(format!(
                            "Route found: {corridor_triangles} corridor triangles; path length {}",
                            format_fixed(*length)
                        ));
                    }
                    route_diagnostics::RouteOutcome::Failed(error) => {
                        ui.label(format!("Route search failed: {}", explain(*error)));
                    }
                }
                ui.weak("Inspection does not place or move an agent, save a scene, or authorize placement.");
            }
        }
    }
}

fn explain(error: NavigationError) -> String {
    match error {
        NavigationError::OutsideTerrain => {
            "outside the loaded walkable triangles or beyond maximum vertical projection".into()
        }
        NavigationError::Unreachable => {
            "endpoints project, but no connected triangle route exists".into()
        }
        NavigationError::BudgetExceeded { expanded } => {
            format!("search budget exhausted after {expanded} expansions; reachability is unknown")
        }
        other => other.to_string(),
    }
}

fn inspect_fields(
    graph: &StaticMeshGraph,
    context: &Context,
) -> Result<route_diagnostics::RouteReport, String> {
    let point = |offset: usize, name: &str| -> Result<[FP; 3], String> {
        let axis = |i: usize| {
            parse_fixed(context.fields[offset + i].as_str())
                .map_err(|error| format!("{name} {}: {error}", ["X", "Y", "Z"][i]))
        };
        Ok([axis(0)?, axis(1)?, axis(2)?])
    };
    let start = point(0, "Start")?;
    let goal = point(3, "Goal")?;
    let max_vertical = parse_fixed(&context.fields[6])
        .map_err(|error| format!("Maximum vertical projection: {error}"))?;
    if max_vertical < FP::ZERO {
        return Err("Maximum vertical projection must be nonnegative".into());
    }
    Ok(route_diagnostics::inspect_route(
        graph,
        start,
        goal,
        max_vertical,
        SearchBudget {
            max_expansions: graph.triangles().len() as u32,
        },
    ))
}

impl Panel {
    fn route_context(&self, editor: &Editor) -> Result<Context, String> {
        let graph = self
            .graph
            .as_ref()
            .ok_or("Create or open a mesh asset to inspect its draft route")?;
        if editor.mode() != Mode::Edit || editor.previewing().is_some() {
            return Err("Stop Play and clear previews before inspecting a draft route".into());
        }
        if self.geometry.dirty()
            || self.geometry_vertex_busy()
            || self.agent_pick_busy()
            || self.props.gesture_busy()
        {
            return Err(
                "Finish or cancel geometry fields and viewport gestures before inspection".into(),
            );
        }
        if self.geometry_staged && !self.geometry_preview_active() {
            return Err("Staged geometry is stale; discard or restage before inspection".into());
        }
        if self.asset_path.trim() != self.loaded_asset_source.as_deref().unwrap_or_default()
            || self.model_path.trim() != self.loaded_model_source.as_deref().unwrap_or_default()
        {
            return Err("Open the selected mesh or model before inspection".into());
        }
        if graph.profile().max_slope != parse_fixed(&self.max_slope)? {
            return Err("Maximum slope differs from the open graph; recreate or reopen it".into());
        }
        Ok(Context {
            revision: graph.revision(),
            scene: editor.path(),
            epoch: editor.scene_load_epoch(),
            fields: [
                self.start[0].clone(),
                self.start[1].clone(),
                self.start[2].clone(),
                self.goal[0].clone(),
                self.goal[1].clone(),
                self.goal[2].clone(),
                self.max_vertical.clone(),
                self.max_slope.clone(),
            ],
        })
    }

    pub(super) fn show_route_diagnostics(&mut self, ui: &mut Ui, editor: &Editor) {
        let context = self.route_context(editor);
        self.route_diagnostics
            .show(ui, self.graph.as_ref(), context);
    }
}

#[cfg(test)]
#[path = "route_diagnostics_panel/tests.rs"]
mod tests;
