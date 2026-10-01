//! 3D math used by `orr_physics3d`: `FPMat3`, quaternion integration and
//! axis-angle, orthonormal bases. Tests whose names start with `golden_`
//! also run on wasm32 in CI.

use orr_fp::{fp, FPMat3, FPQuat, FPVec3, FrameRng, FP};

fn close(a: FP, b: FP, tol_raw: i64) -> bool {
    (a.raw() - b.raw()).abs() <= tol_raw
}

fn vclose(a: FPVec3, b: FPVec3, tol_raw: i64) -> bool {
    close(a.x, b.x, tol_raw) && close(a.y, b.y, tol_raw) && close(a.z, b.z, tol_raw)
}

fn rand_unit(rng: &mut FrameRng) -> FPVec3 {
    loop {
        let v = FPVec3::new(rng.range_fp(fp!(-1), fp!(1)), rng.range_fp(fp!(-1), fp!(1)), rng.range_fp(fp!(-1), fp!(1)));
        if v.length_sq() > fp!(0.1) && v.length_sq() < FP::ONE {
            return v.normalize();
        }
    }
}

#[test]
fn mat_from_quat_matches_quat_rotation() {
    let mut rng = FrameRng::new(11);
    for _ in 0..200 {
        let q = FPQuat::from_axis_angle(rand_unit(&mut rng), rng.range_fp(fp!(-3), fp!(3)));
        let m = FPMat3::from_quat(q);
        let v = FPVec3::new(rng.range_fp(fp!(-5), fp!(5)), rng.range_fp(fp!(-5), fp!(5)), rng.range_fp(fp!(-5), fp!(5)));
        assert!(vclose(m.mul_vec(v), q.rotate_vec3(v), 40), "{:?} vs {:?}", m.mul_vec(v), q.rotate_vec3(v));
        assert!(vclose(m.tmul_vec(m.mul_vec(v)), v, 250));
        assert!(vclose(m.transpose().mul_vec(v), m.tmul_vec(v), 0));
        assert!(close(m.determinant(), FP::ONE, 400));
    }
}

#[test]
fn mat_inverse_and_product() {
    let m = FPMat3::from_rows(
        FPVec3::new(fp!(2), fp!(0.5), fp!(0)),
        FPVec3::new(fp!(0.25), fp!(3), fp!(1)),
        FPVec3::new(fp!(0), fp!(-1), fp!(1.5)),
    );
    let inv = m.inverse().unwrap();
    let id = m * inv;
    for (i, row) in [id.r0, id.r1, id.r2].into_iter().enumerate() {
        for j in 0..3 {
            let want = if i == j { FP::ONE } else { FP::ZERO };
            assert!(close(row.get(j), want, 8), "{id:?}");
        }
    }
    assert!(FPMat3::ZERO.inverse().is_none());
    let k = FPMat3::skew(FPVec3::new(fp!(1), fp!(2), fp!(3)));
    let u = FPVec3::new(fp!(-2), fp!(0.5), fp!(4));
    assert_eq!(k.mul_vec(u), FPVec3::new(fp!(1), fp!(2), fp!(3)).cross(u));
}

#[test]
fn rotate_diag_is_symmetric_and_matches_product() {
    let mut rng = FrameRng::new(5);
    for _ in 0..100 {
        let q = FPQuat::from_axis_angle(rand_unit(&mut rng), rng.range_fp(fp!(-3), fp!(3)));
        let r = FPMat3::from_quat(q);
        let d = FPVec3::new(fp!(0.5), fp!(1.25), fp!(2));
        let w = r.rotate_diag(d);
        assert_eq!(w.r0.y, w.r1.x);
        assert_eq!(w.r0.z, w.r2.x);
        assert_eq!(w.r1.z, w.r2.y);
        let reference = r * FPMat3::from_diag(d) * r.transpose();
        for i in 0..3 {
            assert!(vclose(w.row(i), reference.row(i), 40));
        }
    }
}

