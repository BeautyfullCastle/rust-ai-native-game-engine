use super::geometry_grid::{self, Grid};
use super::parse_fixed;
use egui::Ui;
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};

const MAX_FIELD_BYTES: usize = 48;
const MAX_EXCLUDED_BYTES: usize = 196;
const MAX_GRID_AXIS: u32 = 7;

pub struct Panel {
    origin: [String; 3],
    width: String,
    depth: String,
    columns: String,
    rows: String,
    excluded_cells: String,
    replace_confirmed: bool,
    dirty: bool,
    error: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            origin: ["0".into(), "0".into(), "0".into()],
            width: "12".into(),
            depth: "8".into(),
            columns: "3".into(),
            rows: "3".into(),
            excluded_cells: String::new(),
            replace_confirmed: false,
            dirty: false,
            error: None,
        }
    }
}

impl Panel {
    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        _graph: Option<&StaticMeshGraph>,
        profile: AgentProfile,
        stale: bool,
    ) -> Option<Grid> {
        let mut changed = false;
        let response = egui::CollapsingHeader::new("Walkable grid")
            .default_open(false)
            .show(ui, |ui| {
                ui.label("Excluded cells are holes in the walkable surface, not physical walls.");
                changed |= field(ui, "Grid origin X", &mut self.origin[0], MAX_FIELD_BYTES);
                changed |= field(ui, "Grid origin Y", &mut self.origin[1], MAX_FIELD_BYTES);
                changed |= field(ui, "Grid origin Z", &mut self.origin[2], MAX_FIELD_BYTES);
                changed |= field(ui, "Grid width", &mut self.width, MAX_FIELD_BYTES);
                changed |= field(ui, "Grid depth", &mut self.depth, MAX_FIELD_BYTES);
                changed |= field(ui, "Grid columns", &mut self.columns, MAX_FIELD_BYTES);
                changed |= field(ui, "Grid rows", &mut self.rows, MAX_FIELD_BYTES);
                changed |= field(
                    ui,
                    "Excluded grid cells",
                    &mut self.excluded_cells,
                    MAX_EXCLUDED_BYTES,
                );
                let confirm = ui.checkbox(
                    &mut self.replace_confirmed,
                    "I confirm replacing staged geometry with this grid",
                );
                changed |= confirm.changed();
                if stale {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "Mesh changed; reload before staging a grid.",
                    );
                }
                if ui
                    .add_enabled(!stale, egui::Button::new("Stage walkable grid"))
                    .clicked()
                {
                    match self.stage_candidate(profile, stale) {
                        Ok(grid) => {
                            self.error = None;
                            return Some(grid);
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                if let Some(error) = self.error.as_deref() {
                    ui.colored_label(egui::Color32::RED, error);
                }
                None
            });
        if changed {
            self.dirty = true;
        }
        response.body_returned.flatten()
    }

    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn acknowledge(&mut self) {
        self.dirty = false;
        self.replace_confirmed = false;
        self.error = None;
    }

    fn stage_candidate(&self, profile: AgentProfile, stale: bool) -> Result<Grid, String> {
        if stale {
            return Err("mesh changed; reload before staging a walkable grid".into());
        }
        if !self.replace_confirmed {
            return Err("confirm replacing staged geometry before staging the grid".into());
        }
        let grid = parse_grid(self)?;
        geometry_grid::create(grid, profile)?;
        Ok(grid)
    }
}

fn field(ui: &mut Ui, label: &str, value: &mut String, max_bytes: usize) -> bool {
    let label_response = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(ui.available_width())
            .char_limit(max_bytes),
    )
    .labelled_by(label_response.id)
    .changed()
}

fn parse_grid(panel: &Panel) -> Result<Grid, String> {
    Ok(Grid {
        origin: [
            parse_fixed_bounded(&panel.origin[0])?,
            parse_fixed_bounded(&panel.origin[1])?,
            parse_fixed_bounded(&panel.origin[2])?,
        ],
        width: parse_fixed_bounded(&panel.width)?,
        depth: parse_fixed_bounded(&panel.depth)?,
        columns: parse_axis(&panel.columns, "columns")?,
        rows: parse_axis(&panel.rows, "rows")?,
        excluded_cells: parse_excluded_cells(
            &panel.excluded_cells,
            parse_axis(&panel.columns, "columns")?,
            parse_axis(&panel.rows, "rows")?,
        )?,
    })
}

fn parse_fixed_bounded(value: &str) -> Result<FP, String> {
    if value.len() > MAX_FIELD_BYTES {
        return Err("fixed-point grid field exceeds 48 bytes".into());
    }
    parse_fixed(value)
}

fn parse_axis(value: &str, label: &str) -> Result<u32, String> {
    if value.is_empty()
        || value.len() > MAX_FIELD_BYTES
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!("grid {label} must be an unsigned decimal integer"));
    }
    let axis = value
        .parse::<u32>()
        .map_err(|_| format!("grid {label} is outside the supported range"))?;
    if !(1..=MAX_GRID_AXIS).contains(&axis) {
        return Err(format!(
            "grid {label} must be between 1 and {MAX_GRID_AXIS}"
        ));
    }
    Ok(axis)
}

fn parse_excluded_cells(value: &str, columns: u32, rows: u32) -> Result<u64, String> {
    if value.len() > MAX_EXCLUDED_BYTES {
        return Err("excluded grid cells exceeds 196 bytes".into());
    }
    if value.is_empty() {
        return Ok(0);
    }
    if !value.is_ascii() {
        return Err("excluded grid cells must contain ASCII indices".into());
    }
    let mut mask = 0_u64;
    for segment in value.split(',') {
        let index_text = segment.trim();
        if index_text.is_empty()
            || index_text.len() > 10
            || !index_text.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("excluded grid cells must be comma-separated unsigned indices".into());
        }
        let index = index_text
            .parse::<u32>()
            .map_err(|_| "excluded grid cell index is outside the supported range".to_owned())?;
        let cell_count = columns * rows;
        if index >= cell_count {
            return Err("excluded grid cell index is outside the configured grid".into());
        }
        let bit = 1_u64 << index;
        if mask & bit != 0 {
            return Err("excluded grid cell indices must be unique".into());
        }
        mask |= bit;
    }
    Ok(mask)
}

#[path = "geometry_grid_panel/tests.rs"]
#[cfg(test)]
mod tests;
