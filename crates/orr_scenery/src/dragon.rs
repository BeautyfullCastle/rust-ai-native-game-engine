//! The Eastern dragon as a pass of the imported scene renderer. The
//! simulation flies the head and keeps its path (`orr_games::dragon`); this
//! pass lays a long serpentine body along that path every frame, with a head,
//! horns, whiskers, a mane, four legs and a tail tuft. It uses the outdoor
//! pass's lighting and the shared sun shadow map, and casts into that map.

use crate::math::{add, cross, length, normalize, scale, sub, V3};
use crate::mesh::PropVertex;
use crate::passes::{frame_globals, lighting_source, Globals, COMMON};
use orr_render::orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, Compare, Cull, DepthAttachment, DepthState, PipelineDesc, Rhi, SamplerDesc, TextureDesc,
    TextureFormat, TextureUsage, Topology, VertexAttr, VertexFormat, VertexLayout, VertexStep,
};
use orr_render::{PassFrame, PassStage, RenderPass};
use std::any::Any;

const DEPTH: TextureFormat = TextureFormat::Depth32Float;
/// Length of the body from the snout to the tail tip, meters.
pub const BODY_LENGTH: f32 = 34.0;
const RINGS: usize = 96;
const SIDES: usize = 12;

const PART_SCALES: f32 = 0.0;
const PART_BELLY: f32 = 1.0;
const PART_MANE: f32 = 2.0;
const PART_HORN: f32 = 3.0;
const PART_WHISKER: f32 = 4.0;
const PART_EYE: f32 = 5.0;
const PART_DARK: f32 = 6.0;
const PART_TEETH: f32 = 7.0;

const JADE: V3 = [0.03, 0.2, 0.1];
const BELLY: V3 = [0.78, 0.62, 0.3];
const MANE: V3 = [0.7, 0.07, 0.03];
const GOLD: V3 = [0.85, 0.6, 0.18];
const GOLD_EYE: V3 = [0.95, 0.62, 0.08];
const ANTLER: V3 = [0.86, 0.74, 0.48];
const IVORY: V3 = [0.93, 0.9, 0.8];
const DARK: V3 = [0.015, 0.012, 0.01];

/// One dragon as the view reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct DragonLook {
    /// The head first, then where it was, newest to oldest.
    pub path: Vec<V3>,
    /// Roll into a turn, degrees.
    pub bank: f32,
    /// Seconds of the clock, for the body's wave and the whiskers.
    pub time: f32,
}

/// The path as a curve measured in meters back from the head.
struct Path {
    points: Vec<V3>,
    /// Distance from the head of each point.
    at: Vec<f32>,
    ahead: V3,
}

impl Path {
    fn new(points: &[V3]) -> Self {
        let mut clean: Vec<V3> = Vec::with_capacity(points.len());
        for p in points {
            if clean.last().is_none_or(|q| length(sub(*p, *q)) > 1e-3) {
                clean.push(*p);
            }
        }
        if clean.len() < 2 {
            let head = clean.first().copied().unwrap_or([0.0; 3]);
            clean = vec![head, add(head, [0.0, 0.0, -1.0])];
        }
        let mut at = vec![0.0];
        for w in clean.windows(2) {
            at.push(at.last().unwrap() + length(sub(w[1], w[0])));
        }
        let ahead = normalize(sub(clean[0], clean[1]));
        Self { points: clean, at, ahead }
    }

    /// The point `s` meters back from the head; before the head the curve goes on ahead, past the end it goes on straight.
    fn point(&self, s: f32) -> V3 {
        if s <= 0.0 {
            return add(self.points[0], scale(self.ahead, -s));
        }
        let last = self.points.len() - 1;
        let k = self.at.partition_point(|a| *a < s).clamp(1, last);
        let (a, b) = (self.at[k - 1], self.at[k]);
        let t = (s - a) / (b - a).max(1e-4);
        let p = add(self.points[k - 1], scale(sub(self.points[k], self.points[k - 1]), t));
        if s > self.at[last] {
            let tail = normalize(sub(self.points[last], self.points[last - 1]));
            return add(self.points[last], scale(tail, s - self.at[last]));
        }
        p
    }

    /// Unit direction toward the head at `s`.
    fn forward(&self, s: f32) -> V3 {
        normalize(sub(self.point(s - 0.6), self.point(s + 0.6)))
    }
}

