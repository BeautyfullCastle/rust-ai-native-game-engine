//! What a game tells the default 2D renderer: which entities to draw and how, what
//! lies behind them, and where the camera looks.

use std::rc::Rc;

use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_render::{extract_items, Camera, RenderList};
use orr_view::{Extracted, Extractor, RenderItem, Shape, Style, Transform2, Vec2, ViewWorld};

/// The game's 2D look. The [`Extractor`] part reads drawable entities out of a sim
/// frame (shape, color, and how the entity is smoothed between ticks).
/// The two methods below add the rest of the picture. All three only read the frame.
pub trait View2D: Extractor {
    /// Where the camera looks in the newest predicted frame.
    fn camera(&self, frame: FrameView<'_>) -> Camera;

    /// Items drawn below every extracted entity (floor, background). None by default.
    fn backdrop(&self, frame: FrameView<'_>, out: &mut Vec<RenderItem>) {
        let _ = (frame, out);
    }
}

/// A filled rectangle that belongs to no entity, for [`View2D::backdrop`].
pub fn solid_quad(center: [f32; 2], half_size: [f32; 2], color: [f32; 4]) -> RenderItem {
    RenderItem {
        entity: Entity::NONE,
        transform: Transform2::new(Vec2::new(center[0], center[1]), 0.0),
        style: Style { shape: Shape::Quad, size: half_size[0], half_y: half_size[1], color },
    }
}

/// The view shared by the extracting world and the draw-list builder.
#[derive(Clone)]
pub(crate) struct SharedView(pub Rc<dyn View2D>);

impl Extractor for SharedView {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted>) {
        self.0.extract(frame, out);
    }
}

/// Fills `list` with the backdrop, then every entity of `world`, and returns the camera.
pub(crate) fn fill_draw_list(
    view: &SharedView,
    world: &ViewWorld<SharedView>,
    frame: FrameView<'_>,
    items: &mut Vec<RenderItem>,
    list: &mut RenderList,
) -> Camera {
    items.clear();
    view.0.backdrop(frame, items);
    world.render_items(items);
    list.clear();
    extract_items(items, list);
    view.0.camera(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_ecs::{ComponentRegistryBuilder, Frame};
    use orr_render::list::{SHAPE_CIRCLE, SHAPE_QUAD};
    use orr_view::{InterpMode, ViewConfig};

    struct Look;

    impl Extractor for Look {
        fn extract(&self, _frame: FrameView<'_>, _out: &mut Vec<Extracted>) {}
    }

    impl View2D for Look {
        fn camera(&self, frame: FrameView<'_>) -> Camera {
            Camera::new([frame.tick() as f32, 2.0], 5.0)
        }

        fn backdrop(&self, _frame: FrameView<'_>, out: &mut Vec<RenderItem>) {
            out.push(solid_quad([0.0, 1.0], [4.0, 1.0], [0.1, 0.1, 0.1, 1.0]));
        }
    }

    fn circle_at(x: f32) -> Extracted {
        Extracted {
            entity: Entity::NONE,
            transform: Transform2::new(Vec2::new(x, 0.0), 0.0),
            mode: InterpMode::None,
            style: Style { shape: Shape::Circle, size: 0.5, half_y: 0.0, color: [1.0, 0.0, 0.0, 1.0] },
        }
    }

    #[test]
    fn the_backdrop_is_drawn_first_then_the_entities_and_the_camera_comes_from_the_frame() {
        let view = SharedView(Rc::new(Look));
        let mut world = ViewWorld::new(view.clone(), ViewConfig::default());
        world.push_predicted(1, &[], &[circle_at(3.0)], false);
        world.advance(0.0);
        let frame = Frame::new(ComponentRegistryBuilder::new().build());
        let (mut items, mut list) = (Vec::new(), RenderList::new());

        let camera = fill_draw_list(&view, &world, FrameView::of(&frame), &mut items, &mut list);

        let kinds: Vec<u32> = list.shapes.iter().map(|s| s.shape).collect();
        assert_eq!(kinds, [SHAPE_QUAD, SHAPE_CIRCLE]);
        assert_eq!(list.shapes[1].center, [3.0, 0.0]);
        assert_eq!(camera, Camera::new([0.0, 2.0], 5.0));
    }

    #[test]
    fn a_second_fill_replaces_the_first() {
        let view = SharedView(Rc::new(Look));
        let world = ViewWorld::new(view.clone(), ViewConfig::default());
        let frame = Frame::new(ComponentRegistryBuilder::new().build());
        let (mut items, mut list) = (Vec::new(), RenderList::new());
        for _ in 0..3 {
            fill_draw_list(&view, &world, FrameView::of(&frame), &mut items, &mut list);
        }
        assert_eq!(list.shapes.len(), 1);
    }
}
