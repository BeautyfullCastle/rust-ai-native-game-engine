use super::geometry;
use orr_fp::FP;
use orr_navigation::mesh::StaticMeshGraph;
use orr_render::{Camera3D, Projection};
use std::collections::BTreeSet;

const PICK_RADIUS_PIXELS: f32 = 14.0;
const MAX_VIEWPORT: u32 = 16_384;
const MAX_GRAPH_BYTES: usize = 8_192;
const MAX_VERTEX_COUNT: usize = 1_024;

pub(crate) fn pick_vertex(
    graph: &StaticMeshGraph,
    camera: &Camera3D,
    pixel: [f32; 2],
    viewport: (u32, u32),
) -> Result<Option<u32>, String> {
    if viewport.0 == 0
        || viewport.1 == 0
        || viewport.0 > MAX_VIEWPORT
        || viewport.1 > MAX_VIEWPORT
        || !pixel.iter().all(|coordinate| coordinate.is_finite())
        || pixel[0] < 0.0
        || pixel[1] < 0.0
        || pixel[0] > viewport.0 as f32
        || pixel[1] > viewport.1 as f32
    {
        return Err("vertex pick has invalid viewport coordinates".into());
    }
    let bytes = graph.to_bytes();
    if bytes.len() > MAX_GRAPH_BYTES || graph.vertices().len() > MAX_VERTEX_COUNT {
        return Err("vertex pick exceeds the admitted mesh bound".into());
    }
    let faces = geometry::faces(graph)?;
    let mut referenced = BTreeSet::new();
    for face in faces {
        for index in face {
            if index as usize >= graph.vertices().len() {
                return Err("mesh face references a missing vertex".into());
            }
            referenced.insert(index);
        }
    }
    let forward = camera.forward();
    let right = camera.right();
    let camera_up = camera.camera_up();
    if !camera
        .eye
        .iter()
        .chain(camera.target.iter())
        .chain(camera.up.iter())
        .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= 1.0e6)
        || !forward.iter().all(|coordinate| coordinate.is_finite())
        || !right
            .iter()
            .chain(camera_up.iter())
            .all(|coordinate| coordinate.is_finite())
        || vector_length_squared(forward) <= 1.0e-12
        || vector_length_squared(right) <= 1.0e-12
        || vector_length_squared(camera_up) <= 1.0e-12
        || !valid_projection(camera.projection)
    {
        return Err("vertex pick camera is invalid".into());
    }
    let mut nearest: Option<(f32, f32, u32)> = None;
    for index in referenced {
        let position = graph.vertices()[index as usize];
        let world = [
            exact_display(position[0])?,
            exact_display(position[1])?,
            exact_display(position[2])?,
        ];
        let Some(screen) = camera.world_to_screen(world, viewport) else {
            continue;
        };
        if !screen.iter().all(|coordinate| coordinate.is_finite()) {
            continue;
        }
        let pixel_distance = (screen[0] - pixel[0]).powi(2) + (screen[1] - pixel[1]).powi(2);
        if !pixel_distance.is_finite() || pixel_distance > PICK_RADIUS_PIXELS.powi(2) {
            continue;
        }
        let depth = (0..3)
            .map(|axis| (world[axis] - camera.eye[axis]) * forward[axis])
            .sum::<f32>();
        if !depth.is_finite() || depth <= 0.0 {
            continue;
        }
        let candidate = (pixel_distance, depth, index);
        let replaces = nearest.is_none_or(|current| {
            candidate.0.total_cmp(&current.0).is_lt()
                || (candidate.0.total_cmp(&current.0).is_eq()
                    && (candidate.1.total_cmp(&current.1).is_lt()
                        || (candidate.1.total_cmp(&current.1).is_eq() && candidate.2 < current.2)))
        });
        if replaces {
            nearest = Some(candidate);
        }
    }
    Ok(nearest.map(|(_, _, index)| index))
}

fn vector_length_squared(vector: [f32; 3]) -> f32 {
    vector
        .iter()
        .map(|coordinate| coordinate * coordinate)
        .sum()
}

fn valid_projection(projection: Projection) -> bool {
    match projection {
        Projection::Perspective { fov_y, near, far } => {
            fov_y.is_finite()
                && (0.001..(std::f32::consts::PI - 0.001)).contains(&fov_y)
                && near.is_finite()
                && far.is_finite()
                && near > 0.0
                && far > near
                && far <= 1.0e6
        }
        Projection::Orthographic {
            half_height,
            near,
            far,
        } => {
            half_height.is_finite()
                && (0.001..=1.0e6).contains(&half_height)
                && near.is_finite()
                && far.is_finite()
                && near.abs() <= 1.0e6
                && far.abs() <= 1.0e6
                && far > near
        }
    }
}

pub(crate) fn exact_display(value: FP) -> Result<f32, String> {
    let display = value.to_f32();
    if i128::from(value.raw()).abs() > i128::from(256 * FP::SCALE)
        || !display.is_finite()
        || f64::from(display) * FP::SCALE as f64 != value.raw() as f64
    {
        return Err("mesh vertex cannot be displayed exactly within ±256".into());
    }
    Ok(display)
}

pub(crate) fn quantize_snapped(position: [f32; 3]) -> Result<[FP; 3], String> {
    let mut converted = [FP::ZERO; 3];
    for axis in 0..3 {
        let raw = f64::from(position[axis]) * FP::SCALE as f64;
        if !raw.is_finite() || raw.abs() > (256 * FP::SCALE) as f64 {
            return Err("vertex drag exceeds the ±256 mesh limit".into());
        }
        let rounded = raw.round();
        if (raw - rounded).abs() > 0.000_001 {
            return Err("vertex drag could not be represented exactly".into());
        }
        converted[axis] = FP::from_raw(rounded as i64);
    }
    Ok(converted)
}

#[cfg(test)]
#[path = "geometry_vertex_view/tests.rs"]
mod tests;
