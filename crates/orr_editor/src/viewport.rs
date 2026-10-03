//! The viewport: what the editor draws for a frame, picking, and the GPU side.
//!
//! The editor sees the simulation only through the bridge: the frames it
//! draws are snapshots the host published, read through the game's view
//! mapping (`orr_sample::physics_view::body_views`), never a document or a
//! play session of its own.
//!
//! - [`build_list`] turns the bodies of the frame on screen (the scene's
//!   preview frame in edit mode, the live frame in play mode) into an
//!   `orr_render::RenderList`: grid, bodies (same colors and shapes as the
//!   sample), collider outlines and the selection highlight. Pure data, no GPU.
//! - [`pick`] finds the body under a world point.
//! - [`ViewportGpu`] draws a list into an offscreen texture with
//!   `orr_render`; [`GpuViewport`] shares that texture with egui as a native
//!   texture on eframe's own wgpu device.
//!
//! View layer: floats are fine here.

use orr_ecs::Entity;
use orr_reflect::Guid;
use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu};
use orr_render::{instance_of, Camera, OffscreenTarget, RenderList, Renderer};
use orr_sample::editor_view::{Drawable, Outline};
use orr_view::{RenderItem, Transform2, Vec2};

use crate::model::{EntityRow, Summary};

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

/// Outline of a collider shape at a body pose.
fn shape_outline(list: &mut RenderList, pos: [f32; 2], angle: f32, shape: &Outline, width: f32, color: [f32; 4]) {
    match shape {
        Outline::Circle { radius } => {
            list.circle_outline(pos, *radius, 28, width, color);
            let tip = rot([*radius, 0.0], angle);
            list.line(pos, [pos[0] + tip[0], pos[1] + tip[1]], width, color);
        }
        Outline::Capsule { a, b, radius } => {
            let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let m = rot(mid, angle);
            list.capsule_outline([pos[0] + m[0], pos[1] + m[1]], dx.hypot(dy) * 0.5, *radius, angle + dy.atan2(dx), 24, width, color);
        }
        Outline::Polygon(verts) => {
            let pts: Vec<[f32; 2]> = verts
                .iter()
                .map(|&v| {
                    let r = rot(v, angle);
                    [pos[0] + r[0], pos[1] + r[1]]
                })
                .collect();
            list.polyline(&pts, true, width, color);
        }
    }
}

/// Builds the render list of the bodies of a frame. `vp` is the viewport in
/// pixels (for the grid). `selected` is the frame entity to highlight.
pub fn build_list(bodies: &[Drawable], selected: Option<Entity>, camera: &Camera, vp: (u32, u32)) -> RenderList {
    let mut list = RenderList::new();
    add_grid(&mut list, camera, vp);
    let mut chosen: Option<&Drawable> = None;
    for b in bodies {
        let item = RenderItem { entity: b.entity, transform: Transform2::new(Vec2::new(b.pos[0], b.pos[1]), b.angle + b.turn), style: b.style };
        list.shapes.push(instance_of(&item));
        if selected == Some(b.entity) {
            chosen = Some(b);
        }
    }
    if bodies.len() <= OUTLINE_LIMIT {
        for b in bodies.iter().filter(|b| selected != Some(b.entity)) {
            shape_outline(&mut list, b.pos, b.angle, &b.outline, 1.0, OUTLINE);
        }
    }
    if let Some(b) = chosen {
        shape_outline(&mut list, b.pos, b.angle, &b.outline, 3.0, HIGHLIGHT);
        list.cross(b.pos, 4.0 / camera.pixels_per_unit(vp.0, vp.1), 2.0, HIGHLIGHT);
    }
    list
}

/// Which entities a proposal touches, by GUID: `(changed, added, removed)`.
pub fn preview_marks(summary: &Summary) -> (Vec<Guid>, Vec<Guid>, Vec<Guid>) {
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

/// A frame with the GUID map of its entities, to look bodies up by GUID.
pub struct Scene<'a> {
    /// The bodies of the frame.
    pub bodies: &'a [Drawable],
    /// Its entities (handle, GUID).
    pub rows: &'a [EntityRow],
}

