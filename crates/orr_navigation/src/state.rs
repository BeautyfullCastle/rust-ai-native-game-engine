//! Canonical, bounded, dependency-validated navigator persistence.
use super::*;

const STATE_MAGIC: &[u8; 8] = b"ORRNST\x01\0";
const MAX_STATE_BYTES: usize = 3_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigatorSnapshot(Vec<u8>);

impl NavigatorSnapshot {
    /// Checks only the allocation envelope. `Navigator::restore` validates the
    /// complete encoding and its terrain/graph dependencies before use.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NavigationError> {
        if bytes.len() > MAX_STATE_BYTES || bytes.len() < 8 + 32 + 32 + 24 + 1 + 4 + 1 {
            return Err(NavigationError::MalformedAsset);
        }
        Ok(Self(bytes.to_vec()))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], NavigationError> {
        let end = self
            .cursor
            .checked_add(count)
            .ok_or(NavigationError::MalformedAsset)?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or(NavigationError::MalformedAsset)?;
        self.cursor = end;
        Ok(bytes)
    }
    fn u8(&mut self) -> Result<u8, NavigationError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, NavigationError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn fp(&mut self) -> Result<FP, NavigationError> {
        Ok(FP::from_raw(i64::from_le_bytes(
            self.take(8)?.try_into().unwrap(),
        )))
    }
    fn point(&mut self) -> Result<[FP; 3], NavigationError> {
        Ok([self.fp()?, self.fp()?, self.fp()?])
    }
    fn count(&mut self, max: u32, stride: usize) -> Result<usize, NavigationError> {
        let n = self.u32()?;
        if n > max || self.bytes.len().saturating_sub(self.cursor) / stride < n as usize {
            return Err(NavigationError::MalformedAsset);
        }
        Ok(n as usize)
    }
}

fn put_point(bytes: &mut Vec<u8>, point: [FP; 3]) {
    for fp in point {
        bytes.extend(fp.raw().to_le_bytes());
    }
}

impl Navigator {
    /// Canonical LE state, including every route record and both dependency digests.
    pub fn snapshot(&self) -> NavigatorSnapshot {
        let mut bytes = Vec::new();
        bytes.extend(STATE_MAGIC);
        bytes.extend(self.terrain_revision);
        bytes.extend(self.graph_revision);
        put_point(&mut bytes, self.position);
        bytes.push(match self.status {
            NavigationStatus::Stopped => 0,
            NavigationStatus::Moving => 1,
            NavigationStatus::Arrived => 2,
        });
        bytes.extend(self.next_waypoint.to_le_bytes());
        bytes.push(u8::from(self.path.is_some()));
        if let Some(path) = &self.path {
            bytes.extend(path.terrain_revision);
            bytes.extend(path.graph_revision);
            bytes.extend(path.cost.raw().to_le_bytes());
            bytes.extend((path.corridor.len() as u32).to_le_bytes());
            for key in &path.corridor {
                bytes.extend(key.0.to_le_bytes());
            }
            bytes.extend((path.portals.len() as u32).to_le_bytes());
            for portal in &path.portals {
                bytes.extend(portal.neighbor.0.to_le_bytes());
                for vertex in portal.vertices {
                    bytes.extend(vertex.to_le_bytes());
                }
                put_point(&mut bytes, portal.midpoint);
            }
            bytes.extend((path.waypoints.len() as u32).to_le_bytes());
            for &point in &path.waypoints {
                put_point(&mut bytes, point);
            }
        }
        NavigatorSnapshot(bytes)
    }

