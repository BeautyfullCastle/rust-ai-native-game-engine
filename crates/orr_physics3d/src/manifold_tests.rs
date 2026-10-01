//! Manifold tests for every shape pair (analytic expectations).

use crate::collide::{collide, Manifold};
use crate::geom::Xf;
use crate::types::Shape;
use orr_fp::{fp, FPMat3, FPQuat, FPVec3, FP};

fn xf(p: FPVec3, q: FPQuat) -> Xf {
    Xf { p, r: FPMat3::from_quat(q) }
}

fn at(x: FP, y: FP, z: FP) -> Xf {
    xf(FPVec3::new(x, y, z), FPQuat::IDENTITY)
}

const M: FP = FP::from_raw(1311); // 0.02

fn near(a: FP, b: FP, tol: FP) -> bool {
    (a - b).abs() <= tol
}

fn vnear(a: FPVec3, b: FPVec3, tol: FP) -> bool {
    near(a.x, b.x, tol) && near(a.y, b.y, tol) && near(a.z, b.z, tol)
}

fn check_basic(m: &Manifold) {
    assert!(m.count > 0);
    assert!(near(m.normal.length(), FP::ONE, fp!(0.002)), "normal not unit {:?}", m.normal);
    for i in 1..m.count {
        assert!(m.pts[i].id > m.pts[i - 1].id, "ids not strictly increasing");
    }
}

#[test]
fn sphere_sphere() {
    let s = Shape::sphere(fp!(0.5));
    let m = collide(&s, &at(fp!(0), fp!(0), fp!(0)), &s, &at(fp!(0.8), fp!(0), fp!(0)), M);
    check_basic(&m);
    assert_eq!(m.count, 1);
    assert!(vnear(m.normal, FPVec3::X, fp!(0.001)));
    assert!(near(m.pts[0].sep, fp!(-0.2), fp!(0.001)));
    assert!(near(m.pts[0].point.x, fp!(0.4), fp!(0.002)));
    // Speculative: 0.01 apart is still a contact, 0.05 apart is not.
    let m = collide(&s, &at(fp!(0), fp!(0), fp!(0)), &s, &at(fp!(1.01), fp!(0), fp!(0)), M);
    assert_eq!(m.count, 1);
    assert!(near(m.pts[0].sep, fp!(0.01), fp!(0.001)));
    let m = collide(&s, &at(fp!(0), fp!(0), fp!(0)), &s, &at(fp!(1.05), fp!(0), fp!(0)), M);
    assert_eq!(m.count, 0);
    // Concentric spheres still give a unit normal.
    let m = collide(&s, &at(fp!(1), fp!(1), fp!(1)), &s, &at(fp!(1), fp!(1), fp!(1)), M);
    check_basic(&m);
}

#[test]
fn sphere_capsule() {
    let s = Shape::sphere(fp!(0.3));
    let c = Shape::capsule(fp!(1), fp!(0.2)); // axis along y
    // Beside the cylinder part.
    let m = collide(&s, &at(fp!(0.4), fp!(0.5), fp!(0)), &c, &at(fp!(0), fp!(0), fp!(0)), M);
    check_basic(&m);
    assert!(vnear(m.normal, -FPVec3::X, fp!(0.001)));
    assert!(near(m.pts[0].sep, fp!(-0.1), fp!(0.001)));
    // Above the top cap.
    let m = collide(&s, &at(fp!(0), fp!(1.45), fp!(0)), &c, &at(fp!(0), fp!(0), fp!(0)), M);
    check_basic(&m);
    assert!(vnear(m.normal, -FPVec3::Y, fp!(0.001)));
    assert!(near(m.pts[0].sep, fp!(-0.05), fp!(0.001)));
}

