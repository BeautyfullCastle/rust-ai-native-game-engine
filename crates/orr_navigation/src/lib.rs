//! GPU-free, bounded point-agent navigation on the canonical terrain triangles.
//! This is a heightfield triangle graph, not a clearance-aware navmesh or funnel.
//! See the crate README for the numerical, ownership and movement contracts.
#![deny(clippy::float_arithmetic)]
#![deny(clippy::disallowed_types)]

use orr_fp::FP;
use orr_terrain::Terrain;
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::fmt;

/// Geometry limits bound every wide intermediate, including squared exact slopes.
pub const MIN_SPACING_RAW: i64 = 16;
pub const MAX_SPACING_RAW: i64 = 1 << 32;
pub const MAX_COORDINATE_RAW: i64 = 1 << 40;
pub const MAX_SLOPE_RAW: i64 = 1 << 24;
pub const MAX_TRIANGLES: u32 = 32_768;
pub const MAX_FILE_BYTES: usize = 2_200_000;
const MAGIC: &[u8; 8] = b"ORRNAV\x01\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationError {
    UnsupportedProfile,
    NumericLimit,
    StaleTerrain,
    StaleGraph,
    OutsideTerrain,
    Unwalkable,
    BudgetExceeded { expanded: u32 },
    Unreachable,
    InvalidSegment,
    InvalidDistance,
    MalformedAsset,
    ProfileMismatch,
}
impl fmt::Display for NavigationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for NavigationError {}

