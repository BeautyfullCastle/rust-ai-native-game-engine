//! Bounded, atomic cosmetic grid placement using the admitted graph's height query.
use super::{
    current_identity, duplicate_prop_name, mesh_from_view, validate_document, Document, Editor,
    Panel, PathBuf, Prop, MAX_PROPS,
};
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, MAX_COORDINATE_RAW};

const MAX_INPUT_BYTES: usize = 48;
const MAX_PROJECTION_WORK: usize = 1_048_576;
const MAX_ART_COORDINATE_RAW: i64 = 1_000_000 * FP::SCALE;

#[derive(Clone, Copy, Debug)]
struct Config {
    columns: usize,
    rows: usize,
    spacing: [FP; 2],
    max_vertical: FP,
    height_offset: FP,
}

impl Config {
    fn parse(
        columns: &str,
        rows: &str,
        spacing: &[String; 2],
        max_vertical: &str,
        height_offset: &str,
    ) -> Result<Self, String> {
        let dimension = |text: &str| -> Result<usize, String> {
            if text.len() > MAX_INPUT_BYTES {
                return Err("Surface grid dimensions exceed the 48-byte input limit".into());
            }
            text.trim()
                .parse::<usize>()
                .ok()
                .filter(|value| (1..=16).contains(value))
                .ok_or_else(|| "Surface grid dimensions must be integers from 1 through 16".into())
        };
        let config = Self {
            columns: dimension(columns)?,
            rows: dimension(rows)?,
            spacing: [
                bounded_decimal(&spacing[0], 1, 1024 * FP::SCALE, "spacing X")?,
                bounded_decimal(&spacing[1], 1, 1024 * FP::SCALE, "spacing Z")?,
            ],
            max_vertical: bounded_decimal(
                max_vertical,
                0,
                MAX_ART_COORDINATE_RAW,
                "maximum vertical distance",
            )?,
            height_offset: bounded_decimal(
                height_offset,
                -MAX_ART_COORDINATE_RAW,
                MAX_ART_COORDINATE_RAW,
                "height offset",
            )?,
        };
        config.cells()?;
        Ok(config)
    }

    fn cells(self) -> Result<usize, String> {
        if !(1..=16).contains(&self.columns)
            || !(1..=16).contains(&self.rows)
            || self
                .spacing
                .iter()
                .any(|value| !(1..=1024 * FP::SCALE).contains(&value.raw()))
            || !(0..=MAX_ART_COORDINATE_RAW).contains(&self.max_vertical.raw())
            || !(-MAX_ART_COORDINATE_RAW..=MAX_ART_COORDINATE_RAW)
                .contains(&self.height_offset.raw())
        {
            return Err("Invalid bounded surface grid configuration".into());
        }
        self.columns
            .checked_mul(self.rows)
            .filter(|cells| *cells >= 2)
            .ok_or_else(|| "Surface grid must contain at least two cells".into())
    }
}

fn bounded_decimal(text: &str, minimum: i64, maximum: i64, label: &str) -> Result<FP, String> {
    if text.len() > MAX_INPUT_BYTES {
        return Err(format!(
            "Surface grid {label} exceeds the 48-byte input limit"
        ));
    }
    let value = orr_reflect::decimal::parse_fp(text.trim())
        .map_err(|_| format!("Surface grid {label} must be a bounded plain decimal"))?;
    if !(minimum..=maximum).contains(&value.raw()) {
        return Err(format!(
            "Surface grid {label} is outside its fixed-point range"
        ));
    }
    Ok(value)
}

fn graph_coordinate(raw: i128) -> Result<FP, String> {
    let raw = i64::try_from(raw).map_err(|_| "Surface grid coordinate overflow")?;
    if !(-MAX_COORDINATE_RAW..=MAX_COORDINATE_RAW).contains(&raw) {
        return Err("Surface grid coordinate exceeds the graph bound".into());
    }
    Ok(FP::from_raw(raw))
}

// Existing presentation coordinates are f32. Widen before Q16 rounding so a
// half-step is rounded away from zero rather than truncated or double-rounded.
fn anchor_coordinate(value: f32) -> Result<FP, String> {
    let raw = (f64::from(value) * FP::SCALE as f64).round();
    if !value.is_finite() || raw < i64::MIN as f64 || raw >= -(i64::MIN as f64) {
        return Err("Surface grid source pivot cannot be represented as fixed point".into());
    }
    graph_coordinate(i128::from(raw as i64))
}

fn cell_coordinate(anchor: FP, index: usize, spacing: FP) -> Result<FP, String> {
    let delta = i128::try_from(index)
        .ok()
        .and_then(|index| index.checked_mul(i128::from(spacing.raw())))
        .ok_or("Surface grid cell offset overflow")?;
    let raw = i128::from(anchor.raw())
        .checked_add(delta)
        .ok_or("Surface grid cell coordinate overflow")?;
    graph_coordinate(raw)
}

fn check_projection_work(additions: usize, triangles: usize) -> Result<(), String> {
    if additions
        .checked_mul(triangles)
        .is_none_or(|work| work > MAX_PROJECTION_WORK)
    {
        return Err(format!(
            "Surface grid exceeds the {MAX_PROJECTION_WORK} triangle-query work limit"
        ));
    }
    Ok(())
}

