//! Explicit, bounded inspection of every authored patrol leg. No Frame writes.
use orr_fp::FP;
use orr_navigation::{mesh::StaticMeshGraph, SearchBudget};
use orr_remote::mesh_navigation::patrol::MeshPatrol;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Leg {
    pub points: Vec<[FP; 3]>,
    pub length: FP,
    pub corridor_triangles: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Report {
    pub revision: [u8; 32],
    /// Projected route origin; authored coordinates remain in the form.
    pub start: [FP; 3],
    /// Projected waypoint markers, not authored fallback heights.
    pub waypoints: Vec<[FP; 3]>,
    pub legs: Vec<Leg>,
    pub total_length: FP,
}
fn legs(
    graph: &StaticMeshGraph,
    config: &MeshPatrol,
    start: [FP; 3],
    max_vertical: FP,
) -> Result<Vec<Leg>, String> {
    let n = config.count as usize;
    let mut endpoints = Vec::with_capacity(9);
    endpoints.push((start, config.points[0]));
    endpoints.extend(config.points[..n].windows(2).map(|p| (p[0], p[1])));
    if config.looped == 1 {
        endpoints.push((config.points[n - 1], config.points[0]));
    }
    let budget = SearchBudget {
        max_expansions: graph.triangles().len() as u32,
    };
    endpoints
        .into_iter()
        .enumerate()
        .map(|(index, (a, b))| {
            let label = if config.looped == 1 && index == n {
                format!("Patrol closing leg {}", index + 1)
            } else {
                format!("Patrol leg {}", index + 1)
            };
            let path = graph
                .find_path(a, b, max_vertical, budget)
                .map_err(|e| format!("{label}: {e}"))?;
            let length = graph
                .path_length(&path)
                .map_err(|e| format!("{label}: {e}"))?;
            Ok(Leg {
                points: path.points,
                length,
                corridor_triangles: path.corridor.len(),
            })
        })
        .collect()
}
pub(super) fn inspect(
    graph: &StaticMeshGraph,
    config: &MeshPatrol,
    start: [FP; 3],
    max_vertical: FP,
) -> Result<Report, String> {
    let admission =
        orr_remote::mesh_navigation::patrol::validate_config(config, graph, start, max_vertical);
    if let Err(error) = admission {
        // Core admission remains authoritative. Enrich route failures only,
        // without indexing malformed counts or returning any partial report.
        if error.starts_with("MeshPatrol segment:") && (1..=8).contains(&config.count) {
            if let Err(indexed) = legs(graph, config, start, max_vertical) {
                return Err(indexed);
            }
        }
        return Err(error);
    }
    if config.disabled() {
        return Err("Enable waypoint patrol before inspecting its route".into());
    }
    let legs = legs(graph, config, start, max_vertical)?;
    let total_raw = legs.iter().try_fold(0i64, |sum, leg| {
        sum.checked_add(leg.length.raw())
            .ok_or("Patrol total route length exceeds fixed-point range")
    })?;
    Ok(Report {
        revision: graph.revision(),
        start: graph
            .project(start, max_vertical)
            .map_err(|e| e.to_string())?
            .position,
        waypoints: config.points[..config.count as usize]
            .iter()
            .map(|p| {
                graph
                    .project(*p, max_vertical)
                    .map(|p| p.position)
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
        legs,
        total_length: FP::from_raw(total_raw),
    })
}
#[cfg(test)]
#[path = "patrol_routes_tests.rs"]
mod tests;