/// Only a point agent in open sky is supported. Radius, headroom and step must be zero.
/// max_slope is a nonnegative rise/run ratio, not an angle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentProfile {
    pub max_slope: FP,
    pub radius: FP,
    pub headroom: FP,
    pub max_step: FP,
}
impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            max_slope: FP::ONE,
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        }
    }
}
impl AgentProfile {
    fn validate(self) -> Result<(), NavigationError> {
        if self.radius != FP::ZERO || self.headroom != FP::ZERO || self.max_step != FP::ZERO {
            return Err(NavigationError::UnsupportedProfile);
        }
        if !(0..=MAX_SLOPE_RAW).contains(&self.max_slope.raw()) {
            return Err(NavigationError::NumericLimit);
        }
        Ok(())
    }
}
/// Stable key: row-major terrain cell index * 2 + canonical triangle half (0 or 1).
/// Holes and rejected slopes leave gaps; keys never depend on graph insertion order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TriangleKey(pub u32);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Portal {
    pub neighbor: TriangleKey,
    /// Sorted canonical terrain vertex IDs, an actual full shared edge.
    pub vertices: [u32; 2],
    /// XZ midpoint floors to the raw FP lattice; Y uses Terrain::sample.
    pub midpoint: [FP; 3],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Triangle {
    pub key: TriangleKey,
    pub vertices: [u32; 3],
    /// Sorted by neighbor key, reciprocal. Corner-only contact creates no portal.
    pub portals: Vec<Portal>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Projection {
    pub triangle: TriangleKey,
    pub position: [FP; 3],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchBudget {
    pub max_expansions: u32,
}
impl Default for SearchBudget {
    fn default() -> Self {
        Self {
            max_expansions: MAX_TRIANGLES,
        }
    }
}

/// Immutable dependency-bound graph. All terrain vertices preserve their original IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainGraph {
    asset_id: String,
    terrain_revision: [u8; 32],
    profile: AgentProfile,
    vertices: Vec<[FP; 3]>,
    triangles: Vec<Triangle>,
    /// Dense key lookup is bounded by terrain dimensions, not serialized.
    lookup: Vec<Option<u32>>,
    revision: [u8; 32],
}
impl TerrainGraph {
    pub fn build(terrain: &Terrain, profile: AgentProfile) -> Result<Self, NavigationError> {
        profile.validate()?;
        if !(MIN_SPACING_RAW..=MAX_SPACING_RAW).contains(&terrain.spacing().raw()) {
            return Err(NavigationError::NumericLimit);
        }
        let vertices: Vec<_> = (0..terrain.width() * terrain.depth())
            .map(|i| {
                terrain
                    .vertex_position(i)
                    .expect("validated terrain vertex")
            })
            .collect();
        if vertices
            .iter()
            .flatten()
            .any(|v| i128::from(v.raw()).abs() > i128::from(MAX_COORDINATE_RAW))
        {
            return Err(NavigationError::NumericLimit);
        }
        let mut triangles = Vec::new();
        let mut lookup = vec![None; terrain.holes().len() * 2];
        let spacing = i128::from(terrain.spacing().raw());
        let limit = i128::from(profile.max_slope.raw());
        for z in 0..terrain.depth() - 1 {
            for x in 0..terrain.width() - 1 {
                let cell = z * (terrain.width() - 1) + x;
                if terrain.holes()[cell as usize] {
                    continue;
                }
                let a = z * terrain.width() + x;
                let b = a + 1;
                let c = a + terrain.width();
                let d = c + 1;
                let h = |v: u32| i128::from(vertices[v as usize][1].raw());
                for (half, indices, hx, hz) in [
                    (0, [a, c, d], h(d) - h(c), h(c) - h(a)),
                    (1, [a, d, b], h(b) - h(a), h(d) - h(b)),
                ] {
                    // Compare exact plane coefficients. Never use Terrain::surface's
                    // deliberately quantized diagnostic slope for admission.
                    if (hx * hx + hz * hz) * (1i128 << 32) > spacing * spacing * limit * limit {
                        continue;
                    }
                    let key = TriangleKey(cell * 2 + half);
                    lookup[key.0 as usize] = Some(triangles.len() as u32);
                    triangles.push(Triangle {
                        key,
                        vertices: indices,
                        portals: Vec::new(),
                    });
                }
            }
        }
        let mut edges: BTreeMap<[u32; 2], Vec<u32>> = BTreeMap::new();
        for (i, tri) in triangles.iter().enumerate() {
            for mut edge in [
                [tri.vertices[0], tri.vertices[1]],
                [tri.vertices[1], tri.vertices[2]],
                [tri.vertices[2], tri.vertices[0]],
            ] {
                edge.sort();
                edges.entry(edge).or_default().push(i as u32);
            }
        }
        let mut graph = Self {
            asset_id: terrain.asset_id().to_owned(),
            terrain_revision: terrain.revision(),
            profile,
            vertices,
            triangles,
            lookup,
            revision: [0; 32],
        };
        for (edge, incident) in edges {
            if let [ia, ib] = incident.as_slice() {
                let va = graph.vertices[edge[0] as usize];
                let vb = graph.vertices[edge[1] as usize];
                let midpoint_xz = [midpoint(va[0], vb[0]), midpoint(va[2], vb[2])];
                let owner = graph.project_unchecked(terrain, midpoint_xz)?;
                let ka = graph.triangles[*ia as usize].key;
                let kb = graph.triangles[*ib as usize].key;
                if owner.triangle != ka && owner.triangle != kb {
                    return Err(NavigationError::InvalidSegment);
                }
                graph.triangles[*ia as usize].portals.push(Portal {
                    neighbor: kb,
                    vertices: edge,
                    midpoint: owner.position,
                });
                graph.triangles[*ib as usize].portals.push(Portal {
                    neighbor: ka,
                    vertices: edge,
                    midpoint: owner.position,
                });
            }
        }
        for tri in &mut graph.triangles {
            tri.portals.sort_by_key(|p| p.neighbor);
        }
        graph.revision = Sha256::digest(graph.cook()).into();
        Ok(graph)
    }
    pub fn terrain_asset_id(&self) -> &str {
        &self.asset_id
    }
    pub fn terrain_revision(&self) -> [u8; 32] {
        self.terrain_revision
    }
    pub fn profile(&self) -> AgentProfile {
        self.profile
    }
    pub fn revision(&self) -> [u8; 32] {
        self.revision
    }
    pub fn vertices(&self) -> &[[FP; 3]] {
        &self.vertices
    }
    pub fn triangles(&self) -> &[Triangle] {
        &self.triangles
    }
    pub fn triangle(&self, key: TriangleKey) -> Option<&Triangle> {
        self.lookup
            .get(key.0 as usize)?
            .map(|i| &self.triangles[i as usize])
    }
    pub fn validate_terrain(&self, terrain: &Terrain) -> Result<(), NavigationError> {
        if terrain.asset_id() != self.asset_id || terrain.revision() != self.terrain_revision {
            return Err(NavigationError::StaleTerrain);
        }
        Ok(())
    }
    /// Exact canonical-owner projection at supplied XZ. No nearest-point snap.
    pub fn project(&self, terrain: &Terrain, xz: [FP; 2]) -> Result<Projection, NavigationError> {
        self.validate_terrain(terrain)?;
        self.project_unchecked(terrain, xz)
    }
    fn project_unchecked(
        &self,
        terrain: &Terrain,
        xz: [FP; 2],
    ) -> Result<Projection, NavigationError> {
        let s = i128::from(terrain.spacing().raw());
        let mut cell = [0u32; 2];
        let mut offset = [0i128; 2];
        for axis in 0..2 {
            let side = [terrain.width(), terrain.depth()][axis];
            let delta = i128::from(xz[axis].raw()) - i128::from(terrain.origin()[axis].raw());
            if delta < 0 || delta > i128::from(side - 1) * s {
                return Err(NavigationError::OutsideTerrain);
            }
            cell[axis] = (delta / s).min(i128::from(side - 2)) as u32;
            offset[axis] = delta - i128::from(cell[axis]) * s;
        }
        let cell_index = cell[1] * (terrain.width() - 1) + cell[0];
        let key = TriangleKey(cell_index * 2 + u32::from(offset[0] > offset[1]));
        if self.triangle(key).is_none() {
            return Err(NavigationError::Unwalkable);
        }
        let height = terrain
            .sample(xz[0], xz[1])
            .ok_or(NavigationError::Unwalkable)?;
        Ok(Projection {
            triangle: key,
            position: [xz[0], height, xz[1]],
        })
    }
    /// Stable-tie Dijkstra. Returns a complete path or an explicit failure, never a partial path.
    pub fn find_path(
        &self,
        terrain: &Terrain,
        start: [FP; 2],
        goal: [FP; 2],
        budget: SearchBudget,
    ) -> Result<NavigationPath, NavigationError> {
        self.validate_terrain(terrain)?;
        let start = self.project_unchecked(terrain, start)?;
        let goal = self.project_unchecked(terrain, goal)?;
        let mut distance = vec![u64::MAX; self.lookup.len()];
        let mut previous = vec![None; self.lookup.len()];
        let mut queue = BinaryHeap::new();
        distance[start.triangle.0 as usize] = 0;
        queue.push(Reverse((0u64, start.triangle)));
        let mut expanded = 0;
        let mut found = false;
        while let Some(Reverse((cost, key))) = queue.pop() {
            if distance[key.0 as usize] != cost {
                continue;
            }
            if expanded == budget.max_expansions {
                return Err(NavigationError::BudgetExceeded { expanded });
            }
            expanded += 1;
            if key == goal.triangle {
                found = true;
                break;
            }
            let tri = self
                .triangle(key)
                .expect("queue only contains admitted triangles");
            let center = self.center(tri);
            for portal in &tri.portals {
                let neighbor = self
                    .triangle(portal.neighbor)
                    .expect("reciprocal graph portal");
                let next = cost + distance_ceil(center, self.center(neighbor));
                let slot = portal.neighbor.0 as usize;
                if next < distance[slot] {
                    distance[slot] = next;
                    previous[slot] = Some(key);
                    queue.push(Reverse((next, portal.neighbor)));
                }
            }
        }
        if !found {
            return Err(NavigationError::Unreachable);
        }
        let mut corridor = vec![goal.triangle];
        while *corridor.last().expect("goal exists") != start.triangle {
            corridor.push(
                previous[corridor.last().expect("goal exists").0 as usize]
                    .expect("settled path predecessor"),
            );
        }
        corridor.reverse();
        let mut portals = Vec::with_capacity(corridor.len().saturating_sub(1));
        for pair in corridor.windows(2) {
            portals.push(
                self.triangle(pair[0])
                    .expect("path triangle")
                    .portals
                    .iter()
                    .find(|p| p.neighbor == pair[1])
                    .expect("path edge")
                    .clone(),
            );
        }
        let mut waypoints = Vec::with_capacity(corridor.len() + 1);
        waypoints.push(start.position);
        waypoints.extend(portals.iter().map(|p| p.midpoint));
        waypoints.push(goal.position);
        for (segment, key) in waypoints.windows(2).zip(corridor.iter().copied()) {
            self.validate_segment_unchecked(terrain, key, segment[0], segment[1])?;
        }
        Ok(NavigationPath {
            graph_revision: self.revision,
            terrain_revision: self.terrain_revision,
            corridor,
            portals,
            waypoints,
            cost: FP::from_raw(distance[goal.triangle.0 as usize] as i64),
        })
    }
    fn center(&self, tri: &Triangle) -> [FP; 3] {
        std::array::from_fn(|axis| {
            FP::from_raw(
                tri.vertices
                    .iter()
                    .map(|&v| i128::from(self.vertices[v as usize][axis].raw()))
                    .sum::<i128>()
                    .div_euclid(3) as i64,
            )
        })
    }
    /// Checks an entire raw-quantized segment, not just its endpoints. In one closed
    /// convex triangle, owner changes are confined to the triangle's boundary.
    /// Endpoints and any boundary containing the whole segment must also be walkable.
    pub fn validate_segment(
        &self,
        terrain: &Terrain,
        key: TriangleKey,
        a: [FP; 3],
        b: [FP; 3],
    ) -> Result<(), NavigationError> {
        self.validate_terrain(terrain)?;
        self.validate_segment_unchecked(terrain, key, a, b)
    }
    fn validate_segment_unchecked(
        &self,
        terrain: &Terrain,
        key: TriangleKey,
        a: [FP; 3],
        b: [FP; 3],
    ) -> Result<(), NavigationError> {
        let tri = self.triangle(key).ok_or(NavigationError::InvalidSegment)?;
        for point in [a, b] {
            let p = self.project_unchecked(terrain, [point[0], point[2]])?;
            if p.position != point || !contains(&self.vertices, tri, point) {
                return Err(NavigationError::InvalidSegment);
            }
        }
        for edge in [
            [tri.vertices[0], tri.vertices[1]],
            [tri.vertices[1], tri.vertices[2]],
            [tri.vertices[2], tri.vertices[0]],
        ] {
            let p = self.vertices[edge[0] as usize];
            let q = self.vertices[edge[1] as usize];
            if cross_xz(p, q, a) == 0 && cross_xz(p, q, b) == 0 {
                self.project_unchecked(terrain, [midpoint(a[0], b[0]), midpoint(a[2], b[2])])?;
            }
        }
        Ok(())
    }
    /// Canonical LE format: magic, terrain digest, ID, profile, all canonical
    /// vertices and sorted accepted triangles with reciprocal sorted edge records.
    pub fn cook(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(MAGIC);
        out.extend(self.terrain_revision);
        out.extend((self.asset_id.len() as u16).to_le_bytes());
        out.extend(self.asset_id.as_bytes());
        for fp in [
            self.profile.max_slope,
            self.profile.radius,
            self.profile.headroom,
            self.profile.max_step,
        ] {
            out.extend(fp.raw().to_le_bytes());
        }
        out.extend((self.vertices.len() as u32).to_le_bytes());
        for point in &self.vertices {
            for fp in point {
                out.extend(fp.raw().to_le_bytes());
            }
        }
        out.extend((self.triangles.len() as u32).to_le_bytes());
        for tri in &self.triangles {
            out.extend(tri.key.0.to_le_bytes());
            for v in tri.vertices {
                out.extend(v.to_le_bytes());
            }
            out.push(tri.portals.len() as u8);
            for portal in &tri.portals {
                out.extend(portal.neighbor.0.to_le_bytes());
                for v in portal.vertices {
                    out.extend(v.to_le_bytes());
                }
            }
        }
        out
    }
    /// Reconstructs against the supplied dependency and compares the entire canonical
    /// record. Untrusted counts cannot allocate memory; alternate encodings fail.
    pub fn load(
        bytes: &[u8],
        terrain: &Terrain,
        profile: AgentProfile,
    ) -> Result<Self, NavigationError> {
        if bytes.len() > MAX_FILE_BYTES || bytes.len() < 42 || &bytes[..8] != MAGIC {
            return Err(NavigationError::MalformedAsset);
        }
        if bytes[8..40] != terrain.revision() {
            return Err(NavigationError::StaleTerrain);
        }
        let id_len = usize::from(u16::from_le_bytes([bytes[40], bytes[41]]));
        let header = 42usize
            .checked_add(id_len)
            .ok_or(NavigationError::MalformedAsset)?;
        if id_len > orr_terrain::MAX_ASSET_ID_BYTES || bytes.len() < header + 32 {
            return Err(NavigationError::MalformedAsset);
        }
        if bytes[42..header] != *terrain.asset_id().as_bytes() {
            return Err(NavigationError::StaleTerrain);
        }
        let raw = [
            profile.max_slope,
            profile.radius,
            profile.headroom,
            profile.max_step,
        ];
        for (chunk, fp) in bytes[header..header + 32].chunks_exact(8).zip(raw) {
            if chunk != fp.raw().to_le_bytes() {
                return Err(NavigationError::ProfileMismatch);
            }
        }
        let graph = Self::build(terrain, profile)?;
        if bytes != graph.cook() {
            return Err(NavigationError::MalformedAsset);
        }
        Ok(graph)
    }
}

/// Private immutable payload prevents external forging of dependency bindings or corridors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationPath {
    graph_revision: [u8; 32],
    terrain_revision: [u8; 32],
    corridor: Vec<TriangleKey>,
    portals: Vec<Portal>,
    waypoints: Vec<[FP; 3]>,
    cost: FP,
}
impl NavigationPath {
    pub fn corridor(&self) -> &[TriangleKey] {
        &self.corridor
    }
    pub fn portals(&self) -> &[Portal] {
        &self.portals
    }
    pub fn waypoints(&self) -> &[[FP; 3]] {
        &self.waypoints
    }
    /// Dijkstra graph metric: ceil raw 3D distance between quantized triangle centroids.
    /// This is not the length of the unsmoothed midpoint waypoint polyline.
    pub fn cost(&self) -> FP {
        self.cost
    }
    pub fn graph_revision(&self) -> [u8; 32] {
        self.graph_revision
    }
    pub fn terrain_revision(&self) -> [u8; 32] {
        self.terrain_revision
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationStatus {
    Stopped,
    Moving,
    Arrived,
}

/// Explicit, cloneable deterministic state. Replans and advances publish atomically.
/// This heap-owning helper is not an ECS Pod component; owners must snapshot it explicitly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Navigator {
    position: [FP; 3],
    graph_revision: [u8; 32],
    terrain_revision: [u8; 32],
    path: Option<NavigationPath>,
    next_waypoint: u32,
    status: NavigationStatus,
}
impl Navigator {
    pub fn new(
        graph: &TerrainGraph,
        terrain: &Terrain,
        xz: [FP; 2],
    ) -> Result<Self, NavigationError> {
        let projection = graph.project(terrain, xz)?;
        Ok(Self {
            position: projection.position,
            graph_revision: graph.revision,
            terrain_revision: graph.terrain_revision,
            path: None,
            next_waypoint: 0,
            status: NavigationStatus::Stopped,
        })
    }
    pub fn position(&self) -> [FP; 3] {
        self.position
    }
    pub fn status(&self) -> NavigationStatus {
        self.status
    }
    pub fn path(&self) -> Option<&NavigationPath> {
        self.path.as_ref()
    }
    pub fn next_waypoint(&self) -> u32 {
        self.next_waypoint
    }
    /// Domain-tagged canonical checksum of all replay-relevant state. Counts and
    /// indices use u32; no pointer, allocation capacity or machine-sized integer
    /// participates. This is a checksum, not a persisted state loader contract.
    pub fn checksum(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"ORRNSTATE\x01");
        hash.update(self.graph_revision);
        hash.update(self.terrain_revision);
        for value in self.position {
            hash.update(value.raw().to_le_bytes());
        }
        hash.update([match self.status {
            NavigationStatus::Stopped => 0,
            NavigationStatus::Moving => 1,
            NavigationStatus::Arrived => 2,
        }]);
        hash.update(self.next_waypoint.to_le_bytes());
        hash.update([u8::from(self.path.is_some())]);
        if let Some(path) = &self.path {
            hash.update(path.graph_revision);
            hash.update(path.terrain_revision);
            hash.update(path.cost.raw().to_le_bytes());
            hash.update((path.corridor.len() as u32).to_le_bytes());
            for key in &path.corridor {
                hash.update(key.0.to_le_bytes());
            }
            hash.update((path.portals.len() as u32).to_le_bytes());
            for portal in &path.portals {
                hash.update(portal.neighbor.0.to_le_bytes());
                for vertex in portal.vertices {
                    hash.update(vertex.to_le_bytes());
                }
                for value in portal.midpoint {
                    hash.update(value.raw().to_le_bytes());
                }
            }
            hash.update((path.waypoints.len() as u32).to_le_bytes());
            for point in &path.waypoints {
                for value in point {
                    hash.update(value.raw().to_le_bytes());
                }
            }
        }
        hash.finalize().into()
    }
    pub fn stop(&mut self) {
        self.path = None;
        self.next_waypoint = 0;
        self.status = NavigationStatus::Stopped;
    }
    /// Explicitly accepts a fresh graph/terrain dependency and resamples current XZ.
    /// Failure leaves position, old bindings, old path and status completely unchanged.
    pub fn replan(
        &mut self,
        graph: &TerrainGraph,
        terrain: &Terrain,
        goal: [FP; 2],
        budget: SearchBudget,
    ) -> Result<(), NavigationError> {
        let path = graph.find_path(terrain, [self.position[0], self.position[2]], goal, budget)?;
        let position = path.waypoints[0];
        let status = if path.waypoints.iter().all(|p| *p == position) {
            NavigationStatus::Arrived
        } else {
            NavigationStatus::Moving
        };
        let next_waypoint = if status == NavigationStatus::Arrived {
            path.waypoints.len() as u32
        } else {
            1
        };
        *self = Self {
            position,
            graph_revision: graph.revision,
            terrain_revision: graph.terrain_revision,
            path: Some(path),
            next_waypoint,
            status,
        };
        Ok(())
    }
    /// Moves at most an explicit nonnegative distance, never overshooting a waypoint.
    /// Four floor/ceil lattice candidates are checked against the whole corridor segment
    /// and the 3D ceil distance budget. Tiny budgets may make zero progress.
    /// Stale dependencies fail closed even when stopped or arrived.
    pub fn advance(
        &mut self,
        graph: &TerrainGraph,
        terrain: &Terrain,
        distance: FP,
    ) -> Result<(), NavigationError> {
        graph.validate_terrain(terrain)?;
        if self.terrain_revision != graph.terrain_revision {
            return Err(NavigationError::StaleTerrain);
        }
        if self.graph_revision != graph.revision {
            return Err(NavigationError::StaleGraph);
        }
        if distance.raw() < 0 {
            return Err(NavigationError::InvalidDistance);
        }
        if let Some(path) = &self.path {
            if path.graph_revision != graph.revision
                || path.terrain_revision != graph.terrain_revision
            {
                return Err(NavigationError::StaleGraph);
            }
        }
        if self.status != NavigationStatus::Moving || distance == FP::ZERO {
            return Ok(());
        }
        let mut candidate = self.clone();
        let mut remaining = distance.raw() as u64;
        let path = candidate
            .path
            .as_ref()
            .ok_or(NavigationError::InvalidSegment)?;
        while (candidate.next_waypoint as usize) < path.waypoints.len() {
            let index = candidate.next_waypoint as usize;
            let target = path.waypoints[index];
            let key = path.corridor[index - 1];
            graph.validate_segment_unchecked(terrain, key, candidate.position, target)?;
            let needed = distance_ceil(candidate.position, target);
            if needed <= remaining {
                candidate.position = target;
                remaining -= needed;
                candidate.next_waypoint += 1;
                continue;
            }
            if remaining == 0 {
                break;
            }
            let next = quantized_step(
                graph,
                terrain,
                key,
                candidate.position,
                target,
                remaining,
                needed,
            )?;
            if next == candidate.position {
                break;
            }
            let used = distance_ceil(candidate.position, next);
            candidate.position = next;
            remaining -= used;
            // Continue toward the same waypoint with any residual quantization budget.
            if remaining == 0 {
                break;
            }
        }
        if candidate.next_waypoint as usize >= path.waypoints.len() {
            candidate.status = NavigationStatus::Arrived;
        }
        *self = candidate;
        Ok(())
    }
}
fn quantized_step(
    graph: &TerrainGraph,
    terrain: &Terrain,
    key: TriangleKey,
    current: [FP; 3],
    target: [FP; 3],
    budget: u64,
    length: u64,
) -> Result<[FP; 3], NavigationError> {
    let delta = [
        i128::from(target[0].raw()) - i128::from(current[0].raw()),
        i128::from(target[2].raw()) - i128::from(current[2].raw()),
    ];
    let denom = i128::from(length);
    let exact = std::array::from_fn::<_, 2, _>(|axis| {
        i128::from(current[axis * 2].raw()) * denom + delta[axis] * i128::from(budget)
    });
    let low = exact.map(|v| v.div_euclid(denom));
    let high = std::array::from_fn::<_, 2, _>(|axis| {
        low[axis] + i128::from(exact[axis].rem_euclid(denom) != 0)
    });
    let full_dot = delta[0] * delta[0] + delta[1] * delta[1];
    let mut best = current;
    let mut best_rank = None;
    for x in [low[0], high[0]] {
        for z in [low[1], high[1]] {
            let x = FP::from_raw(x as i64);
            let z = FP::from_raw(z as i64);
            let Some(y) = terrain.sample(x, z) else {
                continue;
            };
            let point = [x, y, z];
            let d = [
                i128::from(x.raw()) - i128::from(current[0].raw()),
                i128::from(z.raw()) - i128::from(current[2].raw()),
            ];
            let dot = d[0] * delta[0] + d[1] * delta[1];
            if dot <= 0 || dot > full_dot || distance_ceil(current, point) > budget {
                continue;
            }
            if distance_ceil(point, target) >= length {
                continue;
            }
            if graph
                .validate_segment_unchecked(terrain, key, current, point)
                .is_err()
                || graph
                    .validate_segment_unchecked(terrain, key, point, target)
                    .is_err()
            {
                continue;
            }
            let off_line = (d[0] * delta[1] - d[1] * delta[0]).abs();
            let rank = (dot, -off_line, -i128::from(x.raw()), -i128::from(z.raw()));
            if best_rank.is_none_or(|r| rank > r) {
                best_rank = Some(rank);
                best = point;
            }
        }
    }
    Ok(best)
}
fn midpoint(a: FP, b: FP) -> FP {
    FP::from_raw((i128::from(a.raw()) + i128::from(b.raw())).div_euclid(2) as i64)
}
fn cross_xz(a: [FP; 3], b: [FP; 3], p: [FP; 3]) -> i128 {
    (i128::from(b[0].raw()) - i128::from(a[0].raw()))
        * (i128::from(p[2].raw()) - i128::from(a[2].raw()))
        - (i128::from(b[2].raw()) - i128::from(a[2].raw()))
            * (i128::from(p[0].raw()) - i128::from(a[0].raw()))
}
fn contains(vertices: &[[FP; 3]], tri: &Triangle, p: [FP; 3]) -> bool {
    let [a, b, c] = tri.vertices.map(|i| vertices[i as usize]);
    [cross_xz(a, b, p), cross_xz(b, c, p), cross_xz(c, a, p)]
        .iter()
        .all(|v| *v <= 0)
}
fn distance_ceil(a: [FP; 3], b: [FP; 3]) -> u64 {
    let squared: u128 = (0..3)
        .map(|axis| {
            let d = i128::from(a[axis].raw()) - i128::from(b[axis].raw());
            (d * d) as u128
        })
        .sum();
    let floor = isqrt(squared);
    (floor + u128::from(floor * floor != squared)) as u64
}
fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = 1u128 << (128 - n.leading_zeros()).div_ceil(2);
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}
