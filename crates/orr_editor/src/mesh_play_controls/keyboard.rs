use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, SearchBudget};

const MAX_GOAL_RAW: i128 = 256 * FP::SCALE as i128;
const MAX_PATH_EXPANSIONS: u32 = 512;

pub(super) fn goal(
    graph: &StaticMeshGraph,
    position: [FP; 3],
    max_vertical: FP,
    direction: [i8; 2],
) -> Result<[FP; 3], String> {
    if direction
        .iter()
        .any(|component| !(-1..=1).contains(component))
        || direction == [0, 0]
    {
        return Err("keyboard direction must use nonzero world-axis steps from -1 to 1".into());
    }
    if max_vertical.raw() < 0 {
        return Err("keyboard goal vertical tolerance cannot be negative".into());
    }
    for coordinate in position {
        validate_coordinate(coordinate)?;
    }

    let half_step_raw = FP::SCALE / 2;
    let mut candidate = position;
    for (direction_axis, coordinate_axis) in [(0, 0), (1, 2)] {
        let component = direction[direction_axis];
        if component == 0 {
            continue;
        }
        let delta = if component < 0 {
            half_step_raw.checked_neg()
        } else {
            Some(half_step_raw)
        }
        .ok_or_else(|| "keyboard goal step overflow".to_owned())?;
        let next = candidate[coordinate_axis]
            .raw()
            .checked_add(delta)
            .ok_or_else(|| "keyboard goal coordinate overflow".to_owned())?;
        candidate[coordinate_axis] = FP::from_raw(next);
    }
    for coordinate in candidate {
        validate_coordinate(coordinate)?;
    }

    let start = graph.project(position, max_vertical).map_err(|error| {
        format!("current keyboard goal origin is not on the walkable mesh: {error}")
    })?;
    let projected_goal = graph
        .project(candidate, max_vertical)
        .map_err(|error| format!("keyboard goal step is not on the walkable mesh: {error}"))?;
    for coordinate in projected_goal.position {
        validate_coordinate(coordinate)?;
    }
    graph
        .find_path(
            start.position,
            projected_goal.position,
            max_vertical,
            SearchBudget {
                max_expansions: MAX_PATH_EXPANSIONS,
            },
        )
        .map_err(|error| {
            format!("keyboard goal is not connected by a bounded walkable path: {error}")
        })?;
    Ok(projected_goal.position)
}

fn validate_coordinate(coordinate: FP) -> Result<(), String> {
    if i128::from(coordinate.raw()).abs() > MAX_GOAL_RAW {
        return Err("keyboard goal must remain within ±256 units".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "keyboard/tests.rs"]
mod tests;
