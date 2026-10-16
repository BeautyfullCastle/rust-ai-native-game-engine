//! Checked fixed-point closest points on the canonical finite terrain mesh.
//!
//! All projections use raw Q16 coordinates and checked i128 intermediates.
//! Region tests precede a single final coordinate division. Shared mesh features
//! have a single ID; internal edges are discarded unless the point is a local
//! distance minimum on every incident triangle (the mesh normal cone).

use crate::asset::TerrainView;
use orr_fp::{FPVec3, FP};
use std::fmt;

/// Geometry or candidate-budget failure. No simulation state is changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryError(pub &'static str);
impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for GeometryError {}

/// Closest Voronoi feature, indexed within the supplied triangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriangleFeature {
    /// Triangle interior.
    Face,
    /// Finite edge between two local vertex indices.
    Edge(u8, u8),
    /// Local vertex index.
    Vertex(u8),
}
/// Finite triangle closest point, rounded to raw Q16 coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClosestPoint {
    /// World-space closest point.
    pub point: FPVec3,
    /// Voronoi feature producing the point.
    pub feature: TriangleFeature,
}
/// One finite-mesh contact. Normal points from the mesh to the sphere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainContact {
    /// Point on the actual triangle, edge, or vertex.
    pub point: FPVec3,
    /// Quantized unit normal from terrain to sphere.
    pub normal: FPVec3,
    /// Closest distance minus radius; positive means speculative separation.
    pub separation: FP,
    /// Stable mesh-wide vertex, edge, or face key.
    pub feature_id: u32,
}

type Wide = [i128; 3];
const ONE: i128 = 1 << 16;
const GEOMETRY_LIMIT: i64 = 1024 << 16;
fn raw(v: FPVec3) -> Wide {
    [v.x.raw() as i128, v.y.raw() as i128, v.z.raw() as i128]
}
fn sub(a: Wide, b: Wide) -> Result<Wide, GeometryError> {
    Ok([
        checked(a[0].checked_sub(b[0]))?,
        checked(a[1].checked_sub(b[1]))?,
        checked(a[2].checked_sub(b[2]))?,
    ])
}
fn checked(v: Option<i128>) -> Result<i128, GeometryError> {
    v.ok_or(GeometryError("wide geometry overflow"))
}
fn dot(a: Wide, b: Wide) -> Result<i128, GeometryError> {
    let mut s = 0;
    for i in 0..3 {
        s = checked(checked(a[i].checked_mul(b[i]))?.checked_add(s))?;
    }
    Ok(s)
}
fn determinant(a: i128, b: i128, c: i128, d: i128) -> Result<i128, GeometryError> {
    checked(checked(a.checked_mul(b))?.checked_sub(checked(c.checked_mul(d))?))
}
fn point(p: Wide) -> Result<FPVec3, GeometryError> {
    let fp = |v| {
        i64::try_from(v)
            .map(FP::from_raw)
            .map_err(|_| GeometryError("geometry coordinate overflow"))
    };
    Ok(FPVec3::new(fp(p[0])?, fp(p[1])?, fp(p[2])?))
}
fn interpolate(a: Wide, ab: Wide, n: i128, d: i128) -> Result<FPVec3, GeometryError> {
    if d <= 0 {
        return Err(GeometryError("degenerate triangle"));
    }
    let mut q = a;
    for i in 0..3 {
        q[i] = checked(a[i].checked_add(checked(ab[i].checked_mul(n))?.div_euclid(d)))?;
    }
    point(q)
}
fn coordinate_ok(v: FPVec3) -> bool {
    [v.x, v.y, v.z]
        .iter()
        .all(|x| (-GEOMETRY_LIMIT..=GEOMETRY_LIMIT).contains(&x.raw()))
}

