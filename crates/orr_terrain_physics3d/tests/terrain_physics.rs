use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{fp, FPQuat, FPVec3, FP};
use orr_physics3d::{Body, Collider, PhysicsState, Scratch, Shape};
use orr_terrain::Terrain;
use orr_terrain_physics3d::*;

fn v(x: FP, y: FP, z: FP) -> FPVec3 {
    FPVec3::new(x, y, z)
}
fn flat(side: u32, spacing: FP, holes: Vec<bool>) -> Terrain {
    Terrain::new(
        "tests/terrain.thf".into(),
        side,
        side,
        [FP::ZERO; 2],
        spacing,
        vec![FP::ZERO; (side * side) as usize],
        holes,
    )
    .unwrap()
}
fn frame(t: &Terrain, friction: FP) -> Frame {
    let mut r = ComponentRegistryBuilder::new();
    orr_physics3d::register(&mut r);
    register(&mut r);
    let mut f = Frame::new(r.build());
    orr_physics3d::init(&mut f, terrain_config());
    admit_heightfield(
        &mut f,
        &t.cook(),
        t.revision(),
        HeightfieldCollider {
            revision: t.revision(),
            friction,
            restitution: FP::ZERO,
            layer: 2,
            mask: 1,
        },
    )
    .unwrap();
    f
}
fn sphere(f: &mut Frame, p: FPVec3) -> Entity {
    let s = Shape::sphere(FP::HALF);
    orr_physics3d::spawn_body(
        f,
        Body::new_dynamic(p, &s, FP::ONE),
        Collider::new(s).with_filter(1, 2),
    )
}
fn run(f: &mut Frame, n: u32) {
    let mut scratch = Scratch::new();
    for tick in 0..n {
        terrain_step(f, &mut scratch).unwrap_or_else(|e| panic!("tick {tick}: {e}"));
    }
}
fn contacts(f: &Frame, p: FPVec3, r: FP, m: FP) -> Vec<TerrainContact> {
    sphere_contacts(
        &terrain_view(f).unwrap().unwrap(),
        p,
        r,
        m,
        MAX_SPHERE_CANDIDATES,
    )
    .unwrap()
}
fn near(a: FP, b: FP, tolerance: FP) {
    assert!((a - b).abs() <= tolerance, "{a} != {b}");
}

#[test]
fn finite_triangle_face_edge_and_vertex_regions_are_distinct() {
    let tri = [
        v(fp!(0), fp!(0), fp!(0)),
        v(fp!(0), fp!(0), fp!(2)),
        v(fp!(2), fp!(0), fp!(0)),
    ];
    let face = closest_point_on_triangle(v(fp!(0.5), fp!(1), fp!(0.5)), tri).unwrap();
    assert_eq!(face.feature, TriangleFeature::Face);
    assert_eq!(face.point.y, FP::ZERO);
    let edge = closest_point_on_triangle(v(fp!(-1), fp!(1), fp!(1)), tri).unwrap();
    assert_eq!(edge.feature, TriangleFeature::Edge(0, 1));
    assert_eq!(edge.point, v(fp!(0), fp!(0), fp!(1)));
    let vertex = closest_point_on_triangle(v(fp!(-1), fp!(1), fp!(-1)), tri).unwrap();
    assert_eq!(vertex.feature, TriangleFeature::Vertex(0));
    assert_eq!(vertex.point, tri[0]);
}

#[test]
fn actual_canonical_triangle_and_surface_apis_match_nonplanar_contacts() {
    let t = Terrain::new(
        "nonplanar.thf".into(),
        2,
        2,
        [FP::ZERO; 2],
        fp!(4),
        vec![fp!(0), fp!(2), fp!(3), fp!(1)],
        vec![false],
    )
    .unwrap();
    let f = frame(&t, FP::ZERO);
    assert_eq!(t.triangles(), vec![[0, 2, 3], [0, 3, 1]]);
    for tri_ids in t.triangles() {
        let tri = tri_ids.map(|i| {
            let p = t.vertex_position(i).unwrap();
            v(p[0], p[1], p[2])
        });
        let q = (tri[0] + tri[1] + tri[2]) / fp!(3);
        let surface = t.surface(q.x, q.z).unwrap();
        near(q.y, t.sample(q.x, q.z).unwrap(), FP::from_raw(3));
        let n = v(surface.normal[0], surface.normal[1], surface.normal[2]);
        let center = q + n * fp!(0.45);
        let found = contacts(&f, center, FP::HALF, fp!(0.1));
        assert_eq!(found.len(), 1, "{found:?}");
        let expected = closest_point_on_triangle(center, tri).unwrap();
        assert_eq!(found[0].point, expected.point);
        assert!(found[0].normal.dot(n) > fp!(0.999));
        let d = (center - found[0].point).length();
        near(found[0].separation, d - FP::HALF, FP::from_raw(3));
    }
}

