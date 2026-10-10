//! Opt-in studio look for the flat Room sample. All geometry is presentation-only:
//! floor inlays and wall-mounted trim never change actors, collisions, or saves.
//! Admission bounds the complete presentation to at most 967 instances.
use orr_bridge::FrameView;
use orr_games::room_escape_game::{RoomActor, EXIT, FLOOR, KEY, PLAYER, WALL};
use orr_render::{Lighting, Material, RenderList3D};
use orr_view::{RenderItem3, Shape3, Vec3};

pub fn lighting() -> Lighting {
    Lighting {
        direction: [-0.55, -0.82, -0.32],
        color: [1.0, 0.79, 0.53],
        intensity: 2.5,
        sky: [0.28, 0.55, 0.86],
        ground: [0.11, 0.14, 0.19],
        ambient: 0.48,
        shadows: true,
        shadow_center: [0.0, 0.0, 0.0],
        // Room actors extend to +/-24 on X/Z and +/-12 on Y.
        // Radius 40 covers every admitted procedural corner and trim, even
        // when interactables lie outside a small off-center wall cluster.
        shadow_radius: 40.0,
        tonemap: true,
        exposure: 1.0,
    }
}

/// Construct a bounded, deterministic view from the same immutable frame.
/// The room admission caps walls at 64 and actor coordinates at +/-16.
pub fn style(frame: FrameView<'_>, visible: &[RenderItem3], list: &mut RenderList3D) {
    let mut styled = visible.to_vec();
    for item in &mut styled {
        let Some(actor) = frame.get::<RoomActor>(item.entity) else { continue };
        item.style.color = match actor.kind {
            WALL => [0.075, 0.16, 0.20, 1.0],
            FLOOR => [0.075, 0.11, 0.14, 1.0],
            PLAYER => [0.18, 0.67, 0.68, 1.0],
            KEY => [1.0, 0.62, 0.10, 1.0],
            EXIT => [0.13, 0.80, 0.51, 1.0],
            _ => item.style.color,
        };
        item.style.roughness = if actor.kind == WALL { 0.72 } else { 0.48 };
    }
    list.clear();
    list.lighting = lighting();
    super::yard3d_view::fill_list(&styled, list);
    let brass = Material::new([0.25, 0.16, 0.07]).rough(0.5).metal(0.2);
    let stone = Material::new([0.25, 0.31, 0.29]).rough(0.76);
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    // Use every actor, including model-bound walls, to fit the decorative slab.
    for item in super::room_view::items(frame) {
        let Some(actor) = frame.get::<RoomActor>(item.entity) else { continue };
        if actor.kind == WALL {
            if let Shape3::Box { half } = item.style.shape {
                for x in [-half[0], half[0]] {
                    for z in [-half[2], half[2]] {
                        let p = item.transform.pos + item.transform.rot.rotate(Vec3::new(x, 0.0, z));
                        low[0] = low[0].min(p.x); low[1] = low[1].min(p.z);
                        high[0] = high[0].max(p.x); high[1] = high[1].max(p.z);
                    }
                }
                // Wall-mounted ribs and inset panels add detail without placing
                // misleading non-colliding furniture in the playable area.
                let long = usize::from(half[2] > half[0]) * 2;
                let short = 2 - long;
                let rib = Material::new([0.045, 0.09, 0.11]).rough(0.8);
                for n in -3..=3 {
                    let offset = n as f32 * 2.0;
                    if offset.abs() + 0.15 < half[long] {
                        let mut pos = item.transform.pos.to_array();
                        pos[long] += offset;
                        let mut size = half;
                        size[long] = 0.055;
                        size[1] = half[1] * 0.86;
                        size[short] += 0.008;
                        list.cuboid(pos, item.transform.rot.to_array(), size, &rib);
                    }
                }
                if half[long] >= 2.0 {
                    let mut pos = item.transform.pos.to_array();
                    let inward = if pos[short] <= 0.0 { 1.0 } else { -1.0 };
                    pos[short] += inward * (half[short] + 0.008);
                    pos[1] += half[1] * 0.2;
                    let mut size = [0.0; 3];
                    size[long] = 0.92;
                    size[1] = half[1] * 0.28;
                    size[short] = 0.009;
                    list.cuboid(pos, item.transform.rot.to_array(), size, &brass);
                    pos[short] += inward * 0.012;
                    size[long] -= 0.07;
                    size[1] *= 0.76;
                    size[short] = 0.004;
                    list.cuboid(pos, item.transform.rot.to_array(), size, &Material::new([0.012, 0.025, 0.035]).rough(0.48));
                    pos[short] += inward * 0.007;
                    size[long] = 0.56;
                    size[1] = 0.025;
                    size[short] = 0.004;
                    list.cuboid(pos, item.transform.rot.to_array(), size, &Material::new([0.85, 0.38, 0.065]).glow(1.4));
                }
                // Trim stays attached to the existing collider's surfaces.
                for (y, thickness, material) in [(half[1], 0.055, &stone), (-half[1] + 0.62, 0.065, &brass)] {
                    let pos = item.transform.pos + item.transform.rot.rotate(Vec3::new(0.0, y, 0.0));
                    list.cuboid(pos.to_array(), item.transform.rot.to_array(), [half[0] + 0.015, thickness, half[2] + 0.015], material);
                }
            }
        }
        if matches!(actor.kind, KEY | EXIT) {
            // A thin floor medallion labels interactables without changing their
            // silhouette or hiding them. Collected keys are absent from items().
            let p = item.transform.pos;
            let (radius, color) = if actor.kind == KEY { (0.42, [1.0, 0.52, 0.06]) } else { (0.76, [0.08, 0.75, 0.44]) };
            let material = Material::new(color).rough(0.4).glow(2.0);
            for n in 0..32 {
                let angle = n as f32 * std::f32::consts::TAU / 32.0;
                let next = (n + 1) as f32 * std::f32::consts::TAU / 32.0;
                // Small inlaid beads avoid screen-space overlays and fake shadows.
                list.sphere([p.x + angle.cos() * radius, 0.022, p.z + angle.sin() * radius], [0.0, 0.0, 0.0, 1.0], radius * (next - angle) * 0.58, &material);
            }
        }
    }
    if low.iter().chain(high.iter()).all(|v| v.is_finite()) {
        let center = [(low[0] + high[0]) * 0.5, -0.09, (low[1] + high[1]) * 0.5];
        // Inset the slab into the walls: coincident exterior slab/wall faces
        // otherwise z-fight even with both shadows and bloom disabled.
        let half = [(high[0] - low[0]) * 0.5 - 0.04, 0.075, (high[1] - low[1]) * 0.5 - 0.04];
        list.cuboid(center, [0.0, 0.0, 0.0, 1.0], half, &Material::new([0.065, 0.10, 0.13]).rough(0.7).checkered());
        // Restrict line count independently from authored dimensions.
        for n in -16..=16 {
            let x = n as f32;
            let grout = Material::new([0.13, 0.19, 0.22]).rough(0.9);
            if x > low[0] && x < high[0] {
                list.cuboid([x, -0.01, center[2]], [0.0, 0.0, 0.0, 1.0], [0.008, 0.003, half[2]], &grout);
            }
            if x > low[1] && x < high[1] {
                list.cuboid([center[0], -0.01, x], [0.0, 0.0, 0.0, 1.0], [half[0], 0.003, 0.008], &grout);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_games::room_escape_game::{RoomConfig, RoomEscapeV1, RoomRun};
    use orr_sim::Simulation;

    #[test]
    fn maximum_admitted_walls_stay_inside_presentation_budget() {
        use orr_physics3d::{Body, Collider};
        let mut sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, 42);
        let wall = sim.frame().entities().find(|&e| sim.frame().get::<RoomActor>(e).is_some_and(|a| a.kind == WALL)).unwrap();
        let body = *sim.frame().get::<Body>(wall).unwrap();
        let collider = *sim.frame().get::<Collider>(wall).unwrap();
        for _ in 4..64 {
            let entity = orr_physics3d::spawn_body(sim.frame_mut(), body, collider);
            sim.frame_mut().add(entity, RoomActor { kind: WALL, ordinal: 0 });
        }
        orr_games::room_escape_game::validate_initial_frame(sim.frame()).unwrap();
        let before = sim.frame().to_bytes();
        let frame = FrameView::of(sim.frame());
        let mut list = RenderList3D::new();
        style(frame, &crate::room_view::items(frame), &mut list);
        assert!(list.instance_count() <= 967);
        assert_eq!(list.boxes.len(), 64 * (3 + 5 + 3) + 1 + 22);
        assert_eq!(sim.frame().to_bytes(), before);
    }

    #[test]
    fn studio_presentation_is_read_only_bounded_and_key_aware() {
        let mut sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, 42);
        let before = sim.frame().to_bytes();
        let frame = FrameView::of(sim.frame());
        let items = crate::room_view::items(frame);
        let mut list = RenderList3D::new();
        style(frame, &items, &mut list);
        assert_eq!(sim.frame().to_bytes(), before);
        assert!(list.lighting.shadows && list.lighting.tonemap);
        assert!(list.instance_count() < 256);
        assert!(list.boxes.len() > 4, "world-space floor and wall details");
        let slab = list.boxes.iter().find(|b| b.pos[1] == -0.09).unwrap();
        assert!(slab.scale[0] < 5.25 && slab.scale[2] < 5.25, "slab must not share coplanar wall exteriors");
        assert!(list.spheres.iter().any(|s| s.material[2] > 0.0), "HDR emission");
        let spheres = list.spheres.len();
        sim.frame_mut().singleton_mut::<RoomRun>().key_collected = 1;
        let before = sim.frame().to_bytes();
        let frame = FrameView::of(sim.frame());
        style(frame, &crate::room_view::items(frame), &mut list);
        assert_eq!(sim.frame().to_bytes(), before);
        assert_eq!(list.spheres.len(), spheres - 33, "key and its 32 inlays disappear together");
    }
}
