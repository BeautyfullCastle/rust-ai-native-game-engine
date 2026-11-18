//! `model_draw_budget`: the model draws a frame can add next to its procedural list. No GPU.
#![cfg(feature = "imported-scene")]

use orr_render::imported_scene::{model_draw_budget, MAX_IMPORTED_DRAWS};
use orr_render::{Material, RenderList3D};

#[test]
fn an_empty_list_leaves_the_whole_draw_limit_to_the_models() {
    assert_eq!(model_draw_budget(&RenderList3D::new()), MAX_IMPORTED_DRAWS);
}

#[test]
fn each_kind_of_procedural_mesh_and_the_lines_take_one_draw_however_many_instances_there_are() {
    let m = Material::new([1.0; 3]);
    let mut list = RenderList3D::new();
    for i in 0..100 {
        list.sphere([i as f32, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], 0.5, &m);
    }
    assert_eq!(model_draw_budget(&list), MAX_IMPORTED_DRAWS - 1);

    list.cuboid([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3], &m);
    list.line([0.0; 3], [1.0; 3], 2.0, [1.0; 4]);
    assert_eq!(model_draw_budget(&list), MAX_IMPORTED_DRAWS - 3);
}
