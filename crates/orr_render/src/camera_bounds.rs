//! Optional 2D map confinement, independent of target selection or simulation.
use crate::camera::Camera;

impl Camera {
    /// Constrains the visible viewport to an axis-aligned world rectangle.
    ///
    /// Call after follow, pan or zoom and again after viewport resizing. Each
    /// axis is clamped independently; when the viewport is larger than the map,
    /// that axis is centered on the map instead. Zoom is preserved.
    ///
    /// Returns `true` for valid input, including an unchanged pose. Non-finite
    /// camera/bounds, nonpositive extent, reversed or empty bounds, and a zero
    /// viewport dimension return `false` without modifying the camera.
    /// This is a view-only operation; it neither clears a follow target nor
    /// constrains movement in the simulation.
    pub fn constrain_to_bounds(
        &mut self,
        min: [f32; 2],
        max: [f32; 2],
        viewport: (u32, u32),
    ) -> bool {
        if viewport.0 == 0
            || viewport.1 == 0
            || !self.half_extent.is_finite()
            || self.half_extent <= 0.0
            || !self
                .center
                .iter()
                .chain(min.iter())
                .chain(max.iter())
                .all(|v| v.is_finite())
            || min[0] >= max[0]
            || min[1] >= max[1]
        {
            return false;
        }
        // Wider intermediates also handle valid bounds spanning the f32 range.
        let extent = f64::from(self.half_extent);
        let w = f64::from(viewport.0);
        let h = f64::from(viewport.1);
        let visible = [extent * (w / h).max(1.0), extent * (h / w).max(1.0)];
        let mut center = self.center;
        for axis in 0..2 {
            let lo = f64::from(min[axis]);
            let hi = f64::from(max[axis]);
            center[axis] = if visible[axis] >= (hi - lo) * 0.5 {
                ((lo + hi) * 0.5) as f32
            } else {
                f64::from(center[axis]).clamp(lo + visible[axis], hi - visible[axis]) as f32
            };
        }
        self.center = center;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::CameraFollow;

    #[test]
    fn landscape_edges_contain_the_visible_world() {
        let mut camera = Camera::new([-100.0, 100.0], 6.0);
        assert!(camera.constrain_to_bounds([-20.0, -15.0], [20.0, 15.0], (800, 600)));
        assert_eq!(camera.center, [-12.0, 9.0]);
        assert_eq!(
            camera.screen_to_world([0.0, 0.0], (800, 600)),
            [-20.0, 15.0]
        );
        camera.center = [100.0, -100.0];
        assert!(camera.constrain_to_bounds([-20.0, -15.0], [20.0, 15.0], (800, 600)));
        assert_eq!(camera.center, [12.0, -9.0]);
        assert_eq!(
            camera.screen_to_world([800.0, 600.0], (800, 600)),
            [20.0, -15.0]
        );
        assert_eq!(camera.half_extent, 6.0);
    }

    #[test]
    fn portrait_resize_and_zoom_recompute_each_axis() {
        let mut camera = Camera::new([50.0, 50.0], 6.0);
        assert!(camera.constrain_to_bounds([0.0, 0.0], [30.0, 40.0], (600, 1200)));
        assert_eq!(camera.center, [24.0, 28.0]);
        assert!(camera.constrain_to_bounds([0.0, 0.0], [30.0, 40.0], (1200, 600)));
        assert_eq!(camera.center, [18.0, 28.0]);
        assert!(camera.constrain_to_bounds([0.0, 0.0], [30.0, 40.0], (600, 1200)));
        assert_eq!(camera.center, [18.0, 28.0]);
        camera.zoom_at(0.5, [300.0, 600.0], (600, 1200));
        assert!(camera.constrain_to_bounds([0.0, 0.0], [30.0, 40.0], (600, 1200)));
        assert_eq!(camera.center, [18.0, 20.0]);
        assert_eq!(camera.half_extent, 12.0);
    }

    #[test]
    fn small_map_centers_only_the_oversized_axis() {
        let mut camera = Camera::new([40.0, -50.0], 10.0);
        assert!(camera.constrain_to_bounds([2.0, -100.0], [8.0, 100.0], (100, 100)));
        assert_eq!(camera.center, [5.0, -50.0]);
        assert!(camera.constrain_to_bounds([2.0, 3.0], [8.0, 9.0], (100, 100)));
        assert_eq!(camera.center, [5.0, 6.0]);
    }

    #[test]
    fn follow_can_return_from_an_edge_without_losing_target() {
        let mut camera = Camera::new([0.0, 0.0], 5.0);
        let mut follow = CameraFollow::new();
        follow.follow(7_u32);
        for (position, expected) in [([99.0, 99.0], [15.0, 15.0]), ([0.0, 0.0], [0.0, 0.0])] {
            assert!(follow.update(&mut camera, |id| {
                assert_eq!(*id, 7);
                Some(position)
            }));
            assert!(camera.constrain_to_bounds([-20.0, -20.0], [20.0, 20.0], (100, 100)));
            assert_eq!(camera.center, expected);
            assert_eq!(follow.target(), Some(&7));
        }
    }

    #[test]
    fn invalid_inputs_leave_pose_untouched() {
        let original = Camera::new([3.0, 4.0], 5.0);
        for (min, max, viewport) in [
            ([0.0, 0.0], [0.0, 10.0], (100, 100)),
            ([0.0, 11.0], [10.0, 10.0], (100, 100)),
            ([f32::NAN, 0.0], [10.0, 10.0], (100, 100)),
            ([0.0, 0.0], [10.0, f32::INFINITY], (100, 100)),
            ([0.0, 0.0], [10.0, 10.0], (0, 100)),
            ([0.0, 0.0], [10.0, 10.0], (100, 0)),
        ] {
            let mut camera = original;
            assert!(!camera.constrain_to_bounds(min, max, viewport));
            assert_eq!(camera, original);
        }
        for extent in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut camera = Camera {
                half_extent: extent,
                ..original
            };
            let bits = camera.half_extent.to_bits();
            assert!(!camera.constrain_to_bounds([-10.0; 2], [10.0; 2], (100, 100)));
            assert_eq!(camera.center, original.center);
            assert_eq!(camera.half_extent.to_bits(), bits);
        }
        let mut camera = Camera {
            center: [f32::INFINITY, 4.0],
            ..original
        };
        assert!(!camera.constrain_to_bounds([-10.0; 2], [10.0; 2], (100, 100)));
        assert_eq!(camera.center, [f32::INFINITY, 4.0]);
    }

    #[test]
    fn extreme_finite_maps_do_not_overflow() {
        let mut camera = Camera::new([f32::MAX; 2], 1.0);
        assert!(camera.constrain_to_bounds([-f32::MAX; 2], [f32::MAX; 2], (1, u32::MAX)));
        assert!(camera.center.iter().all(|v| v.is_finite()));
        camera.half_extent = f32::MAX;
        assert!(camera.constrain_to_bounds([-f32::MAX; 2], [f32::MAX; 2], (1, u32::MAX)));
        assert_eq!(camera.center, [0.0, 0.0]);
    }
}
