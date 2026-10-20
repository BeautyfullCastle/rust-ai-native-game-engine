//! Pure, bounded terrain-ramp candidates. Applying one is the caller's job.
//!
//! A ramp replaces the inclusive axis-aligned vertex rectangle between its two
//! endpoints, with `half_width` vertices on either side of the center line.
//! Heights are constant across the strip and interpolate the existing endpoint
//! heights along it. Outside vertices, holes, identity, origin and spacing are
//! never edited. The strip has no falloff; adjacent cells can therefore slope
//! toward its boundary. This is not a bridge across holes.

use orr_fp::FP;
use orr_terrain::{Edit, Terrain, MAX_EDITS};

/// Vertex coordinates are `[x, z]`; width is in grid intervals, not world units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RampSpec {
    pub start: [u32; 2],
    pub end: [u32; 2],
    pub half_width: u32,
}

/// Inclusive rectangle, in vertex or cell coordinates as documented by its use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RampRect {
    pub min: [u32; 2],
    pub max: [u32; 2],
}

/// Immutable preview and one bounded, changed-only transaction.
///
/// Terrain revision pins all canonical content, including geometry and identity.
/// The UI must additionally pin scene, source, host epoch, selection and settings,
/// and invalidate on context transitions. A revision cannot detect an edit/undo
/// cycle that returns to identical content, or a scene/source replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RampCandidate {
    spec: RampSpec,
    source_revision: [u8; 32],
    footprint: RampRect,
    incident_cells: RampRect,
    endpoint_heights: [FP; 2],
    footprint_vertices: u32,
    edits: Vec<Edit>,
}

impl RampCandidate {
    /// Build without mutating a terrain, document, history or file.
    ///
    /// The minimum half-width is one: three vertex rows and two cells across.
    /// The complete footprint must fit (no clipping). Two adjacent axial vertices
    /// are allowed, including endpoints at an axial terrain edge. Transverse
    /// endpoints must leave room for the requested half-width on both sides.
    ///
    /// Every existing cell incident to *any* footprint vertex must be non-hole,
    /// even when that vertex would not change. Thus the hole check includes the
    /// one-cell exterior halo, truncated only where no terrain cell exists.
    /// Bounds and allocation reuse the validated Terrain dimensions and core
    /// MAX_EDITS limit; large candidates are rejected, never split into history.
    pub fn build(terrain: &Terrain, spec: RampSpec) -> Result<Self, String> {
        let sides = [terrain.width(), terrain.depth()];
        for endpoint in [spec.start, spec.end] {
            if endpoint[0] >= sides[0] || endpoint[1] >= sides[1] {
                return Err("ramp endpoint outside terrain".into());
            }
        }
        if spec.half_width == 0 {
            return Err("ramp half-width must be at least one grid interval".into());
        }
        let axis = match (spec.start[0] == spec.end[0], spec.start[1] == spec.end[1]) {
            (false, true) => 0,
            (true, false) => 1,
            (true, true) => return Err("ramp endpoints must be distinct".into()),
            (false, false) => return Err("ramp endpoints must share the X or Z axis".into()),
        };
        let transverse = 1 - axis;
        let mut footprint = RampRect {
            min: spec.start,
            max: spec.start,
        };
        footprint.min[axis] = spec.start[axis].min(spec.end[axis]);
        footprint.max[axis] = spec.start[axis].max(spec.end[axis]);
        footprint.min[transverse] = spec.start[transverse]
            .checked_sub(spec.half_width)
            .ok_or("ramp width extends outside terrain; clipping is not supported")?;
        footprint.max[transverse] = spec.start[transverse]
            .checked_add(spec.half_width)
            .filter(|&coordinate| coordinate < sides[transverse])
            .ok_or("ramp width extends outside terrain; clipping is not supported")?;

        // A cell [x,z] touches this rectangle iff its coordinate lies between
        // min-1 and max on each axis, intersected with valid cells [0, side-2].
        let incident_cells = RampRect {
            min: [
                footprint.min[0].saturating_sub(1),
                footprint.min[1].saturating_sub(1),
            ],
            max: [
                footprint.max[0].min(sides[0] - 2),
                footprint.max[1].min(sides[1] - 2),
            ],
        };
        for z in incident_cells.min[1]..=incident_cells.max[1] {
            for x in incident_cells.min[0]..=incident_cells.max[0] {
                if terrain.holes()[(z * (sides[0] - 1) + x) as usize] {
                    return Err(format!("ramp touches hole cell [{x}, {z}]"));
                }
            }
        }
        let height_at = |[x, z]: [u32; 2]| terrain.heights()[(z * sides[0] + x) as usize];
        let endpoint_heights = [height_at(spec.start), height_at(spec.end)];
        let length = spec.start[axis].abs_diff(spec.end[axis]);
        let footprint_vertices = (footprint.max[0] - footprint.min[0] + 1)
            .checked_mul(footprint.max[1] - footprint.min[1] + 1)
            .ok_or("ramp footprint size overflow")?;
        let mut edits = Vec::with_capacity((footprint_vertices as usize).min(MAX_EDITS));
        // Row-major output is independent of endpoint order.
        for z in footprint.min[1]..=footprint.max[1] {
            for x in footprint.min[0]..=footprint.max[0] {
                let coordinate = [x, z];
                let distance = coordinate[axis].abs_diff(spec.start[axis]);
                let height = interpolate(endpoint_heights, distance, length)?;
                if height != height_at(coordinate) {
                    if edits.len() == MAX_EDITS {
                        return Err(format!("ramp exceeds the {MAX_EDITS} changed-vertex limit"));
                    }
                    edits.push(Edit::SetHeight { x, z, height });
                }
            }
        }
        Ok(Self {
            spec,
            source_revision: terrain.revision(),
            footprint,
            incident_cells,
            endpoint_heights,
            footprint_vertices,
            edits,
        })
    }