#[test]
fn diagonal_and_cell_seam_features_deduplicate_without_internal_walls() {
    let f = frame(&flat(3, fp!(2), vec![false; 4]), FP::ZERO);
    for (x, z) in [
        (fp!(1), fp!(1)),
        (fp!(2), fp!(1)),
        (fp!(2), fp!(2)),
        (fp!(1.01), fp!(1)),
    ] {
        let c = contacts(&f, v(x, fp!(0.49), z), FP::HALF, fp!(0.1));
        assert_eq!(c.len(), 1, "duplicate/internal contacts at {x},{z}: {c:?}");
        assert_eq!(c[0].normal, FPVec3::Y);
    }
}

#[test]
fn hole_removes_both_triangles_without_floor_or_vertical_wall() {
    let f = frame(&flat(3, fp!(4), vec![true, false, false, false]), FP::ZERO);
    assert!(contacts(&f, v(fp!(1), fp!(0.2), fp!(2)), FP::HALF, FP::ZERO).is_empty());
    assert!(contacts(&f, v(fp!(3), fp!(-4), fp!(2)), FP::HALF, FP::ZERO).is_empty());
    assert!(contacts(&f, v(fp!(-1), fp!(-1), fp!(2)), FP::HALF, FP::ZERO).is_empty());
    let rim = contacts(&f, v(fp!(3.8), fp!(0.4), fp!(2)), FP::HALF, FP::ZERO);
    assert_eq!(rim.len(), 1);
    assert!(rim[0].normal.x < fp!(-0.4) && rim[0].normal.y > fp!(0.8));
    assert_eq!(rim[0].point.x, fp!(4));
}

#[test]
fn convex_ridge_keeps_a_single_finite_edge_normal() {
    let t = Terrain::new(
        "ridge.thf".into(),
        3,
        2,
        [FP::ZERO; 2],
        fp!(2),
        vec![fp!(0), fp!(1), fp!(0), fp!(0), fp!(1), fp!(0)],
        vec![false; 2],
    )
    .unwrap();
    let f = frame(&t, FP::ZERO);
    let c = contacts(&f, v(fp!(2), fp!(1.49), fp!(1)), FP::HALF, fp!(0.1));
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(c[0].normal, FPVec3::Y);
    assert_eq!(c[0].feature_id & 0xc000_0000, 0x4000_0000);
}

#[test]
fn actual_sphere_falls_rests_sleeps_and_wakes_from_impulse() {
    let mut f = frame(&flat(3, fp!(4), vec![false; 4]), fp!(0.8));
    let e = sphere(&mut f, v(fp!(3), fp!(4), fp!(3)));
    run(&mut f, 240);
    let b = *f.get::<Body>(e).unwrap();
    near(b.pos.y, FP::HALF, fp!(0.02));
    assert!(orr_physics3d::is_asleep(&f, e));
    orr_physics3d::apply_impulse(&mut f, e, v(fp!(0.2), fp!(1), fp!(0)), b.pos);
    run(&mut f, 1);
    assert!(!orr_physics3d::is_asleep(&f, e));
    assert!(f.get::<Body>(e).unwrap().pos.y > b.pos.y);
}

#[test]
fn actual_slope_accelerates_sphere_downhill() {
    let t = Terrain::new(
        "slope.thf".into(),
        3,
        3,
        [FP::ZERO; 2],
        fp!(4),
        vec![
            fp!(0),
            fp!(1),
            fp!(2),
            fp!(0),
            fp!(1),
            fp!(2),
            fp!(0),
            fp!(1),
            fp!(2),
        ],
        vec![false; 4],
    )
    .unwrap();
    let mut f = frame(&t, FP::ZERO);
    let e = sphere(&mut f, v(fp!(5), fp!(2), fp!(3)));
    f.get_mut::<Collider>(e).unwrap().friction = FP::ZERO;
    run(&mut f, 90);
    let b = f.get::<Body>(e).unwrap();
    assert!(b.pos.x < fp!(4), "{b:?}");
    assert!(b.vel.x < FP::ZERO);
}

