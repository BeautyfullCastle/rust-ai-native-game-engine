//! Bounded warning bands on the authored sphere surface (not agent-radius inflation).
use super::{add_triangle, exact_f32, point_to_f32, VIEW_ASSET_ID};
use crate::editor::Editor;
use orr_model::{Material, Primitive, StaticModel, Wrap, IDENTITY};
use orr_remote::mesh_navigation::{
    hazards::{self, HazardsView},
    MeshView,
};

const SEGMENTS: usize = 32;
const BAND_HALF_ANGLE: f32 = 0.0125;

pub(super) fn build(
    base: StaticModel,
    mesh: &MeshView,
    hazards: Option<&HazardsView>,
) -> Result<StaticModel, String> {
    let Some(hazards) = hazards else {
        return Ok(base);
    };
    if hazards.config.enabled != 1
        || !(1..=8).contains(&hazards.config.count)
        || hazards.config.graph_revision != mesh.graph.revision()
        || hazards.run.lost > 1
        || (hazards.run.lost == 0 && (hazards.run.hit_index != 0 || hazards.run.lost_tick != 0))
        || (hazards.run.lost == 1
            && (hazards.run.hit_index >= hazards.config.count || hazards.run.lost_tick == 0))
    {
        return Err(
            "Hazard markers require current admitted configuration and runtime state".into(),
        );
    }
    let mut source = base.source().clone();
    for (index, sphere) in hazards
        .config
        .spheres
        .iter()
        .take(hazards.config.count as usize)
        .enumerate()
    {
        let center = point_to_f32(sphere.center)?;
        let radius = exact_f32(sphere.radius)?;
        if radius <= 0.0 {
            return Err("Hazard radius must be positive".into());
        }
        let mut primitive = Primitive {
            id: format!("{VIEW_ASSET_ID}#node={}", source.primitives.len()),
            vertices: Vec::new(),
            indices: Vec::new(),
            material: source.materials.len() as u32,
            transform: IDENTITY,
        };
        sphere_bands(&mut primitive, center, radius);
        let color = if hazards.run.lost == 1 && hazards.run.hit_index as usize == index {
            [1.0, 0.05, 0.1, 1.0]
        } else {
            [1.0, 0.55, 0.05, 1.0]
        };
        source.materials.push(Material {
            base_color: color,
            image: 0,
            linear_filter: false,
            wrap_s: Wrap::Clamp,
            wrap_t: Wrap::Clamp,
        });
        source.primitives.push(primitive);
    }
    StaticModel::new(source).map_err(|error| error.to_string())
}

fn sphere_bands(primitive: &mut Primitive, center: [f32; 3], radius: f32) {
    // Three great circles, each split at its centerline so the displayed extrema
    // reach the authored radius exactly. All vertices lie on the sphere surface.
    for axis in 0..3 {
        for segment in 0..SEGMENTS {
            let angle = |step| std::f32::consts::TAU * step as f32 / SEGMENTS as f32;
            let point = |theta: f32, latitude: f32| {
                let mut p = center;
                p[axis] += radius * latitude.sin();
                p[(axis + 1) % 3] += radius * latitude.cos() * theta.cos();
                p[(axis + 2) % 3] += radius * latitude.cos() * theta.sin();
                p
            };
            for (low, high) in [(-BAND_HALF_ANGLE, 0.0), (0.0, BAND_HALF_ANGLE)] {
                let a = point(angle(segment), low);
                let b = point(angle(segment + 1), low);
                let c = point(angle(segment + 1), high);
                let d = point(angle(segment), high);
                let mut normal = [0.0; 3];
                normal[axis] = 1.0;
                add_triangle(primitive, [a, b, c], normal);
                add_triangle(primitive, [a, c, d], normal);
                // Warning bands remain visible from either side, including inside.
                add_triangle(primitive, [c, b, a], normal.map(|v| -v));
                add_triangle(primitive, [d, c, a], normal.map(|v| -v));
            }
        }
    }
}

/// Legacy frames do not contain hazard singletons; identity must be checked first.
pub(super) fn read(
    game: crate::game::EditorGame,
    frame: orr_bridge::FrameView<'_>,
) -> Result<Option<HazardsView>, String> {
    if !game.is_mesh_hazards() {
        return Ok(None);
    }
    hazards::from_view(frame)
}

/// Profile check precedes every hazard singleton read, including pending host switches.
pub(crate) fn status(editor: &Editor) -> Result<Option<String>, String> {
    if !editor.game().is_mesh_hazards() {
        return Ok(None);
    }
    crate::mesh_context::current(editor)?;
    let frame = editor
        .snapshot()
        .ok_or("Wait for the hazard snapshot")?
        .predicted();
    let view = read(editor.game(), frame)?;
    if let Some(view) = view.as_ref().filter(|view| view.run.lost == 1) {
        return Ok(Some(format!(
            "Lost · hazard {} · tick {} · reset Play to retry",
            view.run.hit_index + 1,
            view.run.lost_tick
        )));
    }
    let mut status = match view {
        Some(view) => format!(
            "Hazards active · {} spheres · orange bands show authored radii",
            view.config.count
        ),
        None => "Hazards disabled".into(),
    };
    if let Some(mission) = orr_remote::mesh_navigation::mission::from_view(frame)? {
        if mission.run.won == 1 {
            status = format!("Mission complete · {status} · reset Play to retry");
        } else {
            status = format!(
                "Collected {}/{} · {status}",
                mission.run.collected.count_ones(),
                mission.config.count
            );
        }
    }
    Ok(Some(status))
}

#[cfg(test)]
#[path = "hazards_view/tests.rs"]
mod tests;
