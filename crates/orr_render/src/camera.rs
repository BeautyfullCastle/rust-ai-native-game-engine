//! 2D camera: pan, zoom and the world/screen transform.
//!
//! World space: y up. Screen space: pixels, origin at the top left, y down.

/// Looks at `center` and shows `half_extent` world units from the center to
/// the nearer edge of the viewport (so the whole square of side
/// `2 * half_extent` is always visible).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub center: [f32; 2],
    pub half_extent: f32,
}

impl Camera {
    pub const MIN_HALF_EXTENT: f32 = 1e-4;

    pub fn new(center: [f32; 2], half_extent: f32) -> Self {
        Self { center, half_extent: half_extent.max(Self::MIN_HALF_EXTENT) }
    }

    /// World to clip scale for a viewport of `width` x `height` pixels.
    pub fn scale(&self, width: u32, height: u32) -> [f32; 2] {
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        let per_unit = 1.0 / self.half_extent;
        if w >= h {
            [per_unit * h / w, per_unit]
        } else {
            [per_unit, per_unit * w / h]
        }
    }

    /// Pixels per world unit.
    pub fn pixels_per_unit(&self, width: u32, height: u32) -> f32 {
        width.min(height).max(1) as f32 * 0.5 / self.half_extent
    }

    /// Screen position (pixels, top left origin) of a world point.
    pub fn world_to_screen(&self, world: [f32; 2], viewport: (u32, u32)) -> [f32; 2] {
        let ppu = self.pixels_per_unit(viewport.0, viewport.1);
        [
            viewport.0 as f32 * 0.5 + (world[0] - self.center[0]) * ppu,
            viewport.1 as f32 * 0.5 - (world[1] - self.center[1]) * ppu,
        ]
    }

    /// World point under a screen position.
    pub fn screen_to_world(&self, screen: [f32; 2], viewport: (u32, u32)) -> [f32; 2] {
        let ppu = self.pixels_per_unit(viewport.0, viewport.1);
        [
            self.center[0] + (screen[0] - viewport.0 as f32 * 0.5) / ppu,
            self.center[1] - (screen[1] - viewport.1 as f32 * 0.5) / ppu,
        ]
    }

    /// Moves the view so the content follows a mouse drag of `delta` pixels.
    pub fn pan_pixels(&mut self, delta: [f32; 2], viewport: (u32, u32)) {
        let ppu = self.pixels_per_unit(viewport.0, viewport.1);
        self.center[0] -= delta[0] / ppu;
        self.center[1] += delta[1] / ppu;
    }

    /// Zooms in by `factor` (> 1 zooms in) keeping the world point under
    /// `anchor` (a screen position) where it is.
    pub fn zoom_at(&mut self, factor: f32, anchor: [f32; 2], viewport: (u32, u32)) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let before = self.screen_to_world(anchor, viewport);
        self.half_extent = (self.half_extent / factor).max(Self::MIN_HALF_EXTENT);
        let after = self.screen_to_world(anchor, viewport);
        self.center[0] += before[0] - after[0];
        self.center[1] += before[1] - after[1];
    }
}

/// Optional target-follow control for an existing [`Camera`].
///
/// `T` is a caller-owned stable target key (for example a generation-aware entity
/// handle). The caller resolves it into a render-world position on each
/// update; this type has no dependency on a simulation or scene representation.
/// Following snaps to that position without smoothing, elapsed time or retained
/// velocity. It never changes the camera's zoom.
///
/// A missing or non-finite position holds the last camera center and retains the
/// target, allowing a temporarily unavailable view to return. Call [`Self::stop`]
/// when a target is permanently removed, play stops, or the scene is replaced.
/// Route manual navigation through this controller's pan/zoom methods to
/// disengage follow. Direct edits to [`Camera`] remain supported, but do not
/// change this separate controller's state.
///
/// # View-loop integration
///
/// After updating the view's entity positions, resolve the selected target and
/// update the camera before rendering. Apply manual input after the follow
/// update (or before it; manual navigation disengages follow either way).
///
/// ```
/// use orr_render::{camera::CameraFollow, Camera};
///
/// let mut camera = Camera::new([0.0, 0.0], 10.0);
/// let mut follow = CameraFollow::new();
/// follow.follow(42_u64); // The sample owns target selection and lifetime.
/// let view_positions = [(42_u64, [3.0, 5.0])];
/// follow.update(&mut camera, |target| {
///     view_positions.iter().find(|(id, _)| id == target).map(|(_, p)| *p)
/// });
/// assert_eq!(camera.center, [3.0, 5.0]);
/// follow.pan_pixels(&mut camera, [30.0, 0.0], (800, 600));
/// assert_eq!(follow.target(), None); // Manual navigation takes control.
/// follow.stop(); // Also call on play-stop / scene replacement.
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraFollow<T> {
    target: Option<T>,
}

impl<T> Default for CameraFollow<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> CameraFollow<T> {
    /// Starts in manual mode, without a target.
    pub const fn new() -> Self {
        Self { target: None }
    }

    /// Selects or replaces the target. The camera moves on the next [`Self::update`].
    pub fn follow(&mut self, target: T) {
        self.target = Some(target);
    }