/// Body radius `s` meters back from the snout tip.
fn radius(s: f32) -> f32 {
    let t = s / BODY_LENGTH;
    if s < 1.4 {
        // A long muzzle that stays broad to its tip.
        0.5 + 0.3 * smooth(s / 1.4)
    } else if s < 2.6 {
        // Cheeks and the back of the head.
        0.8 + 0.25 * smooth((s - 1.4) / 1.2)
    } else if s < 4.5 {
        // A slimmer neck.
        1.05 - 0.23 * smooth((s - 2.6) / 1.9)
    } else if t < 0.55 {
        0.82 + 0.34 * smooth((s - 4.5) / (0.55 * BODY_LENGTH - 4.5))
    } else {
        1.16 * (1.0 - smooth((t - 0.55) / 0.45)).max(0.0).powf(0.8) + 0.06
    }
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

struct Builder {
    vertices: Vec<PropVertex>,
    indices: Vec<u32>,
}

impl Builder {
    fn vertex(&mut self, position: V3, normal: V3, color: V3, params: V3) -> u32 {
        self.vertices.push(PropVertex { position, normal: normalize(normal), color, params });
        (self.vertices.len() - 1) as u32
    }

    /// A sphere of `radius` around `center`, as a tube along x.
    fn sphere(&mut self, center: V3, radius: f32, color: V3, part: f32) {
        let rings = 7;
        let centers: Vec<V3> = (0..rings).map(|k| add(center, [radius * (k as f32 / (rings - 1) as f32 * 2.0 - 1.0), 0.0, 0.0])).collect();
        let radii: Vec<f32> = (0..rings).map(|k| { let x = k as f32 / (rings - 1) as f32 * 2.0 - 1.0; radius * (1.0 - x * x).max(0.0).sqrt() }).collect();
        self.tube(&centers, &radii, 8, color, part);
    }

    /// A tapering tube through `centers` with `sides` sides.
    fn tube(&mut self, centers: &[V3], radii: &[f32], sides: usize, color: V3, part: f32) {
        let base = self.vertices.len() as u32;
        for (i, c) in centers.iter().enumerate() {
            let along = normalize(sub(centers[(i + 1).min(centers.len() - 1)], centers[i.saturating_sub(1)]));
            let helper = if along[1].abs() > 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
            let u = normalize(cross(along, helper));
            let v = cross(u, along);
            for k in 0..sides {
                let a = k as f32 / sides as f32 * std::f32::consts::TAU;
                let n = add(scale(u, a.cos()), scale(v, a.sin()));
                self.vertex(add(*c, scale(n, radii[i])), n, color, [part, i as f32, a]);
            }
        }
        for i in 0..centers.len() as u32 - 1 {
            for k in 0..sides as u32 {
                let (a, b) = (base + i * sides as u32 + k, base + i * sides as u32 + (k + 1) % sides as u32);
                let (c, d) = (a + sides as u32, b + sides as u32);
                self.indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
}

/// The dragon's mesh for `look`: vertices in the scenery prop layout and triangle indices.
pub fn dragon_mesh(look: &DragonLook) -> (Vec<PropVertex>, Vec<u32>) {
    let path = Path::new(&look.path);
    let mut b = Builder { vertices: Vec::with_capacity(RINGS * SIDES + 600), indices: Vec::with_capacity(RINGS * SIDES * 6 + 1200) };
    let bank = look.bank.to_radians();
    // The frame of the body at `s`: forward, right and up, rolled into the turn near the head.
    let frame = |s: f32| -> (V3, V3, V3) {
        let f = path.forward(s);
        let roll = bank * (1.0 - smooth(s / (BODY_LENGTH * 0.6)));
        let helper = if f[1].abs() > 0.95 { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] };
        let right0 = normalize(cross(f, helper));
        let up0 = cross(right0, f);
        let right = add(scale(right0, roll.cos()), scale(up0, roll.sin()));
        let up = cross(right, f);
        (f, right, up)
    };
    // A wave travels down the body, so it swims through the air even when flying straight.
    let center = |s: f32| -> V3 {
        let (_, right, up) = frame(s);
        let reach = smooth(s / 6.0);
        let wave = (s * 0.42 - look.time * 3.2).sin() * 0.7 * reach;
        let rise = (s * 0.31 - look.time * 2.1).cos() * 0.35 * reach;
        add(add(path.point(s - 2.6), scale(right, wave)), scale(up, rise))
    };

    // Body rings from the snout to the tail tip. The snout is 2.6 m ahead of the tracked head.
    let base = b.vertices.len() as u32;
    for i in 0..RINGS {
        let s = i as f32 / (RINGS - 1) as f32 * BODY_LENGTH;
        let (_, right, up) = frame(s);
        let c = center(s);
        let r = radius(s);
        for k in 0..SIDES {
            let a = k as f32 / SIDES as f32 * std::f32::consts::TAU;
            // A slightly flat, deep body. The head is flatter and squarer: a long muzzle, not a round snout.
            let head = 1.0 - smooth((s - 2.8) / 1.2);
            let p = 1.0 - 0.4 * head;
            let (width, height) = (1.0, 0.9 - 0.2 * head);
            let (ca, sa) = (a.cos(), a.sin());
            let (x, y) = (ca.signum() * ca.abs().powf(p), sa.signum() * sa.abs().powf(p));
            let n = add(scale(right, x / width), scale(up, y / (height * height)));
            let offset = add(scale(right, x * r * width), scale(up, y * r * height));
            let belly = a.sin() < -0.45;
            let (color, part) = if belly { (BELLY, PART_BELLY) } else { (JADE, PART_SCALES) };
            b.vertex(add(c, offset), n, color, [part, s, a]);
        }
    }
    for i in 0..RINGS as u32 - 1 {
        for k in 0..SIDES as u32 {
            let (p, q) = (base + i * SIDES as u32 + k, base + i * SIDES as u32 + (k + 1) % SIDES as u32);
            let (r, t) = (p + SIDES as u32, q + SIDES as u32);
            b.indices.extend_from_slice(&[p, q, r, q, t, r]);
        }
    }
    // Close the snout with a cone to its tip.
    let (f0, _, _) = frame(0.0);
    let tip = b.vertex(add(center(0.0), scale(f0, 0.45)), f0, JADE, [PART_SCALES, 0.0, 0.0]);
    for k in 0..SIDES as u32 {
        b.indices.extend_from_slice(&[base + k, tip, base + (k + 1) % SIDES as u32]);
    }

    // Mane: red blades along the spine from behind the head to the hips, leaning back.
    let mut s = 2.9;
    while s < BODY_LENGTH * 0.92 {
        let (f, right, up) = frame(s);
        let c = center(s);
        let r = radius(s);
        let height = 0.55 + 0.5 * (1.0 - s / BODY_LENGTH);
        let root = add(c, scale(up, r * 0.85));
        let a = b.vertex(add(root, scale(right, 0.12)), up, MANE, [PART_MANE, s, 0.0]);
        let d = b.vertex(sub(root, scale(right, 0.12)), up, MANE, [PART_MANE, s, 0.0]);
        let flutter = (s * 1.3 + look.time * 6.0).sin() * 0.15;
        let top = b.vertex(add(add(root, scale(up, height)), add(scale(f, -0.55), scale(right, flutter))), up, MANE, [PART_MANE, s, 1.0]);
        b.indices.extend_from_slice(&[a, d, top]);
        s += 0.55;
    }
    // Tail tuft.
    for k in 0..7 {
        let s = BODY_LENGTH - 1.6 + k as f32 * 0.22;
        let (f, right, up) = frame(s);
        let c = center(s);
        let spread = (k as f32 - 3.0) * 0.35;
        let a = b.vertex(add(c, scale(up, 0.08)), up, MANE, [PART_MANE, s, 0.0]);
        let d = b.vertex(sub(c, scale(up, 0.08)), up, MANE, [PART_MANE, s, 0.0]);
        let end = b.vertex(add(add(c, scale(f, -1.4)), add(scale(right, spread), scale(up, spread.abs() * 0.4))), up, MANE, [PART_MANE, s, 1.0]);
        b.indices.extend_from_slice(&[a, d, end]);
    }

    // The face: a lower jaw with teeth, eyes under brow ridges, nostrils with
    // long whiskers, branching antlers, a beard and fins on the cheeks.
    let at = |s: f32, side: f32, out: f32, rise: f32, ahead: f32| -> V3 {
        let (f, right, up) = frame(s);
        let r = radius(s);
        add(center(s), add(scale(right, side * out * r), add(scale(up, rise * r), scale(f, ahead))))
    };
    // Lower jaw, a little open at the front.
    let jaw: Vec<V3> = (0..8)
        .map(|k| {
            let s = 0.25 + 2.3 * k as f32 / 7.0;
            let open = 0.32 * (1.0 - s / 2.55);
            add(at(s, 0.0, 0.0, -0.55, 0.0), scale(frame(s).2, -open))
        })
        .collect();
    let jaw_radii: Vec<f32> = (0..8).map(|k| 0.26 + 0.2 * k as f32 / 7.0).collect();
    b.tube(&jaw, &jaw_radii, 8, BELLY, PART_BELLY);
    for side in [-1.0f32, 1.0] {
        // Teeth along the upper jaw; the front pair are fangs.
        for (k, s) in [0.32f32, 0.62, 0.92, 1.22, 1.52].into_iter().enumerate() {
            let root = at(s, side, 0.62, -0.42, 0.0);
            let long = if k == 0 { 0.42 } else { 0.22 };
            b.tube(&[root, add(root, scale(frame(s).2, -long))], &[0.065, 0.004], 4, IVORY, PART_TEETH);
        }
        // Eyes: a golden eyeball with a dark pupil, under a ridge of brow.
        let eye = at(2.1, side, 0.9, 0.3, 0.0);
        b.sphere(eye, 0.24, GOLD_EYE, PART_EYE);
        b.sphere(add(eye, add(scale(frame(2.1).1, side * 0.17), scale(frame(2.1).0, 0.05))), 0.11, DARK, PART_DARK);
        let brow: Vec<V3> = (0..5).map(|k| at(1.7 + 0.22 * k as f32, side, 0.8, 0.66 + 0.06 * (k as f32 - 2.0).abs(), 0.0)).collect();
        b.tube(&brow, &[0.06, 0.12, 0.14, 0.12, 0.06], 6, JADE, PART_SCALES);
        // Nostrils, and the whiskers that grow from them and stream back in an S.
        let nostril = at(0.18, side, 0.42, 0.38, 0.0);
        b.sphere(nostril, 0.08, DARK, PART_DARK);
        let (fs, rs, us) = frame(0.18);
        let whisker: Vec<V3> = (0..16)
            .map(|k| {
                let t = k as f32 / 15.0;
                let sway = (look.time * 2.2 + t * 5.0 + side).sin() * 0.55 * t;
                let forward = 0.7 * t - 6.5 * t * t;
                let out = side * (0.25 + 1.3 * t + sway);
                let rise = 0.5 * (t * std::f32::consts::PI).sin() - 0.9 * t;
                add(nostril, add(scale(fs, forward), add(scale(rs, out), scale(us, rise))))
            })
            .collect();
        let radii: Vec<f32> = (0..16).map(|k| 0.055 * (1.0 - k as f32 / 16.0) + 0.008).collect();
        b.tube(&whisker, &radii, 4, GOLD, PART_WHISKER);
        // Antlers: a beam that sweeps back and up, with two tines.
        let (f, right, up) = frame(2.5);
        let root = at(2.5, side, 0.38, 0.7, 0.0);
        let beam = |t: f32| add(root, add(scale(f, -2.3 * t), add(scale(up, 1.5 * t - 0.45 * t * t), scale(right, side * 0.55 * t))));
        let beam_points: Vec<V3> = (0..7).map(|k| beam(k as f32 / 6.0)).collect();
        b.tube(&beam_points, &[0.15, 0.13, 0.11, 0.09, 0.07, 0.05, 0.015], 6, ANTLER, PART_HORN);
        for (t, rise, lean) in [(0.4f32, 0.75f32, 0.35f32), (0.72, 0.55, 0.15)] {
            let start = beam(t);
            let tip = add(start, add(scale(up, rise), add(scale(f, lean), scale(right, side * 0.12))));
            b.tube(&[start, add(start, scale(sub(tip, start), 0.5)), tip], &[0.08, 0.055, 0.012], 5, ANTLER, PART_HORN);
        }
        // Fins fanned out behind the cheeks.
        for k in 0..3 {
            let s = 2.55 + 0.22 * k as f32;
            let (f, right, up) = frame(s);
            let root = at(s, side, 0.85, -0.1, 0.0);
            let a = b.vertex(add(root, scale(up, 0.14)), right, MANE, [PART_MANE, s, 0.0]);
            let d = b.vertex(sub(root, scale(up, 0.14)), right, MANE, [PART_MANE, s, 0.0]);
            let tip = b.vertex(add(root, add(scale(f, -1.0), add(scale(right, side * 0.75), scale(up, 0.25 * (k as f32 - 1.0))))), right, MANE, [PART_MANE, s, 1.0]);
            b.indices.extend_from_slice(&[a, d, tip]);
        }
    }
    // A beard of red strands under the chin.
    for k in 0..6 {
        let s = 0.7 + 0.3 * k as f32;
        let (f, right, up) = frame(s);
        let root = at(s, 0.0, 0.0, -0.95, 0.0);
        let spread = (k as f32 - 2.5) * 0.08;
        let sway = (look.time * 3.0 + k as f32).sin() * 0.12;
        let a = b.vertex(add(root, scale(right, 0.1)), f, MANE, [PART_MANE, s, 0.0]);
        let d = b.vertex(sub(root, scale(right, 0.1)), f, MANE, [PART_MANE, s, 0.0]);
        let tip = b.vertex(add(root, add(scale(up, -0.85), add(scale(f, -0.7), scale(right, spread + sway)))), f, MANE, [PART_MANE, s, 1.0]);
        b.indices.extend_from_slice(&[a, d, tip]);
    }

    // Four legs, each reaching down and forward with three claws.
    for (s, swing) in [(6.5, 0.0), (7.1, std::f32::consts::PI), (20.5, 1.6), (21.1, 4.7)] {
        let side = if (s as i32) % 2 == 0 { -1.0 } else { 1.0 };
        let (f, right, up) = frame(s);
        let c = center(s);
        let r = radius(s);
        let step = (look.time * 3.0 + swing).sin() * 0.35;
        let hip = add(c, scale(right, r * 0.8 * side));
        let knee = add(hip, add(scale(right, 0.6 * side), add(scale(up, -0.7), scale(f, 0.2 + step))));
        let foot = add(knee, add(scale(up, -0.6), scale(f, 0.45 + step)));
        b.tube(&[hip, knee, foot], &[0.32, 0.22, 0.16], 6, JADE, PART_SCALES);
        for claw in [-1.0f32, 0.0, 1.0] {
            let end = add(foot, add(scale(f, 0.32), add(scale(right, 0.12 * claw), scale(up, -0.12))));
            b.tube(&[foot, end], &[0.06, 0.015], 4, GOLD, PART_HORN);
        }
    }
    (b.vertices, b.indices)
}

const ATTRS: [VertexAttr; 4] = [
    VertexAttr { location: 0, format: VertexFormat::Float32x3, offset: 0 },
    VertexAttr { location: 1, format: VertexFormat::Float32x3, offset: 12 },
    VertexAttr { location: 2, format: VertexFormat::Float32x3, offset: 24 },
    VertexAttr { location: 3, format: VertexFormat::Float32x3, offset: 36 },
];

struct Mesh<B: Rhi> {
    vertices: B::Buffer,
    indices: B::Buffer,
    capacity: (u64, u64),
    count: u32,
}

/// The dragon pass. Add it once with `ImportedSceneRenderer::add_pass` and
/// give it the dragons of each frame with [`DragonPass::set_dragons`].
pub struct DragonPass<B: Rhi> {
    shader: B::Shader,
    pipeline: Option<(TextureFormat, B::Pipeline)>,
    shadow_pipeline: B::Pipeline,
    globals: B::Buffer,
    shadow_globals: B::Buffer,
    no_shadow: (B::Texture, B::TextureView),
    sampler: B::Sampler,
    dragons: Vec<DragonLook>,
    meshes: Vec<Mesh<B>>,
    binds: Option<(u64, B::BindGroup, B::BindGroup)>,
}

impl<B: Rhi> DragonPass<B> {
    pub fn new(rhi: &B) -> Self {
        let shader = rhi.create_shader("dragon", &[COMMON, &lighting_source(), include_str!("shaders/dragon.wgsl")].concat());
        let layout = [VertexLayout { stride: 48, step: VertexStep::Vertex, attrs: &ATTRS }];
        let shadow_pipeline = rhi.create_pipeline(&PipelineDesc::<B> {
            label: "dragon shadow",
            shader: &shader,
            vs_entry: "vs_dragon_shadow",
            fs_entry: "",
            vertex_buffers: &layout,
            color_format: None,
            blend: Blend::Opaque,
            topology: Topology::TriangleList,
            cull: Cull::None,
            depth: Some(DepthState { format: DEPTH, write: true, compare: Compare::Less, bias: 2, slope_bias: 2.5 }),
            samples: 1,
        });
        let uniform = |label| rhi.create_buffer(&BufferDesc { label, size: std::mem::size_of::<Globals>() as u64, usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST });
        let no_shadow = rhi.create_texture(&TextureDesc {
            label: "dragon no shadow",
            width: 1,
            height: 1,
            format: DEPTH,
            usage: TextureUsage::TEXTURE_BINDING | TextureUsage::RENDER_ATTACHMENT,
            sample_count: 1,
            view_formats: &[],
        });
        let view = rhi.create_texture_view(&no_shadow, None);
        Self {
            shader,
            pipeline: None,
            shadow_pipeline,
            globals: uniform("dragon globals"),
            shadow_globals: uniform("dragon shadow globals"),
            no_shadow: (no_shadow, view),
            sampler: rhi.create_sampler(&SamplerDesc { linear: true, compare: true }),
            dragons: Vec::new(),
            meshes: Vec::new(),
            binds: None,
        }
    }

    /// The dragons drawn from the next frame on.
    pub fn set_dragons(&mut self, dragons: Vec<DragonLook>) {
        self.dragons = dragons;
    }
}

impl<B: Rhi> RenderPass<B> for DragonPass<B> {
    fn name(&self) -> &str {
        "dragon"
    }

    fn stages(&self) -> &[PassStage] {
        &[PassStage::Shadow, PassStage::Opaque]
    }

    fn prepare(&mut self, rhi: &B, frame: &PassFrame<'_, B>) {
        if self.pipeline.as_ref().is_none_or(|(format, _)| *format != frame.color_format) {
            let layout = [VertexLayout { stride: 48, step: VertexStep::Vertex, attrs: &ATTRS }];
            let pipeline = rhi.create_pipeline(&PipelineDesc::<B> {
                label: "dragon",
                shader: &self.shader,
                vs_entry: "vs_dragon",
                fs_entry: "fs_dragon",
                vertex_buffers: &layout,
                color_format: Some(frame.color_format),
                blend: Blend::Opaque,
                topology: Topology::TriangleList,
                cull: Cull::None,
                depth: Some(DepthState::opaque(DEPTH)),
                samples: 1,
            });
            self.pipeline = Some((frame.color_format, pipeline));
            self.binds = None;
        }
        if self.dragons.is_empty() {
            return;
        }
        let globals = frame_globals(frame, None, self.dragons[0].time);
        rhi.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let mut shadow = globals;
        if let Some(s) = &frame.shadow {
            shadow.view_proj = s.light_vp;
        }
        rhi.write_buffer(&self.shadow_globals, 0, bytemuck::bytes_of(&shadow));
        let map_id = frame.shadow.as_ref().map_or(0, |s| s.map_id);
        if self.binds.as_ref().is_none_or(|(id, ..)| *id != map_id) {
            let map = frame.shadow.as_ref().map_or(&self.no_shadow.1, |s| s.view);
            let pipeline = &self.pipeline.as_ref().expect("made above").1;
            let main = rhi.create_bind_group(pipeline, 0, &[Binding::Uniform { binding: 0, buffer: &self.globals }, Binding::Texture { binding: 1, view: map }, Binding::Sampler { binding: 3, sampler: &self.sampler }]);
            let cast = rhi.create_bind_group(&self.shadow_pipeline, 0, &[Binding::Uniform { binding: 0, buffer: &self.shadow_globals }]);
            self.binds = Some((map_id, main, cast));
        }
        self.meshes.truncate(self.dragons.len());
        for (i, dragon) in self.dragons.iter().enumerate() {
            let (vertices, indices) = dragon_mesh(dragon);
            let bytes = ((vertices.len() * std::mem::size_of::<PropVertex>()) as u64, (indices.len() * 4) as u64);
            if self.meshes.get(i).is_none_or(|m| m.capacity.0 < bytes.0 || m.capacity.1 < bytes.1) {
                let buffer = |label, size: u64, usage| rhi.create_buffer(&BufferDesc { label, size: size.next_power_of_two(), usage: usage | BufferUsage::COPY_DST });
                let mesh = Mesh { vertices: buffer("dragon vertices", bytes.0, BufferUsage::VERTEX), indices: buffer("dragon indices", bytes.1, BufferUsage::INDEX), capacity: (bytes.0.next_power_of_two(), bytes.1.next_power_of_two()), count: 0 };
                if i < self.meshes.len() {
                    self.meshes[i] = mesh;
                } else {
                    self.meshes.push(mesh);
                }
            }
            let mesh = &mut self.meshes[i];
            rhi.write_buffer(&mesh.vertices, 0, bytemuck::cast_slice(&vertices));
            rhi.write_buffer(&mesh.indices, 0, bytemuck::cast_slice(&indices));
            mesh.count = indices.len() as u32;
        }
    }

    fn encode(&self, rhi: &B, stage: PassStage, frame: &PassFrame<'_, B>, encoder: &mut B::Encoder, color: Option<&ColorAttachment<'_, B>>, depth: &DepthAttachment<'_, B>) {
        let (Some((_, pipeline)), Some((_, main, cast))) = (&self.pipeline, &self.binds) else { return };
        if self.dragons.is_empty() || self.meshes.is_empty() {
            return;
        }
        let (pipeline, bind, label) = match stage {
            PassStage::Shadow if frame.shadow.is_some() => (&self.shadow_pipeline, cast, "dragon shadow"),
            PassStage::Opaque => (pipeline, main, "dragon"),
            _ => return,
        };
        let mut commands = vec![Command::SetPipeline(pipeline), Command::SetBindGroup(0, bind)];
        for mesh in self.meshes.iter().take(self.dragons.len()) {
            commands.extend([Command::SetVertexBuffer(0, &mesh.vertices), Command::SetIndexBuffer(&mesh.indices), Command::DrawIndexed { indices: 0..mesh.count, base_vertex: 0, instances: 0..1 }]);
        }
        rhi.encode_pass(encoder, label, color, Some(depth), &commands);
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::dot;

    #[test]
    fn the_body_follows_the_path_back_from_the_head_and_tapers_to_the_tail() {
        // The head flew along +z: the path runs back along -z.
        let path: Vec<V3> = (0..48).map(|k| [0.0, 50.0, -1.25 * k as f32]).collect();
        let look = DragonLook { path, bank: 0.0, time: 0.0 };
        let (vertices, indices) = dragon_mesh(&look);
        assert!(indices.iter().all(|i| (*i as usize) < vertices.len()));
        assert_eq!(indices.len() % 3, 0);
        let body = &vertices[..RINGS * SIDES];
        // The snout is ahead of the tracked head, the tail tip the body length behind the snout.
        let zs: Vec<f32> = body.iter().map(|v| v.position[2]).collect();
        let (front, back) = zs.iter().fold((f32::MIN, f32::MAX), |(a, b), z| (a.max(*z), b.min(*z)));
        assert!((front - 2.6).abs() < 1.6, "the snout leads: {front}");
        assert!((front - back - BODY_LENGTH).abs() < 3.0, "the body is {} m long", front - back);
        // Ring radius around the ring's own center: the swimming wave moves rings sideways.
        let ring = |i: usize| {
            let ring = &body[i * SIDES..(i + 1) * SIDES];
            let c = scale(ring.iter().fold([0.0; 3], |a, v| add(a, v.position)), 1.0 / SIDES as f32);
            ring.iter().map(|v| length(sub(v.position, c))).fold(0.0, f32::max)
        };
        assert!(ring(RINGS / 2) > 0.8 && ring(RINGS - 1) < 0.4, "thick middle, thin tail");
        assert!(vertices.iter().all(|v| v.position.iter().all(|c| c.is_finite()) && dot(v.normal, v.normal) > 0.98));
    }

    #[test]
    fn a_dragon_with_a_short_path_still_has_a_whole_body() {
        let look = DragonLook { path: vec![[3.0, 40.0, 3.0]], bank: 30.0, time: 1.5 };
        let (vertices, _) = dragon_mesh(&look);
        assert!(vertices.iter().all(|v| v.position.iter().all(|c| c.is_finite())));
    }
}
