use super::parse_fixed;
use crate::mesh_navigation::geometry::{self, Quad};
use egui::Ui;
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};

const MAX_FIELD_BYTES: usize = 48;
const MAX_VISIBLE_ROWS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    AppendQuad(Quad),
    MoveVertex { index: u32, position: [FP; 3] },
    RemoveFace(u32),
}

pub struct Panel {
    pub quad_origin: [String; 3],
    pub quad_width: String,
    pub quad_depth: String,
    pub quad_rise: String,
    pub vertex_index: String,
    pub vertex_position: [String; 3],
    pub face_index: String,
    dirty: bool,
    observed_revision: Option<[u8; 32]>,
    revision_initialized: bool,
    error: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            quad_origin: ["0".into(), "0".into(), "0".into()],
            quad_width: "4".into(),
            quad_depth: "2".into(),
            quad_rise: "0".into(),
            vertex_index: "0".into(),
            vertex_position: ["0".into(), "0".into(), "0".into()],
            face_index: "0".into(),
            dirty: false,
            observed_revision: None,
            revision_initialized: false,
            error: None,
        }
    }
}

impl Panel {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        graph: Option<&StaticMeshGraph>,
        profile: AgentProfile,
    ) -> Option<Action> {
        let current_revision = graph.map(StaticMeshGraph::revision);
        if !self.revision_initialized {
            self.observed_revision = current_revision;
            self.revision_initialized = true;
        }
        let stale = self.observed_revision != current_revision;
        if stale {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Mesh changed; reset the form before staging another edit",
            );
        }
        ui.label(
            "Vertex and face indices refer to the current canonical mesh; reload after edits.",
        );
        if let Some(graph) = graph {
            match geometry::faces(graph) {
                Ok(faces) => {
                    ui.label(format!("Vertices ({})", graph.vertices().len()));
                    for (index, point) in graph.vertices().iter().take(MAX_VISIBLE_ROWS).enumerate()
                    {
                        ui.label(format!(
                            "Vertex {index}: [{}, {}, {}]",
                            orr_reflect::decimal::fp_to_decimal(point[0]),
                            orr_reflect::decimal::fp_to_decimal(point[1]),
                            orr_reflect::decimal::fp_to_decimal(point[2]),
                        ));
                    }
                    if graph.vertices().len() > MAX_VISIBLE_ROWS {
                        ui.label("Additional vertices are hidden");
                    }
                    ui.label(format!("Faces ({})", faces.len()));
                    for (index, face) in faces.iter().take(MAX_VISIBLE_ROWS).enumerate() {
                        ui.label(format!(
                            "Face {index}: [{}, {}, {}]",
                            face[0], face[1], face[2]
                        ));
                    }
                    if faces.len() > MAX_VISIBLE_ROWS {
                        ui.label("Additional faces are hidden");
                    }
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::RED, error);
                }
            };
        } else {
            ui.label("No mesh is open; a walkable quad can create the first geometry.");
        }

        let mut changed = false;
        changed |= field(ui, "Quad origin X", &mut self.quad_origin[0]);
        changed |= field(ui, "Quad origin Y", &mut self.quad_origin[1]);
        changed |= field(ui, "Quad origin Z", &mut self.quad_origin[2]);
        changed |= field(ui, "Quad width", &mut self.quad_width);
        changed |= field(ui, "Quad depth", &mut self.quad_depth);
        changed |= field(ui, "Quad rise", &mut self.quad_rise);
        let mut action = None;
        if ui
            .add_enabled(!stale, egui::Button::new("Stage walkable quad"))
            .clicked()
        {
            let result = parse_quad(self).and_then(|quad| {
                geometry::append_quad(graph, profile, quad).map(|_| Action::AppendQuad(quad))
            });
            record_result(result, &mut self.error, &mut action);
        }

        changed |= field(ui, "Move vertex X", &mut self.vertex_position[0]);
        changed |= field(ui, "Move vertex Y", &mut self.vertex_position[1]);
        changed |= field(ui, "Move vertex Z", &mut self.vertex_position[2]);
        changed |= field(ui, "Move vertex index", &mut self.vertex_index);
        if ui
            .add_enabled(
                graph.is_some() && !stale,
                egui::Button::new("Stage vertex move"),
            )
            .clicked()
        {
            let result = graph
                .ok_or_else(|| "open a mesh before moving a vertex".to_owned())
                .and_then(|graph| parse_index(&self.vertex_index).map(|index| (graph, index)))
                .and_then(|(graph, index)| {
                    let position = parse_point(&self.vertex_position)?;
                    geometry::move_vertex(graph, index, position)
                        .map(|_| Action::MoveVertex { index, position })
                });
            record_result(result, &mut self.error, &mut action);
        }

        changed |= field(ui, "Remove face index", &mut self.face_index);
        if ui
            .add_enabled(
                graph.is_some() && !stale,
                egui::Button::new("Stage face removal"),
            )
            .clicked()
        {
            let result = graph
                .ok_or_else(|| "open a mesh before removing a face".to_owned())
                .and_then(|graph| parse_index(&self.face_index).map(|index| (graph, index)))
                .and_then(|(graph, index)| {
                    geometry::remove_face(graph, index).map(|_| Action::RemoveFace(index))
                });
            record_result(result, &mut self.error, &mut action);
        }
        if ui.button("Reset geometry form").clicked() {
            self.reset();
        } else {
            self.dirty |= changed;
        }
        if let Some(error) = self.error.as_deref() {
            ui.colored_label(egui::Color32::RED, error);
        }
        action
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn acknowledge(&mut self) {
        self.dirty = false;
        self.observed_revision = None;
        self.revision_initialized = false;
        self.error = None;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

fn field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let label_response = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(ui.available_width())
            .char_limit(MAX_FIELD_BYTES),
    )
    .labelled_by(label_response.id)
    .changed()
}

fn parse_quad(panel: &Panel) -> Result<Quad, String> {
    Ok(Quad {
        origin: parse_point(&panel.quad_origin)?,
        width: parse_fixed_bounded(&panel.quad_width)?,
        depth: parse_fixed_bounded(&panel.quad_depth)?,
        rise: parse_fixed_bounded(&panel.quad_rise)?,
    })
}

fn parse_point(values: &[String; 3]) -> Result<[FP; 3], String> {
    Ok([
        parse_fixed_bounded(&values[0])?,
        parse_fixed_bounded(&values[1])?,
        parse_fixed_bounded(&values[2])?,
    ])
}

fn parse_fixed_bounded(value: &str) -> Result<FP, String> {
    if value.len() > MAX_FIELD_BYTES {
        return Err("fixed-point field exceeds 48 bytes".into());
    }
    parse_fixed(value)
}

fn parse_index(value: &str) -> Result<u32, String> {
    if value.is_empty() || value.len() > 10 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("mesh index must be an unsigned decimal integer".into());
    }
    value
        .parse::<u32>()
        .map_err(|_| "mesh index is outside the supported range".into())
}

fn record_result(
    result: Result<Action, String>,
    error: &mut Option<String>,
    action: &mut Option<Action>,
) {
    match result {
        Ok(staged) => {
            *error = None;
            *action = Some(staged);
        }
        Err(message) => *error = Some(message),
    }
}

#[path = "geometry_panel/tests.rs"]
#[cfg(test)]
mod tests;
