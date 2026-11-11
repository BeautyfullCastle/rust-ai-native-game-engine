//! Optional, GPU-free presentation-only camera boom against one static terrain.
//!
//! Obstacles are the exact public display bridge's two-sided triangles, not a
//! solid below the terrain. A blocked query never supplies a renderable pose.
//! Read `docs/terrain-camera-boom.md` for numerical admission and consumer duties.
#![allow(clippy::float_arithmetic)]

mod sweep;

use orr_terrain::Terrain;
use std::fmt;
use sweep::{length, sub, Triangle};

/// Maximum canonical geometry and per-query triangle visits.
pub const MAX_TRIANGLES: usize = 32_768;
/// Additional world-space safety inflation, separate from the caller's margin.
pub const NUMERIC_PAD: f64 = 0.00001;
/// World-space retraction from the conservatively estimated first contact.
pub const CONTACT_BACKOFF: f64 = 0.00001;
/// Conservative length-domain error envelope for each primitive boundary.
pub const ARITHMETIC_ENVELOPE: f64 = 0.000001;
pub const MAX_COORDINATE: f64 = 1_000_000.0;
pub const MIN_DISTANCE: f64 = 0.0001;
pub const MAX_DISTANCE: f64 = 1_000_000.0;
pub const MAX_RADIUS: f64 = 10_000.0;
pub const MIN_SPACING: f64 = 1.0 / 256.0;
pub const ROOT_REFINEMENTS: usize = 48;

