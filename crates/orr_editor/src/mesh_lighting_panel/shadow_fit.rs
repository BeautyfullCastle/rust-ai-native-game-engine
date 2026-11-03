//! Bounded presentation-only fit for the authoritative static graph and applied props.
use crate::{mesh_prop_view, viewport3d::SceneModel};
use orr_fp::FP;
use orr_navigation::mesh::StaticMeshGraph;
use orr_render::{renderer3d::light_view_proj, Lighting};

// Same display contract as mesh_navigation::exact_f32, kept private here.
fn exact_f32(value: FP) -> Result<f32, String> {
    let display = value.to_f32();
    if i128::from(value.raw()).abs() > i128::from(256 * 65_536_i64)
        || !display.is_finite()
        || f64::from(display) * 65_536.0 != value.raw() as f64
    {
        return Err("Host mesh view contains a coordinate that cannot be displayed exactly".into());
    }
    Ok(display)
}

pub(super) fn fit(
    graph: &StaticMeshGraph,
    props: &[SceneModel],
    lighting: Lighting,
) -> Result<([f32; 3], f32), String> {
    if props.len() > 128 {
        return Err("Shadow fit exceeds 128 applied props".into());
    }
    let mut count = graph.vertices().len();
    for prop in props {
        let mut per_model = 0usize;
        for primitive in &prop.model.source().primitives {
            per_model = per_model
                .checked_add(primitive.vertices.len())
                .ok_or("Shadow fit vertex count overflow")?;
        }
        if per_model > 100_000 {
            return Err("Shadow fit prop exceeds 100000 vertices".into());
        }
        count = count
            .checked_add(per_model)
            .ok_or("Shadow fit vertex count overflow")?;
        if count > 1_000_000 {
            return Err("Shadow fit exceeds 1000000 total vertices".into());
        }
    }
    if count == 0 {
        return Err("No static geometry to fit".into());
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut include = |point: [f32; 3]| {
        for axis in 0..3 {
            min[axis] = min[axis].min(f64::from(point[axis]));
            max[axis] = max[axis].max(f64::from(point[axis]));
        }
    };
    for vertex in graph.vertices() {
        include([
            exact_f32(vertex[0])?,
            exact_f32(vertex[1])?,
            exact_f32(vertex[2])?,
        ]);
    }
    for prop in props {
        let bounds = mesh_prop_view::bounds(prop)?;
        include(bounds.min);
        include(bounds.max);
    }
    fit_bounds(min, max, lighting)
}

fn fit_bounds(
    min: [f64; 3],
    max: [f64; 3],
    mut lighting: Lighting,
) -> Result<([f32; 3], f32), String> {
    if !lighting.direction.iter().all(|v| v.is_finite()) {
        return Err("Shadow light direction is nonfinite".into());
    }
    let mut center = [0.0_f32; 3];
    let mut squared = 0.0_f64;
    let mut magnitude = 1.0_f64;
    for axis in 0..3 {
        if !min[axis].is_finite() || !max[axis].is_finite() || min[axis] > max[axis] {
            return Err("Invalid or empty shadow bounds".into());
        }
        let midpoint = (min[axis] + max[axis]) * 0.5;
        if midpoint.abs() > 1.0e6 {
            return Err("Shadow center exceeds ±1000000".into());
        }
        center[axis] = midpoint as f32;
        // Include center rounding in the sphere, not just the ideal midpoint.
        let half = (min[axis] - f64::from(center[axis]))
            .abs()
            .max((max[axis] - f64::from(center[axis])).abs());
        squared += half * half;
        magnitude = magnitude.max(min[axis].abs()).max(max[axis].abs());
    }
    // 2% covers texel snapping (worst supported map: 512); the absolute
    // allowance covers large translated f32 matrix construction/cancellation.
    let radius = (squared.sqrt() * 1.02 + magnitude * f64::from(f32::EPSILON) * 32.0).max(0.5);
    if !radius.is_finite() || radius > 1.0e5 {
        return Err("Shadow radius exceeds 100000".into());
    }
    let radius = radius as f32;
    lighting.shadow_center = center;
    lighting.shadow_radius = radius;
    for size in [512, 1024, 2048] {
        let matrix = light_view_proj(&lighting, size);
        for corner in 0..8 {
            let point = std::array::from_fn(|axis| {
                if corner & (1 << axis) == 0 {
                    min[axis] as f32
                } else {
                    max[axis] as f32
                }
            });
            let clip = matrix.transform_point4(point);
            if !clip.iter().all(|v| v.is_finite()) || clip[3] <= 0.0 {
                return Err("Shadow fit projection is nonfinite or invalid".into());
            }
            let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
            if ndc[0].abs() > 0.999 || ndc[1].abs() > 0.999 || ndc[2] < 0.001 || ndc[2] > 0.999 {
                return Err(
                    "Shadow fit cannot retain the required clip margin after renderer snapping"
                        .into(),
                );
            }
        }
    }
    Ok((center, radius))
}
#[cfg(test)]
#[path = "shadow_fit/tests.rs"]
mod tests;
