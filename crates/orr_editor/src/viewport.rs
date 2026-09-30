//! The viewport: what the editor draws for a frame, picking, and the GPU side.
//!
//! - [`build_list`] turns the frame on screen (preview frame in edit mode,
//!   live frame in play mode) into an `orr_render::RenderList`: grid,
//!   bodies (same colors and shapes as the sample, `orr_sample::physics_view`),
//!   collider outlines and the selection highlight. Pure data, no GPU.
//! - [`pick`] finds the body under a world point.
//! - [`ViewportGpu`] draws a list into an offscreen texture with
//!   `orr_render`; [`GpuViewport`] shares that texture with egui as a native
//!   texture on eframe's own wgpu device.
//!
//! View layer: floats are fine here.

use orr_ecs::Entity;
use orr_edit::{ProposalSummary, Target, View};
use orr_reflect::Guid;
use orr_physics::{Body, Collider, Shape, SHAPE_CAPSULE, SHAPE_CIRCLE};
use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu};
use orr_render::{instance_of, Camera, OffscreenTarget, RenderList, Renderer};
use orr_sample::physics_game::PaddleTag;
use orr_sample::physics_view::body_look;
use orr_view::{fp_to_vec2, RenderItem, Transform2, Vec2};

/// Selection outline color.
pub const HIGHLIGHT: [f32; 4] = [1.0, 0.85, 0.2, 1.0];
/// Collider outline color (translucent white).
pub const OUTLINE: [f32; 4] = [1.0, 1.0, 1.0, 0.35];
/// Outline of an entity a previewed proposal changes.
pub const PREVIEW_CHANGED: [f32; 4] = [0.25, 0.85, 1.0, 1.0];
/// Outline of an entity a previewed proposal adds.
pub const PREVIEW_ADDED: [f32; 4] = [0.4, 1.0, 0.45, 1.0];
/// Ghost of an entity a previewed proposal removes, or of where a changed one was.
pub const PREVIEW_GHOST: [f32; 4] = [0.75, 0.75, 0.9, 0.5];
/// Pulse around an entity an agent just edited.
pub const PULSE: [f32; 4] = [1.0, 0.4, 0.9, 1.0];
/// How long an agent edit pulses, in seconds.
pub const PULSE_SECONDS: f64 = 1.5;
const GRID_MINOR: [f32; 4] = [0.085, 0.085, 0.13, 1.0];
const GRID_MAJOR: [f32; 4] = [0.13, 0.13, 0.2, 1.0];
const AXIS_X: [f32; 4] = [0.45, 0.16, 0.16, 1.0];
const AXIS_Y: [f32; 4] = [0.16, 0.42, 0.2, 1.0];

/// The format of the offscreen target: sRGB, like a window surface.
pub const TARGET_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;

/// Bodies drawn with collider outlines up to this many; above it the
/// outlines are skipped to keep the frame cheap.
const OUTLINE_LIMIT: usize = 4000;

/// Grid lines as thin quads (drawn under the bodies).
fn add_grid(list: &mut RenderList, camera: &Camera, vp: (u32, u32)) {
    let ppu = camera.pixels_per_unit(vp.0, vp.1);
    let a = camera.screen_to_world([0.0, 0.0], vp);
    let b = camera.screen_to_world([vp.0 as f32, vp.1 as f32], vp);
    let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
    let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
    let mut step = 1.0f32;
    while step * ppu < 10.0 && step < 1.0e6 {
        step *= 5.0;
    }
    let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    let (hx, hy) = ((x1 - x0) * 0.5 + step, (y1 - y0) * 0.5 + step);
    let mut lines = 0;
    let vertical = |list: &mut RenderList, x: f32, color: [f32; 4], px: f32| {
        list.quad([x, cy], [px * 0.5 / ppu, hy], 0.0, color);
    };
    let mut i = (x0 / step).floor() as i64;
    while (i as f32) * step <= x1 && lines < 600 {
        let x = i as f32 * step;
        let major = i % 5 == 0;
        vertical(list, x, if major { GRID_MAJOR } else { GRID_MINOR }, if major { 1.5 } else { 1.0 });
        i += 1;
        lines += 1;
    }
    let mut j = (y0 / step).floor() as i64;
    while (j as f32) * step <= y1 && lines < 1200 {
        let y = j as f32 * step;
        let major = j % 5 == 0;
        let px = if major { 1.5 } else { 1.0 };
        list.quad([cx, y], [hx, px * 0.5 / ppu], 0.0, if major { GRID_MAJOR } else { GRID_MINOR });
        j += 1;
        lines += 1;
    }
    // Axes on top of the grid.
    let px = 2.0 / ppu;
    if x0 <= 0.0 && x1 >= 0.0 {
        list.quad([0.0, cy], [px * 0.5, hy], 0.0, AXIS_Y);
    }
    if y0 <= 0.0 && y1 >= 0.0 {
        list.quad([cx, 0.0], [hx, px * 0.5], 0.0, AXIS_X);
    }
}