fn stamp_prop_surface_grid(
    document: &Document,
    selected: u64,
    first_id: u64,
    graph: &StaticMeshGraph,
    config: Config,
) -> Result<Document, String> {
    let cells = config.cells()?;
    let additions = cells
        .checked_sub(1)
        .ok_or("Surface grid cell count underflow")?;
    if document
        .props
        .len()
        .checked_add(additions)
        .is_none_or(|count| count > MAX_PROPS)
    {
        return Err(format!(
            "Surface grid would exceed the {MAX_PROPS}-prop scene limit"
        ));
    }
    // Reject the full request before any projection, including cells that miss.
    check_projection_work(additions, graph.triangles().len())?;
    let source = document
        .props
        .iter()
        .find(|prop| prop.id == selected)
        .ok_or("The selected prop no longer exists")?;
    let [x, y, z] = source.art.offset();
    let anchor = [
        anchor_coordinate(x)?,
        anchor_coordinate(y)?,
        anchor_coordinate(z)?,
    ];
    let mut stamped = document.clone();
    let mut identifier = first_id.max(1);
    for row in 0..config.rows {
        for column in 0..config.columns {
            if row == 0 && column == 0 {
                continue; // Preserve the complete source, including non-Q16 f32 coordinates.
            }
            let query = [
                cell_coordinate(anchor[0], column, config.spacing[0])?,
                anchor[1], // Every query uses the original pivot Y, never the last hit.
                cell_coordinate(anchor[2], row, config.spacing[1])?,
            ];
            let mut position = graph
                .project(query, config.max_vertical)
                .map_err(|error| {
                    format!("Surface grid cell [{column},{row}] has no admitted height: {error}")
                })?
                .position;
            // The vertical search limit applies before the explicit offset.
            position[1] = graph_coordinate(
                i128::from(position[1].raw())
                    .checked_add(i128::from(config.height_offset.raw()))
                    .ok_or("Surface grid height offset overflow")?,
            )?;
            if position.iter().any(|value| {
                !(-MAX_ART_COORDINATE_RAW..=MAX_ART_COORDINATE_RAW).contains(&value.raw())
            }) {
                return Err("Surface grid placement exceeds the mesh art coordinate bound".into());
            }
            for _ in 0..=MAX_PROPS {
                if stamped.props.iter().all(|prop| prop.id != identifier) {
                    break;
                }
                identifier = identifier.checked_add(1).unwrap_or(1);
            }
            let name = duplicate_prop_name(&stamped, &source.name, identifier)?;
            // Conversion is presentation rounding, not an exact Q16 contact claim.
            let art = source
                .art
                .clone()
                .with_transform(position.map(FP::to_f32), source.art.scale())?;
            stamped.props.push(Prop {
                id: identifier,
                name,
                art,
                #[cfg(feature = "mesh-prop-shading")]
                shading: source.shading.clone(),
            });
            identifier = identifier.checked_add(1).unwrap_or(1);
        }
    }
    // Includes composed appearances, draw count, art validity and sidecar bytes.
    validate_document(&stamped, graph.revision())?;
    Ok(stamped)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    scene: PathBuf,
    scene_epoch: u64,
    graph_revision: [u8; 32],
    checksum: u64,
}

impl Panel {
    fn surface_grid_context(
        &self,
        editor: &Editor,
        agent_pick_busy: bool,
    ) -> Result<Context, String> {
        if !self.can_edit(editor)
            || editor.history().in_tx
            || editor.sim().in_tx
            || self.pending_prefab()
            || self.candidate.is_some()
            || self.unapplied_form_dirty()
            || self.material_slots.dirty()
            || agent_pick_busy
        {
            return Err("Surface grid requires the matching editable local scene with no pending prop drafts, gestures or endpoint picking".into());
        }
        self.selected_prop()
            .ok_or("Select an applied prop before stamping a surface grid")?;
        let (scene, graph_revision) = current_identity(editor)
            .ok_or("Wait for an admitted mesh graph before stamping a surface grid")?;
        Ok(Context {
            scene,
            scene_epoch: editor.scene_load_epoch(),
            graph_revision,
            checksum: editor.sim().checksum,
        })
    }

    pub(super) fn stamp_surface_grid(&mut self, editor: &Editor, agent_pick_busy: bool) {
        let result = (|| {
            let context = self.surface_grid_context(editor, agent_pick_busy)?;
            let config = Config::parse(
                &self.grid_columns,
                &self.grid_rows,
                &self.grid_spacing,
                &self.surface_grid_max_vertical,
                &self.surface_grid_height_offset,
            )?;
            let view = mesh_from_view(
                editor
                    .snapshot()
                    .ok_or("Wait for an admitted mesh snapshot")?
                    .predicted(),
            )?;
            if view.graph.revision() != context.graph_revision {
                return Err("Surface grid graph changed before projection".into());
            }
            let document = stamp_prop_surface_grid(
                &self.document,
                self.selected.ok_or("Select an applied prop")?,
                self.available_prop_id()
                    .ok_or("No fresh prop identifier is available")?,
                &view.graph,
                config,
            )?;
            let cells = config.cells()?;
            self.commit_surface_grid(editor, agent_pick_busy, &context, document)?;
            Ok::<_, String>(cells)
        })();
        match result {
            Ok(cells) => {
                self.recompute_next_prop_id();
                self.copy_selection_fields();
                self.error = None;
                self.status = format!("Stamped {cells} surface-height prop grid cells as one cosmetic edit; save explicitly to persist.");
            }
            Err(error) => self.error = Some(format!("Surface prop grid was not applied: {error}")),
        }
    }

    fn commit_surface_grid(
        &mut self,
        editor: &Editor,
        agent_pick_busy: bool,
        expected: &Context,
        document: Document,
    ) -> Result<(), String> {
        // No yielding or external mutation occurs between this final check and
        // the single history push. All fallible document work is already done.
        if self.surface_grid_context(editor, agent_pick_busy)? != *expected {
            return Err("Surface grid scene, graph or checksum changed before apply".into());
        }
        self.push_document(document);
        Ok(())
    }
}

#[cfg(test)]
#[path = "surface_grid_tests.rs"]
mod tests;
