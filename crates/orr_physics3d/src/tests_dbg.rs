use crate::collide::collide;
use crate::geom::Xf;
use crate::types::Shape;
use orr_fp::{fp, FPMat3, FPQuat, FPVec3, FP};

fn xf(p: FPVec3, q: FPQuat) -> Xf {
    Xf { p, r: FPMat3::from_quat(q) }
}

#[test]
fn print_aligned_stack_manifold() {
    let a = Shape::cuboid(fp!(0.5), fp!(0.5), fp!(0.5));
    for (i, q) in [FPQuat::IDENTITY, FPQuat::from_axis_angle(FPVec3::Y, fp!(0.001)), FPQuat::from_axis_angle(FPVec3::X, fp!(0.0005))].into_iter().enumerate() {
        let xa = xf(FPVec3::new(FP::ZERO, fp!(0.5), FP::ZERO), FPQuat::IDENTITY);
        let xb = xf(FPVec3::new(FP::ZERO, fp!(1.49), FP::ZERO), q);
        let m = collide(&a, &xa, &a, &xb, fp!(0.02));
        println!("case {i} normal {:?} count {}", m.normal, m.count);
        for p in &m.pts[..m.count] {
            println!("   id {} sep {:?} pt {:?}", p.id, p.sep, p.point);
        }
    }
}