#[test]
fn actual_friction_slows_a_rotation_locked_sphere() {
    let t = flat(3, fp!(8), vec![false; 4]);
    let mut rough = frame(&t, fp!(1));
    let e = sphere(&mut rough, v(fp!(4), FP::HALF, fp!(4)));
    let b = rough.get_mut::<Body>(e).unwrap();
    b.inv_inertia = FPVec3::ZERO;
    b.vel.x = fp!(3);
    rough.get_mut::<Collider>(e).unwrap().friction = fp!(1);
    let mut smooth = rough.clone();
    smooth.get_mut::<Collider>(e).unwrap().friction = FP::ZERO;
    run(&mut rough, 60);
    run(&mut smooth, 60);
    assert!(rough.get::<Body>(e).unwrap().vel.x.abs() < fp!(0.1));
    assert!(smooth.get::<Body>(e).unwrap().vel.x > fp!(2.9));
}

#[test]
fn sphere_falls_through_a_wide_hole_and_beyond_outer_boundary() {
    let mut f = frame(
        &flat(
            4,
            fp!(4),
            vec![false, false, false, false, true, false, false, false, false],
        ),
        FP::ZERO,
    );
    let h = sphere(&mut f, v(fp!(6), fp!(2), fp!(6)));
    let outside = sphere(&mut f, v(fp!(-2), fp!(2), fp!(6)));
    run(&mut f, 100);
    assert!(f.get::<Body>(h).unwrap().pos.y < fp!(-5));
    assert!(f.get::<Body>(outside).unwrap().pos.y < fp!(-5));
}

#[test]
fn actual_motion_crosses_diagonal_cell_seam_and_then_hole() {
    let mut f = frame(
        &flat(
            4,
            fp!(4),
            vec![false, false, false, false, true, false, false, false, false],
        ),
        FP::ZERO,
    );
    let e = sphere(&mut f, v(fp!(1), FP::HALF, fp!(6)));
    f.get_mut::<Body>(e).unwrap().vel.x = fp!(4);
    f.get_mut::<Collider>(e).unwrap().friction = FP::ZERO;
    run(&mut f, 90);
    let b = f.get::<Body>(e).unwrap();
    assert!(b.pos.x > fp!(6.5), "{b:?}");
    assert!(b.pos.y < fp!(-1), "stale floor bridged hole: {b:?}");
}

#[test]
fn actual_ridge_and_outer_rim_contact_change_horizontal_velocity() {
    let t = Terrain::new(
        "ridge.thf".into(),
        3,
        2,
        [FP::ZERO; 2],
        fp!(4),
        vec![fp!(0), fp!(1), fp!(0), fp!(0), fp!(1), fp!(0)],
        vec![false; 2],
    )
    .unwrap();
    let mut f = frame(&t, FP::ZERO);
    let e = sphere(&mut f, v(fp!(4), fp!(2), fp!(2)));
    run(&mut f, 120);
    near(f.get::<Body>(e).unwrap().pos.y, fp!(1.5), fp!(0.03));
    let mut rim = frame(&flat(2, fp!(4), vec![false]), FP::ZERO);
    let r = sphere(&mut rim, v(fp!(-0.2), fp!(0.5), fp!(2)));
    run(&mut rim, 20);
    assert!(
        rim.get::<Body>(r).unwrap().vel.x < FP::ZERO,
        "finite rim must push outward"
    );
}

#[test]
fn all_filter_permitted_unsupported_bodies_are_rejected_atomically() {
    for shape in [
        Shape::cuboid(FP::HALF, FP::HALF, FP::HALF),
        Shape::capsule(FP::HALF, FP::HALF),
        Shape::sphere(FP::HALF),
    ] {
        let mut f = frame(&flat(2, fp!(4), vec![false]), FP::ZERO);
        let mut body = Body::new_dynamic(v(fp!(900), fp!(900), fp!(900)), &shape, FP::ONE);
        if shape.kind == orr_physics3d::SHAPE_SPHERE {
            body.kind = orr_physics3d::BODY_KINEMATIC;
        }
        orr_physics3d::spawn_body(&mut f, body, Collider::new(shape).with_filter(1, 2));
        let before = f.to_bytes();
        assert!(terrain_step(&mut f, &mut Scratch::new()).is_err());
        assert_eq!(before, f.to_bytes());
    }
}