    pub fn spec(&self) -> RampSpec {
        self.spec
    }
    pub fn source_revision(&self) -> [u8; 32] {
        self.source_revision
    }
    /// Content-only check; callers must also validate their contextual pin.
    pub fn is_current(&self, terrain: &Terrain) -> bool {
        self.source_revision == terrain.revision()
    }
    pub fn footprint(&self) -> RampRect {
        self.footprint
    }
    pub fn incident_cells(&self) -> RampRect {
        self.incident_cells
    }
    /// Existing endpoint heights, in the original start/end order.
    pub fn endpoint_heights(&self) -> [FP; 2] {
        self.endpoint_heights
    }
    pub fn footprint_vertices(&self) -> u32 {
        self.footprint_vertices
    }
    pub fn changed_vertices(&self) -> usize {
        self.edits.len()
    }
    /// Only SetHeight operations; empty means no-op and must not create history.
    /// Apply through the existing TerrainSession admission, never directly to
    /// rendered state. Display precision and renderer capacity remain its checks.
    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }
}

/// Round the absolute weighted rational to the nearest raw Q48.16 integer,
/// with exact halves away from zero. In particular -0.5 -> -1 and +0.5 -> +1.
/// Never round a relative delta: that breaks endpoint reversal at signed ties.
/// Positive weights sum to length, so endpoints are exact, reversal is symmetric,
/// and the result stays within the endpoint range, including i64 extrema.
fn interpolate(heights: [FP; 2], distance: u32, length: u32) -> Result<FP, String> {
    let left = length
        .checked_sub(distance)
        .ok_or("ramp interpolation bounds")?;
    if length == 0 {
        return Err("ramp interpolation length is zero".into());
    }
    let numerator = i128::from(heights[0].raw())
        .checked_mul(i128::from(left))
        .and_then(|left| {
            i128::from(heights[1].raw())
                .checked_mul(i128::from(distance))
                .and_then(|right| left.checked_add(right))
        })
        .ok_or("ramp interpolation overflow")?;
    let denominator = i128::from(length);
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let rounded = if remainder.abs() * 2 >= denominator {
        quotient
            .checked_add(numerator.signum())
            .ok_or("ramp rounding overflow")?
    } else {
        quotient
    };
    Ok(FP::from_raw(
        i64::try_from(rounded).map_err(|_| "ramp height overflow")?,
    ))
}
