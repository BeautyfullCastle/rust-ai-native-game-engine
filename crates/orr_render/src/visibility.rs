//! View culling of procedural instances: what is outside the camera's view
//! (frustum culling) or hidden behind large boxes and planes (occlusion
//! culling) is left out of the main pass. Shadow passes still draw every
//! instance, so an object off screen or behind a wall keeps casting its shadow.
//!
//! Occlusion runs on the CPU. The largest boxes and planes on screen are
//! drawn into a small depth buffer ([`OCCLUSION_WIDTH`] x [`OCCLUSION_HEIGHT`]),
//! each face at the depth of its farthest corner and only into cells it
//! covers completely. Faces are drawn whole, not as two triangles, so no
//! uncovered seam runs along their diagonal. An instance is hidden when every cell its bounding
//! sphere covers holds an occluder nearer than the sphere's nearest point.
//! Both choices err toward drawing, so culling never removes a visible object;
//! it misses some hidden ones.
#![allow(clippy::float_arithmetic)]

use crate::{Instance3D, MeshKind};

/// Which culling the procedural instances of a frame get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Culling {
    /// Leave out instances outside the camera's view.
    pub frustum: bool,
    /// Leave out instances hidden behind large boxes and planes.
    pub occlusion: bool,
}

impl Default for Culling {
    fn default() -> Self {
        Self { frustum: true, occlusion: true }
    }
}

/// Columns of the occlusion depth buffer.
pub const OCCLUSION_WIDTH: usize = 128;
/// Rows of the occlusion depth buffer.
pub const OCCLUSION_HEIGHT: usize = 72;
/// Most boxes and planes drawn as occluders in one frame, the largest on screen first.
pub const MAX_OCCLUDERS: usize = 64;
/// Smallest radius over view distance of a box or plane that occludes.
const MIN_OCCLUDER_SIZE: f32 = 0.05;

type Mat4 = [[f32; 4]; 4];

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = cross(u, v).map(|c| 2.0 * c);
    let w = cross(u, t);
    [v[0] + q[3] * t[0] + w[0], v[1] + q[3] * t[1] + w[1], v[2] + q[3] * t[2] + w[2]]
}

/// Radius of a sphere around the instance's center that holds all of it.
pub fn bounding_radius(kind: MeshKind, instance: &Instance3D) -> f32 {
    let s = instance.scale;
    let largest = s[0].max(s[1]).max(s[2]);
    match kind {
        MeshKind::Sphere => largest,
        MeshKind::Box => dot3(s, s).sqrt(),
        MeshKind::Capsule => largest + instance.half_length,
        MeshKind::Plane => (s[0] * s[0] + s[2] * s[2]).sqrt(),
    }
}

/// The camera as culling sees it: its view-projection matrix and its eye and
/// forward direction, for distances along the view.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub view_proj: Mat4,
    pub eye: [f32; 3],
    /// Unit direction the camera looks.
    pub forward: [f32; 3],
}

impl View {
    fn row(&self, i: usize) -> [f32; 4] {
        let m = &self.view_proj;
        [m[0][i], m[1][i], m[2][i], m[3][i]]
    }

    /// Whether a sphere touches the view. Clip depth runs 0..1 (wgpu).
    pub fn sees_sphere(&self, center: [f32; 3], radius: f32) -> bool {
        let [r0, r1, r2, r3] = [0, 1, 2, 3].map(|i| self.row(i));
        let add = |a: [f32; 4], b: [f32; 4], sign: f32| std::array::from_fn::<f32, 4, _>(|i| a[i] + sign * b[i]);
        let planes = [add(r3, r0, 1.0), add(r3, r0, -1.0), add(r3, r1, 1.0), add(r3, r1, -1.0), r2, add(r3, r2, -1.0)];
        planes.iter().all(|p| {
            let length = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            length < 1e-12 || dot3([p[0], p[1], p[2]], center) + p[3] >= -radius * length
        })
    }

    fn depth(&self, p: [f32; 3]) -> f32 {
        dot3([p[0] - self.eye[0], p[1] - self.eye[1], p[2] - self.eye[2]], self.forward)
    }

