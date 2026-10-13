//! Fixed-work conservative continuous sweep. No discrete trajectory sampling.

use crate::{
    CameraBoomError, QueryWork, TriangleId, ARITHMETIC_ENVELOPE, MIN_SPACING, ROOT_REFINEMENTS,
};

type V = [f64; 3];
const PARAMETER_PAD: f64 = 8.0 * f64::EPSILON;
const PARAMETER_ACCURACY: f64 = 1.0 / 281_474_976_710_656.0; // 2^-48

pub(crate) fn sub(a: V, b: V) -> V {
    std::array::from_fn(|axis| a[axis] - b[axis])
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn finite(value: f64) -> Result<f64, CameraBoomError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CameraBoomError::NumericalFailure)
    }
}
pub(crate) fn length(v: V) -> Result<f64, CameraBoomError> {
    for value in v {
        finite(value)?;
    }
    let scale = v.into_iter().map(f64::abs).fold(0.0, f64::max);
    if scale == 0.0 {
        return Ok(0.0);
    }
    let scaled = v.map(|value| value / scale);
    finite(scale * dot(scaled, scaled).sqrt())
}
fn unit(v: V) -> Result<V, CameraBoomError> {
    let norm = length(v)?;
    if norm == 0.0 {
        return Err(CameraBoomError::UnsupportedGeometry(
            "degenerate unit vector",
        ));
    }
    let result = v.map(|value| value / norm);
    if (dot(result, result) - 1.0).abs() > 1.0e-12 {
        return Err(CameraBoomError::NumericalFailure);
    }
    Ok(result)
}

#[derive(Debug)]
pub(crate) struct Triangle {
    pub(crate) id: TriangleId,
    pub(crate) points: [V; 3],
    normal: V,
    inward: [V; 3],
    edge_unit: [V; 3],
    edge_length: [f64; 3],
}
impl Triangle {
    pub(crate) fn new(id: TriangleId, points: [V; 3]) -> Result<Self, CameraBoomError> {
        let [a, b, c] = points;
        // Factor out grid spacing before normalization. Generic cross products
        // of long near-parallel slope edges would have a weaker error bound.
        let spacing = c[0] - a[0];
        if spacing < MIN_SPACING {
            return Err(CameraBoomError::UnsupportedGeometry("camera grid spacing"));
        }
        let slope = match id.triangle {
            0 if b[0] == a[0] && b[2] - a[2] == spacing && c[2] == b[2] => {
                [b[1] - c[1], spacing, a[1] - b[1]]
            }
            1 if b[0] == c[0] && b[2] - a[2] == spacing && c[2] == a[2] => {
                [a[1] - c[1], spacing, c[1] - b[1]]
            }
            _ => {
                return Err(CameraBoomError::UnsupportedGeometry(
                    "noncanonical grid triangle",
                ))
            }
        };
        let normal = unit(slope)?;
        let mut inward = [[0.0; 3]; 3];
        let mut edge_unit = [[0.0; 3]; 3];
        let mut edge_length = [0.0; 3];
        for index in 0..3 {
            let edge = sub(points[(index + 1) % 3], points[index]);
            edge_length[index] = length(edge)?;
            edge_unit[index] = unit(edge)?;
            inward[index] = unit(cross(normal, edge_unit[index]))?;
            if dot(normal, edge).abs() > ARITHMETIC_ENVELOPE
                || dot(inward[index], sub(points[(index + 2) % 3], points[index]))
                    < -ARITHMETIC_ENVELOPE
            {
                return Err(CameraBoomError::UnsupportedGeometry(
                    "uncertain triangle basis",
                ));
            }
        }
        Ok(Self {
            id,
            points,
            normal,
            inward,
            edge_unit,
            edge_length,
        })
    }

    pub(crate) fn contact(
        &self,
        pivot: V,
        delta: V,
        radius: f64,
        limit: f64,
        work: &mut QueryWork,
    ) -> Result<Option<f64>, CameraBoomError> {
        let mut nearest = None;
        work.features += 1;
        let q = sub(pivot, self.points[0]);
        let mut face = Interval::new(limit);
        let slab_radius = radius + ARITHMETIC_ENVELOPE;
        let face_possible = face.slab(
            dot(q, self.normal),
            dot(delta, self.normal),
            -slab_radius,
            slab_radius,
        )?;
        if face_possible {
            let mut inside = true;
            for edge in 0..3 {
                if !face.lower(
                    dot(sub(pivot, self.points[edge]), self.inward[edge]),
                    dot(delta, self.inward[edge]),
                    -ARITHMETIC_ENVELOPE,
                )? {
                    inside = false;
                    break;
                }
            }
            if inside {
                choose(&mut nearest, Some(face.lo));
            }
        }
        for edge in 0..3 {
            work.features += 1;
            let q = sub(pivot, self.points[edge]);
            let direction = self.edge_unit[edge];
            let along_q = finite(dot(q, direction))?;
            let along_delta = finite(dot(delta, direction))?;
            let mut interval = Interval::new(nearest.unwrap_or(limit));
            if interval.slab(
                along_q,
                along_delta,
                -ARITHMETIC_ENVELOPE,
                self.edge_length[edge] + ARITHMETIC_ENVELOPE,
            )? {
                let perpendicular_q =
                    std::array::from_fn(|axis| q[axis] - direction[axis] * along_q);
                let perpendicular_delta =
                    std::array::from_fn(|axis| delta[axis] - direction[axis] * along_delta);
                choose(
                    &mut nearest,
                    radial(
                        perpendicular_q,
                        perpendicular_delta,
                        slab_radius,
                        interval,
                        work,
                    )?,
                );
            }
        }
        for point in self.points {
            work.features += 1;
            let candidate = radial(
                sub(pivot, point),
                delta,
                slab_radius,
                Interval::new(nearest.unwrap_or(limit)),
                work,
            )?;
            choose(&mut nearest, candidate);
        }
        Ok(nearest)
    }
}