fn rot(v: [f32; 2], angle: f32) -> [f32; 2] {
    let (s, c) = angle.sin_cos();
    [c * v[0] - s * v[1], s * v[0] + c * v[1]]
}

fn v2(v: orr_fp::FPVec2) -> [f32; 2] {
    let p = fp_to_vec2(v);
    [p.x, p.y]
}

/// Outline of a collider shape at a body pose.
fn shape_outline(list: &mut RenderList, pos: [f32; 2], angle: f32, shape: &Shape, width: f32, color: [f32; 4]) {
    let radius = orr_view::fp_to_f32(shape.radius);
    match shape.kind {
        SHAPE_CIRCLE => {
            list.circle_outline(pos, radius, 28, width, color);
            let tip = rot([radius, 0.0], angle);
            list.line(pos, [pos[0] + tip[0], pos[1] + tip[1]], width, color);
        }
        SHAPE_CAPSULE => {
            let (a, b) = (v2(shape.verts[0]), v2(shape.verts[1]));
            let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let m = rot(mid, angle);
            list.capsule_outline([pos[0] + m[0], pos[1] + m[1]], dx.hypot(dy) * 0.5, radius, angle + dy.atan2(dx), 24, width, color);
        }
        _ => {
            let pts: Vec<[f32; 2]> = shape.verts[..shape.count as usize]
                .iter()
                .map(|&v| {
                    let r = rot(v2(v), angle);
                    [pos[0] + r[0], pos[1] + r[1]]
                })
                .collect();
            list.polyline(&pts, true, width, color);
        }
    }
}

/// Builds the render list of the frame `view` reads. `vp` is the viewport in
/// pixels (for the grid). Bodies without a `Collider` are not drawn.
pub fn build_list(view: &View<'_>, selection: Option<&Target>, camera: &Camera, vp: (u32, u32)) -> RenderList {
    let mut list = RenderList::new();
    add_grid(&mut list, camera, vp);
    let frame = view.frame();
    let selected: Option<Entity> = selection.and_then(|t| view.resolve(t).ok());
    let (entities, bodies) = frame.dense::<Body>();
    let mut chosen: Option<(&Body, &Collider)> = None;
    for (&entity, body) in entities.iter().zip(bodies) {
        let Some(collider) = frame.get::<Collider>(entity) else { continue };
        let tag = frame.get::<PaddleTag>(entity).map(|t| t.slot);
        let (style, turn) = body_look(body, collider, tag);
        let item = RenderItem {
            entity,
            transform: Transform2::new(Vec2::new(fp_to_vec2(body.pos).x, fp_to_vec2(body.pos).y), orr_view::fp_to_f32(body.angle) + turn),
            style,
        };
        list.shapes.push(instance_of(&item));
        if selected == Some(entity) {
            chosen = Some((body, collider));
        }
    }
    if bodies.len() <= OUTLINE_LIMIT {
        for (&entity, body) in entities.iter().zip(bodies) {
            if selected == Some(entity) {
                continue;
            }
            if let Some(collider) = frame.get::<Collider>(entity) {
                shape_outline(&mut list, v2(body.pos), orr_view::fp_to_f32(body.angle), &collider.shape, 1.0, OUTLINE);
            }
        }
    }
    if let Some((body, collider)) = chosen {
        let (pos, angle) = (v2(body.pos), orr_view::fp_to_f32(body.angle));
        shape_outline(&mut list, pos, angle, &collider.shape, 3.0, HIGHLIGHT);
        list.cross(pos, 4.0 / camera.pixels_per_unit(vp.0, vp.1), 2.0, HIGHLIGHT);
    }
    list
}

/// Which entities a proposal touches, by GUID: `(changed, added, removed)`.
pub fn preview_marks(summary: &ProposalSummary) -> (Vec<Guid>, Vec<Guid>, Vec<Guid>) {
    let mut changed: Vec<Guid> = summary.fields_changed.iter().filter_map(|f| f.entity.clone()).collect();
    changed.extend(summary.entities_renamed.iter().map(|r| r.guid.clone()));
    changed.extend(summary.components_added.iter().map(|(g, _)| g.clone()));
    changed.extend(summary.components_removed.iter().map(|(g, _)| g.clone()));
    changed.sort();
    changed.dedup();
    let added = summary.entities_added.iter().map(|e| e.guid.clone()).collect();
    let removed = summary.entities_removed.iter().map(|e| e.guid.clone()).collect();
    (changed, added, removed)
}