/// Exact finite-triangle Voronoi classification with checked wide arithmetic.
/// Coordinates must lie in the terrain physics profile (±1024 units).
/// Degenerate triangles return an error rather than inventing a plane.
pub fn closest_point_on_triangle(
    p: FPVec3,
    triangle: [FPVec3; 3],
) -> Result<ClosestPoint, GeometryError> {
    if !coordinate_ok(p) || triangle.iter().any(|&v| !coordinate_ok(v)) {
        return Err(GeometryError("geometry coordinate limit"));
    }
    let [a, b, c] = triangle.map(raw);
    let ab = sub(b, a)?;
    let ac = sub(c, a)?;
    let ap = sub(raw(p), a)?;
    let area = cross(ab, ac)?;
    if area == [0; 3] {
        return Err(GeometryError("degenerate triangle"));
    }
    let d1 = dot(ab, ap)?;
    let d2 = dot(ac, ap)?;
    if d1 <= 0 && d2 <= 0 {
        return Ok(ClosestPoint {
            point: triangle[0],
            feature: TriangleFeature::Vertex(0),
        });
    }
    let bp = sub(raw(p), b)?;
    let d3 = dot(ab, bp)?;
    let d4 = dot(ac, bp)?;
    if d3 >= 0 && d4 <= d3 {
        return Ok(ClosestPoint {
            point: triangle[1],
            feature: TriangleFeature::Vertex(1),
        });
    }
    let vc = determinant(d1, d4, d3, d2)?;
    if vc <= 0 && d1 >= 0 && d3 <= 0 {
        return Ok(ClosestPoint {
            point: interpolate(a, ab, d1, checked(d1.checked_sub(d3))?)?,
            feature: TriangleFeature::Edge(0, 1),
        });
    }
    let cp = sub(raw(p), c)?;
    let d5 = dot(ab, cp)?;
    let d6 = dot(ac, cp)?;
    if d6 >= 0 && d5 <= d6 {
        return Ok(ClosestPoint {
            point: triangle[2],
            feature: TriangleFeature::Vertex(2),
        });
    }
    let vb = determinant(d5, d2, d1, d6)?;
    if vb <= 0 && d2 >= 0 && d6 <= 0 {
        return Ok(ClosestPoint {
            point: interpolate(a, ac, d2, checked(d2.checked_sub(d6))?)?,
            feature: TriangleFeature::Edge(0, 2),
        });
    }
    let va = determinant(d3, d6, d5, d4)?;
    let d43 = checked(d4.checked_sub(d3))?;
    let d56 = checked(d5.checked_sub(d6))?;
    if va <= 0 && d43 >= 0 && d56 >= 0 {
        return Ok(ClosestPoint {
            point: interpolate(b, sub(c, b)?, d43, checked(d43.checked_add(d56))?)?,
            feature: TriangleFeature::Edge(1, 2),
        });
    }
    let denom = checked(checked(va.checked_add(vb))?.checked_add(vc))?;
    if denom <= 0 {
        return Err(GeometryError("degenerate triangle"));
    }
    let mut q = a;
    for i in 0..3 {
        let n =
            checked(checked(ab[i].checked_mul(vb))?.checked_add(checked(ac[i].checked_mul(vc))?))?;
        q[i] = checked(a[i].checked_add(n.div_euclid(denom)))?;
    }
    Ok(ClosestPoint {
        point: point(q)?,
        feature: TriangleFeature::Face,
    })
}
fn cross(a: Wide, b: Wide) -> Result<Wide, GeometryError> {
    Ok([
        determinant(a[1], b[2], a[2], b[1])?,
        determinant(a[2], b[0], a[0], b[2])?,
        determinant(a[0], b[1], a[1], b[0])?,
    ])
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
fn normal_distance(v: Wide, fallback: Wide) -> Result<(FPVec3, FP), GeometryError> {
    let d2 = dot(v, v)?;
    let distance = isqrt(d2 as u128) as i128;
    let (v, length) = if distance == 0 {
        (fallback, isqrt(dot(fallback, fallback)? as u128) as i128)
    } else {
        (v, distance)
    };
    if length == 0 {
        return Err(GeometryError("degenerate normal"));
    }
    let mut n = [0; 3];
    for i in 0..3 {
        n[i] = checked(v[i].checked_mul(ONE))? / length;
    }
    Ok((
        point(n)?,
        FP::from_raw(i64::try_from(distance).map_err(|_| GeometryError("distance overflow"))?),
    ))
}
fn vertex(view: &TerrainView<'_>, index: u32) -> Result<FPVec3, GeometryError> {
    let a = view.asset;
    let x = a.origin[0].raw() as i128 + (index % a.width) as i128 * a.spacing.raw() as i128;
    let z = a.origin[1].raw() as i128 + (index / a.width) as i128 * a.spacing.raw() as i128;
    let y = view
        .heights
        .get(index as usize)
        .ok_or(GeometryError("height index"))?
        .0
        .raw() as i128;
    point([x, y, z])
}
fn cell_triangles(view: &TerrainView<'_>, x: u32, z: u32) -> Option<[[u32; 3]; 2]> {
    let a = view.asset;
    if x >= a.width - 1 || z >= a.depth - 1 || view.holes[(z * (a.width - 1) + x) as usize].0 != 0 {
        return None;
    }
    let i = z * a.width + x;
    Some([
        [i, i + a.width, i + a.width + 1],
        [i, i + a.width + 1, i + 1],
    ])
}
fn feature_id(feature: TriangleFeature, ids: [u32; 3], face: u32) -> u32 {
    match feature {
        TriangleFeature::Face => 0x8000_0000 | face,
        TriangleFeature::Vertex(i) => ids[i as usize],
        TriangleFeature::Edge(i, j) => {
            let (a, b) = (ids[i as usize], ids[j as usize]);
            0x4000_0000 | (a.min(b) << 13) | a.max(b)
        }
    }
}
/// Discard edges/vertices hidden by another incident triangle. The derivative
/// of squared distance along any incident tangent must be nonnegative. The
/// tolerance covers the at-most-one-raw-unit final coordinate rounding.
fn local_minimum(
    view: &TerrainView<'_>,
    center: FPVec3,
    q: ClosestPoint,
    ids: [u32; 3],
) -> Result<bool, GeometryError> {
    let (anchor, second) = match q.feature {
        TriangleFeature::Face => return Ok(true),
        TriangleFeature::Vertex(i) => (ids[i as usize], None),
        TriangleFeature::Edge(i, j) => (ids[i as usize], Some(ids[j as usize])),
    };
    let w = view.asset.width;
    let (vx, vz) = (anchor % w, anchor / w);
    let radial = sub(raw(center), raw(q.point))?;
    for z in vz.saturating_sub(1)..=vz.min(view.asset.depth - 2) {
        for x in vx.saturating_sub(1)..=vx.min(w - 2) {
            if let Some(tris) = cell_triangles(view, x, z) {
                for tri in tris {
                    if !tri.contains(&anchor) || second.is_some_and(|v| !tri.contains(&v)) {
                        continue;
                    }
                    for id in tri {
                        let tangent = sub(raw(vertex(view, id)?), raw(q.point))?;
                        let tolerance = (tangent.iter().map(|v| v.abs()).sum::<i128>()
                            + radial.iter().map(|v| v.abs()).sum::<i128>())
                            * 4
                            + 12;
                        if dot(radial, tangent)? > tolerance {
                            return Ok(false);
                        }
                    }
                }
            }
        }
    }
    Ok(true)
}

/// Contacts against the finite canonical triangles. Holes remove both triangles
/// and produce no bottom or wall. Two-sided face normals point toward the sphere;
/// rims and ridges use the actual nearest finite edge/vertex normal.
///
/// `max_candidates` limits triangles visited, including removed hole triangles.
/// Inputs are validated independently; callers must first validate `TerrainView`.
pub fn sphere_contacts(
    view: &TerrainView<'_>,
    center: FPVec3,
    radius: FP,
    margin: FP,
    max_candidates: u32,
) -> Result<Vec<TerrainContact>, GeometryError> {
    if !coordinate_ok(center)
        || !(FP::from_ratio(1, 4)..=FP::from_int(8)).contains(&radius)
        || margin < FP::ZERO
        || margin > FP::from_int(2)
    {
        return Err(GeometryError("sphere query profile"));
    }
    let a = view.asset;
    let reach = radius.raw() as i128 + margin.raw() as i128;
    let axis = |p: FP, o: FP, count: u32| -> Option<(u32, u32)> {
        let delta = p.raw() as i128 - o.raw() as i128;
        let spacing = a.spacing.raw() as i128;
        let max = (count - 1) as i128 * spacing;
        if delta + reach < 0 || delta - reach > max {
            return None;
        }
        let lo = (delta - reach)
            .div_euclid(spacing)
            .max(0)
            .min((count - 2) as i128) as u32;
        let hi = (delta + reach)
            .div_euclid(spacing)
            .max(0)
            .min((count - 2) as i128) as u32;
        Some((lo, hi))
    };
    let Some((x0, x1)) = axis(center.x, a.origin[0], a.width) else {
        return Ok(Vec::new());
    };
    let Some((z0, z1)) = axis(center.z, a.origin[1], a.depth) else {
        return Ok(Vec::new());
    };
    let candidates = (x1 - x0 + 1) as u64 * (z1 - z0 + 1) as u64 * 2;
    if candidates > max_candidates as u64 {
        return Err(GeometryError("terrain candidate budget"));
    }
    let mut contacts = Vec::new();
    for z in z0..=z1 {
        for x in x0..=x1 {
            let Some(triangles) = cell_triangles(view, x, z) else {
                continue;
            };
            for (local, ids) in triangles.into_iter().enumerate() {
                let tri = [
                    vertex(view, ids[0])?,
                    vertex(view, ids[1])?,
                    vertex(view, ids[2])?,
                ];
                let q = closest_point_on_triangle(center, tri)?;
                let radial = sub(raw(center), raw(q.point))?;
                if dot(radial, radial)? > checked(reach.checked_mul(reach))? {
                    continue;
                }
                if !local_minimum(view, center, q, ids)? {
                    continue;
                }
                let fallback = cross(
                    sub(raw(tri[1]), raw(tri[0]))?,
                    sub(raw(tri[2]), raw(tri[0]))?,
                )?;
                let (normal, distance) = normal_distance(radial, fallback)?;
                contacts.push(TerrainContact {
                    point: q.point,
                    normal,
                    separation: distance - radius,
                    feature_id: feature_id(
                        q.feature,
                        ids,
                        ((z * (a.width - 1) + x) * 2) + local as u32,
                    ),
                });
            }
        }
    }
    contacts.sort_unstable_by_key(|c| c.feature_id);
    contacts.dedup_by_key(|c| c.feature_id);
    Ok(contacts)
}
