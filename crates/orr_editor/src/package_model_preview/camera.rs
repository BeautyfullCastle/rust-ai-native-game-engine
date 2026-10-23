//! Finite model bounds and view-only camera arithmetic.
use orr_model::StaticModel;
use orr_render::{Camera3D, Projection};

const MAX_TARGET_SIZE: u32 = 1024;
const MAX_WORLD: f64 = 1.0e9;
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 - 0.05;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreviewBounds {
    pub center: [f32; 3],
    pub radius: f32,
}

impl PreviewBounds {
    pub(super) fn from_model(model: &StaticModel) -> Result<Self, String> {
        // StaticModel admission already bounds total vertices. Use transformed
        // positions, including imported node transforms, not source-local bounds.
        Self::from_points(model.source().primitives.iter().flat_map(|primitive| {
            primitive.vertices.iter().map(|vertex| {
                std::array::from_fn(|axis| {
                    (0..3)
                        .map(|column| {
                            f64::from(primitive.transform[column][axis])
                                * f64::from(vertex.position[column])
                        })
                        .sum::<f64>()
                        + f64::from(primitive.transform[3][axis])
                })
            })
        }))
    }

    #[cfg(feature = "room-animated-models")]
    pub(super) fn from_animation(bounds: orr_model::animation::Bounds) -> Result<Self, String> {
        if (0..3).any(|axis| bounds.min[axis] > bounds.max[axis]) {
            return Err("animated preview bounds are inverted".into());
        }
        Self::from_points([bounds.min.map(f64::from), bounds.max.map(f64::from)])
    }

    fn from_points(points: impl IntoIterator<Item = [f64; 3]>) -> Result<Self, String> {
        let mut minimum = [f64::INFINITY; 3];
        let mut maximum = [f64::NEG_INFINITY; 3];
        let mut count = 0usize;
        for point in points {
            if point
                .iter()
                .any(|value| !value.is_finite() || value.abs() > MAX_WORLD)
            {
                return Err("model preview requires finite, bounded transformed vertices".into());
            }
            count += 1;
            if count > orr_model::MAX_VERTICES {
                return Err("model preview exceeds the admitted vertex limit".into());
            }
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(point[axis]);
                maximum[axis] = maximum[axis].max(point[axis]);
            }
        }
        if count == 0 {
            return Err("model preview has no vertices".into());
        }
        let center = std::array::from_fn(|axis| ((minimum[axis] + maximum[axis]) * 0.5) as f32);
        // The half-diagonal encloses the model at every orbit angle. Preserve a
        // little space around point/flat geometry, without inventing large bounds.
        let radius = (minimum
            .iter()
            .zip(maximum)
            .map(|(min, max)| {
                let half = (max - min) * 0.5;
                half * half
            })
            .sum::<f64>()
            .sqrt())
        .max(0.125) as f32;
        Ok(Self { center, radius })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreviewCamera {
    angles: [f32; 2],
    zoom_factor: f32,
}

impl Default for PreviewCamera {
    fn default() -> Self {
        Self {
            angles: [0.45, 0.25],
            zoom_factor: 1.0,
        }
    }
}

impl PreviewCamera {
    pub fn reset(&mut self) -> bool {
        let changed = *self != Self::default();
        *self = Self::default();
        changed
    }

    pub fn orbit(&mut self, delta: [f32; 2]) -> bool {
        if !delta.into_iter().all(f32::is_finite) || delta == [0.0, 0.0] {
            return false;
        }
        let angles = [
            (self.angles[0] - delta[0].clamp(-1.0e6, 1.0e6) * 0.01 + std::f32::consts::PI)
                .rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI,
            (self.angles[1] - delta[1].clamp(-1.0e6, 1.0e6) * 0.01).clamp(-MAX_PITCH, MAX_PITCH),
        ];
        let changed = self.angles != angles;
        self.angles = angles;
        changed
    }

    pub fn zoom(&mut self, scroll: f32) -> bool {
        if !scroll.is_finite() {
            return false;
        }
        let zoom = (self.zoom_factor * (scroll.clamp(-32.0, 32.0) * 0.15).exp()).clamp(0.125, 8.0);
        let changed = self.zoom_factor != zoom;
        self.zoom_factor = zoom;
        changed
    }

