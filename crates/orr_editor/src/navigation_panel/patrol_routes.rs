//! Read-only complete patrol inspection using the admitted terrain and graph.
use orr_fp::FP;
use orr_navigation::{SearchBudget, TerrainGraph};
use orr_remote::navigation_yard3d::patrol::{self, TerrainPatrol, MAX_ROUTE_TRIANGLES};
use orr_terrain::Terrain;
pub(crate) const MAX_LEG_POINTS: usize = MAX_ROUTE_TRIANGLES + 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Leg {
    pub points: Vec<[FP; 3]>,
    pub corridor_triangles: usize,
    /// Dijkstra centroid metric, not polyline length or travel time.
    pub graph_cost: FP,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Report {
    pub terrain_revision: [u8; 32],
    pub graph_revision: [u8; 32],
    pub start: [FP; 3],
    pub waypoints: Vec<[FP; 3]>,
    pub legs: Vec<Leg>,
    pub total_graph_cost: FP,
}
fn inspect_legs(
    t: &Terrain,
    g: &TerrainGraph,
    c: &TerrainPatrol,
    start: [FP; 2],
) -> Result<Vec<Leg>, String> {
    let n = c.count as usize;
    let mut endpoints = Vec::with_capacity(9);
    endpoints.push((start, c.points[0]));
    endpoints.extend(c.points[..n].windows(2).map(|p| (p[0], p[1])));
    if c.looped == 1 {
        endpoints.push((c.points[n - 1], c.points[0]));
    }
    endpoints
        .into_iter()
        .enumerate()
        .map(|(i, (a, b))| {
            let label = if c.looped == 1 && i == n {
                format!("Terrain patrol closing leg {}", i + 1)
            } else {
                format!("Terrain patrol leg {}", i + 1)
            };
            let path = g
                .find_path(
                    t,
                    a,
                    b,
                    SearchBudget {
                        max_expansions: MAX_ROUTE_TRIANGLES as u32,
                    },
                )
                .map_err(|e| format!("{label}: {e}"))?;
            if path.corridor().len() > MAX_ROUTE_TRIANGLES
                || path.waypoints().is_empty()
                || path.waypoints().len() > MAX_LEG_POINTS
            {
                return Err(format!("{label}: path exceeds runtime route bound"));
            }
            Ok(Leg {
                points: path.waypoints().to_vec(),
                corridor_triangles: path.corridor().len(),
                graph_cost: path.cost(),
            })
        })
        .collect()
}
pub(crate) fn inspect(
    t: &Terrain,
    g: &TerrainGraph,
    c: &TerrainPatrol,
    start: [FP; 2],
) -> Result<Report, String> {
    let admission = patrol::validate_config(c, t, g, start);
    if let Err(error) = admission {
        // Counts stay bounded even on failure; enrich a rejected route with its
        // leg index while preserving authoritative admission for other errors.
        if c.enabled == 1
            && (1..=8).contains(&c.count)
            && c.looped <= 1
            && c.graph_revision == g.revision()
        {
            inspect_legs(t, g, c, start)?;
        }
        return Err(error);
    }
    if c.disabled() {
        return Err("Enable terrain waypoint patrol before inspecting".into());
    }
    let legs = inspect_legs(t, g, c, start)?;
    let total = legs.iter().try_fold(0i64, |sum, l| {
        sum.checked_add(l.graph_cost.raw())
            .ok_or("Terrain patrol total graph cost exceeds fixed-point range")
    })?;
    Ok(Report {
        terrain_revision: t.revision(),
        graph_revision: g.revision(),
        start: g.project(t, start).map_err(|e| e.to_string())?.position,
        waypoints: c.points[..c.count as usize]
            .iter()
            .map(|p| {
                g.project(t, *p)
                    .map(|p| p.position)
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
        legs,
        total_graph_cost: FP::from_raw(total),
    })
}
#[cfg(test)]
#[path = "patrol_routes_tests.rs"]
mod tests;
