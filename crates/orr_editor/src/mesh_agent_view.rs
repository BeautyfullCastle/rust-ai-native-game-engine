use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, MAX_TRIANGLES};

const MAX_DISPLAY_RAW: i128 = 256 * FP::SCALE as i128;
const MAX_RAY_COMPONENT: f32 = 1.0e6;
const MAX_VERTICES: usize = 65_536;
const EDGE_EPSILON: f64 = 1.0e-10;
const PARALLEL_EPSILON: f64 = 1.0e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SurfaceHit {
    pub position: [FP; 3],
    pub triangle: u32,
}

pub(crate) fn pick(
    graph: &StaticMeshGraph,
    origin: [f32; 3],
    direction: [f32; 3],
    max_vertical: FP,
) -> Result<Option<SurfaceHit>, String> {
    if max_vertical.raw() < 0 {
        return Err("mesh picking vertical tolerance cannot be negative".into());
    }
    if origin
        .iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_RAY_COMPONENT)
    {
        return Err("mesh picking ray origin exceeds the safe viewing range".into());
    }
    if direction
        .iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_RAY_COMPONENT)
    {
        return Err("mesh picking ray direction is invalid or unbounded".into());
    }
    if graph.triangles().len() > MAX_TRIANGLES as usize || graph.vertices().len() > MAX_VERTICES {
        return Err("mesh picking graph exceeds its geometry bound".into());
    }

    let direction = normalize(direction)?;
    let origin = origin.map(f64::from);
    let mut nearest: Option<(f64, u32, [f64; 3])> = None;
    for triangle in graph.triangles() {
        let mut points = [[0.0; 3]; 3];
        for (slot, vertex_index) in triangle.vertices.iter().enumerate() {
            let vertex = graph
                .vertices()
                .get(*vertex_index as usize)
                .ok_or_else(|| "mesh picking triangle references a missing vertex".to_owned())?;
            for axis in 0..3 {
                let raw = i128::from(vertex[axis].raw());
                if raw.abs() > MAX_DISPLAY_RAW {
                    return Err("mesh picking triangle exceeds the display bounds".into());
                }
                points[slot][axis] = vertex[axis].raw() as f64 / FP::SCALE as f64;
            }
        }
        let Some(distance) = ray_triangle(origin, direction, points) else {
            continue;
        };
        let position = std::array::from_fn(|axis| origin[axis] + direction[axis] * distance);
        if position
            .iter()
            .any(|value| !value.is_finite() || value.abs() > 256.0)
        {
            continue;
        }
        let nearer = nearest.is_none_or(|(best_distance, best_triangle, _)| {
            distance
                .total_cmp(&best_distance)
                .then_with(|| triangle.key.cmp(&best_triangle))
                .is_lt()
        });
        if nearer {
            nearest = Some((distance, triangle.key, position));
        }
    }

    let Some((_, triangle, position)) = nearest else {
        return Ok(None);
    };
    let position = match quantize(position) {
        Some(position) => position,
        None => return Ok(None),
    };
    let Ok(projected) = graph.project(position, max_vertical) else {
        return Ok(None);
    };
    if projected.triangle != triangle {
        return Ok(None);
    }
    Ok(Some(SurfaceHit {
        position: projected.position,
        triangle,
    }))
}

fn normalize(direction: [f32; 3]) -> Result<[f64; 3], String> {
    let direction = direction.map(f64::from);
    let scale = direction
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f64, f64::max);
    if scale == 0.0 {
        return Err("mesh picking ray direction must be nonzero".into());
    }
    let scaled = direction.map(|value| value / scale);
    let length = dot(scaled, scaled).sqrt();
    if !length.is_finite() || length == 0.0 {
        return Err("mesh picking ray direction cannot be normalized".into());
    }
    Ok(scaled.map(|value| value / length))
}

fn quantize(position: [f64; 3]) -> Option<[FP; 3]> {
    let mut quantized = [FP::ZERO; 3];
    for axis in 0..3 {
        if !position[axis].is_finite() || position[axis].abs() > 256.0 {
            return None;
        }
        let raw = (position[axis] * FP::SCALE as f64).round();
        if !raw.is_finite() || raw.abs() > MAX_DISPLAY_RAW as f64 {
            return None;
        }
        quantized[axis] = FP::from_raw(raw as i64);
    }
    Some(quantized)
}

fn ray_triangle(origin: [f64; 3], direction: [f64; 3], points: [[f64; 3]; 3]) -> Option<f64> {
    let first_edge = subtract(points[1], points[0]);
    let second_edge = subtract(points[2], points[0]);
    let perpendicular = cross(direction, second_edge);
    let determinant = dot(first_edge, perpendicular);
    if !determinant.is_finite() || determinant.abs() <= PARALLEL_EPSILON {
        return None;
    }
    let inverse = 1.0 / determinant;
    let offset = subtract(origin, points[0]);
    let first_weight = dot(offset, perpendicular) * inverse;
    if !first_weight.is_finite() || !(-EDGE_EPSILON..=1.0 + EDGE_EPSILON).contains(&first_weight) {
        return None;
    }
    let cross_offset = cross(offset, first_edge);
    let second_weight = dot(direction, cross_offset) * inverse;
    if !second_weight.is_finite()
        || second_weight < -EDGE_EPSILON
        || first_weight + second_weight > 1.0 + EDGE_EPSILON
    {
        return None;
    }
    let distance = dot(second_edge, cross_offset) * inverse;
    (distance.is_finite() && distance >= 0.0).then_some(distance)
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| left[axis] - right[axis])
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    (0..3).map(|axis| left[axis] * right[axis]).sum()
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

#[cfg(test)]
#[path = "mesh_agent_view/tests.rs"]
mod tests;