    /// Buffer cell coordinates (x right, y down, fractional) of a point, or
    /// `None` at or behind the near plane.
    fn cell(&self, p: [f32; 3]) -> Option<[f32; 2]> {
        let m = &self.view_proj;
        let clip: [f32; 4] = std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]);
        if clip[3] <= 1e-6 || clip[2] < 0.0 {
            return None;
        }
        let (x, y) = (clip[0] / clip[3], clip[1] / clip[3]);
        Some([(x * 0.5 + 0.5) * OCCLUSION_WIDTH as f32, (0.5 - y * 0.5) * OCCLUSION_HEIGHT as f32])
    }
}

/// The occluder depth of every cell: the nearest occluder's farthest depth, or infinity.
pub struct OcclusionBuffer {
    depth: Vec<f32>,
}

impl OcclusionBuffer {
    /// Draws the faces turned toward the eye of the boxes and planes of `instances` (by kind) that are large enough on screen.
    pub fn build(view: &View, instances: [&[Instance3D]; 4]) -> Self {
        let mut buffer = Self { depth: vec![f32::INFINITY; OCCLUSION_WIDTH * OCCLUSION_HEIGHT] };
        let mut candidates: Vec<(f32, MeshKind, &Instance3D)> = [MeshKind::Box, MeshKind::Plane]
            .into_iter()
            .flat_map(|kind| instances[kind as usize].iter().map(move |i| (kind, i)))
            .filter_map(|(kind, i)| {
                let distance = view.depth(i.pos);
                let size = bounding_radius(kind, i) / distance.max(1e-3);
                (distance > 0.0 && size >= MIN_OCCLUDER_SIZE).then_some((size, kind, i))
            })
            .collect();
        // Largest first; equal sizes keep list order, so the choice is the same every frame.
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, kind, instance) in candidates.into_iter().take(MAX_OCCLUDERS) {
            for (face, normal) in faces(kind, instance) {
                // A face turned away from the eye is culled in the main pass, so it hides nothing.
                if dot3(normal, [view.eye[0] - face[0][0], view.eye[1] - face[0][1], view.eye[2] - face[0][2]]) > 0.0 {
                    buffer.draw(view, face);
                }
            }
        }
        buffer
    }

    /// Writes a flat convex face into the cells it covers completely, at its
    /// farthest corner's depth. A face that reaches the near plane is skipped.
    fn draw(&mut self, view: &View, corners: [[f32; 3]; 4]) {
        let mut cells = [[0.0f32; 2]; 4];
        for (cell, &corner) in cells.iter_mut().zip(&corners) {
            let Some(c) = view.cell(corner) else { return };
            *cell = c;
        }
        let far = corners.iter().map(|&p| view.depth(p)).fold(f32::MIN, f32::max);
        // Twice the signed area gives the winding the edge tests follow.
        let area: f32 = (0..4).map(|i| {
            let (u, v) = (cells[i], cells[(i + 1) % 4]);
            u[0] * v[1] - v[0] * u[1]
        }).sum();
        if area.abs() < 1e-6 {
            return;
        }
        let inside = |p: [f32; 2]| {
            (0..4).all(|i| {
                let (u, v) = (cells[i], cells[(i + 1) % 4]);
                ((v[0] - u[0]) * (p[1] - u[1]) - (v[1] - u[1]) * (p[0] - u[0])) * area.signum() >= 0.0
            })
        };
        let (lo, hi) = cells.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), c| ([lo[0].min(c[0]), lo[1].min(c[1])], [hi[0].max(c[0]), hi[1].max(c[1])]));
        let x0 = lo[0].floor().max(0.0) as usize;
        let x1 = (hi[0].ceil().max(0.0) as usize).min(OCCLUSION_WIDTH);
        let y0 = lo[1].floor().max(0.0) as usize;
        let y1 = (hi[1].ceil().max(0.0) as usize).min(OCCLUSION_HEIGHT);
        for y in y0..y1 {
            for x in x0..x1 {
                let (fx, fy) = (x as f32, y as f32);
                // A convex face covers a cell completely when it holds all four of the cell's corners.
                if [[fx, fy], [fx + 1.0, fy], [fx, fy + 1.0], [fx + 1.0, fy + 1.0]].into_iter().all(inside) {
                    let cell = &mut self.depth[y * OCCLUSION_WIDTH + x];
                    *cell = cell.min(far);
                }
            }
        }
    }

    /// Whether a sphere is hidden: every cell it may cover holds an occluder
    /// nearer than its nearest point. Spheres reaching the near plane are never hidden.
    pub fn hides(&self, view: &View, center: [f32; 3], radius: f32) -> bool {
        let nearest = view.depth(center) - radius;
        if nearest <= 0.0 {
            return false;
        }
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        for corner in 0..8 {
            let offset = |bit: usize| if corner >> bit & 1 == 0 { -radius } else { radius };
            let Some(cell) = view.cell([center[0] + offset(0), center[1] + offset(1), center[2] + offset(2)]) else { return false };
            for i in 0..2 {
                lo[i] = lo[i].min(cell[i]);
                hi[i] = hi[i].max(cell[i]);
            }
        }
        // Off the buffer is off screen; the frustum test decides that part.
        let x0 = lo[0].floor().max(0.0) as usize;
        let y0 = lo[1].floor().max(0.0) as usize;
        let x1 = (hi[0].ceil().max(0.0) as usize).min(OCCLUSION_WIDTH);
        let y1 = (hi[1].ceil().max(0.0) as usize).min(OCCLUSION_HEIGHT);
        if x0 >= x1 || y0 >= y1 {
            return false;
        }
        (y0..y1).all(|y| self.depth[y * OCCLUSION_WIDTH + x0..y * OCCLUSION_WIDTH + x1].iter().all(|&d| d < nearest))
    }
}