#[test]
fn unsupported_shapes_with_excluded_filters_can_use_ordinary_physics() {
    let mut f = frame(&flat(2, fp!(4), vec![false]), FP::ZERO);
    let s = Shape::cuboid(FP::HALF, FP::HALF, FP::HALF);
    let e = orr_physics3d::spawn_body(
        &mut f,
        Body::new_dynamic(v(fp!(2), fp!(2), fp!(2)), &s, FP::ONE),
        Collider::new(s).with_filter(4, 4),
    );
    run(&mut f, 1);
    assert!(f.get::<Body>(e).unwrap().pos.y < fp!(2));
}

#[test]
fn unsafe_dt_substeps_speed_radius_and_state_reject_without_mutation() {
    for case in 0..9 {
        let mut f = frame(&flat(2, fp!(4), vec![false]), FP::ZERO);
        let e = sphere(&mut f, v(fp!(2), fp!(2), fp!(2)));
        match case {
            0 => f.singleton_mut::<PhysicsState>().config.dt = FP::ZERO,
            1 => f.singleton_mut::<PhysicsState>().config.substeps = 0,
            2 => f.singleton_mut::<PhysicsState>().config.substeps = 65,
            3 => f.singleton_mut::<PhysicsState>().config.max_linear_speed = fp!(128),
            4 => f.get_mut::<Collider>(e).unwrap().shape.radius = fp!(0.1),
            5 => f.get_mut::<Body>(e).unwrap().pos.x = FP::from_raw(i64::MAX),
            6 => {
                f.get_mut::<Body>(e).unwrap().rot =
                    FPQuat::new(FP::ZERO, FP::ZERO, FP::ZERO, FP::ZERO)
            }
            7 => f.get_mut::<Body>(e).unwrap().pos.y = fp!(0.1),
            _ => f.singleton_mut::<PhysicsState>().config.gravity.y = FP::from_raw(i64::MIN),
        }
        let before = f.to_bytes();
        assert!(
            terrain_step(&mut f, &mut Scratch::new()).is_err(),
            "case {case}"
        );
        assert_eq!(before, f.to_bytes());
    }
}

#[test]
fn conservative_displacement_guard_blocks_high_speed_crossing() {
    let mut f = frame(&flat(2, fp!(4), vec![false]), FP::ZERO);
    let e = sphere(&mut f, v(fp!(2), fp!(1), fp!(2)));
    f.get_mut::<Body>(e).unwrap().vel.y = fp!(-128);
    f.singleton_mut::<PhysicsState>().config.max_linear_speed = fp!(128);
    let before = f.to_bytes();
    let e = terrain_step(&mut f, &mut Scratch::new()).unwrap_err();
    assert!(e.to_string().contains("configuration"));
    assert_eq!(before, f.to_bytes());
}

#[test]
fn shallow_falling_sphere_cannot_cross_a_finite_triangle() {
    let mut f = frame(&flat(2, fp!(4), vec![false]), FP::ZERO);
    let e = sphere(&mut f, v(fp!(2), fp!(0.55), fp!(2)));
    f.get_mut::<Body>(e).unwrap().vel.y = fp!(-16);
    run(&mut f, 10);
    assert!(f.get::<Body>(e).unwrap().pos.y > fp!(0.45));
}

#[test]
fn candidate_and_geometry_extremes_fail_deterministically() {
    let f = frame(&flat(65, fp!(0.25), vec![false; 4096]), FP::ZERO);
    let view = terrain_view(&f).unwrap().unwrap();
    assert!(sphere_contacts(
        &view,
        v(fp!(8), fp!(8), fp!(8)),
        fp!(8),
        fp!(0.1),
        MAX_SPHERE_CANDIDATES
    )
    .is_err());
    let bad = v(FP::from_raw(i64::MAX), FP::ZERO, FP::ZERO);
    assert!(closest_point_on_triangle(bad, [FPVec3::ZERO, FPVec3::X, FPVec3::Z]).is_err());
    assert!(closest_point_on_triangle(FPVec3::Y, [FPVec3::ZERO; 3]).is_err());
}

#[test]
fn rollback_and_fresh_scratch_replay_terrain_contacts_bit_exactly() {
    let mut f = frame(&flat(3, fp!(4), vec![false; 4]), fp!(0.7));
    sphere(&mut f, v(fp!(3), fp!(4), fp!(3)));
    run(&mut f, 35);
    let snapshot = f.clone();
    let mut a = Scratch::new();
    let mut expected = Vec::new();
    for _ in 0..100 {
        terrain_step(&mut f, &mut a).unwrap();
        expected.push(f.checksum());
    }
    let mut restored =
        Frame::from_bytes(snapshot.registry().clone(), &snapshot.to_bytes()).unwrap();
    drop(snapshot);
    for checksum in expected {
        terrain_step(&mut restored, &mut Scratch::new()).unwrap();
        assert_eq!(restored.checksum(), checksum);
    }
    assert_eq!(f.to_bytes(), restored.to_bytes());
}

