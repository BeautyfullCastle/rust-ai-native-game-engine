//! Independent quantized-plane interval for the shared-shadow acceptance test.
//! Vulkan v1.3.280 (69ba4fef) limits.adoc:5092 allows four subpixel bits.
//! primsrast.adoc:3796-3812 permits Euclidean or max-component depth slope;
//! :3876 defines the floating depth unit. A full 1/16-pixel perturbation is a
//! conservative MODEL of quantized coordinates, not a universal shader theorem.
//! No observed device depth or selected snapping configuration enters this bound.

const H: f64 = 1.0 / 16.0;
const EDGE_MARGIN: f64 = std::f64::consts::SQRT_2 * H;
const CONSTANT_BIAS_MAX: f64 = 1.0 / 4_194_304.0; // 2 * 2^-23, z in [0,1].
const READBACK_ERROR: f64 = 1.0 / 65_536.0 + 1.0 / 8_388_608.0;
const NUMERICAL_ERROR: f64 = 5e-6; // Existing CPU/GPU plane allowance, unchanged.
// Fail an uninformative oracle; this cap is not an extra comparison tolerance.
const MAX_INTERVAL_WIDTH: f64 = 0.001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Error {
    NonFinite,
    DepthRange,
    Degenerate,
    IllConditioned,
    AmbiguousCoverage,
    Uninformative,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Coverage {
    Inside,
    Outside,
    Ambiguous,
}
#[derive(Debug, Clone, Copy)]
pub(super) struct Interval {
    pub lower: f64,
    pub upper: f64,
}
impl Interval {
    pub fn contains(self, depth: f64) -> bool {
        depth.is_finite() && depth >= self.lower && depth <= self.upper
    }
    pub fn width(self) -> f64 {
        self.upper - self.lower
    }
}
#[derive(Debug, Clone)]
pub(super) struct RasterTriangle {
    p: [[f64; 3]; 3],
    determinant: f64,
    gradient: [f64; 2],
    gradient_error: f64,
    plane_error: f64,
    opposite_edges: [f64; 3],
}
impl RasterTriangle {
    pub fn new(p: [[f32; 3]; 3]) -> Result<Self, Error> {
        let p = p.map(|v| v.map(f64::from));
        if !p.iter().flatten().all(|x| x.is_finite()) {
            return Err(Error::NonFinite);
        }
        if !p.iter().all(|v| (0.0..=1.0).contains(&v[2])) {
            return Err(Error::DepthRange);
        }
        let [a, b, c] = p;
        let matrix = [[b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]]];
        let determinant = matrix[0][0] * matrix[1][1] - matrix[0][1] * matrix[1][0];
        if determinant == 0.0 || !determinant.is_finite() {
            return Err(Error::Degenerate);
        }
        let inverse_norm = (matrix[1][1].abs() + matrix[0][1].abs())
            .max(matrix[1][0].abs() + matrix[0][0].abs())
            / determinant.abs();
        let eta = inverse_norm * 4.0 * H;
        if !eta.is_finite() || eta >= 1.0 {
            return Err(Error::IllConditioned);
        }
        let gradient = [
            ((b[2] - a[2]) * matrix[1][1] - (c[2] - a[2]) * matrix[0][1]) / determinant,
            (matrix[0][0] * (c[2] - a[2]) - matrix[1][0] * (b[2] - a[2])) / determinant,
        ];
        let gradient_error = eta / (1.0 - eta) * gradient[0].abs().max(gradient[1].abs());
        let plane_error = H * (gradient[0].abs() + gradient[1].abs());
        let opposite_edges = std::array::from_fn(|i| {
            let q = p[(i + 1) % 3];
            let r = p[(i + 2) % 3];
            (q[0] - r[0]).hypot(q[1] - r[1])
        });
        Ok(Self {
            p,
            determinant,
            gradient,
            gradient_error,
            plane_error,
            opposite_edges,
        })
    }
    fn barycentric(&self, point: [f64; 2]) -> [f64; 3] {
        let [a, b, c] = self.p;
        let u = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1]))
            / self.determinant;
        let v = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1]))
            / self.determinant;
        [u, v, 1.0 - u - v]
    }
    pub fn coverage(&self, point: [f64; 2]) -> Coverage {
        if !point.iter().all(|x| x.is_finite()) {
            return Coverage::Ambiguous;
        }
        let distances = std::array::from_fn::<_, 3, _>(|i| {
            self.barycentric(point)[i] * self.determinant.abs() / self.opposite_edges[i]
        });
        if distances.iter().any(|&d| d < -EDGE_MARGIN) {
            Coverage::Outside
        } else if distances.iter().all(|&d| d > EDGE_MARGIN) {
            Coverage::Inside
        } else {
            Coverage::Ambiguous
        }
    }
    pub fn interval(&self, point: [f64; 2]) -> Result<Interval, Error> {
        if self.coverage(point) != Coverage::Inside {
            return Err(Error::AmbiguousCoverage);
        }
        let b = self.barycentric(point);
        let z: f64 = b.iter().zip(self.p).map(|(w, p)| w * p[2]).sum();
        let [dx, dy] = self.gradient;
        let minimum_slope = (dx.abs().max(dy.abs()) - self.gradient_error).max(0.0);
        let maximum_slope = dx.hypot(dy) + std::f64::consts::SQRT_2 * self.gradient_error;
        let extra = READBACK_ERROR + NUMERICAL_ERROR;
        let interval = Interval {
            lower: z - self.plane_error + 2.0 * minimum_slope - extra,
            upper: z + self.plane_error + 2.0 * maximum_slope + CONSTANT_BIAS_MAX + extra,
        };
        if interval.lower < 0.0 || interval.upper > 1.0 {
            return Err(Error::DepthRange);
        }
        if !interval.lower.is_finite()
            || !interval.upper.is_finite()
            || interval.width() >= MAX_INTERVAL_WIDTH
        {
            return Err(Error::Uninformative);
        }
        Ok(interval)
    }
}

