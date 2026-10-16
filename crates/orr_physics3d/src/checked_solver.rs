//! Fallible arithmetic for the optional static-contact path. The algorithms
//! and rounding match `solver`; every narrow intermediate is checked before
//! conversion. Malformed caches or extreme over-constraint fail atomically
//! instead of wrapping in release builds. The convex-only fast path is intact.

use orr_fp::{FPMat3, FPVec3, FP};

use crate::solver::{BodyInv, Constraint, ContactPt, Row, Vw};
use crate::PhysicsConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NumericOverflow;
type Result<T> = core::result::Result<T, NumericOverflow>;

fn narrow(raw: i128) -> Result<i64> {
    i64::try_from(raw).map_err(|_| NumericOverflow)
}
fn add(a: FP, b: FP) -> Result<FP> {
    Ok(FP::from_raw(narrow(
        i128::from(a.raw()) + i128::from(b.raw()),
    )?))
}
fn sub(a: FP, b: FP) -> Result<FP> {
    Ok(FP::from_raw(narrow(
        i128::from(a.raw()) - i128::from(b.raw()),
    )?))
}
fn neg(a: FP) -> Result<FP> {
    sub(FP::ZERO, a)
}
fn sum(a: i64, b: i64) -> Result<i64> {
    narrow(i128::from(a) + i128::from(b))
}
fn difference(a: i64, b: i64) -> Result<i64> {
    narrow(i128::from(a) - i128::from(b))
}
fn shift(raw: i64) -> Result<FP> {
    Ok(FP::from_raw(sum(raw, 32768)? >> 16))
}
fn mul(a: FP, b: FP) -> Result<FP> {
    shift(narrow(i128::from(a.raw()) * i128::from(b.raw()))?)
}
fn dot(a: FPVec3, b: FPVec3) -> Result<i64> {
    let mut raw = 0;
    for axis in 0..3 {
        let product = narrow(i128::from(a.get(axis).raw()) * i128::from(b.get(axis).raw()))?;
        raw = sum(raw, product)?;
    }
    Ok(raw)
}
fn vector_add(a: FPVec3, b: FPVec3) -> Result<FPVec3> {
    Ok(FPVec3::new(add(a.x, b.x)?, add(a.y, b.y)?, add(a.z, b.z)?))
}
fn vector_sub(a: FPVec3, b: FPVec3) -> Result<FPVec3> {
    Ok(FPVec3::new(sub(a.x, b.x)?, sub(a.y, b.y)?, sub(a.z, b.z)?))
}
fn scale(v: FPVec3, s: FP) -> Result<FPVec3> {
    Ok(FPVec3::new(mul(v.x, s)?, mul(v.y, s)?, mul(v.z, s)?))
}
fn cross(a: FPVec3, b: FPVec3) -> Result<FPVec3> {
    Ok(FPVec3::new(
        sub(mul(a.y, b.z)?, mul(a.z, b.y)?)?,
        sub(mul(a.z, b.x)?, mul(a.x, b.z)?)?,
        sub(mul(a.x, b.y)?, mul(a.y, b.x)?)?,
    ))
}
fn mat_vec(m: &FPMat3, v: FPVec3) -> Result<FPVec3> {
    Ok(FPVec3::new(
        shift(dot(m.r0, v)?)?,
        shift(dot(m.r1, v)?)?,
        shift(dot(m.r2, v)?)?,
    ))
}
fn rel_vel(va: &Vw, vb: &Vw, direction: FPVec3, row: &Row) -> Result<FP> {
    let linear = dot(vector_sub(vb.v, va.v)?, direction)?;
    shift(difference(
        sum(linear, dot(vb.w, row.rbd)?)?,
        dot(va.w, row.rad)?,
    )?)
}
fn apply(
    va: &mut Vw,
    vb: &mut Vw,
    direction: FPVec3,
    ima: FP,
    imb: FP,
    row: &Row,
    impulse: FP,
) -> Result<()> {
    if ima != FP::ZERO {
        va.v = vector_sub(va.v, scale(direction, mul(ima, impulse)?)?)?;
        va.w = vector_sub(va.w, scale(row.wad, impulse)?)?;
    }
    if imb != FP::ZERO {
        vb.v = vector_add(vb.v, scale(direction, mul(imb, impulse)?)?)?;
        vb.w = vector_add(vb.w, scale(row.wbd, impulse)?)?;
    }
    Ok(())
}

