use crate::{
    AgentProfile, NavigationError, SearchBudget, MAX_COORDINATE_RAW, MAX_FILE_BYTES, MAX_TRIANGLES,
};
use orr_fp::FP;
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

const MESH_MAGIC: &[u8; 8] = b"ORRMESH\x01";
const MAX_VERTICES: usize = 65_536;
const MAX_FACES: usize = 32_768;
const MAX_EDGE_DELTA_RAW: i64 = 1 << 22;
const MAX_SLOPE_RAW: i64 = 4 * FP::SCALE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshProjection {
    pub triangle: u32,
    pub position: [FP; 3],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshPortal {
    pub neighbor: u32,
    pub vertices: [u32; 2],
    pub position: [FP; 3],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshTriangle {
    pub key: u32,
    pub vertices: [u32; 3],
    pub portals: Vec<MeshPortal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshPath {
    pub revision: [u8; 32],
    pub corridor: Vec<u32>,
    pub points: Vec<[FP; 3]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticMeshGraph {
    profile: AgentProfile,
    vertices: Vec<[FP; 3]>,
    faces: Vec<[u32; 3]>,
    triangles: Vec<MeshTriangle>,
    revision: [u8; 32],
}

impl StaticMeshGraph {
    pub fn bake(
        vertices: &[[FP; 3]],
        faces: &[[u32; 3]],
        profile: AgentProfile,
    ) -> Result<Self, NavigationError> {
        validate_profile(profile)?;
        if vertices.len() > MAX_VERTICES || faces.len() > MAX_FACES {
            return Err(NavigationError::NumericLimit);
        }
        if vertices
            .iter()
            .flatten()
            .any(|coordinate| i128::from(coordinate.raw()).abs() > i128::from(MAX_COORDINATE_RAW))
        {
            return Err(NavigationError::NumericLimit);
        }

        let mut canonical_vertices = vertices.to_vec();
        canonical_vertices.sort_by_key(|point| point.map(FP::raw));
        canonical_vertices.dedup();
        let vertex_lookup: BTreeMap<[i64; 3], u32> = canonical_vertices
            .iter()
            .enumerate()
            .map(|(index, point)| (point.map(FP::raw), index as u32))
            .collect();
        let mut remapped_faces = Vec::with_capacity(faces.len());
        for face in faces {
            let mut remapped = [0u32; 3];
            for (slot, original_index) in face.iter().enumerate() {
                let point = vertices
                    .get(*original_index as usize)
                    .ok_or(NavigationError::MalformedAsset)?;
                remapped[slot] = *vertex_lookup
                    .get(&point.map(FP::raw))
                    .ok_or(NavigationError::MalformedAsset)?;
            }
            if remapped[0] == remapped[1]
                || remapped[1] == remapped[2]
                || remapped[2] == remapped[0]
            {
                return Err(NavigationError::InvalidSegment);
            }
            let rotated = [
                remapped,
                [remapped[1], remapped[2], remapped[0]],
                [remapped[2], remapped[0], remapped[1]],
            ];
            let canonical = *rotated.iter().min().expect("three rotations");
            let triangle_points = canonical.map(|index| canonical_vertices[index as usize]);
            if cross_xz(triangle_points[0], triangle_points[1], triangle_points[2]) >= 0 {
                return Err(NavigationError::InvalidSegment);
            }
            validate_edge_deltas(triangle_points)?;
            remapped_faces.push(canonical);
        }
        remapped_faces.sort();
        if remapped_faces.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(NavigationError::InvalidSegment);
        }

        let mut edge_incidents: BTreeMap<[u32; 2], Vec<u32>> = BTreeMap::new();
        for (face_index, face) in remapped_faces.iter().enumerate() {
            for mut edge in [[face[0], face[1]], [face[1], face[2]], [face[2], face[0]]] {
                edge.sort();
                let incident = edge_incidents.entry(edge).or_default();
                incident.push(face_index as u32);
                if incident.len() > 2 {
                    return Err(NavigationError::InvalidSegment);
                }
            }
        }

        let mut triangles = Vec::new();
        let mut face_to_triangle = vec![None; remapped_faces.len()];
        for (face_index, face) in remapped_faces.iter().enumerate() {
            let points = face.map(|index| canonical_vertices[index as usize]);
            if slope_admitted(points, profile.max_slope.raw())? {
                let key = face_index as u32;
                face_to_triangle[face_index] = Some(key);
                triangles.push(MeshTriangle {
                    key,
                    vertices: *face,
                    portals: Vec::new(),
                });
            }
        }
        let mut triangle_slots = vec![None; remapped_faces.len()];
        for (slot, triangle) in triangles.iter().enumerate() {
            triangle_slots[triangle.key as usize] = Some(slot as u32);
        }
        for (edge, incident) in edge_incidents {
            if let [first_face, second_face] = incident.as_slice() {
                let (Some(first_key), Some(second_key)) = (
                    face_to_triangle[*first_face as usize],
                    face_to_triangle[*second_face as usize],
                ) else {
                    continue;
                };
                let position = edge_lattice_midpoint(
                    canonical_vertices[edge[0] as usize],
                    canonical_vertices[edge[1] as usize],
                )?;
                let first_slot = triangle_slots[first_key as usize]
                    .ok_or(NavigationError::InvalidSegment)?
                    as usize;
                let second_slot = triangle_slots[second_key as usize]
                    .ok_or(NavigationError::InvalidSegment)?
                    as usize;
                triangles[first_slot].portals.push(MeshPortal {
                    neighbor: second_key,
                    vertices: edge,
                    position,
                });
                triangles[second_slot].portals.push(MeshPortal {
                    neighbor: first_key,
                    vertices: edge,
                    position,
                });
            }
        }
        for triangle in &mut triangles {
            triangle.portals.sort_by_key(|portal| portal.neighbor);
        }
        if triangles.len() > MAX_TRIANGLES as usize {
            return Err(NavigationError::NumericLimit);
        }
        let mut graph = Self {
            profile,
            vertices: canonical_vertices,
            faces: remapped_faces,
            triangles,
            revision: [0; 32],
        };
        graph.revision = graph.compute_revision();
        if graph.to_bytes().len() > MAX_FILE_BYTES {
            return Err(NavigationError::NumericLimit);
        }
        Ok(graph)
    }

    pub fn revision(&self) -> [u8; 32] {
        self.revision
    }
    pub fn profile(&self) -> AgentProfile {
        self.profile
    }
    pub fn vertices(&self) -> &[[FP; 3]] {
        &self.vertices
    }
    pub fn triangles(&self) -> &[MeshTriangle] {
        &self.triangles
    }

    pub fn project(
        &self,
        position: [FP; 3],
        max_vertical: FP,
    ) -> Result<MeshProjection, NavigationError> {
        if max_vertical.raw() < 0
            || position.iter().any(|coordinate| {
                i128::from(coordinate.raw()).abs() > i128::from(MAX_COORDINATE_RAW)
            })
        {
            return Err(NavigationError::NumericLimit);
        }
        let mut best: Option<(i128, u32, [FP; 3])> = None;
        for triangle in &self.triangles {
            let Some(projected) =
                project_to_triangle(&self.vertices, triangle, position[0], position[2])?
            else {
                continue;
            };
            let delta = (i128::from(projected[1].raw()) - i128::from(position[1].raw())).abs();
            if delta > i128::from(max_vertical.raw()) {
                continue;
            }
            let candidate = (delta, triangle.key, projected);
            if best.as_ref().is_none_or(|current| {
                candidate.0 < current.0 || (candidate.0 == current.0 && candidate.1 < current.1)
            }) {
                best = Some(candidate);
            }
        }
        best.map(|(_, triangle, position)| MeshProjection { triangle, position })
            .ok_or(NavigationError::OutsideTerrain)
    }

    pub fn find_path(
        &self,
        start: [FP; 3],
        goal: [FP; 3],
        max_vertical: FP,
        budget: SearchBudget,
    ) -> Result<MeshPath, NavigationError> {
        let start = self.project(start, max_vertical)?;
        let goal = self.project(goal, max_vertical)?;
        if start.triangle == goal.triangle {
            return Ok(MeshPath {
                revision: self.revision,
                corridor: vec![start.triangle],
                points: vec![start.position, goal.position],
            });
        }
        let mut distances = vec![u64::MAX; self.faces.len()];
        let mut previous = vec![None; self.faces.len()];
        let mut queue = BinaryHeap::new();
        distances[start.triangle as usize] = 0;
        queue.push(Reverse((0u64, start.triangle)));
        let mut expanded = 0u32;
        while let Some(Reverse((cost, key))) = queue.pop() {
            if distances[key as usize] != cost {
                continue;
            }
            if expanded >= budget.max_expansions {
                return Err(NavigationError::BudgetExceeded { expanded });
            }
            expanded += 1;
            if key == goal.triangle {
                break;
            }
            let triangle = self.triangle(key).ok_or(NavigationError::InvalidSegment)?;
            let center = self.center(triangle);
            for portal in &triangle.portals {
                let neighbor = self
                    .triangle(portal.neighbor)
                    .ok_or(NavigationError::InvalidSegment)?;
                let edge_cost = graph_edge_cost(center, self.center(neighbor));
                let next_cost = cost
                    .checked_add(edge_cost)
                    .ok_or(NavigationError::NumericLimit)?
                    .max(1);
                let slot = portal.neighbor as usize;
                if next_cost < distances[slot] {
                    distances[slot] = next_cost;
                    previous[slot] = Some(key);
                    queue.push(Reverse((next_cost, portal.neighbor)));
                }
            }
        }
        if distances[goal.triangle as usize] == u64::MAX {
            return Err(NavigationError::Unreachable);
        }
        let mut corridor = vec![goal.triangle];
        while *corridor.last().expect("goal key") != start.triangle {
            let predecessor = previous[*corridor.last().expect("goal key") as usize]
                .ok_or(NavigationError::Unreachable)?;
            corridor.push(predecessor);
        }
        corridor.reverse();
        let mut points = Vec::with_capacity(corridor.len() + 1);
        points.push(start.position);
        for pair in corridor.windows(2) {
            let portal = self
                .triangle(pair[0])
                .and_then(|triangle| {
                    triangle
                        .portals
                        .iter()
                        .find(|portal| portal.neighbor == pair[1])
                })
                .ok_or(NavigationError::InvalidSegment)?;
            points.push(portal.position);
        }
        points.push(goal.position);
        let path = MeshPath {
            revision: self.revision,
            corridor,
            points,
        };
        self.validate_path(&path)?;
        Ok(path)
    }

    pub fn validate_path(&self, path: &MeshPath) -> Result<(), NavigationError> {
        if path.revision != self.revision {
            return Err(NavigationError::StaleGraph);
        }
        if path.corridor.is_empty() || path.points.len() != path.corridor.len() + 1 {
            return Err(NavigationError::InvalidSegment);
        }
        for (index, key) in path.corridor.iter().enumerate() {
            let triangle = self.triangle(*key).ok_or(NavigationError::InvalidSegment)?;
            for point in [path.points[index], path.points[index + 1]] {
                if !point_on_triangle(&self.vertices, triangle, point)? {
                    return Err(NavigationError::InvalidSegment);
                }
            }
            if let Some(next_key) = path.corridor.get(index + 1) {
                let portal = triangle
                    .portals
                    .iter()
                    .find(|portal| portal.neighbor == *next_key)
                    .ok_or(NavigationError::InvalidSegment)?;
                if path.points[index + 1] != portal.position {
                    return Err(NavigationError::InvalidSegment);
                }
            }
        }
        Ok(())
    }

    pub fn path_length(&self, path: &MeshPath) -> Result<FP, NavigationError> {
        self.validate_path(path)?;
        let mut total = 0u128;
        for pair in path.points.windows(2) {
            total = total
                .checked_add(u128::from(distance_ceil(pair[0], pair[1])))
                .ok_or(NavigationError::NumericLimit)?;
        }
        i64::try_from(total)
            .map(FP::from_raw)
            .map_err(|_| NavigationError::NumericLimit)
    }

    pub fn point_at_distance(
        &self,
        path: &MeshPath,
        distance: FP,
    ) -> Result<[FP; 3], NavigationError> {
        self.validate_path(path)?;
        if distance.raw() < 0 {
            return Err(NavigationError::InvalidDistance);
        }
        let mut remaining = u128::from(distance.raw() as u64);
        for (segment_index, pair) in path.points.windows(2).enumerate() {
            let length = u128::from(distance_ceil(pair[0], pair[1]));
            if length == 0 {
                if remaining == 0 {
                    return Ok(pair[0]);
                }
                continue;
            }
            if remaining <= length {
                let triangle = self
                    .triangle(path.corridor[segment_index])
                    .ok_or(NavigationError::InvalidSegment)?;
                if remaining == 0 {
                    return Ok(pair[0]);
                }
                if remaining >= length {
                    return Ok(pair[1]);
                }
                let mut candidate = [FP::ZERO; 3];
                for axis in 0..3 {
                    let delta = i128::from(pair[1][axis].raw()) - i128::from(pair[0][axis].raw());
                    let raw = i128::from(pair[0][axis].raw())
                        + delta * remaining as i128 / length as i128;
                    candidate[axis] = FP::from_raw(
                        i64::try_from(raw).map_err(|_| NavigationError::NumericLimit)?,
                    );
                }
                let mut best: Option<(i128, i64, i64, [FP; 3])> = None;
                for x_offset in -1i64..=1 {
                    for z_offset in -1i64..=1 {
                        let candidate_x = FP::from_raw(
                            candidate[0]
                                .raw()
                                .checked_add(x_offset)
                                .ok_or(NavigationError::NumericLimit)?,
                        );
                        let candidate_z = FP::from_raw(
                            candidate[2]
                                .raw()
                                .checked_add(z_offset)
                                .ok_or(NavigationError::NumericLimit)?,
                        );
                        let Some(projected) = project_to_triangle(
                            &self.vertices,
                            triangle,
                            candidate_x,
                            candidate_z,
                        )?
                        else {
                            continue;
                        };
                        if !point_on_triangle(&self.vertices, triangle, projected)? {
                            continue;
                        }
                        let x_delta =
                            i128::from(candidate_x.raw()) - i128::from(candidate[0].raw());
                        let z_delta =
                            i128::from(candidate_z.raw()) - i128::from(candidate[2].raw());
                        let rank = (
                            x_delta * x_delta + z_delta * z_delta,
                            candidate_x.raw(),
                            candidate_z.raw(),
                            projected,
                        );
                        if best.as_ref().is_none_or(|current| {
                            (rank.0, rank.1, rank.2) < (current.0, current.1, current.2)
                        }) {
                            best = Some(rank);
                        }
                    }
                }
                return best
                    .map(|rank| rank.3)
                    .ok_or(NavigationError::InvalidSegment);
            }
            remaining -= length;
        }
        path.points
            .last()
            .copied()
            .ok_or(NavigationError::InvalidSegment)
    }

    /// Returns the traversed portion of a path, retaining every crossed portal.
    ///
    /// Endpoints use the same fixed-point sampling as [`Self::point_at_distance`],
    /// and segment boundaries use the same rounded-up lengths. Negative or
    /// reversed distances are invalid; distances beyond the goal clamp to it.
    /// Consecutive duplicate positions are omitted, so stationary movement is
    /// represented by one point. The result has at most `path.points.len()`
    /// points (and therefore stays bounded for callers with bounded paths).
    pub fn movement_polyline(
        &self,
        path: &MeshPath,
        from_distance: FP,
        to_distance: FP,
    ) -> Result<Vec<[FP; 3]>, NavigationError> {
        self.validate_path(path)?;
        if from_distance.raw() < 0 || to_distance < from_distance {
            return Err(NavigationError::InvalidDistance);
        }
        let start = self.point_at_distance(path, from_distance)?;
        let end = self.point_at_distance(path, to_distance)?;
        let mut points = vec![start];
        let from_raw = from_distance.raw() as u128;
        let to_raw = to_distance.raw() as u128;
        let mut traversed = 0u128;
        for pair in path.points.windows(2) {
            traversed = traversed
                .checked_add(u128::from(distance_ceil(pair[0], pair[1])))
                .ok_or(NavigationError::NumericLimit)?;
            if traversed >= to_raw {
                break;
            }
            if traversed > from_raw && points.last() != Some(&pair[1]) {
                points.push(pair[1]);
            }
        }
        if points.last() != Some(&end) {
            points.push(end);
        }
        Ok(points)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MESH_MAGIC);
        for value in [
            self.profile.max_slope,
            self.profile.radius,
            self.profile.headroom,
            self.profile.max_step,
        ] {
            bytes.extend_from_slice(&value.raw().to_le_bytes());
        }
        bytes.extend_from_slice(&(self.vertices.len() as u32).to_le_bytes());
        for point in &self.vertices {
            for coordinate in point {
                bytes.extend_from_slice(&coordinate.raw().to_le_bytes());
            }
        }
        bytes.extend_from_slice(&(self.faces.len() as u32).to_le_bytes());
        for face in &self.faces {
            for index in face {
                bytes.extend_from_slice(&index.to_le_bytes());
            }
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NavigationError> {
        if bytes.len() > MAX_FILE_BYTES
            || bytes.len() < 8
            || bytes.get(..8) != Some(MESH_MAGIC.as_slice())
        {
            return Err(NavigationError::MalformedAsset);
        }
        let mut reader = Reader { bytes, offset: 8 };
        let profile = AgentProfile {
            max_slope: FP::from_raw(reader.i64()?),
            radius: FP::from_raw(reader.i64()?),
            headroom: FP::from_raw(reader.i64()?),
            max_step: FP::from_raw(reader.i64()?),
        };
        let vertex_count = reader.u32()? as usize;
        if vertex_count > MAX_VERTICES {
            return Err(NavigationError::MalformedAsset);
        }
        let required_vertices = vertex_count
            .checked_mul(24)
            .ok_or(NavigationError::MalformedAsset)?;
        if reader.remaining() < required_vertices + 4 {
            return Err(NavigationError::MalformedAsset);
        }
        let mut vertices = Vec::with_capacity(vertex_count);
        for _ in 0..vertex_count {
            vertices.push([
                FP::from_raw(reader.i64()?),
                FP::from_raw(reader.i64()?),
                FP::from_raw(reader.i64()?),
            ]);
        }
        let face_count = reader.u32()? as usize;
        if face_count > MAX_FACES
            || reader.remaining()
                != face_count
                    .checked_mul(12)
                    .ok_or(NavigationError::MalformedAsset)?
        {
            return Err(NavigationError::MalformedAsset);
        }
        let mut faces = Vec::with_capacity(face_count);
        for _ in 0..face_count {
            faces.push([reader.u32()?, reader.u32()?, reader.u32()?]);
        }
        let graph = Self::bake(&vertices, &faces, profile)?;
        if graph.to_bytes() != bytes {
            return Err(NavigationError::MalformedAsset);
        }
        Ok(graph)
    }

    fn compute_revision(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"orr-navigation-static-mesh-v1\0");
        hasher.update(self.to_bytes());
        hasher.finalize().into()
    }

    fn triangle(&self, key: u32) -> Option<&MeshTriangle> {
        self.triangles
            .binary_search_by_key(&key, |triangle| triangle.key)
            .ok()
            .map(|slot| &self.triangles[slot])
    }

    fn center(&self, triangle: &MeshTriangle) -> [FP; 3] {
        std::array::from_fn(|axis| {
            FP::from_raw(
                triangle
                    .vertices
                    .iter()
                    .map(|index| i128::from(self.vertices[*index as usize][axis].raw()))
                    .sum::<i128>()
                    .div_euclid(3) as i64,
            )
        })
    }
}

fn validate_profile(profile: AgentProfile) -> Result<(), NavigationError> {
    if profile.radius != FP::ZERO || profile.headroom != FP::ZERO || profile.max_step != FP::ZERO {
        return Err(NavigationError::UnsupportedProfile);
    }
    if !(0..=MAX_SLOPE_RAW).contains(&profile.max_slope.raw()) {
        return Err(NavigationError::NumericLimit);
    }
    Ok(())
}

fn validate_edge_deltas(points: [[FP; 3]; 3]) -> Result<(), NavigationError> {
    for (first, second) in [(0, 1), (1, 2), (2, 0)] {
        for (first_coordinate, second_coordinate) in points[first].iter().zip(&points[second]) {
            let delta =
                (i128::from(first_coordinate.raw()) - i128::from(second_coordinate.raw())).abs();
            if delta > i128::from(MAX_EDGE_DELTA_RAW) {
                return Err(NavigationError::NumericLimit);
            }
        }
    }
    Ok(())
}

fn slope_admitted(points: [[FP; 3]; 3], max_slope: i64) -> Result<bool, NavigationError> {
    let [first, second, third] = points;
    let dx_first = i128::from(second[0].raw()) - i128::from(first[0].raw());
    let dy_first = i128::from(second[1].raw()) - i128::from(first[1].raw());
    let dz_first = i128::from(second[2].raw()) - i128::from(first[2].raw());
    let dx_second = i128::from(third[0].raw()) - i128::from(first[0].raw());
    let dy_second = i128::from(third[1].raw()) - i128::from(first[1].raw());
    let dz_second = i128::from(third[2].raw()) - i128::from(first[2].raw());
    let normal_vertical = dz_first * dx_second - dx_first * dz_second;
    if normal_vertical <= 0 {
        return Err(NavigationError::InvalidSegment);
    }
    let normal_x = dy_first * dz_second - dz_first * dy_second;
    let normal_z = dx_first * dy_second - dy_first * dx_second;
    let slope_numerator = normal_x
        .checked_mul(normal_x)
        .and_then(|value| value.checked_add(normal_z.checked_mul(normal_z)?))
        .ok_or(NavigationError::NumericLimit)?;
    let lhs = slope_numerator
        .checked_mul(i128::from(FP::SCALE).pow(2))
        .ok_or(NavigationError::NumericLimit)?;
    let rhs = normal_vertical
        .checked_mul(normal_vertical)
        .and_then(|value| value.checked_mul(i128::from(max_slope).pow(2)))
        .ok_or(NavigationError::NumericLimit)?;
    Ok(lhs <= rhs)
}

fn cross_xz(first: [FP; 3], second: [FP; 3], point: [FP; 3]) -> i128 {
    (i128::from(second[0].raw()) - i128::from(first[0].raw()))
        * (i128::from(point[2].raw()) - i128::from(first[2].raw()))
        - (i128::from(second[2].raw()) - i128::from(first[2].raw()))
            * (i128::from(point[0].raw()) - i128::from(first[0].raw()))
}

fn project_to_triangle(
    vertices: &[[FP; 3]],
    triangle: &MeshTriangle,
    input_x: FP,
    input_z: FP,
) -> Result<Option<[FP; 3]>, NavigationError> {
    let points = triangle.vertices.map(|index| vertices[index as usize]);
    let area = cross_xz(points[0], points[1], points[2]);
    let weights = [
        cross_xz(points[1], points[2], [input_x, FP::ZERO, input_z]),
        cross_xz(points[2], points[0], [input_x, FP::ZERO, input_z]),
        cross_xz(points[0], points[1], [input_x, FP::ZERO, input_z]),
    ];
    if weights.iter().any(|weight| *weight > 0) || area >= 0 {
        return Ok(None);
    }
    let numerator = points
        .iter()
        .zip(weights)
        .map(|(point, weight)| i128::from(point[1].raw()) * weight)
        .sum::<i128>();
    let projected_height = (-numerator).div_euclid(-area);
    let projected_height =
        i64::try_from(projected_height).map_err(|_| NavigationError::NumericLimit)?;
    Ok(Some([input_x, FP::from_raw(projected_height), input_z]))
}

fn point_on_triangle(
    vertices: &[[FP; 3]],
    triangle: &MeshTriangle,
    point: [FP; 3],
) -> Result<bool, NavigationError> {
    Ok(project_to_triangle(vertices, triangle, point[0], point[2])? == Some(point))
}

fn edge_lattice_midpoint(first: [FP; 3], second: [FP; 3]) -> Result<[FP; 3], NavigationError> {
    let deltas = std::array::from_fn::<_, 3, _>(|component_index| {
        i128::from(second[component_index].raw()) - i128::from(first[component_index].raw())
    });
    let divisor = gcd(gcd(deltas[0].abs(), deltas[1].abs()), deltas[2].abs());
    if divisor == 0 {
        return Err(NavigationError::InvalidSegment);
    }
    let steps = divisor / 2;
    let point = std::array::from_fn(|component_index| {
        let primitive = deltas[component_index] / divisor;
        FP::from_raw((i128::from(first[component_index].raw()) + primitive * steps) as i64)
    });
    Ok(point)
}

fn gcd(mut first: i128, mut second: i128) -> i128 {
    while second != 0 {
        let remainder = first % second;
        first = second;
        second = remainder;
    }
    first
}

fn graph_edge_cost(first: [FP; 3], second: [FP; 3]) -> u64 {
    distance_ceil(first, second).max(1)
}

fn distance_ceil(first: [FP; 3], second: [FP; 3]) -> u64 {
    let squared = (0..3)
        .map(|component_index| {
            let delta = i128::from(first[component_index].raw())
                - i128::from(second[component_index].raw());
            (delta * delta) as u128
        })
        .sum::<u128>();
    let floor = isqrt(squared);
    (floor + u128::from(floor * floor != squared)) as u64
}

fn isqrt(value: u128) -> u128 {
    if value < 2 {
        return value;
    }
    let mut estimate = 1u128 << (128 - value.leading_zeros()).div_ceil(2);
    loop {
        let next = (estimate + value / estimate) / 2;
        if next >= estimate {
            return estimate;
        }
        estimate = next;
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Reader<'_> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }
    fn take<const BYTE_COUNT: usize>(&mut self) -> Result<[u8; BYTE_COUNT], NavigationError> {
        let end = self
            .offset
            .checked_add(BYTE_COUNT)
            .ok_or(NavigationError::MalformedAsset)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(NavigationError::MalformedAsset)?;
        self.offset = end;
        value
            .try_into()
            .map_err(|_| NavigationError::MalformedAsset)
    }
    fn i64(&mut self) -> Result<i64, NavigationError> {
        Ok(i64::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, NavigationError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
}

#[cfg(test)]
mod tests {
    use super::{distance_ceil, graph_edge_cost};
    use orr_fp::FP;

    #[test]
    fn graph_edges_remain_positive_when_quantized_centers_coincide() {
        let center = [FP::from_raw(1), FP::ZERO, FP::ZERO];
        assert_eq!(distance_ceil(center, center), 0);
        assert_eq!(graph_edge_cost(center, center), 1);
        assert_eq!(graph_edge_cost([FP::ZERO; 3], [FP::from_raw(1); 3]), 2);
    }
}