fn pose_of(view: &View<'_>, guid: &Guid) -> Option<([f32; 2], f32, Collider)> {
    let e = view.entity_of(guid)?;
    let body = view.frame().get::<Body>(e)?;
    let collider = view.frame().get::<Collider>(e)?;
    Some((v2(body.pos), orr_view::fp_to_f32(body.angle), *collider))
}

/// Builds the render list of a proposal preview: `preview` is the staged
/// frame (drawn like any frame), `base` the document's own. Changed entities
/// get a cyan outline, added ones a green one; a changed entity that moved
/// leaves a ghost at its old place, and removed ones are drawn as ghosts
/// from `base`.
pub fn build_preview_list(preview: &View<'_>, base: &View<'_>, summary: &ProposalSummary, selection: Option<&Target>, camera: &Camera, vp: (u32, u32)) -> RenderList {
    let mut list = build_list(preview, selection, camera, vp);
    let (changed, added, removed) = preview_marks(summary);
    let mark = |list: &mut RenderList, guid: &Guid, color: [f32; 4]| {
        if let Some((pos, angle, c)) = pose_of(preview, guid) {
            shape_outline(list, pos, angle, &c.shape, 3.0, color);
        }
    };
    for g in &changed {
        mark(&mut list, g, PREVIEW_CHANGED);
        if let (Some((new, ..)), Some((old, angle, c))) = (pose_of(preview, g), pose_of(base, g)) {
            if new != old {
                shape_outline(&mut list, old, angle, &c.shape, 2.0, PREVIEW_GHOST);
                list.line(old, new, 1.5, PREVIEW_GHOST);
            }
        }
    }
    for g in &added {
        mark(&mut list, g, PREVIEW_ADDED);
    }
    for g in &removed {
        if let Some((pos, angle, c)) = pose_of(base, g) {
            shape_outline(&mut list, pos, angle, &c.shape, 2.0, PREVIEW_GHOST);
            list.cross(pos, 6.0 / camera.pixels_per_unit(vp.0, vp.1), 2.0, PREVIEW_GHOST);
        }
    }
    list
}

/// Draws the pulses of entities an agent just edited: an outline and a ring that
/// grows and fades. `fade` is 1 at the start of a pulse and 0 at its end. Targets
/// that are not drawn (no body and collider, or gone) are skipped.
pub fn add_pulses(list: &mut RenderList, view: &View<'_>, pulses: &[(Target, f32)]) {
    for (target, fade) in pulses {
        let Ok(e) = view.resolve(target) else { continue };
        let (Some(body), Some(collider)) = (view.frame().get::<Body>(e), view.frame().get::<Collider>(e)) else { continue };
        let (pos, angle) = (v2(body.pos), orr_view::fp_to_f32(body.angle));
        let color = [PULSE[0], PULSE[1], PULSE[2], fade.clamp(0.0, 1.0)];
        shape_outline(list, pos, angle, &collider.shape, 2.0 + 3.0 * fade, color);
        let ring = shape_extent(&collider.shape) + 0.25 + (1.0 - fade) * 1.5;
        list.circle_outline(pos, ring, 40, 2.0, color);
    }
}

/// Largest distance of the shape from its origin (a bounding radius).
fn shape_extent(shape: &Shape) -> f32 {
    let radius = orr_view::fp_to_f32(shape.radius);
    match shape.kind {
        SHAPE_CIRCLE => radius,
        SHAPE_CAPSULE => v2(shape.verts[0]).iter().chain(v2(shape.verts[1]).iter()).fold(0.0f32, |m, c| m.max(c.abs())) + radius,
        _ => shape.verts[..shape.count as usize].iter().map(|&v| v2(v)[0].hypot(v2(v)[1])).fold(0.0f32, f32::max),
    }
}