#[test]
fn capsule_capsule_crossing_and_parallel() {
    let c = Shape::capsule(fp!(1), fp!(0.25));
    let rot_x = FPQuat::from_axis_angle(FPVec3::Z, FP::HALF_PI); // axis along x
    // Two perpendicular capsules, one above the other: one point, vertical.
    let m = collide(
        &c,
        &xf(FPVec3::new(FP::ZERO, fp!(0.4), FP::ZERO), FPQuat::from_axis_angle(FPVec3::X, FP::HALF_PI)),
        &c,
        &xf(FPVec3::new(FP::ZERO, FP::ZERO, FP::ZERO), rot_x),
        M,
    );
    check_basic(&m);
    assert_eq!(m.count, 1);
    assert!(vnear(m.normal, -FPVec3::Y, fp!(0.002)));
    assert!(near(m.pts[0].sep, fp!(-0.1), fp!(0.002)));
    // Two parallel capsules lying on each other: two points.
    let m = collide(
        &c,
        &xf(FPVec3::new(FP::ZERO, FP::ZERO, FP::ZERO), rot_x),
        &c,
        &xf(FPVec3::new(fp!(0.5), fp!(0.45), FP::ZERO), rot_x),
        M,
    );
    check_basic(&m);
    assert_eq!(m.count, 2, "parallel capsules need two points");
    assert!(vnear(m.normal, FPVec3::Y, fp!(0.002)));
    for p in &m.pts[..2] {
        assert!(near(p.sep, fp!(-0.05), fp!(0.002)));
    }
    // Their x positions are the ends of the overlap [-0.5, 1].
    let xs = [m.pts[0].point.x, m.pts[1].point.x];
    assert!(xs.iter().any(|&x| near(x, fp!(-0.5), fp!(0.01))) && xs.iter().any(|&x| near(x, fp!(1), fp!(0.01))), "{xs:?}");
}

#[test]
fn sphere_box_regions() {
    let b = Shape::cuboid(fp!(1), fp!(0.5), fp!(1));
    let s = Shape::sphere(fp!(0.4));
    let bx = at(fp!(0), fp!(0), fp!(0));
    // Face: normal from the sphere to the box points down.
    let m = collide(&s, &at(fp!(0.3), fp!(0.8), fp!(-0.2)), &b, &bx, M);
    check_basic(&m);
    assert!(vnear(m.normal, -FPVec3::Y, fp!(0.001)));
    assert!(near(m.pts[0].sep, fp!(-0.1), fp!(0.001)));
    // Edge (x = 1, y = 0.5).
    let m = collide(&s, &at(fp!(1.2), fp!(0.7), fp!(0)), &b, &bx, M);
    check_basic(&m);
    let n = FPVec3::new(-fp!(0.7071), -fp!(0.7071), FP::ZERO);
    assert!(vnear(m.normal, n, fp!(0.01)), "{:?}", m.normal);
    assert!(near(m.pts[0].sep, fp!(0.2828) - fp!(0.4), fp!(0.005)));
    // Corner.
    let m = collide(&s, &at(fp!(1.2), fp!(0.7), fp!(1.2)), &b, &bx, M);
    check_basic(&m);
    assert!(near(m.normal.x, -fp!(0.5774), fp!(0.01)) && near(m.normal.z, -fp!(0.5774), fp!(0.01)));
    // Sphere center inside the box: pushes out through the nearest face.
    let m = collide(&s, &at(fp!(0.9), fp!(0.1), fp!(0)), &b, &bx, M);
    check_basic(&m);
    assert!(vnear(m.normal, -FPVec3::X, fp!(0.001)), "{:?}", m.normal);
    assert!(near(m.pts[0].sep, -(fp!(0.1) + fp!(0.4)), fp!(0.002)));
    // Too far.
    assert_eq!(collide(&s, &at(fp!(0), fp!(1.0), fp!(0)), &b, &bx, M).count, 0);
}

#[test]
fn capsule_box_lying_standing_and_edge() {
    let b = Shape::cuboid(fp!(2), fp!(0.5), fp!(2));
    let bx = at(fp!(0), fp!(0), fp!(0));
    let c = Shape::capsule(fp!(0.8), fp!(0.2));
    let lie = FPQuat::from_axis_angle(FPVec3::Z, FP::HALF_PI);
    // Lying along x on the top face, slightly sunk: two points.
    let m = collide(&c, &xf(FPVec3::new(fp!(0.3), fp!(0.69), fp!(0.2)), lie), &b, &bx, M);
    check_basic(&m);
    assert_eq!(m.count, 2);
    assert!(vnear(m.normal, -FPVec3::Y, fp!(0.001)));
    for p in &m.pts[..2] {
        assert!(near(p.sep, -fp!(0.01), fp!(0.002)));
    }
    // Standing upright: only the lower end is near the face.
    let m = collide(&c, &xf(FPVec3::new(fp!(0.3), fp!(1.5), fp!(0.2)), FPQuat::IDENTITY), &b, &bx, M);
    check_basic(&m);
    assert_eq!(m.count, 1);
    assert!(near(m.pts[0].sep, fp!(0.0), fp!(0.002)));
    // Hanging over the +x edge, axis along x: the clip covers the part
    // that is over the face only.
    let m = collide(&c, &xf(FPVec3::new(fp!(2.0), fp!(0.69), fp!(0)), lie), &b, &bx, M);
    check_basic(&m);
    assert!(vnear(m.normal, -FPVec3::Y, fp!(0.001)));
    assert!(m.pts[..m.count].iter().all(|p| p.point.x <= fp!(2.01)));
    // Crossing straight through the box (deep): pushed out along y.
    let m = collide(&c, &xf(FPVec3::new(fp!(0), fp!(0.2), fp!(0)), lie), &b, &bx, M);
    check_basic(&m);
    assert!(near(m.normal.y.abs(), FP::ONE, fp!(0.01)));
    assert!(m.pts[0].sep < -fp!(0.4));
    // Axis skew to the edge of the box: single edge contact.
    let skew = FPQuat::from_axis_angle(FPVec3::Y, fp!(0.7)).hamilton_mul(lie);
    let m = collide(&c, &xf(FPVec3::new(fp!(2.3), fp!(0.65), fp!(0.5)), skew), &b, &bx, M);
    if m.count > 0 {
        check_basic(&m);
    }
}