/// The frustum and occlusion tests of one frame, for the bounding sphere of anything drawn.
pub struct Culler {
    view: View,
    frustum: bool,
    occlusion: Option<OcclusionBuffer>,
}

impl Culler {
    /// Builds the occlusion buffer from the boxes and planes of `occluders` (by kind) when `culling` asks for it.
    pub fn new(view: View, culling: Culling, occluders: [&[Instance3D]; 4]) -> Self {
        let occlusion = culling.occlusion.then(|| OcclusionBuffer::build(&view, occluders));
        Self { view, frustum: culling.frustum, occlusion }
    }

    /// Whether a sphere may show: it touches the view and no occluder hides it.
    pub fn shows(&self, center: [f32; 3], radius: f32) -> bool {
        if self.frustum && !self.view.sees_sphere(center, radius) {
            return false;
        }
        !self.occlusion.as_ref().is_some_and(|b| b.hides(&self.view, center, radius))
    }

    /// Whether a world-space axis-aligned box may show, tested by the sphere around it.
    pub fn shows_box(&self, min: [f32; 3], max: [f32; 3]) -> bool {
        let center = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
        let half = std::array::from_fn(|i| (max[i] - min[i]) * 0.5);
        self.shows(center, dot3(half, half).sqrt())
    }
}

/// The world faces of a box or plane instance, each with its corners in order around it
/// and its outward normal. The main pass draws only the side a face's normal points to.
fn faces(kind: MeshKind, i: &Instance3D) -> Vec<([[f32; 3]; 4], [f32; 3])> {
    let world = |local: [f32; 3]| {
        let r = rotate(i.rot, [local[0] * i.scale[0], local[1] * i.scale[1], local[2] * i.scale[2]]);
        [i.pos[0] + r[0], i.pos[1] + r[1], i.pos[2] + r[2]]
    };
    if kind == MeshKind::Plane {
        return vec![([[-1.0, 0.0, -1.0], [1.0, 0.0, -1.0], [1.0, 0.0, 1.0], [-1.0, 0.0, 1.0]].map(world), rotate(i.rot, [0.0, 1.0, 0.0]))];
    }
    let corner = |x: f32, y: f32, z: f32| world([x, y, z]);
    let axes = [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];
    let corners = [
        [corner(1.0, -1.0, -1.0), corner(1.0, 1.0, -1.0), corner(1.0, 1.0, 1.0), corner(1.0, -1.0, 1.0)],
        [corner(-1.0, -1.0, -1.0), corner(-1.0, -1.0, 1.0), corner(-1.0, 1.0, 1.0), corner(-1.0, 1.0, -1.0)],
        [corner(-1.0, 1.0, -1.0), corner(-1.0, 1.0, 1.0), corner(1.0, 1.0, 1.0), corner(1.0, 1.0, -1.0)],
        [corner(-1.0, -1.0, -1.0), corner(1.0, -1.0, -1.0), corner(1.0, -1.0, 1.0), corner(-1.0, -1.0, 1.0)],
        [corner(-1.0, -1.0, 1.0), corner(1.0, -1.0, 1.0), corner(1.0, 1.0, 1.0), corner(-1.0, 1.0, 1.0)],
        [corner(-1.0, -1.0, -1.0), corner(-1.0, 1.0, -1.0), corner(1.0, 1.0, -1.0), corner(1.0, -1.0, -1.0)],
    ];
    corners.into_iter().zip(axes).map(|(face, axis)| (face, rotate(i.rot, axis))).collect()
}