/// Is the world point `p` inside the collider shape at this pose?
pub fn hit_shape(shape: &Shape, pos: [f32; 2], angle: f32, p: [f32; 2]) -> bool {
    let local = rot([p[0] - pos[0], p[1] - pos[1]], -angle);
    match shape.kind {
        SHAPE_CIRCLE => local[0].hypot(local[1]) <= orr_view::fp_to_f32(shape.radius),
        SHAPE_CAPSULE => {
            let (a, b) = (v2(shape.verts[0]), v2(shape.verts[1]));
            let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
            let len2 = abx * abx + aby * aby;
            let t = if len2 > 0.0 { (((local[0] - a[0]) * abx + (local[1] - a[1]) * aby) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let (cx, cy) = (a[0] + abx * t, a[1] + aby * t);
            (local[0] - cx).hypot(local[1] - cy) <= orr_view::fp_to_f32(shape.radius)
        }
        _ => {
            let n = shape.count as usize;
            (0..n).all(|i| {
                let (a, b) = (v2(shape.verts[i]), v2(shape.verts[(i + 1) % n]));
                (b[0] - a[0]) * (local[1] - a[1]) - (b[1] - a[1]) * (local[0] - a[0]) >= 0.0
            })
        }
    }
}

/// The body under a world point: the last one drawn (on top) that contains
/// it. The target is the entity's GUID when it has one, else its handle.
pub fn pick(view: &View<'_>, world: [f32; 2]) -> Option<Target> {
    let frame = view.frame();
    let (entities, bodies) = frame.dense::<Body>();
    for (&entity, body) in entities.iter().zip(bodies).rev() {
        let Some(collider) = frame.get::<Collider>(entity) else { continue };
        if hit_shape(&collider.shape, v2(body.pos), orr_view::fp_to_f32(body.angle), world) {
            return Some(target_of(view, entity));
        }
    }
    None
}

/// The target naming a frame entity: its GUID if it has one.
pub fn target_of(view: &View<'_>, entity: Entity) -> Target {
    view.guid_of(entity).map_or(Target::Entity(entity), |g| Target::Guid(g.clone()))
}

/// World position of the body of a target, as the frame has it.
pub fn body_pos(view: &View<'_>, target: &Target) -> Option<[f32; 2]> {
    let e = view.resolve(target).ok()?;
    view.frame().get::<Body>(e).map(|b| v2(b.pos))
}

// ---- GPU ----

/// The offscreen target and renderer of the viewport, with no UI attached
/// (tests use it directly).
pub struct ViewportGpu {
    target: OffscreenTarget<Wgpu>,
    renderer: Renderer<Wgpu>,
}

impl ViewportGpu {
    /// A target of `size` pixels on `rhi`.
    pub fn new(rhi: &Wgpu, size: (u32, u32)) -> Self {
        let target = OffscreenTarget::new(rhi, size.0, size.1, TARGET_FORMAT);
        let renderer = Renderer::new(rhi.clone(), target.format());
        Self { target, renderer }
    }

    /// The target.
    pub fn target(&self) -> &OffscreenTarget<Wgpu> {
        &self.target
    }

    /// Resizes the target if `size` differs, then draws.
    pub fn render(&mut self, size: (u32, u32), list: &RenderList, camera: &Camera) {
        self.target.resize(size.0, size.1);
        self.target.render(&mut self.renderer, list, camera);
    }

    /// Waits for the GPU and reads the pixels back (RGBA8, sRGB encoded, top row first).
    pub fn read_rgba8(&self) -> Vec<u8> {
        self.target.read_rgba8()
    }

    /// Adapter name, for the status bar.
    pub fn adapter_name(&self) -> String {
        self.renderer.rhi().adapter_name()
    }
}

/// [`ViewportGpu`] on eframe's device, shown in egui as a native texture.
pub struct GpuViewport {
    gpu: ViewportGpu,
    state: egui_wgpu::RenderState,
    id: egui::TextureId,
    generation: u64,
}

impl GpuViewport {
    /// Wraps eframe's device and queue (`orr_rhi::Wgpu::from_parts`) and
    /// registers the offscreen texture with egui.
    pub fn new(state: &egui_wgpu::RenderState, size: (u32, u32)) -> Self {
        let rhi = Wgpu::from_parts(state.instance.clone(), state.adapter.clone(), state.device.clone(), state.queue.clone());
        let gpu = ViewportGpu::new(&rhi, size);
        let id = state.renderer.write().register_native_texture(&state.device, gpu.target().sample_view(), egui_wgpu::wgpu::FilterMode::Linear);
        let generation = gpu.target().generation();
        Self { gpu, state: state.clone(), id, generation }
    }

    /// Draws `list` at `size` pixels and returns the texture to show.
    pub fn render(&mut self, size: (u32, u32), list: &RenderList, camera: &Camera) -> egui::TextureId {
        self.gpu.render(size, list, camera);
        let generation = self.gpu.target().generation();
        if generation != self.generation {
            self.generation = generation;
            self.state.renderer.write().update_egui_texture_from_wgpu_texture(
                &self.state.device,
                self.gpu.target().sample_view(),
                egui_wgpu::wgpu::FilterMode::Linear,
                self.id,
            );
        }
        self.id
    }

    /// The inner offscreen target (for readback in tests).
    pub fn gpu(&self) -> &ViewportGpu {
        &self.gpu
    }
}

impl Drop for GpuViewport {
    fn drop(&mut self) {
        self.state.renderer.write().free_texture(&self.id);
    }
}