#[test]
fn box_box_face_edge_and_offsets() {
    let b = Shape::cuboid(fp!(0.5), fp!(0.5), fp!(0.5));
    // Aligned face contact: four corner points.
    let m = collide(&b, &at(fp!(0), fp!(0.5), fp!(0)), &b, &at(fp!(0), fp!(1.49), fp!(0)), M);
    check_basic(&m);
    assert_eq!(m.count, 4);
    assert!(vnear(m.normal, FPVec3::Y, fp!(0.001)));
    for p in &m.pts[..4] {
        assert!(near(p.sep, -fp!(0.01), fp!(0.001)));
        assert!(near(p.point.x.abs(), fp!(0.5), fp!(0.001)) && near(p.point.z.abs(), fp!(0.5), fp!(0.001)));
    }
    // Offset by half a box: the overlap is a 0.5 x 1 rectangle.
    let m = collide(&b, &at(fp!(0), fp!(0.5), fp!(0)), &b, &at(fp!(0.5), fp!(1.49), fp!(0)), M);
    check_basic(&m);
    assert_eq!(m.count, 4);
    for p in &m.pts[..4] {
        assert!(p.point.x >= fp!(-0.001) && p.point.x <= fp!(0.501), "{:?}", p.point);
    }
    // Swapped argument order gives the opposite normal and the same points.
    let m2 = collide(&b, &at(fp!(0.5), fp!(1.49), fp!(0)), &b, &at(fp!(0), fp!(0.5), fp!(0)), M);
    assert_eq!(m2.count, 4);
    assert!(vnear(m2.normal, -FPVec3::Y, fp!(0.001)));
    // Rotated about y by 45 degrees: still four points on the plane.
    let q = FPQuat::from_axis_angle(FPVec3::Y, FP::HALF_PI / 2);
    let m = collide(&b, &at(fp!(0), fp!(0.5), fp!(0)), &b, &xf(FPVec3::new(FP::ZERO, fp!(1.49), FP::ZERO), q), M);
    check_basic(&m);
    assert!(m.count >= 3 && m.count <= 4);
    assert!(vnear(m.normal, FPVec3::Y, fp!(0.001)));
    // Edge on edge: B rotated 45 degrees about x and the other box about
    // z, crossing like an X.
    let qa = FPQuat::from_axis_angle(FPVec3::X, FP::HALF_PI / 2);
    let qb = FPQuat::from_axis_angle(FPVec3::Z, FP::HALF_PI / 2);
    let d = fp!(0.7071);
    let m = collide(&b, &xf(FPVec3::ZERO, qa), &b, &xf(FPVec3::new(FP::ZERO, d * 2 - fp!(0.02), FP::ZERO), qb), M);
    check_basic(&m);
    assert_eq!(m.count, 1);
    assert!(near(m.normal.y, FP::ONE, fp!(0.02)), "{:?}", m.normal);
    // Separated beyond the margin.
    assert_eq!(collide(&b, &at(fp!(0), fp!(0.5), fp!(0)), &b, &at(fp!(0), fp!(1.6), fp!(0)), M).count, 0);
    // Slightly separated within the margin: speculative contact.
    let m = collide(&b, &at(fp!(0), fp!(0.5), fp!(0)), &b, &at(fp!(0), fp!(1.51), fp!(0)), M);
    assert_eq!(m.count, 4);
    assert!(near(m.pts[0].sep, fp!(0.01), fp!(0.001)));
}