/// Minimum after bias, including every robust covering triangle. An uncertain
/// competitor is an error, not a reason to hide a potentially winning surface.
pub(super) fn sample(
    triangles: &[RasterTriangle],
    point: [f64; 2],
) -> Result<Option<Interval>, Error> {
    let mut result: Option<Interval> = None;
    for t in triangles {
        match t.coverage(point) {
            Coverage::Outside => (),
            Coverage::Ambiguous => return Err(Error::AmbiguousCoverage),
            Coverage::Inside => {
                let interval = t.interval(point)?;
                result = Some(match result {
                    None => interval,
                    Some(old) => Interval {
                        lower: old.lower.min(interval.lower),
                        upper: old.upper.min(interval.upper),
                    },
                });
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    const P: [[f32; 3]; 3] = [
        [100.0, 100.0, 0.25],
        [500.0, 100.0, 0.35],
        [100.0, 500.0, 0.45],
    ];
    const POINT: [f64; 2] = [200.5, 200.5];
    // Separate scalar reference: determinants/barycentric plane, not interval helper.
    fn plane(p: [[f64; 3]; 3], point: [f64; 2]) -> (f64, f64, f64) {
        let [a, b, c] = p;
        let ux = b[0] - a[0];
        let uy = b[1] - a[1];
        let vx = c[0] - a[0];
        let vy = c[1] - a[1];
        let det = ux * vy - uy * vx;
        let s = ((point[0] - a[0]) * vy - (point[1] - a[1]) * vx) / det;
        let t = (ux * (point[1] - a[1]) - uy * (point[0] - a[0])) / det;
        (
            a[2] + s * (b[2] - a[2]) + t * (c[2] - a[2]),
            ((b[2] - a[2]) * vy - (c[2] - a[2]) * uy) / det,
            (ux * (c[2] - a[2]) - vx * (b[2] - a[2])) / det,
        )
    }
    fn check_variants(p: [[f32; 3]; 3], point: [f64; 2]) {
        let interval = RasterTriangle::new(p).unwrap().interval(point).unwrap();
        assert!(interval.width() < MAX_INTERVAL_WIDTH);
        let original = p.map(|v| v.map(f64::from));
        for mask in 0..64 {
            let perturbed = std::array::from_fn(|i| {
                std::array::from_fn(|k| {
                    original[i][k]
                        + if k < 2 {
                            if mask & (1 << (2 * i + k)) == 0 {
                                -H
                            } else {
                                H
                            }
                        } else {
                            0.0
                        }
                })
            });
            let (z, dx, dy) = plane(perturbed, point);
            for slope in [dx.abs().max(dy.abs()), dx.hypot(dy)] {
                for constant in [0.0, CONSTANT_BIAS_MAX] {
                    assert!(interval.contains(z + 2.0 * slope + constant));
                }
            }
        }
        for bits in [4, 8, 24] {
            let scale = 2.0_f64.powi(bits);
            let snapped = original.map(|v| {
                [
                    (v[0] * scale).round() / scale,
                    (v[1] * scale).round() / scale,
                    v[2],
                ]
            });
            let (z, dx, dy) = plane(snapped, point);
            assert!(interval.contains(z + 2.0 * dx.abs().max(dy.abs())));
            assert!(interval.contains(z + 2.0 * dx.hypot(dy) + CONSTANT_BIAS_MAX));
        }
    }
    #[test]
    fn perturbation_corners_and_precision_variants_stay_in_derived_interval() {
        check_variants(P, POINT);
        check_variants(
            [
                [530.1008, 475.5484, 0.49219415],
                [498.78018, 434.70935, 0.44409862],
                [413.4035, 427.81595, 0.4564609],
            ],
            [496.5, 436.5],
        );
    }
    #[test]
    fn fixed_wrong_depth_pose_and_bias_controls_are_rejected() {
        let interval = RasterTriangle::new(P).unwrap().interval(POINT).unwrap();
        let (z, dx, dy) = plane(P.map(|v| v.map(f64::from)), POINT);
        let slope = dx.abs().max(dy.abs());
        assert!(interval.contains(z + 2.0 * slope));
        for wrong in [
            z,
            z + 4.0 * slope,
            z + 2.0 * slope + 0.01,
            z + 2.0 * slope - 0.01,
        ] {
            assert!(!interval.contains(wrong));
        }
        let wrong_pose = P.map(|v| [v[0] + 20.0, v[1] - 40.0, v[2]]);
        let (wz, wx, wy) = plane(wrong_pose.map(|v| v.map(f64::from)), POINT);
        assert!(!interval.contains(wz + 2.0 * wx.abs().max(wy.abs())));
    }
    #[test]
    fn nonfinite_range_degeneracy_and_ambiguous_coverage_fail() {
        let mut p = P;
        p[0][0] = f32::NAN;
        assert!(matches!(RasterTriangle::new(p), Err(Error::NonFinite)));
        let mut p = P;
        p[0][2] = 1.1;
        assert!(matches!(RasterTriangle::new(p), Err(Error::DepthRange)));
        assert!(matches!(
            RasterTriangle::new([P[0]; 3]),
            Err(Error::Degenerate)
        ));
        let thin = [[0.0, 0.0, 0.2], [0.01, 0.0, 0.3], [0.0, 0.01, 0.4]];
        assert!(matches!(
            RasterTriangle::new(thin),
            Err(Error::IllConditioned)
        ));
        let t = RasterTriangle::new(P).unwrap();
        assert_eq!(t.coverage([100.0, 200.0]), Coverage::Ambiguous);
        assert!(matches!(
            sample(&[t], [100.0, 200.0]),
            Err(Error::AmbiguousCoverage)
        ));
        let steep = [[0.0, 0.0, 0.3], [1.0, 0.0, 0.31], [0.0, 1.0, 0.3]];
        assert!(matches!(
            RasterTriangle::new(steep).unwrap().interval([0.3, 0.3]),
            Err(Error::Uninformative)
        ));
    }
    #[test]
    fn overlapping_depth_competition_uses_biased_intervals() {
        // Lower unbiased plane is steep; the slightly higher flat plane wins after bias.
        let steep = [
            [100.0, 100.0, 0.25],
            [500.0, 100.0, 0.65],
            [100.0, 500.0, 0.25],
        ];
        let (z, dx, dy) = plane(steep.map(|v| v.map(f64::from)), POINT);
        let flat_depth = (z + 0.0008) as f32;
        let flat = steep.map(|v| [v[0], v[1], flat_depth]);
        let a = RasterTriangle::new(steep).unwrap();
        let b = RasterTriangle::new(flat).unwrap();
        let expected = b.interval(POINT).unwrap();
        let actual = sample(&[a, b], POINT).unwrap().unwrap();
        assert_eq!(actual.lower, expected.lower);
        assert_eq!(actual.upper, expected.upper);
        assert!(actual.contains(f64::from(flat_depth)));
        assert!(
            !actual.contains(z + 2.0 * dx.abs().max(dy.abs())),
            "wrong unbiased winner"
        );
    }
}
