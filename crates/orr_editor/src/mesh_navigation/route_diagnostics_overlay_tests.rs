use super::*;
fn point(x: i32, y: i32, z: i32) -> [FP; 3] {
    [FP::from_int(x), FP::from_int(y), FP::from_int(z)]
}
#[test]
fn cyan_path_and_colored_endpoint_boxes_are_exact() {
    let mut list = RenderList3D::default();
    append(&mut list, &[point(1, 0, 2), point(3, 1, 4), point(5, 0, 6)]).unwrap();
    assert_eq!(list.lines.len(), 26);
    assert_eq!(list.lines[0].a, [1.0, 0.0, 2.0]);
    assert_eq!(list.lines[0].b, [3.0, 1.0, 4.0]);
    assert_eq!(list.lines[0].color, ROUTE);
    assert!(list.lines[2..14].iter().all(|l| l.color == START));
    assert!(list.lines[14..].iter().all(|l| l.color == GOAL));
    assert_eq!(list.instance_count(), 0);
}
#[test]
fn zero_length_route_still_draws_endpoint_markers() {
    let mut list = RenderList3D::default();
    append(&mut list, &[point(1, 0, 1)]).unwrap();
    assert_eq!(list.lines.len(), 24);
}
#[test]
fn invalid_points_never_append_partial_commands() {
    let mut list = RenderList3D::default();
    list.line([0.0; 3], [1.0; 3], 1.0, [1.0; 4]);
    for points in [
        vec![],
        vec![point(0, 0, 0), point(257, 0, 0)],
        vec![[FP::from_raw(i64::MAX); 3]],
        vec![point(0, 0, 0); MAX_POINTS + 1],
    ] {
        assert!(append(&mut list, &points).is_err());
        assert_eq!(list.lines.len(), 1);
        assert_eq!(list.lines[0].color, [1.0; 4]);
    }
}
#[test]
fn largest_core_path_and_exact_coordinate_limits_are_bounded() {
    let mut list = RenderList3D::default();
    let points = vec![point(256, -256, 256); MAX_POINTS];
    append(&mut list, &points).unwrap();
    assert_eq!(list.lines.len(), MAX_POINTS - 1 + 24);
}