    /// Fully verifies dependency binding, canonical route geometry, and safe
    /// continuation from the current point before replacing `self`; errors are
    /// atomic. A snapshot is not an authenticated replay transcript: a forged
    /// current point within the current walkable triangle that can safely
    /// continue toward its next waypoint need not be historically reachable
    /// from an earlier snapshot. Callers requiring provenance must authenticate
    /// the Frame snapshot or replay it from trusted inputs.
    pub fn restore(
        &mut self,
        graph: &TerrainGraph,
        terrain: &Terrain,
        snapshot: &NavigatorSnapshot,
    ) -> Result<(), NavigationError> {
        graph.validate_terrain(terrain)?;
        let mut r = Reader {
            bytes: snapshot.as_bytes(),
            cursor: 0,
        };
        if r.take(8)? != STATE_MAGIC {
            return Err(NavigationError::MalformedAsset);
        }
        let terrain_revision: [u8; 32] = r.take(32)?.try_into().unwrap();
        let graph_revision: [u8; 32] = r.take(32)?.try_into().unwrap();
        if terrain_revision != graph.terrain_revision() {
            return Err(NavigationError::StaleTerrain);
        }
        if graph_revision != graph.revision() {
            return Err(NavigationError::StaleGraph);
        }
        let position = r.point()?;
        let status = match r.u8()? {
            0 => NavigationStatus::Stopped,
            1 => NavigationStatus::Moving,
            2 => NavigationStatus::Arrived,
            _ => return Err(NavigationError::MalformedAsset),
        };
        let next_waypoint = r.u32()?;
        let path = match r.u8()? {
            0 => None,
            1 => {
                let path_terrain_revision: [u8; 32] = r.take(32)?.try_into().unwrap();
                let path_graph_revision: [u8; 32] = r.take(32)?.try_into().unwrap();
                if path_terrain_revision != terrain_revision {
                    return Err(NavigationError::StaleTerrain);
                }
                if path_graph_revision != graph_revision {
                    return Err(NavigationError::StaleGraph);
                }
                let cost = r.fp()?;
                let corridor_count = r.count(MAX_TRIANGLES, 4)?;
                if corridor_count == 0 {
                    return Err(NavigationError::MalformedAsset);
                }
                let mut corridor = Vec::with_capacity(corridor_count);
                for _ in 0..corridor_count {
                    corridor.push(TriangleKey(r.u32()?));
                }
                let portal_count = r.count(MAX_TRIANGLES - 1, 4 + 8 + 24)?;
                if portal_count + 1 != corridor_count {
                    return Err(NavigationError::MalformedAsset);
                }
                let mut portals = Vec::with_capacity(portal_count);
                for _ in 0..portal_count {
                    portals.push(Portal {
                        neighbor: TriangleKey(r.u32()?),
                        vertices: [r.u32()?, r.u32()?],
                        midpoint: r.point()?,
                    });
                }
                let waypoint_count = r.count(MAX_TRIANGLES + 1, 24)?;
                if waypoint_count != corridor_count + 1 {
                    return Err(NavigationError::MalformedAsset);
                }
                let mut waypoints = Vec::with_capacity(waypoint_count);
                for _ in 0..waypoint_count {
                    waypoints.push(r.point()?);
                }
                Some(NavigationPath {
                    graph_revision,
                    terrain_revision,
                    corridor,
                    portals,
                    waypoints,
                    cost,
                })
            }
            _ => return Err(NavigationError::MalformedAsset),
        };
        if r.cursor != r.bytes.len() {
            return Err(NavigationError::MalformedAsset);
        }
        let projected = graph.project(terrain, [position[0], position[2]])?;
        if projected.position != position {
            return Err(NavigationError::InvalidSegment);
        }
        match (&path, status) {
            (None, NavigationStatus::Stopped) if next_waypoint == 0 => {}
            (Some(path), NavigationStatus::Moving) => {
                if next_waypoint == 0 || next_waypoint as usize >= path.waypoints.len() {
                    return Err(NavigationError::MalformedAsset);
                }
                let index = next_waypoint as usize;
                let key = path.corridor[index - 1];
                graph.validate_segment(terrain, key, path.waypoints[index - 1], position)?;
                graph.validate_segment(terrain, key, position, path.waypoints[index])?;
            }
            (Some(path), NavigationStatus::Arrived) => {
                if next_waypoint as usize != path.waypoints.len()
                    || Some(&position) != path.waypoints.last()
                {
                    return Err(NavigationError::MalformedAsset);
                }
            }
            _ => return Err(NavigationError::MalformedAsset),
        }
        if let Some(path) = &path {
            let start = path.waypoints[0];
            let goal = *path.waypoints.last().unwrap();
            let expected = graph.find_path(
                terrain,
                [start[0], start[2]],
                [goal[0], goal[2]],
                SearchBudget::default(),
            )?;
            if expected != *path {
                return Err(NavigationError::MalformedAsset);
            }
            let expected_status = if path.waypoints.iter().all(|p| *p == start) {
                NavigationStatus::Arrived
            } else {
                NavigationStatus::Moving
            };
            if expected_status == NavigationStatus::Arrived && status != NavigationStatus::Arrived {
                return Err(NavigationError::MalformedAsset);
            }
        }
        *self = Self {
            position,
            graph_revision,
            terrain_revision,
            path,
            next_waypoint,
            status,
        };
        Ok(())
    }
}
