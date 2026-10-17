use crate::viewport3d::SceneModel;
use orr_render::{
    math3::{quat_rotate, Mat4},
    Camera3D,
};

const MAX_SOURCE_VERTICES: usize = 100_000;
const MAX_COORDINATE: f32 = 1.0e9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

pub(crate) fn bounds(model: &SceneModel) -> Result<Bounds, String> {
    let instance = instance_matrix(
        model.instance.translation,
        model.instance.rotation,
        model.instance.scale,
    )?;
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    let mut vertex_count = 0usize;
    for primitive in &model.model.source().primitives {
        vertex_count = vertex_count
            .checked_add(primitive.vertices.len())
            .ok_or_else(|| "prop vertex count overflow".to_owned())?;
        if vertex_count > MAX_SOURCE_VERTICES {
            return Err("prop exceeds the viewport vertex bound".into());
        }
        let node = Mat4(primitive.transform);
        let world = instance.mul(&node);
        if !world.0.iter().flatten().all(|value| value.is_finite()) {
            return Err("prop transform is nonfinite".into());
        }
        for vertex in &primitive.vertices {
            let transformed = world.transform_point4(vertex.position);
            if !transformed.iter().all(|value| value.is_finite()) || transformed[3].abs() < 1.0e-6 {
                return Err("prop vertex transform is invalid".into());
            }
            let position = [
                transformed[0] / transformed[3],
                transformed[1] / transformed[3],
                transformed[2] / transformed[3],
            ];
            if !position
                .iter()
                .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE)
            {
                return Err("prop bounds exceed the viewport coordinate limit".into());
            }
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(position[axis]);
                maximum[axis] = maximum[axis].max(position[axis]);
            }
        }
    }
    if vertex_count == 0 {
        return Err("prop has no vertices to bound".into());
    }
    Ok(Bounds {
        min: minimum,
        max: maximum,
    })
}

pub(crate) fn ray_hit(bounds: Bounds, origin: [f32; 3], direction: [f32; 3]) -> Option<f32> {
    if !valid_bounds(bounds)
        || !origin
            .iter()
            .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE)
        || !direction.iter().all(|value| value.is_finite())
        || direction.iter().all(|value| *value == 0.0)
    {
        return None;
    }
    let mut near = 0.0_f32;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        if direction[axis] == 0.0 {
            if origin[axis] < bounds.min[axis] || origin[axis] > bounds.max[axis] {
                return None;
            }
            continue;
        }
        let first = (bounds.min[axis] - origin[axis]) / direction[axis];
        let second = (bounds.max[axis] - origin[axis]) / direction[axis];
        if !first.is_finite() || !second.is_finite() {
            return None;
        }
        near = near.max(first.min(second));
        far = far.min(first.max(second));
        if near > far {
            return None;
        }
    }
    (far >= 0.0 && near.is_finite()).then_some(near.max(0.0))
}

pub(crate) fn pick(
    models: &[(u64, SceneModel)],
    origin: [f32; 3],
    direction: [f32; 3],
) -> Result<Option<u64>, String> {
    let mut nearest: Option<(f32, u64)> = None;
    for (id, model) in models {
        let Some(distance) = ray_hit(bounds(model)?, origin, direction) else {
            continue;
        };
        let nearer = match nearest {
            None => true,
            Some((best_distance, best_id)) => {
                distance.total_cmp(&best_distance).is_lt()
                    || (distance.total_cmp(&best_distance).is_eq() && *id < best_id)
            }
        };
        if nearer {
            nearest = Some((distance, *id));
        }
    }
    Ok(nearest.map(|(_, id)| id))
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Drag {
    height: f32,
    grab_offset: [f32; 2],
}

impl Drag {
    pub(crate) fn new(
        camera: &Camera3D,
        pixel: [f32; 2],
        viewport: (u32, u32),
        position: [f32; 3],
    ) -> Option<Self> {
        if viewport.0 == 0
            || viewport.1 == 0
            || !pixel.iter().all(|value| value.is_finite())
            || !position
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 1.0e6)
        {
            return None;
        }
        let cursor = camera.screen_to_plane_y(pixel, viewport, position[1])?;
        if !cursor.iter().all(|value| value.is_finite()) {
            return None;
        }
        Some(Self {
            height: position[1],
            grab_offset: [position[0] - cursor[0], position[2] - cursor[2]],
        })
    }

    pub(crate) fn position(
        &self,
        camera: &Camera3D,
        pixel: [f32; 2],
        viewport: (u32, u32),
        snap: Option<f32>,
    ) -> Option<[f32; 3]> {
        if viewport.0 == 0
            || viewport.1 == 0
            || !pixel.iter().all(|value| value.is_finite())
            || snap.is_some_and(|step| !step.is_finite() || step <= 0.0)
        {
            return None;
        }
        let cursor = camera.screen_to_plane_y(pixel, viewport, self.height)?;
        if !cursor.iter().all(|value| value.is_finite()) {
            return None;
        }
        let mut result = [
            cursor[0] + self.grab_offset[0],
            self.height,
            cursor[2] + self.grab_offset[1],
        ];
        if let Some(step) = snap {
            result[0] = (result[0] / step).round() * step;
            result[2] = (result[2] / step).round() * step;
        }
        result
            .iter()
            .all(|value| value.is_finite() && value.abs() <= 1.0e6)
            .then_some(result)
    }
}

fn valid_bounds(bounds: Bounds) -> bool {
    (0..3).all(|axis| {
        bounds.min[axis].is_finite()
            && bounds.max[axis].is_finite()
            && bounds.min[axis].abs() <= MAX_COORDINATE
            && bounds.max[axis].abs() <= MAX_COORDINATE
            && bounds.min[axis] <= bounds.max[axis]
    })
}

fn instance_matrix(
    translation: [f32; 3],
    rotation: [f32; 4],
    scale: [f32; 3],
) -> Result<Mat4, String> {
    let norm = rotation.iter().map(|value| value * value).sum::<f32>();
    if !translation
        .iter()
        .all(|value| value.is_finite() && value.abs() <= 1.0e6)
        || !scale
            .iter()
            .all(|value| value.is_finite() && *value > 0.0 && *value <= 1.0e6)
        || !norm.is_finite()
        || (norm - 1.0).abs() > 1.0e-3
    {
        return Err("prop instance transform is invalid".into());
    }
    let unit_rotation = rotation.map(|value| value / norm.sqrt());
    let mut matrix = Mat4::IDENTITY;
    for axis in 0..3 {
        let mut basis = [0.0; 3];
        basis[axis] = scale[axis];
        let rotated = quat_rotate(unit_rotation, basis);
        matrix.0[axis][..3].copy_from_slice(&rotated);
    }
    matrix.0[3][..3].copy_from_slice(&translation);
    Ok(matrix)
}

#[cfg(test)]
#[path = "mesh_prop_view/tests.rs"]
mod tests;