pub(crate) fn prepare(
    cons: &mut [Constraint],
    pool: &mut [ContactPt],
    bodies: &[BodyInv],
    vw: &[Vw],
    cfg: &PhysicsConfig,
) -> Result<()> {
    for c in cons {
        let (a, b) = (c.a as usize, c.b as usize);
        let (ba, bb) = (&bodies[a], &bodies[b]);
        let (ima, imb) = (ba.inv_mass, bb.inv_mass);
        c.ima = ima;
        c.imb = imb;
        let (t1, t2) = crate::solver::basis(c.normal);
        c.t1 = t1;
        c.t2 = t2;
        let (va, vb) = (vw[a], vw[b]);
        let a_ang = ima > FP::ZERO || va.w != FPVec3::ZERO;
        let b_ang = imb > FP::ZERO || vb.w != FPVec3::ZERO;
        for p in &mut pool[c.first as usize..(c.first + c.count) as usize] {
            let ra = vector_sub(p.point, ba.pos)?;
            let rb = vector_sub(p.point, bb.pos)?;
            for (row, d) in p.rows.iter_mut().zip([c.normal, t1, t2]) {
                row.rad = if a_ang { cross(ra, d)? } else { FPVec3::ZERO };
                row.rbd = if b_ang { cross(rb, d)? } else { FPVec3::ZERO };
                row.wad = if ima > FP::ZERO {
                    mat_vec(&ba.inv_inertia, row.rad)?
                } else {
                    FPVec3::ZERO
                };
                row.wbd = if imb > FP::ZERO {
                    mat_vec(&bb.inv_inertia, row.rbd)?
                } else {
                    FPVec3::ZERO
                };
                let kk = add(
                    add(ima, imb)?,
                    shift(sum(dot(row.wad, row.rad)?, dot(row.wbd, row.rbd)?)?)?,
                )?;
                row.mass = if kk > FP::ZERO {
                    FP::ONE / kk
                } else {
                    FP::ZERO
                };
            }
            let vn0 = rel_vel(&va, &vb, c.normal, &p.rows[0])?;
            p.rest = if p.sep <= FP::ZERO && vn0 < neg(cfg.restitution_threshold)? {
                neg(mul(c.restitution, vn0)?)?
            } else {
                FP::ZERO
            };
            p.j1 = shift(dot(p.jt, t1)?)?;
            p.j2 = shift(dot(p.jt, t2)?)?;
        }
    }
    Ok(())
}

pub(crate) fn update_targets(
    cons: &[Constraint],
    pool: &mut [ContactPt],
    pw: &[Vw],
    cfg: &PhysicsConfig,
    h: FP,
) -> Result<()> {
    if h <= FP::ZERO {
        return Err(NumericOverflow);
    }
    let inv_h = FP::ONE / h;
    for c in cons {
        let (pa, pb) = (&pw[c.a as usize], &pw[c.b as usize]);
        let moved = pa.v != FPVec3::ZERO
            || pa.w != FPVec3::ZERO
            || pb.v != FPVec3::ZERO
            || pb.w != FPVec3::ZERO;
        for p in &mut pool[c.first as usize..(c.first + c.count) as usize] {
            let sep = if moved && !c.refreshed {
                add(p.sep, rel_vel(pa, pb, c.normal, &p.rows[0])?)?
            } else {
                p.sep
            };
            let bias = if sep >= FP::ZERO {
                mul(sep, inv_h)?
            } else {
                mul(
                    mul(cfg.baumgarte, add(sep, cfg.linear_slop)?.min(FP::ZERO))?,
                    inv_h,
                )?
                .max(neg(cfg.max_correction_speed)?)
            };
            // A separated refreshed contact permits approach by the gap/h.
            // Restitution zero must not turn the query envelope into a floor.
            p.target = if c.refreshed && sep > FP::ZERO {
                neg(bias)?
            } else {
                neg(bias)?.max(p.rest)
            };
        }
    }
    Ok(())
}

/// Current endpoint separation using the same checked anchor-motion terms
/// as target updates. Refreshed geometry already supplies endpoint separation.
pub(crate) fn endpoint_separation(
    constraint: &Constraint,
    point: &ContactPt,
    motion: &[Vw],
) -> Result<FP> {
    if constraint.refreshed {
        Ok(point.sep)
    } else {
        add(
            point.sep,
            rel_vel(
                &motion[constraint.a as usize],
                &motion[constraint.b as usize],
                constraint.normal,
                &point.rows[0],
            )?,
        )
    }
}

