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
}
