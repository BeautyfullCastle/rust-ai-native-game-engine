use orr_remote::mesh_navigation::MeshView;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Facing {
    #[default]
    Fixed,
    Route,
}

pub(crate) fn is_fixed(facing: &Facing) -> bool {
    *facing == Facing::Fixed
}

pub(crate) fn rotation(
    facing: Facing,
    authored: [f32; 4],
    view: &MeshView,
) -> Result<[f32; 4], String> {
    crate::mesh_rotation::validate_normalize(authored)?;
    if facing == Facing::Fixed {
        return Ok(authored);
    }
    let Some(yaw) = route_yaw(view)? else {
        return Ok(authored);
    };
    rotation_with_yaw(facing, authored, Some(yaw))
}

fn rotation_with_yaw(
    facing: Facing,
    authored: [f32; 4],
    route_yaw: Option<f32>,
) -> Result<[f32; 4], String> {
    let normalized_authored = crate::mesh_rotation::validate_normalize(authored)?;
    if facing == Facing::Fixed {
        return Ok(authored);
    }
    let Some(yaw) = route_yaw else {
        return Ok(authored);
    };
    let half_yaw = yaw * 0.5;
    let route_rotation = [0.0, half_yaw.sin(), 0.0, half_yaw.cos()];
    crate::mesh_rotation::validate_normalize(multiply(route_rotation, normalized_authored))
}

fn route_yaw(view: &MeshView) -> Result<Option<f32>, String> {
    view.graph
        .validate_path(&view.agent.path)
        .map_err(|error| format!("invalid admitted mesh route: {error}"))?;
    Ok(route_yaw_on_path(
        &view.agent.path.points,
        view.agent.distance,
    ))
}

fn route_yaw_on_path(points: &[[orr_fp::FP; 3]], route_distance: orr_fp::FP) -> Option<f32> {
    if points.len() < 2 || route_distance.raw() < 0 {
        return None;
    }

    let mut cumulative_raw = 0u64;
    let mut best_distance_raw = u64::MAX;
    let mut best_yaw = None;
    let mut containing_yaw = None;
    let route_distance_raw = route_distance.raw() as u64;
    for pair in points.windows(2) {
        let delta_x = i128::from(pair[1][0].raw()) - i128::from(pair[0][0].raw());
        let delta_y = i128::from(pair[1][1].raw()) - i128::from(pair[0][1].raw());
        let delta_z = i128::from(pair[1][2].raw()) - i128::from(pair[0][2].raw());
        let segment_length_raw = distance_ceil_raw(delta_x, delta_y, delta_z)?;
        let segment_end_raw = cumulative_raw.checked_add(segment_length_raw)?;
        if (delta_x != 0 || delta_z != 0) && segment_length_raw != 0 {
            let nearest_raw = route_distance_raw.clamp(cumulative_raw, segment_end_raw);
            let distance_from_agent_raw = route_distance_raw.abs_diff(nearest_raw);
            if distance_from_agent_raw <= best_distance_raw {
                best_distance_raw = distance_from_agent_raw;
                let delta_x = delta_x as f64;
                let delta_z = delta_z as f64;
                let yaw = delta_x.atan2(delta_z) as f32;
                if !yaw.is_finite() {
                    return None;
                }
                best_yaw = Some(yaw);
                if route_distance_raw >= cumulative_raw && route_distance_raw < segment_end_raw {
                    containing_yaw = Some(yaw);
                    break;
                }
            }
        }
        cumulative_raw = segment_end_raw;
    }
    containing_yaw.or(best_yaw)
}

fn distance_ceil_raw(delta_x: i128, delta_y: i128, delta_z: i128) -> Option<u64> {
    let squared_x = delta_x.unsigned_abs().checked_mul(delta_x.unsigned_abs())?;
    let squared_y = delta_y.unsigned_abs().checked_mul(delta_y.unsigned_abs())?;
    let squared_z = delta_z.unsigned_abs().checked_mul(delta_z.unsigned_abs())?;
    let squared_length = squared_x.checked_add(squared_y)?.checked_add(squared_z)?;
    let floor_length = squared_length.isqrt();
    let ceil_length = if floor_length.checked_mul(floor_length)? == squared_length {
        floor_length
    } else {
        floor_length.checked_add(1)?
    };
    u64::try_from(ceil_length).ok()
}

fn multiply(left: [f32; 4], right: [f32; 4]) -> [f32; 4] {
    let [left_x, left_y, left_z, left_w] = left;
    let [right_x, right_y, right_z, right_w] = right;
    [
        left_w * right_x + left_x * right_w + left_y * right_z - left_z * right_y,
        left_w * right_y - left_x * right_z + left_y * right_w + left_z * right_x,
        left_w * right_z + left_x * right_y - left_y * right_x + left_z * right_w,
        left_w * right_w - left_x * right_x - left_y * right_y - left_z * right_z,
    ]
}

#[cfg(test)]
#[path = "mesh_facing/tests.rs"]
mod tests;