    /// The selected target, including while its position is temporarily missing.
    pub fn target(&self) -> Option<&T> {
        self.target.as_ref()
    }

    /// Returns to manual mode without changing the current camera pose.
    pub fn stop(&mut self) {
        self.target = None;
    }

    /// Resolves the selected target once and snaps to its finite world position.
    ///
    /// Returns `true` when a valid position was applied (even if unchanged).
    /// Returns `false` and leaves the camera untouched in manual mode, or when
    /// resolution returns `None` or a non-finite coordinate. In manual mode the
    /// resolver is not called. The resolver must honor the full target identity,
    /// including any entity generation, rather than silently selecting another
    /// entity with a recycled index.
    pub fn update(
        &self,
        camera: &mut Camera,
        resolve: impl FnOnce(&T) -> Option<[f32; 2]>,
    ) -> bool {
        let Some(target) = self.target.as_ref() else { return false };
        let Some(position) = resolve(target) else { return false };
        if !position.iter().all(|v| v.is_finite()) {
            return false;
        }
        camera.center = position;
        true
    }

    /// A finite, nonzero manual drag disengages follow, then uses [`Camera::pan_pixels`].
    /// Zero and non-finite deltas are ignored without changing the follow mode.
    pub fn pan_pixels(&mut self, camera: &mut Camera, delta: [f32; 2], viewport: (u32, u32)) {
        if delta == [0.0, 0.0] || !delta.iter().all(|v| v.is_finite()) {
            return;
        }
        self.stop();
        camera.pan_pixels(delta, viewport);
    }

