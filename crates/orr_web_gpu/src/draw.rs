//! Draw lists (integers from `orr_web`) to render lists (`orr_render`). No GPU: tested natively.

use orr_render::{Camera, RenderList};

/// Integers per body in the physics draw list (`orr_web::PHYS_STRIDE`).
const PHYS_STRIDE: usize = 8;
const DRAW_CIRCLE: i32 = 0;
const DRAW_QUAD: i32 = 1;
/// Half the side of the arena's square view in world units (as in `web/index.html`).
pub const ARENA_HALF: f32 = 2000.0;

const SLOT_COLORS: [[f32; 3]; 8] = [
    [0.306, 0.631, 1.0],
    [1.0, 0.42, 0.42],
    [0.373, 0.831, 0.541],
    [1.0, 0.824, 0.306],
    [0.773, 0.545, 1.0],
    [1.0, 0.616, 0.306],
    [0.306, 0.863, 0.847],
    [0.91, 0.91, 0.91],
];
const PADDLES: [[f32; 3]; 4] = [[0.25, 0.63, 1.0], [1.0, 0.55, 0.2], [0.4, 0.9, 0.4], [0.95, 0.4, 0.75]];
const BASE: [[f32; 3]; 3] = [[0.25, 0.85, 0.75], [0.95, 0.72, 0.28], [0.85, 0.42, 0.9]];

/// Colors are written as the sRGB values a viewer sees. An sRGB surface encodes on write, so
/// those values are made linear first; a plain `Unorm` surface (the browser's usual canvas
/// format) stores them as they are.
fn tint(c: [f32; 3], k: f32, srgb_surface: bool) -> [f32; 4] {
    let f = |v: f32| {
        let v = (v * k).clamp(0.0, 1.0);
        if srgb_surface {
            v.powf(2.2)
        } else {
            v
        }
    };
    [f(c[0]), f(c[1]), f(c[2]), 1.0]
}

/// The physics scene: the box floor, then every body. `data` is `orr_web::render_phys`'s output,
/// `scene_box` is `[half width, height]` in world units.
pub fn phys_list(data: &[i32], scene_box: &[i32], srgb_surface: bool) -> Option<(RenderList, Camera)> {
    let [half_w, height] = <[i32; 2]>::try_from(scene_box).ok()?;
    let (half_w, height) = (half_w as f32, height as f32);
    let mut list = RenderList::new();
    list.quad([0.0, height / 2.0], [half_w, height / 2.0], 0.0, tint([0.07, 0.07, 0.125], 1.0, srgb_surface));
    for b in data.chunks_exact(PHYS_STRIDE) {
        let (shape, class) = (b[0], b[1]);
        let q = |v: i32| v as f32 / 256.0;
        let (center, rot, size, half_y) = ([q(b[2]), q(b[3])], q(b[4]), q(b[5]), q(b[6]));
        let color = match class {
            0 => {
                let k = 0.4 + 0.6 * (b[7] as f32 / 256.0).min(1.0);
                tint(BASE[(shape as usize).min(2)], k, srgb_surface)
            }
            1 => tint([0.36, 0.38, 0.47], 1.0, srgb_surface),
            2 => tint([0.72, 0.42, 0.86], 1.0, srgb_surface),
            slot => tint(PADDLES[(slot as usize - 3) % PADDLES.len()], 1.0, srgb_surface),
        };
        match shape {
            DRAW_CIRCLE => list.circle(center, size, color),
            DRAW_QUAD => list.quad(center, [size, half_y], rot, color),
            _ => list.capsule(center, size, half_y, rot, color),
        }
    }
    Some((list, Camera::new([0.0, height / 2.0], half_w.max(height / 2.0) * 1.06)))
}

/// The arena: players (circles) and bullets, `orr_web::render_arena`'s four integers per entity.
pub fn arena_list(data: &[i32], srgb_surface: bool) -> (RenderList, Camera) {
    let mut list = RenderList::new();
    for e in data.chunks_exact(4) {
        let (kind, slot) = (e[0], e[1] as usize);
        let radius = if kind == 0 { 56.0 } else { 19.0 };
        list.circle([e[2] as f32, e[3] as f32], radius, tint(SLOT_COLORS[slot % SLOT_COLORS.len()], 1.0, srgb_surface));
    }
    (list, Camera::new([0.0, 0.0], ARENA_HALF))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_constants_match_orr_web() {
        assert_eq!(PHYS_STRIDE, orr_web::PHYS_STRIDE);
        assert_eq!(DRAW_CIRCLE, orr_web::DRAW_CIRCLE);
        assert_eq!(DRAW_QUAD, orr_web::DRAW_QUAD);
        assert_eq!(2, orr_web::DRAW_CAPSULE);
    }

    #[test]
    fn a_physics_list_has_the_floor_and_one_shape_per_body() {
        // A circle (dynamic), a box (static) and a capsule (paddle of slot 1).
        let data = [
            0, 0, 256 * 3, 256 * 4, 0, 128, 0, 100, //
            1, 1, 0, 0, 0, 256 * 5, 256, 0, //
            2, 4, 256 * 2, 256 * 2, 256, 256 * 2, 128, 0,
        ];
        let (list, cam) = phys_list(&data, &[20, 40], true).expect("list");
        assert_eq!(list.shapes.len(), 4);
        assert_eq!(list.shapes[1].center, [3.0, 4.0]);
        assert_eq!(list.shapes[1].half_size, [0.5, 0.5]);
        assert_eq!(list.shapes[3].half_size, [2.0, 0.5]);
        assert!((list.shapes[3].rot - 1.0 / 256.0 * 256.0).abs() < 1e-6);
        assert_eq!(cam.center, [0.0, 20.0]);
        assert!(cam.half_extent > 20.0);
        assert!(phys_list(&data, &[], true).is_none(), "no scene box yet");
    }

    #[test]
    fn srgb_surfaces_get_linear_colors() {
        let data = [1, 1, 0, 0, 0, 256, 256, 0];
        let lin = phys_list(&data, &[10, 10], true).unwrap().0.shapes[1].color;
        let raw = phys_list(&data, &[10, 10], false).unwrap().0.shapes[1].color;
        assert!(lin[0] < raw[0] && (raw[0] - 0.36).abs() < 1e-6);
    }

    #[test]
    fn the_arena_list_draws_players_larger_than_bullets() {
        let (list, cam) = arena_list(&[0, 1, 100, -200, 1, 1, 0, 0], false);
        assert_eq!(list.shapes.len(), 2);
        assert!(list.shapes[0].half_size[0] > list.shapes[1].half_size[0]);
        assert_eq!(cam.half_extent, ARENA_HALF);
    }
}