/// A positive normal impulse must act against gravity to ground a dynamic
/// island. Tangential/friction-only wall support is conservatively excluded.
pub(crate) fn supports_gravity(
    normal: FPVec3,
    gravity: FPVec3,
    dynamic_is_b: bool,
) -> Result<bool> {
    let projection = dot(normal, gravity)?;
    Ok(if dynamic_is_b {
        projection < 0
    } else {
        projection > 0
    })
}

pub(crate) fn warm_start(cons: &[Constraint], pool: &[ContactPt], vw: &mut [Vw]) -> Result<()> {
    for c in cons {
        let (a, b) = (c.a as usize, c.b as usize);
        let (mut va, mut vb) = (vw[a], vw[b]);
        for p in &pool[c.first as usize..(c.first + c.count) as usize] {
            for (impulse, direction, row) in [
                (p.j1, c.t1, &p.rows[1]),
                (p.j2, c.t2, &p.rows[2]),
                (p.jn, c.normal, &p.rows[0]),
            ] {
                if impulse != FP::ZERO {
                    apply(&mut va, &mut vb, direction, c.ima, c.imb, row, impulse)?;
                }
            }
        }
        vw[a] = va;
        vw[b] = vb;
    }
    Ok(())
}

pub(crate) fn solve(
    cons: &mut [Constraint],
    pool: &mut [ContactPt],
    vw: &mut [Vw],
    iterations: u32,
) -> Result<()> {
    let n = cons.len();
    for iteration in 0..iterations {
        for k in 0..n {
            let c = &mut cons[if iteration & 1 == 0 { k } else { n - 1 - k }];
            let (a, b) = (c.a as usize, c.b as usize);
            let (mut va, mut vb) = (vw[a], vw[b]);
            let points = &mut pool[c.first as usize..(c.first + c.count) as usize];
            for p in points.iter_mut() {
                let max_f = mul(c.friction, p.jn)?;
                if max_f < FP::ZERO {
                    return Err(NumericOverflow);
                }
                if max_f == FP::ZERO && p.j1 == FP::ZERO && p.j2 == FP::ZERO {
                    continue;
                }
                let vt1 = rel_vel(&va, &vb, c.t1, &p.rows[1])?;
                let vt2 = rel_vel(&va, &vb, c.t2, &p.rows[2])?;
                let new1 = sub(p.j1, mul(p.rows[1].mass, vt1)?)?.clamp(neg(max_f)?, max_f);
                let new2 = sub(p.j2, mul(p.rows[2].mass, vt2)?)?.clamp(neg(max_f)?, max_f);
                let (lambda1, lambda2) = (sub(new1, p.j1)?, sub(new2, p.j2)?);
                p.j1 = new1;
                p.j2 = new2;
                apply(&mut va, &mut vb, c.t1, c.ima, c.imb, &p.rows[1], lambda1)?;
                apply(&mut va, &mut vb, c.t2, c.ima, c.imb, &p.rows[2], lambda2)?;
            }
            for p in points {
                let vn = rel_vel(&va, &vb, c.normal, &p.rows[0])?;
                let new_jn = add(p.jn, mul(p.rows[0].mass, sub(p.target, vn)?)?)?.max(FP::ZERO);
                let lambda = sub(new_jn, p.jn)?;
                p.jn = new_jn;
                apply(&mut va, &mut vb, c.normal, c.ima, c.imb, &p.rows[0], lambda)?;
            }
            vw[a] = va;
            vw[b] = vb;
        }
    }
    Ok(())
}