    /// A valid manual anchored zoom disengages follow, then uses [`Camera::zoom_at`].
    /// This prevents the next follow update from undoing the anchor's center shift.
    /// Nonpositive/non-finite factors, factor 1 and non-finite anchors are ignored.
    pub fn zoom_at(
        &mut self,
        camera: &mut Camera,
        factor: f32,
        anchor: [f32; 2],
        viewport: (u32, u32),
    ) {
        if !factor.is_finite() || factor <= 0.0 || factor == 1.0
            || !anchor.iter().all(|v| v.is_finite())
        {
            return;
        }
        self.stop();
        camera.zoom_at(factor, anchor, viewport);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VP: (u32, u32) = (800, 600);

    #[test]
    fn center_maps_to_screen_center_and_y_is_flipped() {
        let cam = Camera::new([10.0, -5.0], 20.0);
        assert_eq!(cam.world_to_screen([10.0, -5.0], VP), [400.0, 300.0]);
        // The nearer edge (height) shows 20 units: 300 px = 20 units.
        let up = cam.world_to_screen([10.0, 15.0], VP);
        assert!((up[1] - 0.0).abs() < 1e-3 && (up[0] - 400.0).abs() < 1e-3, "{up:?}");
    }

    #[test]
    fn round_trip() {
        let cam = Camera::new([3.0, 4.0], 7.5);
        for p in [[0.0, 0.0], [-2.5, 9.0], [3.0, 4.0]] {
            let back = cam.screen_to_world(cam.world_to_screen(p, VP), VP);
            assert!((back[0] - p[0]).abs() < 1e-4 && (back[1] - p[1]).abs() < 1e-4);
        }
    }

    #[test]
    fn zoom_keeps_the_anchor_fixed() {
        let mut cam = Camera::new([0.0, 0.0], 10.0);
        let anchor = [650.0, 120.0];
        let world = cam.screen_to_world(anchor, VP);
        cam.zoom_at(2.0, anchor, VP);
        assert!((cam.half_extent - 5.0).abs() < 1e-6);
        let after = cam.screen_to_world(anchor, VP);
        assert!((after[0] - world[0]).abs() < 1e-4 && (after[1] - world[1]).abs() < 1e-4);
    }

    #[test]
    fn pan_follows_the_mouse() {
        let mut cam = Camera::new([0.0, 0.0], 10.0);
        let world = cam.screen_to_world([100.0, 100.0], VP);
        cam.pan_pixels([30.0, -20.0], VP);
        let now = cam.world_to_screen(world, VP);
        assert!((now[0] - 130.0).abs() < 1e-3 && (now[1] - 80.0).abs() < 1e-3, "{now:?}");
    }

    #[test]
    fn follow_starts_manual_without_requiring_a_default_target() {
        struct Target;
        let follow = CameraFollow::<Target>::default();
        let mut cam = Camera::new([1.0, 2.0], 10.0);
        let before = cam;
        assert!(follow.target().is_none());
        assert!(!follow.update(&mut cam, |_| panic!("manual mode must not resolve")));
        assert_eq!(cam, before);
    }

    #[test]
    fn follow_resolves_selected_target_once_and_preserves_zoom() {
        let mut follow = CameraFollow::new();
        let mut cam = Camera::new([1.0, 2.0], 7.5);
        follow.follow(42);
        assert_eq!(cam.center, [1.0, 2.0]);
        let mut calls = 0;
        assert!(follow.update(&mut cam, |target| {
            calls += 1;
            assert_eq!(*target, 42);
            Some([3.0, -5.0])
        }));
        assert_eq!(calls, 1);
        assert_eq!(cam.center, [3.0, -5.0]);
        assert_eq!(cam.half_extent, 7.5);
        assert_eq!(cam.world_to_screen([3.0, -5.0], VP), [400.0, 300.0]);
        assert!(follow.update(&mut cam, |_| Some([9.0, 12.0])));
        assert_eq!(cam.center, [9.0, 12.0]);
    }

    #[test]
    fn missing_and_invalid_targets_hold_pose_then_recover() {
        let mut follow = CameraFollow::new();
        let mut cam = Camera::new([1.0, 2.0], 10.0);
        let before = cam;
        follow.follow(42);
        for position in [None, Some([f32::NAN, 0.0]), Some([0.0, f32::INFINITY]),
            Some([f32::NEG_INFINITY, 0.0])]
        {
            assert!(!follow.update(&mut cam, |_| position));
            assert_eq!(cam, before);
            assert_eq!(follow.target(), Some(&42));
        }
        assert!(follow.update(&mut cam, |_| Some([6.0, 7.0])));
        assert_eq!(cam.center, [6.0, 7.0]);
    }

    #[test]
    fn switching_targets_and_stopping_do_not_reuse_old_positions() {
        let mut follow = CameraFollow::new();
        let mut cam = Camera::new([0.0, 0.0], 10.0);
        follow.follow((7, 1));
        follow.update(&mut cam, |_| Some([1.0, 2.0]));
        follow.follow((7, 2));
        assert!(!follow.update(&mut cam, |key| {
            assert_eq!(*key, (7, 2));
            None
        }));
        assert_eq!(cam.center, [1.0, 2.0]);
        follow.stop();
        follow.stop();
        assert_eq!(follow.target(), None);
        assert!(!follow.update(&mut cam, |_| panic!("stopped follow must not resolve")));
        assert_eq!(cam.center, [1.0, 2.0]);
        follow.follow((8, 1));
        assert!(follow.update(&mut cam, |_| Some([-1.0, -2.0])));
        assert_eq!(cam.center, [-1.0, -2.0]);
    }

    #[test]
    fn manual_pan_disengages_and_matches_existing_pan() {
        let mut follow = CameraFollow::new();
        let mut cam = Camera::new([1.0, 2.0], 10.0);
        follow.follow(42);
        let mut expected = cam;
        expected.pan_pixels([30.0, -20.0], VP);
        follow.pan_pixels(&mut cam, [30.0, -20.0], VP);
        assert_eq!(cam, expected);
        assert_eq!(follow.target(), None);
        assert!(!follow.update(&mut cam, |_| Some([99.0, 99.0])));
        assert_eq!(cam, expected);
    }

    #[test]
    fn anchored_zoom_disengages_and_keeps_anchor_fixed() {
        let mut follow = CameraFollow::new();
        let mut cam = Camera::new([1.0, 2.0], 10.0);
        follow.follow(42);
        let anchor = [650.0, 120.0];
        let world = cam.screen_to_world(anchor, VP);
        let mut expected = cam;
        expected.zoom_at(2.0, anchor, VP);
        follow.zoom_at(&mut cam, 2.0, anchor, VP);
        assert_eq!(cam, expected);
        assert_eq!(follow.target(), None);
        let after = cam.screen_to_world(anchor, VP);
        assert!((after[0] - world[0]).abs() < 1e-4 && (after[1] - world[1]).abs() < 1e-4);
        assert!(!follow.update(&mut cam, |_| Some([99.0, 99.0])));
        assert_eq!(cam, expected);
    }

    #[test]
    fn invalid_and_empty_manual_input_preserves_follow() {
        let mut follow = CameraFollow::new();
        let mut cam = Camera::new([1.0, 2.0], 10.0);
        follow.follow(42);
        let before = cam;
        for delta in [[0.0, 0.0], [f32::NAN, 1.0], [0.0, f32::INFINITY]] {
            follow.pan_pixels(&mut cam, delta, VP);
            assert_eq!(cam, before);
            assert_eq!(follow.target(), Some(&42));
        }
        for factor in [0.0, -1.0, 1.0, f32::NAN, f32::INFINITY] {
            follow.zoom_at(&mut cam, factor, [200.0, 300.0], VP);
            assert_eq!(cam, before);
            assert_eq!(follow.target(), Some(&42));
        }
        follow.zoom_at(&mut cam, 2.0, [f32::NAN, 0.0], VP);
        assert_eq!(cam, before);
        assert_eq!(follow.target(), Some(&42));
    }

    #[test]
    fn follow_is_independent_of_update_count_and_handles_zero_viewport() {
        let mut follow = CameraFollow::new();
        let mut cam = Camera::new([0.0, 0.0], 10.0);
        follow.follow(42);
        for _ in 0..20 {
            assert!(follow.update(&mut cam, |_| Some([3.0, 4.0])));
            assert_eq!(cam.center, [3.0, 4.0]);
        }
        follow.pan_pixels(&mut cam, [1.0, -1.0], (0, 0));
        assert!(cam.center.iter().all(|v| v.is_finite()));
        assert_eq!(follow.target(), None);
    }

}
