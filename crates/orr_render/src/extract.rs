//! Extract: turns the view world's items into a render list (Bevy-like
//! extract, then render). This is the only place the renderer touches
//! `orr_view` types; the renderer itself never sees sim or view state.

use orr_view::{RenderItem, Shape};

use crate::list::{RenderList, ShapeInstance, SHAPE_CAPSULE, SHAPE_CIRCLE, SHAPE_QUAD};

/// The GPU instance of one render item.
pub fn instance_of(i: &RenderItem) -> ShapeInstance {
    let style = &i.style;
    let (shape, half_size) = match style.shape {
        Shape::Circle => (SHAPE_CIRCLE, [style.size; 2]),
        // `half_y == 0` keeps the old meaning: a square of side `2 * size`.
        Shape::Quad => (SHAPE_QUAD, [style.size, if style.half_y > 0.0 { style.half_y } else { style.size }]),
        // Capsule: `size` is the half length of the axis (along local x),
        // `half_y` the radius.
        Shape::Capsule => (SHAPE_CAPSULE, [style.size, style.half_y]),
    };
    ShapeInstance {
        center: [i.transform.pos.x, i.transform.pos.y],
        half_size,
        rot: i.transform.rot,
        shape,
        color: style.color,
    }
}

/// Appends the shapes of `items` (in order) to `list`.
pub fn extract_items(items: &[RenderItem], list: &mut RenderList) {
    list.shapes.extend(items.iter().map(instance_of));
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_ecs::Entity;
    use orr_view::{Style, Transform2, Vec2};

    fn item(shape: Shape, size: f32, half_y: f32, rot: f32) -> RenderItem {
        RenderItem {
            entity: Entity::NONE,
            transform: Transform2::new(Vec2::new(3.0, -4.0), rot),
            style: Style { shape, size, half_y, color: [1.0, 0.5, 0.25, 1.0] },
        }
    }

    #[test]
    fn items_map_to_instances() {
        let b = instance_of(&item(Shape::Quad, 2.0, 0.5, 1.25));
        assert_eq!((b.shape, b.half_size, b.rot, b.center), (SHAPE_QUAD, [2.0, 0.5], 1.25, [3.0, -4.0]));
        assert_eq!(instance_of(&item(Shape::Quad, 2.0, 0.0, 0.0)).half_size, [2.0, 2.0]);
        let c = instance_of(&item(Shape::Circle, 0.7, 0.0, 0.0));
        assert_eq!((c.shape, c.half_size), (SHAPE_CIRCLE, [0.7, 0.7]));
        let k = instance_of(&item(Shape::Capsule, 1.5, 0.4, 0.0));
        assert_eq!((k.shape, k.half_size), (SHAPE_CAPSULE, [1.5, 0.4]));
    }
}