#[test]
fn small_sphere_requires_enough_substeps_and_then_has_real_contact() {
    let mut f = frame(&flat(2, fp!(4), vec![false]), FP::ZERO);
    let shape = Shape::sphere(fp!(0.25));
    let e = orr_physics3d::spawn_body(
        &mut f,
        Body::new_dynamic(v(fp!(2), fp!(1), fp!(2)), &shape, FP::ONE),
        Collider::new(shape).with_filter(1, 2),
    );
    assert!(validate_frame(&f)
        .unwrap_err()
        .to_string()
        .contains("displacement"));
    f.singleton_mut::<PhysicsState>().config.substeps = 16;
    run(&mut f, 180);
    near(f.get::<Body>(e).unwrap().pos.y, fp!(0.25), fp!(0.02));
    assert!(orr_physics3d::is_asleep(&f, e));
}

#[test]
fn stale_cache_handle_and_duplicate_feature_are_atomic_errors() {
    let mut f = frame(&flat(2, fp!(4), vec![false]), FP::ZERO);
    sphere(&mut f, v(fp!(2), fp!(1), fp!(2)));
    let original = f.clone();
    let handle = f.singleton::<PhysicsState>().contacts;
    f.list_free(handle);
    let before = f.to_bytes();
    assert!(terrain_step(&mut f, &mut Scratch::new())
        .unwrap_err()
        .to_string()
        .contains("cache"));
    assert_eq!(before, f.to_bytes());
    let mut f = original;
    run(&mut f, 60);
    let handle = f.singleton::<PhysicsState>().contacts;
    let cache = f.list(handle)[0];
    f.list_push(handle, cache);
    let before = f.to_bytes();
    assert!(terrain_step(&mut f, &mut Scratch::new())
        .unwrap_err()
        .to_string()
        .contains("ordering"));
    assert_eq!(before, f.to_bytes());
}

#[test]
fn transformed_terrain_and_excess_body_count_are_rejected_before_motion() {
    let mut f = frame(&flat(2, fp!(4), vec![false]), FP::ZERO);
    let terrain = terrain_entity(&f).unwrap().unwrap();
    sphere(&mut f, v(fp!(2), fp!(1), fp!(2)));
    f.add(terrain, Body::new_static(FPVec3::X));
    let before = f.to_bytes();
    assert!(terrain_step(&mut f, &mut Scratch::new()).is_err());
    assert_eq!(before, f.to_bytes());
    let mut f = frame(&flat(2, fp!(4), vec![false]), FP::ZERO);
    for _ in 0..=MAX_TERRAIN_BODIES {
        sphere(&mut f, v(fp!(2), fp!(1), fp!(2)));
    }
    let before = f.to_bytes();
    assert!(terrain_step(&mut f, &mut Scratch::new())
        .unwrap_err()
        .to_string()
        .contains("count"));
    assert_eq!(before, f.to_bytes());
}

#[test]
fn fractional_sloped_shared_edge_keeps_its_true_nearest_feature() {
    let t = Terrain::new(
        "fractional.thf".into(),
        3,
        2,
        [FP::ZERO; 2],
        fp!(0.25),
        vec![
            fp!(0),
            fp!(0.125),
            fp!(0),
            fp!(0.03125),
            fp!(0.15625),
            fp!(0.03125),
        ],
        vec![false; 2],
    )
    .unwrap();
    let f = frame(&t, FP::ZERO);
    let c = contacts(&f, v(fp!(0.25), fp!(0.6), fp!(0.137)), FP::HALF, fp!(0.1));
    assert!(!c.is_empty());
    assert!(c.iter().any(|p| p.feature_id & 0xc000_0000 == 0x4000_0000));
}