impl<'a> Scene<'a> {
    fn body_of(&self, guid: &Guid) -> Option<&'a Drawable> {
        let row = self.rows.iter().find(|r| r.guid.as_ref() == Some(guid))?;
        self.bodies.iter().find(|b| b.entity == row.entity)
    }

    /// The frame entity of a GUID.
    pub fn entity_of(&self, guid: &Guid) -> Option<Entity> {
        self.rows.iter().find(|r| r.guid.as_ref() == Some(guid)).map(|r| r.entity)
    }
}

/// Builds the render list of a proposal preview: `preview` is the staged
/// frame (drawn like any frame), `base` the document's own. Changed entities
/// get a cyan outline, added ones a green one; a changed entity that moved
/// leaves a ghost at its old place, and removed ones are drawn as ghosts
/// from `base`. `selected` is the entity (of `preview`) to highlight.
pub fn build_preview_list(preview: &Scene<'_>, base: &Scene<'_>, summary: &Summary, selected: Option<Entity>, camera: &Camera, vp: (u32, u32)) -> RenderList {
    let mut list = build_list(preview.bodies, selected, camera, vp);
    let (changed, added, removed) = preview_marks(summary);
    let mark = |list: &mut RenderList, guid: &Guid, color: [f32; 4]| {
        if let Some(b) = preview.body_of(guid) {
            shape_outline(list, b.pos, b.angle, &b.outline, 3.0, color);
        }
    };
    for g in &changed {
        mark(&mut list, g, PREVIEW_CHANGED);
        if let (Some(new), Some(old)) = (preview.body_of(g), base.body_of(g)) {
            if new.pos != old.pos {
                shape_outline(&mut list, old.pos, old.angle, &old.outline, 2.0, PREVIEW_GHOST);
                list.line(old.pos, new.pos, 1.5, PREVIEW_GHOST);
            }
        }
    }
    for g in &added {
        mark(&mut list, g, PREVIEW_ADDED);
    }
    for g in &removed {
        if let Some(b) = base.body_of(g) {
            shape_outline(&mut list, b.pos, b.angle, &b.outline, 2.0, PREVIEW_GHOST);
            list.cross(b.pos, 6.0 / camera.pixels_per_unit(vp.0, vp.1), 2.0, PREVIEW_GHOST);
        }
    }
    list
}

/// Draws the pulses of entities an agent just edited: an outline and a ring that
/// grows and fades. `fade` is 1 at the start of a pulse and 0 at its end. Entities
/// that are not drawn (no body and collider, or gone) are skipped.
pub fn add_pulses(list: &mut RenderList, bodies: &[Drawable], pulses: &[(Entity, f32)]) {
    for (entity, fade) in pulses {
        let Some(b) = bodies.iter().find(|b| b.entity == *entity) else { continue };
        let color = [PULSE[0], PULSE[1], PULSE[2], fade.clamp(0.0, 1.0)];
        shape_outline(list, b.pos, b.angle, &b.outline, 2.0 + 3.0 * fade, color);
        let ring = b.outline.extent() + 0.25 + (1.0 - fade) * 1.5;
        list.circle_outline(b.pos, ring, 40, 2.0, color);
    }
}

/// The body under a world point: the last one drawn (on top) that contains it.
pub fn pick(bodies: &[Drawable], world: [f32; 2]) -> Option<Entity> {
    bodies.iter().rev().find(|b| b.hit(world)).map(|b| b.entity)
}

/// World position of a frame entity's body.
pub fn body_pos(bodies: &[Drawable], entity: Entity) -> Option<[f32; 2]> {
    bodies.iter().find(|b| b.entity == entity).map(|b| b.pos)
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
