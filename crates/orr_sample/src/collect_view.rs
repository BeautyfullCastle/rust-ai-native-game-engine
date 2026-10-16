//! Read-only CollectDodgeV1 geometry, shared by the runtime and editor.
use crate::collect_game::{self as game, CollectActor, CollectRun};
use crate::editor_view::{Drawable, Outline};
use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_render::Camera;
use orr_view::{fp_to_f32, fp_to_vec2, RenderItem, Shape, Style, Transform2, Vec2};

pub fn actor_style(actor: &CollectActor) -> Style {
    let color = match actor.kind {
        game::PLAYER => [0.12, 0.42, 1.0, 1.0],
        game::COLLECTIBLE => [1.0, 0.85, 0.04, 1.0],
        _ => [1.0, 0.12, 0.12, 1.0],
    };
    // Simulation contact is an inclusive axis-aligned square, not a circle.
    Style {
        shape: Shape::Quad,
        size: fp_to_f32(game::RADIUS),
        half_y: fp_to_f32(game::RADIUS),
        color,
    }
}

fn actors(frame: FrameView<'_>) -> Vec<(Entity, &CollectActor)> {
    let mut actors: Vec<_> = frame
        .iter::<CollectActor>()
        .filter(|(_, a)| a.active != 0)
        .collect();
    actors.sort_by_key(|(_, a)| (a.kind, a.ordinal));
    actors
}
pub fn editor_drawables(frame: FrameView<'_>) -> Vec<Drawable> {
    body_views(frame)
}

pub fn body_views(frame: FrameView<'_>) -> Vec<Drawable> {
    actors(frame)
        .into_iter()
        .map(|(entity, actor)| {
            let p = fp_to_vec2(actor.position);
            let r = fp_to_f32(game::RADIUS);
            Drawable {
                entity,
                pos: [p.x, p.y],
                angle: 0.0,
                style: actor_style(actor),
                turn: 0.0,
                outline: Outline::Polygon(vec![[-r, -r], [r, -r], [r, r], [-r, r]]),
            }
        })
        .collect()
}

pub fn scene_floor() -> RenderItem {
    RenderItem {
        entity: Entity::NONE,
        transform: Transform2::new(Vec2::ZERO, 0.0),
        style: Style {
            shape: Shape::Quad,
            size: fp_to_f32(game::HALF_EXTENT),
            half_y: fp_to_f32(game::HALF_EXTENT),
            color: [0.055, 0.065, 0.09, 1.0],
        },
    }
}
/// Stable framing of authored initial actors. A moving hazard reserves its full
/// patrol axis; collected actors remain in these bounds, avoiding zoom jumps.
pub fn scene_camera(frame: FrameView<'_>) -> Camera {
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    for (_, actor) in frame.iter::<CollectActor>() {
        let p = [
            fp_to_f32(actor.initial_position.x),
            fp_to_f32(actor.initial_position.y),
        ];
        let velocity = [actor.initial_velocity.x, actor.initial_velocity.y];
        let current = [fp_to_f32(actor.position.x), fp_to_f32(actor.position.y)];
        for axis in 0..2 {
            if actor.kind == game::HAZARD && velocity[axis] != orr_fp::FP::ZERO {
                low[axis] = -fp_to_f32(game::HALF_EXTENT);
                high[axis] = fp_to_f32(game::HALF_EXTENT);
            } else {
                low[axis] = low[axis].min(p[axis].min(current[axis]) - fp_to_f32(game::RADIUS));
                high[axis] = high[axis].max(p[axis].max(current[axis]) + fp_to_f32(game::RADIUS));
            }
        }
    }
    if !low[0].is_finite() {
        return Camera::new([0.0, 0.0], 270.0);
    }
    Camera::new(
        [(low[0] + high[0]) * 0.5, (low[1] + high[1]) * 0.5],
        ((high[0] - low[0]).max(high[1] - low[1]) * 0.6).max(40.0),
    )
}
pub fn render_items(frame: FrameView<'_>) -> Vec<RenderItem> {
    let mut items = vec![scene_floor()];
    items.extend(actors(frame).into_iter().map(|(entity, a)| RenderItem {
        entity,
        transform: Transform2::new(fp_to_vec2(a.position), 0.0),
        style: actor_style(a),
    }));
    items
}
pub fn status(run: &CollectRun) -> &'static str {
    match run.phase {
        game::PLAYING => "PLAYING",
        game::WON => "WON",
        game::LOST_HAZARD => "LOST: hazard",
        game::LOST_TIMEOUT => "LOST: time",
        _ => "INVALID",
    }
}
pub fn title(frame: FrameView<'_>) -> String {
    let run = frame.singleton::<CollectRun>();
    format!("CollectDodgeV1 | {} | score {}/{} | time {:.1}s | WASD/arrows move | Space restart | Esc quit",
        status(run), run.score, run.goal, run.time_limit_ticks.saturating_sub(run.elapsed_ticks) as f32 / 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_fp::{FPVec2, FP};
    use orr_sim::{Simulation, TickInputs};
    #[test]
    fn exact_square_hidden_coin_and_read_only_mapping() {
        let level = game::CollectLevel::new(
            FPVec2::ZERO,
            vec![FPVec2::ZERO],
            vec![game::HazardSpec {
                position: FPVec2::new(FP::from_int(50), FP::ZERO),
                velocity: FPVec2::ZERO,
            }],
            60,
        )
        .unwrap();
        let mut simulation = Simulation::<game::CollectDodgeV1>::new(level, 60, 42);
        let checksum = simulation.frame().checksum();
        let drawables = editor_drawables(FrameView::of(simulation.frame()));
        assert_eq!(drawables.len(), 3);
        for drawable in &drawables {
            assert_eq!(drawable.style.shape, Shape::Quad);
            assert_eq!(drawable.style.size, 2.0);
            assert_eq!(drawable.style.half_y, 2.0);
            assert_eq!(
                drawable.outline,
                Outline::Polygon(vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]])
            );
        }
        assert_eq!(render_items(FrameView::of(simulation.frame())).len(), 4);
        assert_eq!(simulation.frame().checksum(), checksum);
        simulation.step(&TickInputs::new(1, 1));
        let frame = FrameView::of(simulation.frame());
        assert_eq!(frame.singleton::<CollectRun>().phase, game::WON);
        assert_eq!(editor_drawables(frame).len(), 2);
        assert_eq!(render_items(frame).len(), 3);
        assert!(title(frame).contains("WON | score 1/1"));
    }
}

#[cfg(test)]
mod camera_tests {
    use super::*;
    use orr_fp::{FPVec2, FP};
    use orr_sim::Simulation;
    #[test]
    fn camera_covers_full_valid_authored_bounds_and_patrol_axes() {
        let p = |x, y| FPVec2::new(FP::from_int(x), FP::from_int(y));
        let level = game::CollectLevel::new(
            p(-254, -254),
            vec![p(254, 254)],
            vec![game::HazardSpec {
                position: p(0, 0),
                velocity: p(1, 0),
            }],
            600,
        )
        .unwrap();
        let sim = Simulation::<game::CollectDodgeV1>::new(level, 60, 42);
        let camera = scene_camera(FrameView::of(sim.frame()));
        for point in [[-256.0, -256.0], [256.0, 256.0]] {
            let pixel = camera.world_to_screen(point, (1024, 1024));
            assert!(pixel.iter().all(|&v| (0.0..1024.0).contains(&v)));
        }
    }
}