pub(crate) fn store_friction(cons: &[Constraint], pool: &mut [ContactPt]) -> Result<()> {
    for c in cons {
        for p in &mut pool[c.first as usize..(c.first + c.count) as usize] {
            p.jt = vector_add(scale(c.t1, p.j1)?, scale(c.t2, p.j2)?)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solver;
    use orr_fp::fp;

    #[test]
    fn checked_and_original_solver_match_each_safe_stage() {
        let cfg = PhysicsConfig::default();
        let bodies = [
            BodyInv {
                pos: FPVec3::ZERO,
                ..BodyInv::default()
            },
            BodyInv {
                pos: FPVec3::new(FP::ONE, FP::HALF, fp!(2)),
                inv_mass: FP::ONE,
                inv_inertia: FPMat3::IDENTITY,
            },
            BodyInv {
                pos: FPVec3::new(-FP::ONE, FP::ONE, FP::ONE),
                inv_mass: FP::HALF,
                inv_inertia: FPMat3::IDENTITY,
            },
        ];
        let mut cons = [
            Constraint {
                a: 0,
                b: 1,
                count: 2,
                normal: FPVec3::Y,
                friction: fp!(0.7),
                restitution: fp!(0.8),
                first: 0,
                ..Constraint::default()
            },
            Constraint {
                a: 1,
                b: 2,
                count: 1,
                normal: FPVec3::X,
                friction: fp!(0.5),
                restitution: fp!(0.1),
                first: 2,
                ..Constraint::default()
            },
        ];
        let mut pool = [
            ContactPt {
                point: FPVec3::new(FP::ONE, FP::ZERO, fp!(2)),
                sep: fp!(-0.05),
                jn: fp!(0.2),
                jt: FPVec3::new(fp!(0.1), FP::ZERO, fp!(-0.1)),
                ..ContactPt::default()
            },
            ContactPt {
                point: FPVec3::new(fp!(1.1), FP::ZERO, fp!(2.1)),
                sep: fp!(0.01),
                jn: fp!(0.1),
                ..ContactPt::default()
            },
            ContactPt {
                point: FPVec3::new(FP::ZERO, FP::ONE, FP::ONE),
                sep: fp!(-0.02),
                jn: fp!(0.3),
                ..ContactPt::default()
            },
        ];
        let mut velocities = [
            Vw::default(),
            Vw {
                v: FPVec3::new(fp!(0.3), fp!(-2), fp!(-0.7)),
                w: FPVec3::new(fp!(0.2), fp!(-0.1), fp!(0.3)),
            },
            Vw {
                v: FPVec3::new(fp!(-0.1), fp!(-0.2), fp!(0.4)),
                w: FPVec3::ZERO,
            },
        ];
        let (mut checked_cons, mut checked_pool, mut checked_velocities) = (cons, pool, velocities);
        solver::prepare(&mut cons, &mut pool, &bodies, &velocities, &cfg);
        prepare(
            &mut checked_cons,
            &mut checked_pool,
            &bodies,
            &checked_velocities,
            &cfg,
        )
        .unwrap();
        assert_eq!(cons, checked_cons);
        assert_eq!(pool, checked_pool);
        let movement = [
            Vw::default(),
            Vw {
                v: FPVec3::new(fp!(0.01), fp!(-0.01), fp!(0.02)),
                w: FPVec3::new(fp!(0.001), FP::ZERO, fp!(-0.002)),
            },
            Vw::default(),
        ];
        for _ in 0..8 {
            let h = FP::from_raw(cfg.dt.raw() / 8);
            solver::update_targets(&cons, &mut pool, &movement, &cfg, h);
            update_targets(&checked_cons, &mut checked_pool, &movement, &cfg, h).unwrap();
            assert_eq!(pool, checked_pool);
            solver::warm_start(&cons, &pool, &mut velocities);
            warm_start(&checked_cons, &checked_pool, &mut checked_velocities).unwrap();
            assert_eq!(velocities, checked_velocities);
            solver::solve(&mut cons, &mut pool, &mut velocities, 3);
            solve(
                &mut checked_cons,
                &mut checked_pool,
                &mut checked_velocities,
                3,
            )
            .unwrap();
            assert_eq!(velocities, checked_velocities);
            assert_eq!(pool, checked_pool);
            solver::store_friction(&cons, &mut pool);
            store_friction(&checked_cons, &mut checked_pool).unwrap();
            assert_eq!(pool, checked_pool);
        }
    }

    #[test]
    fn overflow_is_an_error_before_narrow_multiplication() {
        assert_eq!(
            mul(FP::from_int(50_000), FP::from_int(50_000)),
            Err(NumericOverflow)
        );
        assert_eq!(
            dot(
                FPVec3::splat(FP::from_int(30_000)),
                FPVec3::splat(FP::from_int(30_000))
            ),
            Err(NumericOverflow)
        );
    }

    #[test]
    fn refreshed_speculative_contact_allows_approach() {
        let cfg = PhysicsConfig::default();
        let cons = [Constraint {
            a: 0,
            b: 1,
            count: 1,
            refreshed: true,
            ..Constraint::default()
        }];
        let mut pool = [ContactPt {
            sep: fp!(0.1),
            ..ContactPt::default()
        }];
        update_targets(&cons, &mut pool, &[Vw::default(); 2], &cfg, fp!(0.01)).unwrap();
        assert!(pool[0].target < fp!(-9));
    }
}