#[derive(Debug)]
pub enum CameraBoomError {
    Display(orr_terrain_view::Error),
    UnsupportedGeometry(&'static str),
    InvalidQuery(&'static str),
    RevisionMismatch,
    NumericalFailure,
}
impl fmt::Display for CameraBoomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Display(error) => write!(f, "terrain display admission failed: {error}"),
            Self::UnsupportedGeometry(reason) => write!(f, "unsupported camera terrain: {reason}"),
            Self::InvalidQuery(reason) => write!(f, "invalid camera boom: {reason}"),
            Self::RevisionMismatch => {
                f.write_str("camera obstacle revision differs from the render terrain")
            }
            Self::NumericalFailure => {
                f.write_str("camera sweep could not certify finite arithmetic")
            }
        }
    }
}
impl std::error::Error for CameraBoomError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Display(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriangleId {
    /// [x,z] cell index, independent of holes preceding this cell.
    pub cell: [u32; 2],
    /// 0=[a,c,d], 1=[a,d,b], the terrain's canonical a-d diagonal.
    pub triangle: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoomQuery {
    pub expected_revision: [u8; 32],
    pub pivot: [f64; 3],
    pub desired_eye: [f64; 3],
    pub radius: f64,
    pub margin: f64,
    pub min_distance: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    pub eye: [f64; 3],
    pub distance: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoomHit {
    pub triangle: TriangleId,
    /// Conservative contact distance for the inflated probe, before backoff.
    pub distance: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockedReason {
    StartOverlap,
    InsufficientDistance,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoomResolution {
    Clear(CameraPose),
    Retracted {
        pose: CameraPose,
        hit: BoomHit,
    },
    Blocked {
        reason: BlockedReason,
        hit: Option<BoomHit>,
    },
}

/// Actual bounded work counters. Diagnostics do not allocate or change results.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryWork {
    pub triangles: usize,
    pub features: usize,
    pub root_refinements: usize,
    /// Checked low/high endpoint probes for an optional untrusted bracket hint.
    pub bracket_probes: usize,
    /// Hints whose checked endpoint pair strictly narrowed the proven bracket.
    pub brackets_narrowed: usize,
}

/// Immutable CPU data bound to one admitted display snapshot/revision.
#[derive(Debug)]
pub struct TerrainCameraObstacles {
    revision: [u8; 32],
    triangles: Vec<Triangle>,
}

impl TerrainCameraObstacles {
    pub fn new(terrain: &Terrain) -> Result<Self, CameraBoomError> {
        // This call owns display precision admission. Do not independently cast
        // authoritative coordinates into a second, potentially different mesh.
        let model =
            orr_terrain_view::to_static_model(terrain, &[]).map_err(CameraBoomError::Display)?;
        let revision = terrain.revision();
        let canonical = terrain.triangles();
        if canonical.len() > MAX_TRIANGLES {
            return Err(CameraBoomError::UnsupportedGeometry("triangle budget"));
        }
        let Some(model) = model else {
            if !canonical.is_empty() {
                return Err(CameraBoomError::UnsupportedGeometry(
                    "missing display geometry",
                ));
            }
            return Ok(Self {
                revision,
                triangles: Vec::new(),
            });
        };
        if terrain.spacing().raw() < 256 {
            return Err(CameraBoomError::UnsupportedGeometry(
                "spacing is below 1/256",
            ));
        }
        let source = model.source();
        let revision_hex: String = revision.iter().map(|byte| format!("{byte:02x}")).collect();
        if source.asset_id != terrain.asset_id()
            || source.primitives.len() != 1
            || source.dependencies.len() != 1
            || source.dependencies[0].uri != "$source"
            || source.dependencies[0].sha256 != revision_hex
        {
            return Err(CameraBoomError::UnsupportedGeometry(
                "display source identity",
            ));
        }
        let primitive = &source.primitives[0];
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        if primitive.transform != identity
            || primitive.vertices.len() != canonical.len() * 3
            || primitive.indices.len() != canonical.len() * 3
            || primitive.id != format!("{}#node=0/mesh=0/primitive=0", terrain.asset_id())
        {
            return Err(CameraBoomError::UnsupportedGeometry(
                "display topology or transform",
            ));
        }
        let mut triangles = Vec::with_capacity(canonical.len());
        let mut ordinal = 0;
        for z in 0..terrain.depth() - 1 {
            for x in 0..terrain.width() - 1 {
                if terrain.holes()[(z * (terrain.width() - 1) + x) as usize] {
                    continue;
                }
                let a = z * terrain.width() + x;
                let b = a + 1;
                let c = a + terrain.width();
                let d = c + 1;
                for (local, indices) in [[a, c, d], [a, d, b]].into_iter().enumerate() {
                    if canonical[ordinal] != indices {
                        return Err(CameraBoomError::UnsupportedGeometry(
                            "canonical diagonal changed",
                        ));
                    }
                    let mut points = [[0.0; 3]; 3];
                    for (corner, vertex) in indices.into_iter().enumerate() {
                        let index = ordinal * 3 + corner;
                        if primitive.indices[index] as usize != index {
                            return Err(CameraBoomError::UnsupportedGeometry(
                                "display index order",
                            ));
                        }
                        let emitted = primitive.vertices[index].position;
                        let fixed = terrain.vertex_position(vertex).ok_or(
                            CameraBoomError::UnsupportedGeometry("missing terrain vertex"),
                        )?;
                        for axis in 0..3 {
                            let value = f64::from(emitted[axis]);
                            if !value.is_finite()
                                || value.abs() > MAX_COORDINATE
                                || value * 65536.0 != fixed[axis].raw() as f64
                            {
                                return Err(CameraBoomError::UnsupportedGeometry(
                                    "display coordinate correspondence",
                                ));
                            }
                            points[corner][axis] = value;
                        }
                    }
                    triangles.push(Triangle::new(
                        TriangleId {
                            cell: [x, z],
                            triangle: local as u8,
                        },
                        points,
                    )?);
                    ordinal += 1;
                }
            }
        }
        if ordinal != canonical.len() {
            return Err(CameraBoomError::UnsupportedGeometry(
                "triangle count changed",
            ));
        }
        Ok(Self {
            revision,
            triangles,
        })
    }

    pub fn revision(&self) -> [u8; 32] {
        self.revision
    }
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// Inspect a copy of admitted display positions and their stable identity.
    pub fn triangle(&self, index: usize) -> Option<(TriangleId, [[f64; 3]; 3])> {
        self.triangles
            .get(index)
            .map(|triangle| (triangle.id, triangle.points))
    }

    /// Retained snapshot bytes, excluding allocator bookkeeping.
    pub fn storage_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.triangles.capacity() * std::mem::size_of::<Triangle>()
    }

    pub fn resolve(&self, query: BoomQuery) -> Result<BoomResolution, CameraBoomError> {
        self.resolve_with_work(query)
            .map(|(resolution, _)| resolution)
    }

    /// Same resolver with actual operation counters for bounded-work evidence.
    pub fn resolve_with_work(
        &self,
        query: BoomQuery,
    ) -> Result<(BoomResolution, QueryWork), CameraBoomError> {
        if query.expected_revision != self.revision {
            return Err(CameraBoomError::RevisionMismatch);
        }
        if query
            .pivot
            .into_iter()
            .chain(query.desired_eye)
            .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE)
        {
            return Err(CameraBoomError::InvalidQuery("position bounds"));
        }
        if !query.radius.is_finite()
            || !(MIN_DISTANCE..=MAX_RADIUS).contains(&query.radius)
            || !query.margin.is_finite()
            || !(0.0..=MAX_RADIUS).contains(&query.margin)
            || !query.min_distance.is_finite()
            || !(MIN_DISTANCE..=MAX_DISTANCE).contains(&query.min_distance)
        {
            return Err(CameraBoomError::InvalidQuery(
                "radius, margin or minimum distance",
            ));
        }
        let delta = sub(query.desired_eye, query.pivot);
        let distance = length(delta)?;
        if !(MIN_DISTANCE..=MAX_DISTANCE).contains(&distance) {
            return Err(CameraBoomError::InvalidQuery("boom length"));
        }
        let mut work = QueryWork::default();
        let radius = query.radius + query.margin + NUMERIC_PAD;
        let mut nearest: Option<BoomHit> = None;
        let mut limit = 1.0;
        for triangle in &self.triangles {
            work.triangles += 1;
            if let Some(parameter) =
                triangle.contact(query.pivot, delta, radius, limit, &mut work)?
            {
                let contact_distance = parameter * distance;
                if nearest.is_none_or(|old| contact_distance < old.distance) {
                    nearest = Some(BoomHit {
                        triangle: triangle.id,
                        distance: contact_distance,
                    });
                    limit = parameter;
                }
                if parameter == 0.0 {
                    return Ok((
                        BoomResolution::Blocked {
                            reason: BlockedReason::StartOverlap,
                            hit: nearest,
                        },
                        work,
                    ));
                }
            }
        }
        let Some(hit) = nearest else {
            let result = if distance < query.min_distance {
                BoomResolution::Blocked {
                    reason: BlockedReason::InsufficientDistance,
                    hit: None,
                }
            } else {
                BoomResolution::Clear(CameraPose {
                    eye: query.desired_eye,
                    distance,
                })
            };
            return Ok((result, work));
        };
        let safe_distance = (hit.distance - CONTACT_BACKOFF).max(0.0);
        if safe_distance < query.min_distance {
            return Ok((
                BoomResolution::Blocked {
                    reason: BlockedReason::InsufficientDistance,
                    hit: Some(hit),
                },
                work,
            ));
        }
        let parameter = safe_distance / distance;
        let eye = std::array::from_fn(|axis| query.pivot[axis] + delta[axis] * parameter);
        if eye.into_iter().any(|value| !value.is_finite()) {
            return Err(CameraBoomError::NumericalFailure);
        }
        Ok((
            BoomResolution::Retracted {
                pose: CameraPose {
                    eye,
                    distance: safe_distance,
                },
                hit,
            },
            work,
        ))
    }
}
