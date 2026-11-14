//! Two-bone inverse kinematics on a pose: an arm or leg bends so its end
//! reaches a target, with the elbow or knee turned toward a pole point.
use super::graph::{quat_conjugate, quat_mul};
use super::{normalize_quaternion, AnimatedModel, Pose};
use crate::{invalid, Error};

type Vec3 = [f32; 3];

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

fn unit(a: Vec3) -> Option<Vec3> {
    let l = length(a);
    (l > 1e-6).then(|| a.map(|v| v / l))
}

/// Angle between two unit vectors.
fn angle(a: Vec3, b: Vec3) -> f32 {
    dot(a, b).clamp(-1.0, 1.0).acos()
}

fn axis_angle(axis: Vec3, radians: f32) -> [f32; 4] {
    let (s, c) = (radians * 0.5).sin_cos();
    [axis[0] * s, axis[1] * s, axis[2] * s, c]
}

fn position(pose: &Pose, node: usize) -> Vec3 {
    let m = pose.global[node];
    [m[3][0], m[3][1], m[3][2]]
}

impl AnimatedModel {
    /// Rotation of `node` in model space: its ancestors' local rotations
    /// composed with its own. Scale is ignored, so nonuniform scale on the
    /// chain makes the result approximate.
    fn model_rotation(&self, pose: &Pose, node: usize) -> [f32; 4] {
        let mut chain = vec![node];
        while let Some(parent) = self.parents[*chain.last().expect("starts with node")] {
            chain.push(parent);
        }
        chain.iter().rev().fold([0.0, 0.0, 0.0, 1.0], |q, &n| normalize_quaternion(quat_mul(q, pose.local[n].rotation)))
    }

    /// Rotates `node` so that, in model space, `rotation` is applied after its current rotation.
    fn rotate_node(&self, pose: &Pose, node: usize, rotation: [f32; 4]) -> Result<Pose, Error> {
        let global = self.model_rotation(pose, node);
        let in_local = quat_mul(quat_mul(quat_conjugate(global), rotation), global);
        let mut local = pose.local.clone();
        local[node].rotation = normalize_quaternion(quat_mul(local[node].rotation, in_local));
        self.pose_from_local(local)
    }

    /// Bends the parent and grandparent of `end` so that `end` reaches
    /// `target`, with the middle joint turned toward `pole`; both points are
    /// in model space. A target out of reach straightens the chain toward it.
    /// Only those two local rotations change.
    pub fn two_bone_ik(&self, pose: &Pose, end: &str, target: Vec3, pole: Vec3) -> Result<Pose, Error> {
        self.validate_pose(pose)?;
        if !target.iter().chain(&pole).all(|v| v.is_finite()) {
            return Err(invalid("nonfinite IK target or pole"));
        }
        let c_node = self.node_index(end).ok_or_else(|| invalid("IK end node not found"))? as usize;
        let b_node = self.parents[c_node].ok_or_else(|| invalid("IK end node has no parent"))?;
        let a_node = self.parents[b_node].ok_or_else(|| invalid("IK middle node has no parent"))?;
        let (a, b, c) = (position(pose, a_node), position(pose, b_node), position(pose, c_node));
        let (upper, lower) = (length(sub(b, a)), length(sub(c, b)));
        if upper < 1e-6 || lower < 1e-6 {
            return Err(invalid("IK bones need nonzero length"));
        }
        let reach = length(sub(target, a)).clamp((upper - lower).abs() + 1e-4, (upper + lower) * 0.9999);
        // 1. Bend the middle joint so the chain spans the reach, in the plane that holds the pole.
        let (Some(ba), Some(bc), Some(ac)) = (unit(sub(a, b)), unit(sub(c, b)), unit(sub(c, a))) else {
            return Err(invalid("IK chain is folded onto itself"));
        };
        let middle_wanted = ((reach * reach - upper * upper - lower * lower) / (-2.0 * upper * lower)).clamp(-1.0, 1.0).acos();
        let plane = unit(cross(sub(pole, a), ac)).or_else(|| unit(cross(ba, bc))).ok_or_else(|| invalid("IK pole is on the chain's line"))?;
        let bend = self.rotate_node(pose, b_node, axis_angle(plane, angle(ba, bc) - middle_wanted))?;
        // 2. Swing the whole chain from the root so the end points at the target.
        let now = unit(sub(position(&bend, c_node), a)).ok_or_else(|| invalid("IK chain is folded onto itself"))?;
        let wanted = unit(sub(target, a)).unwrap_or(now);
        let swing_axis = unit(cross(now, wanted));
        let swung = match swing_axis {
            Some(axis) => self.rotate_node(&bend, a_node, axis_angle(axis, angle(now, wanted)))?,
            None => bend,
        };
        // 3. Twist about the root-to-target line so the middle joint faces the pole.
        let knee = sub(position(&swung, b_node), a);
        let off_line = |v: Vec3| sub(v, wanted.map(|w| w * dot(v, wanted)));
        match (unit(off_line(knee)), unit(off_line(sub(pole, a)))) {
            (Some(from), Some(to)) => {
                let sign = if dot(cross(from, to), wanted) < 0.0 { -1.0 } else { 1.0 };
                self.rotate_node(&swung, a_node, axis_angle(wanted, sign * angle(from, to)))
            }
            _ => Ok(swung),
        }
    }
}