#[test]
fn empty_optional_adapter_is_bit_exact_original_physics_with_default_config() {
    let mut registry = ComponentRegistryBuilder::new();
    orr_physics3d::register(&mut registry);
    register(&mut registry);
    let mut f = Frame::new(registry.build());
    orr_physics3d::init(&mut f, orr_physics3d::PhysicsConfig::default());
    let floor = Shape::cuboid(fp!(8), FP::HALF, fp!(8));
    orr_physics3d::spawn_body(
        &mut f,
        Body::new_static(v(FP::ZERO, -FP::HALF, FP::ZERO)),
        Collider::new(floor),
    );
    let ball = Shape::sphere(FP::HALF);
    orr_physics3d::spawn_body(
        &mut f,
        Body::new_dynamic(v(FP::ZERO, fp!(2), FP::ZERO), &ball, FP::ONE),
        Collider::new(ball),
    );
    let mut ordinary = f.clone();
    let mut a = Scratch::new();
    let mut b = Scratch::new();
    for _ in 0..180 {
        terrain_step(&mut f, &mut a).unwrap();
        orr_physics3d::step(&mut ordinary, &mut b);
        assert_eq!(f.to_bytes(), ordinary.to_bytes());
    }
}

#[test]
fn despawning_ordinary_support_preserves_orphan_wake_semantics() {
    let mut f = frame(&flat(2, fp!(4), vec![false]), FP::ZERO);
    let floor = Shape::cuboid(fp!(1), FP::HALF, fp!(1));
    let support = orr_physics3d::spawn_body(
        &mut f,
        Body::new_static(v(fp!(2), FP::ZERO, fp!(2))),
        Collider::new(floor).with_filter(4, 4),
    );
    let s = Shape::sphere(FP::HALF);
    let e = orr_physics3d::spawn_body(
        &mut f,
        Body::new_dynamic(v(fp!(2), fp!(2), fp!(2)), &s, FP::ONE),
        Collider::new(s).with_filter(4, 4),
    );
    run(&mut f, 180);
    assert!(orr_physics3d::is_asleep(&f, e));
    assert!(f.despawn(support));
    run(&mut f, 1);
    assert!(!orr_physics3d::is_asleep(&f, e));
    assert!(f.get::<Body>(e).unwrap().vel.y < FP::ZERO);
}

#[test]
fn teleporting_a_sleeper_away_from_cached_terrain_support_is_explicitly_rejected() {
    let mut f = frame(&flat(3, fp!(4), vec![false, true, false, false]), FP::ZERO);
    let e = sphere(&mut f, v(fp!(2), fp!(1), fp!(2)));
    run(&mut f, 180);
    assert!(orr_physics3d::is_asleep(&f, e));
    f.get_mut::<Body>(e).unwrap().pos.x = fp!(6);
    let before = f.to_bytes();
    assert!(terrain_step(&mut f, &mut Scratch::new())
        .unwrap_err()
        .to_string()
        .contains("stale terrain support"));
    assert_eq!(before, f.to_bytes());
    orr_physics3d::wake(&mut f, e);
    run(&mut f, 1);
    assert!(f.get::<Body>(e).unwrap().vel.y < FP::ZERO);
}

#[test]
fn empty_adapter_reports_missing_physics_and_invalid_timestep_instead_of_panicking() {
    let mut registry = ComponentRegistryBuilder::new();
    register(&mut registry);
    let mut missing = Frame::new(registry.build());
    let before = missing.to_bytes();
    assert!(terrain_step(&mut missing, &mut Scratch::new())
        .unwrap_err()
        .to_string()
        .contains("not registered"));
    assert_eq!(before, missing.to_bytes());
    let mut registry = ComponentRegistryBuilder::new();
    register(&mut registry);
    orr_physics3d::register(&mut registry);
    let mut f = Frame::new(registry.build());
    let cfg = orr_physics3d::PhysicsConfig {
        dt: FP::ZERO,
        ..Default::default()
    };
    orr_physics3d::init(&mut f, cfg);
    let before = f.to_bytes();
    assert!(terrain_step(&mut f, &mut Scratch::new())
        .unwrap_err()
        .to_string()
        .contains("timestep"));
    assert_eq!(before, f.to_bytes());
}

