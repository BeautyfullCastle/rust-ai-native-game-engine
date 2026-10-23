use super::*;
use orr_view::{Quat, Style3, Transform3, Vec3};

fn item(shape: Shape3) -> RenderItem3 {
    RenderItem3 {
        entity: orr_ecs::Entity {
            index: 7,
            version: 2,
        },
        transform: Transform3::new(Vec3::new(0.0, 0.0, 0.0), Quat::IDENTITY),
        style: Style3 {
            shape,
            color: [1.0; 4],
            roughness: 0.5,
            metallic: 0.0,
            checker: false,
        },
    }
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.00001, "{a} != {b}");
}

#[test]
fn thin_box_has_twelve_true_edges_not_max_extent_cube() {
    let wire = segments(&item(Shape3::Box {
        half: [0.2, 2.0, 0.2],
    }))
    .unwrap();
    assert_eq!(wire.len(), 12);
    for (a, b) in wire {
        for p in [a, b] {
            assert_eq!(p.map(f32::abs), [0.2, 2.0, 0.2]);
        }
        assert_eq!((0..3).filter(|&axis| a[axis] != b[axis]).count(), 1);
    }
}
#[test]
fn box_rotation_and_translation_match_render_transform() {
    let mut source = item(Shape3::Box {
        half: [0.2, 2.0, 0.4],
    });
    source.transform = Transform3::new(
        Vec3::new(3.0, 4.0, 5.0),
        Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), std::f32::consts::FRAC_PI_2),
    );
    for (a, b) in segments(&source).unwrap() {
        for p in [a, b] {
            close((p[0] - 3.0).abs(), 0.4);
            close((p[1] - 4.0).abs(), 2.0);
            close((p[2] - 5.0).abs(), 0.2);
        }
    }
}
#[test]
fn sphere_three_circles_stay_on_surface() {
    let wire = segments(&item(Shape3::Sphere { radius: 0.7 })).unwrap();
    assert_eq!(wire.len(), 96);
    for (a, b) in wire {
        for p in [a, b] {
            close(p.iter().map(|v| v * v).sum::<f32>().sqrt(), 0.7);
            assert!(p.contains(&0.0));
        }
    }
}
#[test]
fn capsule_rings_sides_and_caps_match_surface_and_extrema() {
    for half_length in [0.0, 2.0] {
        let wire = segments(&item(Shape3::Capsule {
            half_length,
            radius: 0.5,
        }))
        .unwrap();
        assert_eq!(wire.len(), MAX_SEGMENTS);
        let mut top = f32::NEG_INFINITY;
        let mut bottom = f32::INFINITY;
        for (a, b) in wire {
            for p in [a, b] {
                let dy = (p[1].abs() - half_length).max(0.0);
                close((p[0] * p[0] + dy * dy + p[2] * p[2]).sqrt(), 0.5);
                top = top.max(p[1]);
                bottom = bottom.min(p[1]);
            }
        }
        close(top, half_length + 0.5);
        close(bottom, -half_length - 0.5);
    }
}
#[test]
fn plane_stays_horizontal_like_existing_fill_list() {
    let mut source = item(Shape3::Plane {
        half_x: 2.0,
        half_z: 3.0,
    });
    source.transform = Transform3::new(
        Vec3::new(1.0, 4.0, 2.0),
        Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), 0.8),
    );
    let wire = segments(&source).unwrap();
    assert_eq!(wire.len(), 4);
    for (a, b) in wire {
        for p in [a, b] {
            assert_eq!(p[1], 4.0);
            close((p[0] - 1.0).abs(), 2.0);
            close((p[2] - 2.0).abs(), 3.0);
        }
    }
}
#[test]
fn invalid_geometry_never_partially_appends() {
    let mut cases = vec![
        item(Shape3::Sphere { radius: f32::NAN }),
        item(Shape3::Box {
            half: [1.0, -1.0, 1.0],
        }),
        item(Shape3::Capsule {
            half_length: f32::MAX,
            radius: f32::MAX,
        }),
        item(Shape3::Plane {
            half_x: 0.0,
            half_z: 1.0,
        }),
    ];
    let mut bad = item(Shape3::Sphere { radius: 1.0 });
    bad.transform.rot = Quat::new(0.0, 0.0, 0.0, 0.0);
    cases.push(bad);
    bad.transform.rot = Quat::IDENTITY;
    bad.transform.pos = Vec3::new(f32::INFINITY, 0.0, 0.0);
    cases.push(bad);
    bad.transform.pos = Vec3::new(f32::MAX, 0.0, 0.0);
    bad.style.shape = Shape3::Sphere { radius: f32::MAX };
    cases.push(bad);
    for source in cases {
        let mut list = RenderList3D::default();
        list.line([1.0; 3], [2.0; 3], 3.0, [0.5; 4]);
        append(&mut list, &source);
        assert_eq!(list.lines.len(), 1);
        assert_eq!(list.lines[0].a, [1.0; 3]);
    }
}
#[test]
fn append_preserves_source_and_existing_primitives() {
    let source = item(Shape3::Box {
        half: [0.2, 2.0, 0.2],
    });
    let before = format!("{source:?}");
    let mut list = RenderList3D::default();
    list.line([1.0; 3], [2.0; 3], 3.0, [0.5; 4]);
    append(&mut list, &source);
    assert_eq!(format!("{source:?}"), before);
    assert_eq!(list.lines.len(), 13);
    assert_eq!(list.lines[0].width, 3.0);
    for line in &list.lines[1..] {
        assert_eq!(line.width, 2.0);
        assert_eq!(line.color, [1.0, 0.85, 0.2, 1.0]);
    }
    assert_eq!(list.instance_count(), 0);
}