#[test]
fn box_box_ids_are_stable_for_a_tiny_rotation() {
    let b = Shape::cuboid(fp!(0.5), fp!(0.5), fp!(0.5));
    let base = collide(&b, &at(fp!(0), fp!(0.5), fp!(0)), &b, &at(fp!(0), fp!(1.49), fp!(0)), M);
    let mut ids: Vec<u32> = base.pts[..base.count].iter().map(|p| p.id).collect();
    ids.sort();
    for k in 1..40 {
        let q = FPQuat::from_axis_angle(FPVec3::Y, FP::from_raw(k * 9));
        let q2 = FPQuat::from_axis_angle(FPVec3::X, FP::from_raw(k * 5));
        let m = collide(&b, &at(fp!(0), fp!(0.5), fp!(0)), &b, &xf(FPVec3::new(FP::ZERO, fp!(1.49), FP::ZERO), q.hamilton_mul(q2)), M);
        let mut got: Vec<u32> = m.pts[..m.count].iter().map(|p| p.id).collect();
        got.sort();
        assert_eq!(got, ids, "ids changed for k={k}");
    }
}

#[test]
fn fast_rotation_integration_matches_the_library_function() {
    use crate::step::integrate_rot;
    use orr_fp::FrameRng;
    let mut rng = FrameRng::new(77);
    let mut q = FPQuat::from_axis_angle(FPVec3::new(fp!(0.6), fp!(0), fp!(0.8)), fp!(0.3));
    for _ in 0..2000 {
        let w = FPVec3::new(rng.range_fp(fp!(-9), fp!(9)), rng.range_fp(fp!(-9), fp!(9)), rng.range_fp(fp!(-9), fp!(9)));
        let dt = FP::from_ratio(1, 60);
        let (a, b) = (integrate_rot(q, w, dt), q.integrate_angular(w, dt));
        assert_eq!(a, b);
        q = a;
    }
}

#[test]
fn closest_point_of_segment_and_box_matches_brute_force() {
    use crate::geom::closest_seg_seg;
    use orr_fp::FrameRng;
    let mut rng = FrameRng::new(5);
    let mut checked = 0;
    for _ in 0..4000 {
        let h = FPVec3::new(rng.range_fp(fp!(0.2), fp!(1.5)), rng.range_fp(fp!(0.2), fp!(1.5)), rng.range_fp(fp!(0.2), fp!(1.5)));
        let mut rp = |s: i32| FPVec3::new(rng.range_fp(fp!(-4), fp!(4)) * s, rng.range_fp(fp!(-4), fp!(4)), rng.range_fp(fp!(-4), fp!(4)));
        let (p0, p1) = (rp(1), rp(1));
        let d = p1 - p0;
        // Skip segments that touch the box (handled by the SAT path).
        let (mut lo, mut hi) = (FP::ZERO, FP::ONE);
        let mut touching = true;
        for j in 0..3 {
            let (pj, dj, hj) = (p0.get(j), d.get(j), h.get(j));
            if dj.raw() == 0 {
                if pj.abs() > hj {
                    touching = false;
                }
            } else {
                let (mut a, mut b) = ((-hj - pj) / dj, (hj - pj) / dj);
                if a > b {
                    core::mem::swap(&mut a, &mut b);
                }
                lo = lo.max(a);
                hi = hi.min(b);
            }
        }
        if touching && lo <= hi {
            continue;
        }
        // Brute force: end points and the 12 edges.
        let clamp = |p: FPVec3| FPVec3::new(p.x.clamp(-h.x, h.x), p.y.clamp(-h.y, h.y), p.z.clamp(-h.z, h.z));
        let mut best = FP::MAX;
        for p in [p0, p1] {
            best = best.min((p - clamp(p)).length_sq());
        }
        for axis in 0..3 {
            let (j, k) = ((axis + 1) % 3, (axis + 2) % 3);
            for sj in [-1i32, 1] {
                for sk in [-1i32, 1] {
                    let mut e0 = FPVec3::ZERO;
                    e0.set(j, h.get(j) * sj);
                    e0.set(k, h.get(k) * sk);
                    let mut e1 = e0;
                    e0.set(axis, -h.get(axis));
                    e1.set(axis, h.get(axis));
                    let (s, t) = closest_seg_seg(p0, p1, e0, e1);
                    let (a, b) = (p0 + d * s, e0 + (e1 - e0) * t);
                    best = best.min((a - b).length_sq());
                }
            }
        }
        let (_, qb, qs, d2) = crate::collide::seg_box_closest(p0, d, h);
        let got = FP::from_raw((d2 >> 16) as i64);
        assert!((got - best).abs() <= fp!(0.002) + best / 200, "closest {got} vs brute {best} for {p0:?} {p1:?} {h:?}");
        assert!((qs - qb).length_sq() - got < fp!(0.002));
        checked += 1;
    }
    assert!(checked > 1000, "only {checked} disjoint cases");
}