#[test]
fn concave_valley_has_two_real_face_normals_and_solves_both() {
    let t = Terrain::new(
        "valley.thf".into(),
        3,
        2,
        [FP::ZERO; 2],
        fp!(2),
        vec![fp!(1), fp!(0), fp!(1), fp!(1), fp!(0), fp!(1)],
        vec![false; 2],
    )
    .unwrap();
    let mut f = frame(&t, FP::ZERO);
    let points = contacts(&f, v(fp!(2), fp!(0.54), fp!(1)), FP::HALF, fp!(0.1));
    assert_eq!(points.len(), 2, "valley must retain both faces: {points:?}");
    assert!(points.iter().any(|p| p.normal.x > fp!(0.4)));
    assert!(points.iter().any(|p| p.normal.x < fp!(-0.4)));
    assert!(points.iter().all(|p| p.normal.y > fp!(0.8)));
    assert_ne!(points[0].feature_id, points[1].feature_id);
    let e = sphere(&mut f, v(fp!(2), fp!(2), fp!(1)));
    run(&mut f, 240);
    let body = f.get::<Body>(e).unwrap();
    near(body.pos.x, fp!(2), fp!(0.05));
    // The center is supported by both slopes: r / cos(arctan(1/2)).
    near(body.pos.y, fp!(0.559), fp!(0.03));
    let state = f.singleton::<PhysicsState>();
    let terrain = terrain_entity(&f).unwrap().unwrap();
    let retained = f
        .list(state.contacts)
        .iter()
        .filter(|c| {
            (c.a == e.index && c.b == terrain.index) || (c.b == e.index && c.a == terrain.index)
        })
        .count();
    assert_eq!(
        retained, 2,
        "both valley normals must survive into the actual persistent solve cache"
    );
}

#[test]
fn sphere_spanning_multiple_cells_physically_bridges_a_narrow_hole() {
    let mut holes = vec![false; 16];
    for z in 1..3 {
        for x in 1..3 {
            holes[z * 4 + x] = true;
        }
    }
    let t = flat(5, fp!(0.25), holes);
    assert!(t.sample(fp!(0.5), fp!(0.5)).is_none());
    let mut f = frame(&t, FP::ZERO);
    let points = contacts(&f, v(fp!(0.5), fp!(0.4), fp!(0.5)), FP::HALF, fp!(0.1));
    assert!(
        points.len() >= 4,
        "sphere should meet four finite rims: {points:?}"
    );
    assert!(points.iter().any(|p| p.normal.x > fp!(0.4)));
    assert!(points.iter().any(|p| p.normal.x < fp!(-0.4)));
    assert!(points.iter().any(|p| p.normal.z > fp!(0.4)));
    assert!(points.iter().any(|p| p.normal.z < fp!(-0.4)));
    let e = sphere(&mut f, v(fp!(0.5), fp!(1), fp!(0.5)));
    run(&mut f, 240);
    let body = f.get::<Body>(e).unwrap();
    // Radius .5 reaches each rim .25 away despite no triangle under the center.
    near(body.pos.x, fp!(0.5), fp!(0.03));
    near(body.pos.z, fp!(0.5), fp!(0.03));
    near(body.pos.y, fp!(0.433), fp!(0.03));
}

