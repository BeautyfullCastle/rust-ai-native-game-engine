//! Pins a checksum over a fixed sequence of `orr_fp` operations.
//!
//! This is the test CI should run on x86_64, aarch64 and wasm32 and
//! confirm all three report the same `GOLDEN` checksum: since every
//! operation in this crate is implemented with plain integer arithmetic
//! (no floats), the checksum must be bit-identical on every target.

use orr_fp::{fp, FPQuat, FPVec3, FrameRng, FP};

/// FNV-1a over the little-endian bytes of every raw value produced by the
/// fixed sequence below.
fn fnv1a(bytes: &[u8], mut h: u64) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn checksum(vals: &[i64]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &v in vals {
        h = fnv1a(&v.to_le_bytes(), h);
    }
    h
}

/// The pinned golden checksum. Regenerate by running this test with
/// `--nocapture` and reading the printed value if the sequence below is
/// ever intentionally changed.
const GOLDEN: u64 = 0x1555d30109e62487;

#[test]
fn determinism_golden() {
    let mut vals: Vec<i64> = Vec::new();

    // Basic arithmetic.
    let a = fp!(3.25);
    let b = fp!(-1.75);
    vals.push((a + b).raw());
    vals.push((a - b).raw());
    vals.push((a * b).raw());
    vals.push((a / b).raw());
    vals.push(a.mul_fast(b).raw());
    vals.push((a % b).raw());
    vals.push(FP::from_ratio(22, 7).raw());

    // floor/ceil/round/fract/lerp/clamp.
    vals.push(a.floor().raw());
    vals.push(a.ceil().raw());
    vals.push(a.round().raw());
    vals.push(a.fract().raw());
    vals.push(a.lerp(b, fp!(0.3)).raw());
    vals.push(a.clamp(FP::ZERO, fp!(2)).raw());

    // Trig.
    for i in 0..16 {
        let angle = FP::from_int(i) * fp!(0.4) - fp!(3.0);
        let (s, c) = angle.sin_cos();
        vals.push(s.raw());
        vals.push(c.raw());
        vals.push(angle.atan().raw());
    }
    vals.push(fp!(0.5).asin().raw());
    vals.push(fp!(0.5).acos().raw());
    vals.push(fp!(3.0).atan2(fp!(-2.0)).raw());

    // sqrt / exp / ln / pow_int.
    for i in 1..12 {
        let x = FP::from_int(i) * fp!(0.75);
        vals.push(x.sqrt().raw());
        vals.push(x.exp().raw());
        vals.push(x.ln().raw());
        vals.push(x.pow_int(3).raw());
    }

    // Vectors.
    let v1 = FPVec3::new(fp!(1.0), fp!(-2.0), fp!(3.5));
    let v2 = FPVec3::new(fp!(-0.5), fp!(4.0), fp!(-1.25));
    vals.push(v1.dot(v2).raw());
    let cross = v1.cross(v2);
    vals.push(cross.x.raw());
    vals.push(cross.y.raw());
    vals.push(cross.z.raw());
    vals.push(v1.length().raw());
    let n = v1.normalize();
    vals.push(n.x.raw());
    vals.push(n.y.raw());
    vals.push(n.z.raw());

    // Quaternion.
    let q1 = FPQuat::from_axis_angle(FPVec3::Y, fp!(0.7));
    let q2 = FPQuat::from_euler(fp!(0.3), fp!(-0.2), fp!(0.1));
    let q3 = q1 * q2;
    vals.push(q3.x.raw());
    vals.push(q3.y.raw());
    vals.push(q3.z.raw());
    vals.push(q3.w.raw());
    let rotated = q3.normalize().rotate_vec3(v1);
    vals.push(rotated.x.raw());
    vals.push(rotated.y.raw());
    vals.push(rotated.z.raw());
    let (yaw, pitch, roll) = q2.to_euler();
    vals.push(yaw.raw());
    vals.push(pitch.raw());
    vals.push(roll.raw());

    // RNG.
    let mut rng = FrameRng::new(0xDEAD_BEEF_1234_5678);
    for _ in 0..32 {
        vals.push(rng.next_u32() as i64);
    }
    vals.push(rng.next_u64() as i64);
    vals.push(rng.range_i32(-100, 100) as i64);
    vals.push(rng.next_fp01().raw());
    vals.push(rng.range_fp(fp!(-5), fp!(5)).raw());
    let mut forked = rng.fork(7);
    vals.push(forked.next_u32() as i64);

    let sum = checksum(&vals);
    eprintln!("determinism checksum = {sum:#018x} (over {} values)", vals.len());
    assert_eq!(sum, GOLDEN, "determinism checksum changed! new value: {sum:#018x}");
}
