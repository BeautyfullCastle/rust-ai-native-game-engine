//! What the 3D renderer draws: instances of the procedural meshes with a
//! material each, debug lines, and the lighting.

use bytemuck::{Pod, Zeroable};

use crate::mesh::MeshKind;

/// Surface look of one instance: a metal/roughness Blinn-Phong model.
///
/// - `color`: base color in linear RGB (diffuse color of a non metal, the
///   specular tint of a metal).
/// - `roughness`: `1.0` is a matte surface with a broad dim highlight, `0.1`
///   is glossy.
/// - `metallic`: `0.0` dielectric, `1.0` metal (no diffuse, tinted highlight).
/// - `emissive`: adds `color * emissive` (self lit).
/// - `checker`: multiplies the color with a 2 unit checker pattern in world
///   xz (for ground planes: it makes shadows and distance readable).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Material {
    pub color: [f32; 3],
    pub roughness: f32,
    pub metallic: f32,
    pub emissive: f32,
    pub checker: bool,
}

impl Material {
    /// A plain dielectric of `color` (roughness 0.6).
    pub const fn new(color: [f32; 3]) -> Self {
        Self { color, roughness: 0.6, metallic: 0.0, emissive: 0.0, checker: false }
    }

    pub const fn rough(mut self, roughness: f32) -> Self {
        self.roughness = roughness;
        self
    }

    pub const fn metal(mut self, metallic: f32) -> Self {
        self.metallic = metallic;
        self
    }

    pub const fn glow(mut self, emissive: f32) -> Self {
        self.emissive = emissive;
        self
    }

    pub const fn checkered(mut self) -> Self {
        self.checker = true;
        self
    }
}

/// One mesh instance as the GPU reads it (80 bytes, 5 `vec4`s).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Instance3D {
    /// World position, then the capsule half length (0 for other meshes).
    pub pos: [f32; 3],
    pub half_length: f32,
    /// Orientation, unit quaternion `x, y, z, w`.
    pub rot: [f32; 4],
    /// Scale of the unit mesh, then the flags (`1.0` = checker).
    pub scale: [f32; 3],
    pub flags: f32,
    /// Base color (linear), alpha unused.
    pub color: [f32; 4],
    /// Roughness, metallic, emissive, unused.
    pub material: [f32; 4],
}

impl Instance3D {
    fn new(pos: [f32; 3], rot: [f32; 4], scale: [f32; 3], half_length: f32, m: &Material) -> Self {
        Self {
            pos,
            half_length,
            rot,
            scale,
            flags: if m.checker { 1.0 } else { 0.0 },
            color: [m.color[0], m.color[1], m.color[2], 1.0],
            material: [m.roughness, m.metallic, m.emissive, 0.0],
        }
    }
}

/// A debug line segment, drawn with a constant width in pixels (48 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LineInstance3D {
    pub a: [f32; 3],
    pub width: f32,
    pub b: [f32; 3],
    pub pad: f32,
    /// Linear RGBA, straight alpha.
    pub color: [f32; 4],
}

pub const IDENTITY_ROT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// The sun, the sky/ground ambient light and the output settings.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Lighting {
    /// The direction the sunlight travels (from the sun toward the scene).
    pub direction: [f32; 3],
    /// Sunlight color (linear) and intensity.
    pub color: [f32; 3],
    pub intensity: f32,
    /// Hemisphere ambient: the color from above, from below, and its strength.
    pub sky: [f32; 3],
    pub ground: [f32; 3],
    pub ambient: f32,
    /// Shadow mapping on or off.
    pub shadows: bool,
    /// The shadow map covers a sphere of `shadow_radius` around `shadow_center`.
    pub shadow_center: [f32; 3],
    pub shadow_radius: f32,
    /// Filmic (ACES) tone mapping instead of a plain clamp.
    pub tonemap: bool,
    /// Multiplies the lit color before tone mapping.
    pub exposure: f32,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            direction: [-0.45, -0.8, -0.35],
            color: [1.0, 0.96, 0.88],
            intensity: 2.6,
            sky: [0.55, 0.7, 1.0],
            ground: [0.28, 0.25, 0.22],
            ambient: 0.55,
            shadows: true,
            shadow_center: [0.0, 0.0, 0.0],
            shadow_radius: 30.0,
            tonemap: true,
            exposure: 1.0,
        }
    }
}

/// One frame of 3D drawing. Fill it each frame (`clear` keeps the buffers'
/// capacity), then give it to a [`crate::Renderer3D`].
#[derive(Clone, Debug, Default)]
pub struct RenderList3D {
    pub spheres: Vec<Instance3D>,
    pub boxes: Vec<Instance3D>,
    pub capsules: Vec<Instance3D>,
    pub planes: Vec<Instance3D>,
    pub lines: Vec<LineInstance3D>,
    pub lighting: Lighting,
}

impl RenderList3D {
    pub fn new() -> Self {
        Self::default()
    }