/// The instances of each kind reordered with the visible ones first (in list
/// order, then the hidden ones in list order), and how many are visible.
pub fn partition(view: &View, culling: Culling, instances: [&[Instance3D]; 4]) -> ([Vec<Instance3D>; 4], [u32; 4]) {
    let culler = Culler::new(*view, culling, instances);
    let mut counts = [0u32; 4];
    let ordered = std::array::from_fn(|k| {
        let kind = MeshKind::ALL[k];
        let (mut shown, mut hidden) = (Vec::with_capacity(instances[k].len()), Vec::new());
        for instance in instances[k] {
            if culler.shows(instance.pos, bounding_radius(kind, instance)) { shown.push(*instance) } else { hidden.push(*instance) }
        }
        counts[k] = shown.len() as u32;
        shown.extend(hidden);
        shown
    });
    (ordered, counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera3D, Material};

    fn view(eye: [f32; 3], target: [f32; 3]) -> View {
        let camera = Camera3D::perspective(eye, target, 60.0);
        let f = camera.forward();
        View { view_proj: camera.view_proj(16.0 / 9.0).0, eye, forward: f }
    }

    fn list() -> crate::RenderList3D {
        crate::RenderList3D::default()
    }

    #[test]
    fn the_frustum_keeps_what_is_in_view_and_drops_what_is_behind_or_beside() {
        let v = view([0.0, 0.0, 10.0], [0.0; 3]);
        assert!(v.sees_sphere([0.0; 3], 0.5));
        assert!(!v.sees_sphere([0.0, 0.0, 20.0], 0.5), "behind the camera");
        assert!(!v.sees_sphere([40.0, 0.0, 0.0], 0.5), "far to the side");
        assert!(v.sees_sphere([0.0, 0.0, 20.0], 15.0), "a sphere around the camera");
    }

    #[test]
    fn a_wall_hides_what_stands_fully_behind_it_but_not_what_peeks_out() {
        let v = view([0.0, 1.0, 10.0], [0.0, 1.0, 0.0]);
        let mut l = list();
        let m = Material::new([0.5; 3]);
        // A wall 8 m wide and 4 m tall, 4 m in front of the origin.
        l.cuboid([0.0, 2.0, 4.0], crate::IDENTITY_ROT, [4.0, 2.0, 0.2], &m);
        let buffer = OcclusionBuffer::build(&v, [&l.spheres, &l.boxes, &l.capsules, &l.planes]);
        assert!(buffer.hides(&v, [0.0, 1.0, 0.0], 0.5), "a ball behind the middle of the wall");
        assert!(!buffer.hides(&v, [0.0, 7.0, 0.0], 0.5), "a ball high enough to show over the wall");
        assert!(buffer.hides(&v, [0.0, 4.6, 0.0], 0.5), "a ball just above the top edge, still hidden from this low eye");
        assert!(!buffer.hides(&v, [8.0, 1.0, 0.0], 0.5), "a ball far enough beside the wall to show past its edge");
        assert!(!buffer.hides(&v, [0.0, 1.0, 6.0], 0.5), "a ball in front of the wall");
        // The wall does not hide itself.
        assert!(!buffer.hides(&v, [0.0, 2.0, 4.0], bounding_radius(MeshKind::Box, &l.boxes[0])));
    }

    #[test]
    fn a_face_leaves_no_uncovered_seam_along_its_diagonal() {
        // Square view, so the ball sits on the wall face's diagonal in the buffer.
        let camera = Camera3D::perspective([0.0, 3.0, 12.0], [0.0, 1.0, 0.0], 60.0);
        let v = View { view_proj: camera.view_proj(1.0).0, eye: camera.eye, forward: camera.forward() };
        let mut l = list();
        l.cuboid([0.0, 2.0, 4.0], crate::IDENTITY_ROT, [4.0, 2.0, 0.2], &Material::new([0.5; 3]));
        let buffer = OcclusionBuffer::build(&v, [&l.spheres, &l.boxes, &l.capsules, &l.planes]);
        assert!(buffer.hides(&v, [0.0, 1.0, 0.0], 0.5));
    }

    #[test]
    fn partition_puts_the_visible_first_in_list_order_and_counts_them() {
        let v = view([0.0, 1.0, 10.0], [0.0, 1.0, 0.0]);
        let mut l = list();
        let m = Material::new([0.5; 3]);
        l.cuboid([0.0, 2.0, 4.0], crate::IDENTITY_ROT, [4.0, 2.0, 0.2], &m);
        for x in [-0.5, 30.0, 0.5, 9.0] {
            l.sphere([x, 1.0, 0.0], crate::IDENTITY_ROT, 0.3, &m);
        }
        let all = [&l.spheres[..], &l.boxes[..], &l.capsules[..], &l.planes[..]];
        let (ordered, counts) = partition(&v, Culling::default(), all);
        // Behind the wall (-0.5, 0.5) and off screen (30): only the one at 9 shows.
        assert_eq!(counts, [1, 1, 0, 0]);
        assert_eq!(ordered[0].iter().map(|i| i.pos[0]).collect::<Vec<_>>(), vec![9.0, -0.5, 30.0, 0.5]);
        let (_, frustum_only) = partition(&v, Culling { frustum: true, occlusion: false }, all);
        assert_eq!(frustum_only, [3, 1, 0, 0]);
        let (_, none) = partition(&v, Culling { frustum: false, occlusion: false }, all);
        assert_eq!(none, [4, 1, 0, 0]);
    }

    #[test]
    fn a_plane_seen_from_its_back_hides_nothing() {
        let mut l = list();
        let m = Material::new([0.5; 3]);
        l.plane([0.0; 3], 4.0, 4.0, &m);
        let lists = [&l.spheres[..], &l.boxes[..], &l.capsules[..], &l.planes[..]];
        // From below the plane's back is turned to the eye and is not drawn, so the ball above shows.
        let below = view([0.0, -6.0, 4.0], [0.0, 4.0, 0.0]);
        assert!(!OcclusionBuffer::build(&below, lists).hides(&below, [0.0, 4.0, 0.0], 0.5));
        // From above its front is drawn and hides the ball under it.
        let above = view([0.0, 6.0, 4.0], [0.0, -4.0, 0.0]);
        assert!(OcclusionBuffer::build(&above, lists).hides(&above, [0.0, -4.0, 0.0], 0.5));
    }

    #[test]
    fn a_box_seen_from_inside_hides_nothing_beyond_its_walls() {
        let mut l = list();
        l.cuboid([0.0; 3], crate::IDENTITY_ROT, [20.0; 3], &Material::new([0.5; 3]));
        let lists = [&l.spheres[..], &l.boxes[..], &l.capsules[..], &l.planes[..]];
        // The walls' backs face the eye, so none is drawn and the ball past the far wall shows.
        let inside = view([0.0, 0.0, 5.0], [0.0; 3]);
        assert!(!OcclusionBuffer::build(&inside, lists).hides(&inside, [0.0, 0.0, -30.0], 0.5));
        // From outside the near wall hides it.
        let outside = view([0.0, 0.0, 60.0], [0.0; 3]);
        assert!(OcclusionBuffer::build(&outside, lists).hides(&outside, [0.0, 0.0, -10.0], 0.5));
    }
}