fn choose(nearest: &mut Option<f64>, candidate: Option<f64>) {
    if let Some(candidate) = candidate {
        if nearest.is_none_or(|old| candidate < old) {
            *nearest = Some(candidate);
        }
    }
}

#[derive(Clone, Copy)]
struct Interval {
    lo: f64,
    hi: f64,
}
impl Interval {
    fn new(limit: f64) -> Self {
        Self { lo: 0.0, hi: limit }
    }

    fn lower(&mut self, start: f64, slope: f64, bound: f64) -> Result<bool, CameraBoomError> {
        finite(start)?;
        finite(slope)?;
        finite(bound)?;
        let first = finite(start + slope * self.lo)?;
        let last = finite(start + slope * self.hi)?;
        if first >= bound && last >= bound {
            return Ok(true);
        }
        if first < bound && last < bound {
            return Ok(false);
        }
        // Endpoint straddling proves that the required division has a finite,
        // in-segment crossing. Never drop a small nonzero slope.
        if slope == 0.0 {
            return Err(CameraBoomError::NumericalFailure);
        }
        let crossing = finite((bound - start) / slope)?;
        if slope > 0.0 {
            self.lo = self.lo.max((crossing - PARAMETER_PAD).clamp(0.0, 1.0));
        } else {
            self.hi = self.hi.min((crossing + PARAMETER_PAD).clamp(0.0, 1.0));
        }
        Ok(self.lo <= self.hi)
    }

    fn slab(
        &mut self,
        start: f64,
        slope: f64,
        low: f64,
        high: f64,
    ) -> Result<bool, CameraBoomError> {
        Ok(self.lower(start, slope, low)? && self.lower(-start, -slope, -high)?)
    }
}

fn radial(
    q: V,
    delta: V,
    radius: f64,
    interval: Interval,
    work: &mut QueryWork,
) -> Result<Option<f64>, CameraBoomError> {
    let at = |parameter: f64| -> Result<f64, CameraBoomError> {
        length(std::array::from_fn(|axis| {
            q[axis] + delta[axis] * parameter
        }))
    };
    let first = at(interval.lo)?;
    if first <= radius {
        return Ok(Some(interval.lo));
    }
    let speed_squared = finite(dot(delta, delta))?;
    if speed_squared <= 1.0e-24 {
        // Motion too small to safely divide by its squared speed. The reverse
        // triangle inequality can certify separation; otherwise fail closed.
        return Ok(
            (first - speed_squared.sqrt() * (interval.hi - interval.lo) <= radius)
                .then_some(interval.lo),
        );
    }
    let minimum = finite(-finite(dot(q, delta))? / speed_squared)?.clamp(interval.lo, interval.hi);
    if at(minimum)? > radius {
        return Ok(None);
    }
    // A convex radial-distance minimum inside the expanded obstacle proves a
    // continuous contact exists. Refine only the monotone entry interval; this
    // never relies on discrete trajectory samples to discover an obstacle.
    let mut low = interval.lo;
    let mut high = minimum;
    let radius_squared = finite(radius * radius)?;
    let squared_at = |parameter: f64| -> Result<f64, CameraBoomError> {
        let point = std::array::from_fn(|axis| q[axis] + delta[axis] * parameter);
        // Nonnegative-square summation has no cancellation. Near the radius
        // boundary, comparison rounding is <=8*u*radius in length units
        // (<2e-11 here), in addition to the existing point/projection envelope.
        finite(dot(point, point))
    };
    // An analytic root is only an untrusted hint. In particular, catastrophic
    // discriminant cancellation NEVER rules out the already-proven contact.
    // Convexity plus checked outside/inside endpoints certifies a useful pair;
    // every failed hint condition keeps the full, already-proven bracket.
    let b = finite(dot(q, delta))?;
    let c = finite(dot(q, q) - radius_squared)?;
    if b < 0.0 && c > 0.0 {
        let discriminant = finite(b * b - speed_squared * c)?;
        if discriminant >= 0.0 {
            let denominator = finite(-b + discriminant.sqrt())?;
            if denominator > 0.0 {
                let hint = finite(c / denominator)?;
                let pad = ARITHMETIC_ENVELOPE / speed_squared.sqrt() + 32.0 * f64::EPSILON;
                let trial_low = (hint - pad).clamp(low, high);
                let trial_high = (hint + pad).clamp(low, high);
                if trial_low < trial_high {
                    work.bracket_probes += 2;
                    let outside = squared_at(trial_low)? > radius_squared;
                    let inside = squared_at(trial_high)? <= radius_squared;
                    if outside && inside && (trial_low > low || trial_high < high) {
                        work.brackets_narrowed += 1;
                        low = trial_low;
                        high = trial_high;
                    }
                }
            }
        }
    }
    for _ in 0..ROOT_REFINEMENTS {
        // Preserve the original worst-case spatial bracket width. Narrowing
        // the certified bracket only saves redundant iterations, not accuracy.
        if high - low <= PARAMETER_ACCURACY {
            break;
        }
        work.root_refinements += 1;
        let middle = low + (high - low) * 0.5;
        if squared_at(middle)? <= radius_squared {
            high = middle;
        } else {
            low = middle;
        }
    }
    Ok(Some((low - PARAMETER_PAD).max(0.0)))
}