    /// Empties the instances and lines; the lighting stays.
    pub fn clear(&mut self) {
        self.spheres.clear();
        self.boxes.clear();
        self.capsules.clear();
        self.planes.clear();
        self.lines.clear();
    }

    pub fn instance_count(&self) -> usize {
        self.spheres.len() + self.boxes.len() + self.capsules.len() + self.planes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instance_count() == 0 && self.lines.is_empty()
    }

    /// The instance slice of one mesh kind.
    pub fn instances(&self, kind: MeshKind) -> &[Instance3D] {
        match kind {
            MeshKind::Sphere => &self.spheres,
            MeshKind::Box => &self.boxes,
            MeshKind::Capsule => &self.capsules,
            MeshKind::Plane => &self.planes,
        }
    }

    pub fn sphere(&mut self, pos: [f32; 3], rot: [f32; 4], radius: f32, m: &Material) {
        self.spheres.push(Instance3D::new(pos, rot, [radius; 3], 0.0, m));
    }

    /// A box with the half extents `half`.
    pub fn cuboid(&mut self, pos: [f32; 3], rot: [f32; 4], half: [f32; 3], m: &Material) {
        self.boxes.push(Instance3D::new(pos, rot, half, 0.0, m));
    }

    /// A capsule along the local y axis: the segment runs `half_length` to
    /// each side of `pos`, grown by `radius`.
    pub fn capsule(&mut self, pos: [f32; 3], rot: [f32; 4], half_length: f32, radius: f32, m: &Material) {
        self.capsules.push(Instance3D::new(pos, rot, [radius; 3], half_length, m));
    }

    /// A horizontal ground rectangle (normal +y) centered at `pos`.
    pub fn plane(&mut self, pos: [f32; 3], half_x: f32, half_z: f32, m: &Material) {
        self.planes.push(Instance3D::new(pos, IDENTITY_ROT, [half_x, 1.0, half_z], 0.0, m));
    }

    /// A line of `width_px` pixels.
    pub fn line(&mut self, a: [f32; 3], b: [f32; 3], width_px: f32, color: [f32; 4]) {
        self.lines.push(LineInstance3D { a, width: width_px, b, pad: 0.0, color });
    }

    /// The 12 edges of the axis aligned box `min..max`.
    pub fn aabb(&mut self, min: [f32; 3], max: [f32; 3], width_px: f32, color: [f32; 4]) {
        let c = |i: usize| [if i & 1 == 0 { min[0] } else { max[0] }, if i & 2 == 0 { min[1] } else { max[1] }, if i & 4 == 0 { min[2] } else { max[2] }];
        for i in 0..8usize {
            for bit in [1usize, 2, 4] {
                if i & bit == 0 {
                    self.line(c(i), c(i | bit), width_px, color);
                }
            }
        }
    }

    /// An arrow from `from` along `vector` with a small head.
    pub fn arrow(&mut self, from: [f32; 3], vector: [f32; 3], width_px: f32, color: [f32; 4]) {
        use crate::math3::{add, cross, length, normalize, scale, sub};
        let tip = add(from, vector);
        self.line(from, tip, width_px, color);
        let len = length(vector);
        if len < 1e-6 {
            return;
        }
        let dir = scale(vector, 1.0 / len);
        let helper = if dir[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
        let side = normalize(cross(dir, helper));
        let up = cross(dir, side);
        let head = len * 0.2;
        for d in [side, scale(side, -1.0), up, scale(up, -1.0)] {
            let base = sub(tip, scale(dir, head));
            self.line(tip, add(base, scale(d, head * 0.5)), width_px, color);
        }
    }

    /// A three axis cross at `p` (contact points).
    pub fn cross_marker(&mut self, p: [f32; 3], half: f32, width_px: f32, color: [f32; 4]) {
        for axis in 0..3 {
            let mut a = p;
            let mut b = p;
            a[axis] -= half;
            b[axis] += half;
            self.line(a, b, width_px, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_layouts_are_what_the_shader_reads() {
        assert_eq!(std::mem::size_of::<Instance3D>(), 80);
        assert_eq!(std::mem::size_of::<LineInstance3D>(), 48);
    }

    #[test]
    fn aabb_has_twelve_edges_and_arrow_five_lines() {
        let mut l = RenderList3D::new();
        l.aabb([0.0; 3], [1.0; 3], 1.0, [1.0; 4]);
        assert_eq!(l.lines.len(), 12);
        l.lines.clear();
        l.arrow([0.0; 3], [0.0, 2.0, 0.0], 1.0, [1.0; 4]);
        assert_eq!(l.lines.len(), 5);
    }

    #[test]
    fn clear_keeps_the_lighting() {
        let mut l = RenderList3D::new();
        l.lighting.exposure = 3.0;
        l.sphere([0.0; 3], IDENTITY_ROT, 1.0, &Material::new([1.0; 3]));
        l.clear();
        assert!(l.is_empty());
        assert_eq!(l.lighting.exposure, 3.0);
    }
}