fn settled_sleeping_fixture() -> (Frame, Entity) {
    let mut f = frame(&flat(3, fp!(4), vec![false; 4]), fp!(0.5));
    let body = sphere(&mut f, v(fp!(2), fp!(3), fp!(2)));
    run(&mut f, 240);
    assert!(orr_physics3d::is_asleep(&f, body));
    (f, body)
}
fn restored(f: &Frame) -> Frame {
    Frame::from_bytes(f.registry().clone(), &f.to_bytes()).unwrap()
}
fn rejects_sleeping_snapshot(f: &Frame) {
    let mut copy = restored(f);
    let bytes = copy.to_bytes();
    assert!(
        validate_frame(&copy).is_err(),
        "unsupported sleeper passed admission"
    );
    assert!(terrain_step(&mut copy, &mut Scratch::new()).is_err());
    assert_eq!(
        copy.to_bytes(),
        bytes,
        "sleep rejection must be byte-atomic"
    );
}
#[test]
fn restored_sleepers_require_current_matching_grounded_support() {
    let (base, body) = settled_sleeping_fixture();
    let cache = base.singleton::<PhysicsState>().contacts;
    let mut missing = base.clone();
    missing.list_clear(cache);
    missing.get_mut::<Body>(body).unwrap().pos.y = fp!(2);
    rejects_sleeping_snapshot(&missing);

    let mut distant = base.clone();
    distant.get_mut::<Body>(body).unwrap().pos.y += fp!(0.05);
    rejects_sleeping_snapshot(&distant);

    let mut stale_feature = base.clone();
    for contact in stale_feature.list_mut(cache) {
        contact.id ^= 0x2000_0000;
    }
    rejects_sleeping_snapshot(&stale_feature);

    let mut ceiling = base.clone();
    ceiling.get_mut::<Body>(body).unwrap().pos.y = -FP::HALF;
    rejects_sleeping_snapshot(&ceiling);

    let mut sideways = base.clone();
    sideways.singleton_mut::<PhysicsState>().config.gravity = v(fp!(0.1), FP::ZERO, FP::ZERO);
    rejects_sleeping_snapshot(&sideways);
}
#[test]
fn restored_grounded_stack_is_idle_but_shared_island_label_cannot_ground_free_body() {
    let mut f = frame(&flat(3, fp!(4), vec![false; 4]), fp!(0.5));
    let lower = sphere(&mut f, v(fp!(2), FP::HALF, fp!(2)));
    let upper = sphere(&mut f, v(fp!(2), fp!(3), fp!(2)));
    for entity in [lower, upper] {
        f.get_mut::<Collider>(entity).unwrap().mask = u32::MAX;
    }
    run(&mut f, 360);
    assert!(orr_physics3d::is_asleep(&f, lower));
    assert!(orr_physics3d::is_asleep(&f, upper));
    assert!(f.get::<Body>(upper).unwrap().pos.y > fp!(1.4));
    assert_eq!(
        f.get::<Body>(lower).unwrap().island,
        f.get::<Body>(upper).unwrap().island
    );
    assert_ne!(f.get::<Body>(lower).unwrap().island, 0);
    let mut copy = restored(&f);
    let bytes = copy.to_bytes();
    for _ in 0..5 {
        terrain_step(&mut copy, &mut Scratch::new()).unwrap();
    }
    assert_eq!(
        copy.to_bytes(),
        bytes,
        "valid transitive sleepers stay bit-identical"
    );

    // Both bodies retain their legitimate common island label, but the upper
    // body no longer has any current ordinary contact to carry that support.
    f.get_mut::<Body>(upper).unwrap().pos.y = fp!(3);
    rejects_sleeping_snapshot(&f);
}
#[test]
fn zero_gravity_sleepers_revalidate_when_gravity_changes_after_restore() {
    let mut f = frame(&flat(3, fp!(4), vec![false; 4]), FP::ZERO);
    f.singleton_mut::<PhysicsState>().config.gravity = FPVec3::ZERO;
    f.singleton_mut::<PhysicsState>().config.sleep_ticks = 1;
    let body = sphere(&mut f, v(fp!(2), fp!(3), fp!(2)));
    run(&mut f, 1);
    assert!(orr_physics3d::is_asleep(&f, body));
    let mut copy = restored(&f);
    let bytes = copy.to_bytes();
    terrain_step(&mut copy, &mut Scratch::new()).unwrap();
    assert_eq!(copy.to_bytes(), bytes);
    copy.singleton_mut::<PhysicsState>().config.gravity = v(FP::ZERO, fp!(-0.1), FP::ZERO);
    rejects_sleeping_snapshot(&copy);
    orr_physics3d::wake(&mut copy, body);
    terrain_step(&mut copy, &mut Scratch::new()).unwrap();
    assert!(!orr_physics3d::is_asleep(&copy, body));
}
#[test]
fn restored_touching_sleeping_cluster_needs_a_static_anchor() {
    let mut f = frame(&flat(3, fp!(4), vec![false; 4]), FP::ZERO);
    let cfg = &mut f.singleton_mut::<PhysicsState>().config;
    cfg.gravity = FPVec3::ZERO;
    cfg.sleep_ticks = 1;
    cfg.sleep_linear_speed = FP::ONE;
    let lower = sphere(&mut f, v(fp!(2), fp!(3), fp!(2)));
    let upper = sphere(&mut f, v(fp!(2), fp!(3.989), fp!(2)));
    for entity in [lower, upper] {
        f.get_mut::<Collider>(entity).unwrap().mask = u32::MAX;
    }
    run(&mut f, 10);
    assert!(orr_physics3d::is_asleep(&f, lower));
    assert!(orr_physics3d::is_asleep(&f, upper));
    let cache = f.singleton::<PhysicsState>().contacts;
    assert!(!f.list(cache).is_empty());
    // A restored cache can retain positive pair impulses. Those impulses may
    // connect the two dynamic members, but cannot manufacture a static anchor.
    for contact in f.list_mut(cache) {
        contact.normal_impulse = fp!(0.1);
    }
    f.singleton_mut::<PhysicsState>().config.gravity = v(FP::ZERO, fp!(-0.1), FP::ZERO);
    rejects_sleeping_snapshot(&f);
}