    pub fn camera(&self, bounds: PreviewBounds, size: (u32, u32)) -> Camera3D {
        let size = bounded_size(size);
        let aspect = size.0 as f32 / size.1 as f32;
        let half_height = bounds.radius * 1.2 / aspect.min(1.0) / self.zoom_factor;
        let [yaw, pitch] = self.angles;
        let (sin_yaw, cos_yaw) = yaw.sin_cos();
        let (sin_pitch, cos_pitch) = pitch.sin_cos();
        let direction = [sin_yaw * cos_pitch, sin_pitch, cos_yaw * cos_pitch];
        // A tiny model far from the origin needs enough eye separation to survive
        // f32 rounding. This is a depth adjustment only, not a zoom adjustment.
        let precision_depth = bounds
            .center
            .iter()
            .map(|value| value.abs())
            .fold(1.0_f32, f32::max)
            * f32::EPSILON
            * 32.0;
        let depth = (bounds.radius * 4.0).max(1.0).max(precision_depth);
        let eye = std::array::from_fn(|axis| bounds.center[axis] + direction[axis] * depth);
        let mut camera = Camera3D::orthographic(eye, bounds.center, half_height);
        let extent = depth + bounds.radius * 2.0 + precision_depth;
        camera.projection = Projection::Orthographic {
            half_height,
            near: -extent,
            far: extent,
        };
        camera
    }
}

pub(crate) fn bounded_size(size: (u32, u32)) -> (u32, u32) {
    (
        size.0.clamp(1, MAX_TARGET_SIZE),
        size.1.clamp(1, MAX_TARGET_SIZE),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_and_empty_bounds_are_rejected() {
        for points in [
            vec![],
            vec![[f64::NAN, 0.0, 0.0]],
            vec![[f64::INFINITY; 3]],
            vec![[MAX_WORLD + 1.0; 3]],
        ] {
            assert!(PreviewBounds::from_points(points).is_err());
        }
    }

    #[test]
    fn bounds_cover_diagonal_and_degenerate_vertices() {
        let bounds = PreviewBounds::from_points([[-2.0, 3.0, -4.0], [4.0, 7.0, 2.0]]).unwrap();
        assert_eq!(bounds.center, [1.0, 5.0, -1.0]);
        assert!((bounds.radius - 22.0_f32.sqrt()).abs() < 1.0e-6);
        let point = PreviewBounds::from_points([[17.0; 3]]).unwrap();
        assert_eq!(point.center, [17.0; 3]);
        assert_eq!(point.radius, 0.125);
    }

    #[test]
    fn camera_fits_all_bounds_corners_across_orbits_and_aspects() {
        let bounds = PreviewBounds::from_points([[-3.0, -2.0, -1.0], [3.0, 2.0, 1.0]]).unwrap();
        for size in [(512, 512), (128, 768), (768, 128)] {
            let mut controls = PreviewCamera::default();
            for _ in 0..12 {
                let camera = controls.camera(bounds, size);
                for x in [-3.0, 3.0] {
                    for y in [-2.0, 2.0] {
                        for z in [-1.0, 1.0] {
                            let screen = camera.world_to_screen([x, y, z], size).unwrap();
                            assert!(screen[0] >= 0.0 && screen[0] <= size.0 as f32);
                            assert!(screen[1] >= 0.0 && screen[1] <= size.1 as f32);
                        }
                    }
                }
                controls.orbit([31.0, 17.0]);
            }
        }
    }

    #[test]
    fn camera_input_and_viewport_sizes_are_bounded_and_resettable() {
        let mut camera = PreviewCamera::default();
        let initial = camera;
        assert!(!camera.orbit([f32::NAN, 0.0]));
        assert!(!camera.zoom(f32::INFINITY));
        assert_eq!(camera, initial);
        camera.orbit([f32::MAX, -f32::MAX]);
        camera.zoom(f32::MAX);
        assert_eq!(camera.zoom_factor, 8.0);
        assert!(camera.angles[1].abs() <= MAX_PITCH);
        camera.zoom(-f32::MAX);
        camera.zoom(-f32::MAX);
        assert_eq!(camera.zoom_factor, 0.125);
        assert!(camera.reset());
        assert_eq!(camera, initial);
        assert_eq!(bounded_size((0, u32::MAX)), (1, 1024));
    }

    #[test]
    fn extreme_admitted_bounds_keep_camera_matrices_finite() {
        for bounds in [
            PreviewBounds::from_points([[MAX_WORLD; 3]]).unwrap(),
            PreviewBounds::from_points([[-MAX_WORLD; 3], [MAX_WORLD; 3]]).unwrap(),
        ] {
            let camera = PreviewCamera::default().camera(bounds, (0, u32::MAX));
            assert!(camera
                .eye
                .into_iter()
                .chain(camera.target)
                .all(f32::is_finite));
            assert_ne!(camera.eye, camera.target);
            assert!(camera
                .view_proj(1.0 / 1024.0)
                .0
                .into_iter()
                .flatten()
                .all(f32::is_finite));
            assert!(
                camera
                    .forward()
                    .into_iter()
                    .map(|value| value * value)
                    .sum::<f32>()
                    > 0.9
            );
        }
    }
}