#[test]
fn axis_angle_round_trip() {
    let mut rng = FrameRng::new(3);
    for _ in 0..200 {
        let axis = rand_unit(&mut rng);
        let angle = rng.range_fp(fp!(0.05), fp!(3.0));
        let q = FPQuat::from_axis_angle(axis, angle);
        let (a, ang) = q.to_axis_angle();
        assert!(close(ang, angle, 400), "{ang} vs {angle}");
        assert!(vclose(a, axis, 1500), "{a:?} vs {axis:?}");
    }
    let (a, ang) = FPQuat::IDENTITY.to_axis_angle();
    assert_eq!((a, ang), (FPVec3::X, FP::ZERO));
    // q and -q are the same rotation.
    let q = FPQuat::from_axis_angle(FPVec3::Y, fp!(1));
    let nq = FPQuat::new(-q.x, -q.y, -q.z, -q.w);
    assert!(close(nq.to_axis_angle().1, fp!(1), 100));
}

#[test]
fn integrate_angular_tracks_the_exact_rotation() {
    let omega = FPVec3::new(fp!(0), fp!(2), fp!(0));
    let dt = FP::from_ratio(1, 240);
    let mut q = FPQuat::IDENTITY;
    for _ in 0..240 {
        q = q.integrate_angular(omega, dt);
    }
    // One second at 2 rad/s about Y.
    let want = FPQuat::from_axis_angle(FPVec3::Y, fp!(2));
    assert!(close(q.y, want.y, 700), "{q:?} vs {want:?}");
    assert!(close(q.w, want.w, 700));
    // Stays normalized.
    assert!(close(q.length_sq(), FP::ONE, 80));
    // Zero velocity is a no-op.
    assert_eq!(FPQuat::IDENTITY.integrate_angular(FPVec3::ZERO, dt), FPQuat::IDENTITY);
}

#[test]
fn orthonormal_basis_is_right_handed() {
    let mut rng = FrameRng::new(9);
    let mut axes = vec![FPVec3::X, FPVec3::Y, FPVec3::Z, -FPVec3::X, -FPVec3::Y, -FPVec3::Z];
    for _ in 0..200 {
        axes.push(rand_unit(&mut rng));
    }
    for n in axes {
        let (t1, t2) = n.orthonormal_basis();
        assert!(close(t1.length_sq(), FP::ONE, 80));
        assert!(close(t2.length_sq(), FP::ONE, 160));
        assert!(t1.dot(n).abs().raw() < 80);
        assert!(t2.dot(n).abs().raw() < 160);
        assert!(t1.dot(t2).abs().raw() < 160);
    }
}

fn fnv(vals: &[i64]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &v in vals {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

const GOLDEN: u64 = 0x1e80d8a6af9b420d;

/// Pins the new 3D math over a fixed sequence of operations.
#[test]
fn golden_math3d() {
    let mut rng = FrameRng::new(2024);
    let mut vals: Vec<i64> = Vec::new();
    let mut q = FPQuat::from_axis_angle(FPVec3::new(fp!(0.6), fp!(0), fp!(0.8)), fp!(0.7));
    for i in 0..64 {
        let w = FPVec3::new(rng.range_fp(fp!(-4), fp!(4)), rng.range_fp(fp!(-4), fp!(4)), rng.range_fp(fp!(-4), fp!(4)));
        q = q.integrate_angular(w, FP::from_ratio(1, 60));
        let m = q.to_mat3();
        let d = FPVec3::new(fp!(0.5), fp!(1.25), fp!(2) + FP::from_int(i % 3));
        let iw = m.rotate_diag(d);
        let v = m.mul_vec(w) + iw.tmul_vec(w);
        let (ax, an) = q.to_axis_angle();
        let (t1, t2) = ax.orthonormal_basis();
        for x in [q.x, q.y, q.z, q.w, v.x, v.y, v.z, an, ax.x, ax.y, ax.z, t1.x, t1.y, t1.z, t2.x, t2.y, t2.z, iw.r0.x, iw.r1.y, iw.r2.z, iw.r0.y] {
            vals.push(x.raw());
        }
        if let Some(inv) = iw.inverse() {
            vals.push(inv.r0.x.raw());
            vals.push(inv.r1.y.raw());
            vals.push(inv.determinant().raw());
        }
    }
    let h = fnv(&vals);
    println!("golden math3d: {h:#018x}");
    assert_eq!(h, GOLDEN, "3D math golden changed");
}
