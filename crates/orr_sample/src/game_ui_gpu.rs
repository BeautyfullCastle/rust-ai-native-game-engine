//! Mesh-only egui overlay on the game's existing color view.
//!
//! Uploads use the backend's device/queue interoperability. Command encoders
//! and submission remain owned by Rhi. No sampling view or extra target is used.
use egui_wgpu::wgpu;
use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverlayError {
    UnsupportedFormat,
    InvalidScale,
    CallbackUnsupported,
    CallbackCommandsUnsupported,
    InvalidTexture(egui::TextureId),
    MissingTexture(egui::TextureId),
    InvalidMesh,
    PendingCapacityExceeded,
}
impl std::fmt::Display for OverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "game UI overlay: {self:?}")
    }
}
impl std::error::Error for OverlayError {}

/// Ordered texture epochs while surface acquisition is skipped. Only the newest
/// geometry is retained. Unlike FullOutput::append, this preserves free/reuse
/// ordering and a previous upload needed by a later draw-and-free frame.
///
/// At most 256 epochs and 64 MiB of texture payload are retained. Exceeding a
/// limit is an explicit error; existing pending work remains unchanged.
#[derive(Default)]
pub struct PendingOverlay {
    frames: VecDeque<(egui::FullOutput, usize)>,
    texture_bytes: usize,
}
impl PendingOverlay {
    pub fn push(&mut self, output: egui::FullOutput) -> Result<(), String> {
        self.push_with_limits(output, 256, 64 * 1024 * 1024)
            .map_err(|e| e.to_string())
    }

    fn push_with_limits(
        &mut self,
        mut output: egui::FullOutput,
        max_epochs: usize,
        max_bytes: usize,
    ) -> Result<(), OverlayError> {
        let result =
            (|| {
                validate_shapes(&output)?;
                let bytes = output.textures_delta.set.values().flatten().try_fold(
                    0usize,
                    |sum, delta| {
                        let egui::ImageData::Color(image) = &delta.image;
                        image
                            .pixels
                            .len()
                            .checked_mul(std::mem::size_of::<egui::Color32>())
                            .and_then(|n| sum.checked_add(n))
                            .ok_or(OverlayError::PendingCapacityExceeded)
                    },
                )?;
                let replaces_empty = self
                    .frames
                    .back()
                    .is_some_and(|(old, _)| old.textures_delta.is_empty());
                let count = self
                    .frames
                    .len()
                    .checked_add(1)
                    .and_then(|n| n.checked_sub(usize::from(replaces_empty)))
                    .ok_or(OverlayError::PendingCapacityExceeded)?;
                let total = self
                    .texture_bytes
                    .checked_add(bytes)
                    .ok_or(OverlayError::PendingCapacityExceeded)?;
                if count > max_epochs || total > max_bytes {
                    return Err(OverlayError::PendingCapacityExceeded);
                }
                // Platform output is already handled by the window integration.
                output.platform_output = Default::default();
                output.viewport_output = Default::default();
                if replaces_empty {
                    self.frames.pop_back();
                }
                if let Some((previous, _)) = self.frames.back_mut() {
                    drop(std::mem::take(&mut previous.shapes));
                }
                self.frames.push_back((std::mem::take(&mut output), bytes));
                self.texture_bytes = total;
                Ok(())
            })();
        // Also explicitly consume the rejected epoch without touching older ones.
        output.textures_delta.clear();
        result
    }

    pub fn paint(
        &mut self,
        overlay: &mut GpuOverlay,
        gpu: &Wgpu,
        view: &wgpu::TextureView,
        size: (u32, u32),
        context: &egui::Context,
    ) -> Result<(), String> {
        while let Some((output, bytes)) = self.frames.pop_front() {
            self.texture_bytes -= bytes;
            let size = if self.frames.is_empty() { size } else { (0, 0) };
            overlay.paint(gpu, view, size, context, output)?;
        }
        Ok(())
    }
}
impl Drop for PendingOverlay {
    fn drop(&mut self) {
        for (output, _) in &mut self.frames {
            output.textures_delta.clear();
        }
    }
}

pub struct GpuOverlay {
    renderer: Option<egui_wgpu::Renderer>,
    textures: BTreeMap<egui::TextureId, [usize; 2]>,
}
fn has_callback(shape: &egui::Shape) -> bool {
    match shape {
        egui::Shape::Callback(_) => true,
        egui::Shape::Vec(shapes) => shapes.iter().any(has_callback),
        _ => false,
    }
}
fn invalid_mesh(shape: &egui::Shape) -> bool {
    match shape {
        egui::Shape::Mesh(mesh) => {
            !mesh.is_valid()
                || mesh
                    .vertices
                    .iter()
                    .any(|v| !v.pos.is_finite() || !v.uv.is_finite())
        }
        egui::Shape::Vec(shapes) => shapes.iter().any(invalid_mesh),
        _ => false,
    }
}
fn validate_shapes(output: &egui::FullOutput) -> Result<(), OverlayError> {
    if !output.pixels_per_point.is_finite() || output.pixels_per_point <= 0.0 {
        return Err(OverlayError::InvalidScale);
    }
    if output.shapes.iter().any(|shape| has_callback(&shape.shape)) {
        return Err(OverlayError::CallbackUnsupported);
    }
    if output.shapes.iter().any(|shape| invalid_mesh(&shape.shape)) {
        return Err(OverlayError::InvalidMesh);
    }
    Ok(())
}
impl GpuOverlay {
    pub fn new(gpu: &Wgpu, format: TextureFormat) -> Self {
        let format = match format {
            TextureFormat::Rgba8Unorm => Some(wgpu::TextureFormat::Rgba8Unorm),
            TextureFormat::Rgba8UnormSrgb => Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            TextureFormat::Bgra8Unorm => Some(wgpu::TextureFormat::Bgra8Unorm),
            TextureFormat::Bgra8UnormSrgb => Some(wgpu::TextureFormat::Bgra8UnormSrgb),
            TextureFormat::Depth32Float => None,
        };
        Self {
            renderer: format.map(|format| {
                egui_wgpu::Renderer::new(
                    gpu.device(),
                    format,
                    egui_wgpu::RendererOptions::PREDICTABLE,
                )
            }),
            textures: BTreeMap::new(),
        }
    }

    pub fn paint(
        &mut self,
        gpu: &Wgpu,
        view: &wgpu::TextureView,
        size: (u32, u32),
        context: &egui::Context,
        output: egui::FullOutput,
    ) -> Result<(), String> {
        self.paint_checked(gpu, view, size, context, output)
            .map_err(|e| e.to_string())
    }

    /// Typed variant for tests and integrations that need to distinguish failures.
    /// Invalid input and callbacks are rejected before any GPU or texture mutation.
    /// A minimized frame still applies texture deltas so restoration cannot lose
    /// an atlas update; it does not create a render pass or divide by zero.
    pub fn paint_checked(
        &mut self,
        gpu: &Wgpu,
        view: &wgpu::TextureView,
        size: (u32, u32),
        context: &egui::Context,
        mut output: egui::FullOutput,
    ) -> Result<(), OverlayError> {
        let result = self.paint_frame(gpu, view, size, context, &mut output);
        // Explicitly consume even rejected output: egui 0.36 checks deltas on Drop.
        output.textures_delta.clear();
        result
    }

    fn paint_frame(
        &mut self,
        gpu: &Wgpu,
        view: &wgpu::TextureView,
        size: (u32, u32),
        context: &egui::Context,
        output: &mut egui::FullOutput,
    ) -> Result<(), OverlayError> {
        if self.renderer.is_none() {
            return Err(OverlayError::UnsupportedFormat);
        }
        validate_shapes(output)?;
        let jobs = context.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
        if jobs
            .iter()
            .any(|job| matches!(job.primitive, egui::epaint::Primitive::Callback(_)))
        {
            return Err(OverlayError::CallbackUnsupported);
        }
        let mut textures = self.textures.clone();
        let limit = gpu.device().limits().max_texture_dimension_2d as usize;
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                let [width, height] = delta.image.size();
                let egui::ImageData::Color(image) = &delta.image;
                if width == 0
                    || height == 0
                    || width > limit
                    || height > limit
                    || width.checked_mul(height) != Some(image.pixels.len())
                {
                    return Err(OverlayError::InvalidTexture(*id));
                }
                if let Some([x, y]) = delta.pos {
                    let bounds = textures.get(id).ok_or(OverlayError::MissingTexture(*id))?;
                    if x.checked_add(width).is_none_or(|n| n > bounds[0])
                        || y.checked_add(height).is_none_or(|n| n > bounds[1])
                    {
                        return Err(OverlayError::InvalidTexture(*id));
                    }
                } else {
                    textures.insert(*id, [width, height]);
                }
            }
        }
        for job in &jobs {
            if let egui::epaint::Primitive::Mesh(mesh) = &job.primitive {
                if !mesh.is_valid() {
                    return Err(OverlayError::InvalidMesh);
                }
                if !textures.contains_key(&mesh.texture_id) {
                    return Err(OverlayError::MissingTexture(mesh.texture_id));
                }
            }
        }
        let renderer = self.renderer.as_mut().expect("format checked above");
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(gpu.device(), gpu.queue(), *id, delta);
            }
        }
        self.textures = textures;
        if size.0 != 0 && size.1 != 0 {
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [size.0, size.1],
                pixels_per_point: output.pixels_per_point,
            };
            let mut encoder = gpu.create_encoder("game UI overlay");
            let callbacks =
                renderer.update_buffers(gpu.device(), gpu.queue(), &mut encoder, &jobs, &screen);
            // Mesh-only jobs cannot return these; never bypass Rhi if that changes.
            if !callbacks.is_empty() {
                return Err(OverlayError::CallbackCommandsUnsupported);
            }
            {
                let mut pass = encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("game UI overlay"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    })
                    .forget_lifetime();
                renderer.render(&mut pass, &jobs, &screen);
            }
            gpu.submit(encoder);
        }
        // Egui may use a texture in the same frame in which it asks to free it.
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
            self.textures.remove(id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, Color32, Rect, TextureId};
    use orr_render::orr_rhi::{TextureDesc, TextureUsage, WgpuOptions};

    fn target(gpu: &Wgpu, size: u32, format: TextureFormat) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = gpu.create_texture(&TextureDesc {
            label: "single-view game UI test",
            width: size,
            height: size,
            format,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
            sample_count: 1,
            view_formats: &[],
        });
        let view = gpu.create_texture_view(&texture, None);
        (texture, view)
    }
    fn pixel(data: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        data[(y * size + x) * 4..(y * size + x) * 4 + 4]
            .try_into()
            .unwrap()
    }
    fn near(actual: [u8; 4], expected: [u8; 4]) {
        assert!(
            actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2),
            "actual {actual:?}, expected {expected:?}"
        );
    }
    fn image(color: Color32) -> egui::epaint::ImageDelta {
        egui::epaint::ImageDelta::full(
            egui::ColorImage::filled([2, 2], color),
            egui::TextureOptions::NEAREST,
        )
    }
    fn output(id: TextureId, scale: f32) -> egui::FullOutput {
        let rect = Rect::from_min_max(pos2(4.0, 4.0), pos2(24.0, 24.0));
        let mut mesh = egui::Mesh::with_texture(id);
        mesh.add_rect_with_uv(
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        egui::FullOutput {
            pixels_per_point: scale,
            shapes: vec![egui::epaint::ClippedShape {
                clip_rect: Rect::from_min_max(pos2(4.0, 4.0), pos2(16.0, 16.0)),
                shape: egui::Shape::mesh(mesh),
            }],
            ..Default::default()
        }
    }
    fn scene(gpu: &Wgpu, view: &wgpu::TextureView, size: u32, format: TextureFormat) {
        let mut renderer = orr_render::Renderer::new(gpu.clone(), format);
        renderer.clear = Some([0.0, 0.0, 0.0, 1.0]);
        let mut list = orr_render::RenderList::new();
        list.quad([0.0, 0.0], [0.75, 0.75], 0.0, [0.0, 1.0, 1.0, 1.0]);
        renderer.draw(
            view,
            (size, size),
            &list,
            &orr_render::Camera::new([0.0, 0.0], 1.0),
        );
    }

    #[test]
    fn single_view_overlay_readback_and_texture_lifecycle() {
        // ORR_REQUIRE_GPU=1 makes absent software rendering a hard failure in acceptance.
        let gpu = match Wgpu::headless(WgpuOptions {
            force_software: true,
            ..Default::default()
        }) {
            Ok(gpu) => gpu,
            Err(e) => {
                assert!(
                    std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                    "required software GPU: {e}"
                );
                eprintln!("SKIP: {e}");
                return;
            }
        };
        assert!(gpu.is_software());
        eprintln!("game UI offscreen adapter: {}", gpu.adapter_name());
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |_| {})
            .textures_delta
            .clear();
        let id = TextureId::Managed(987);
        for format in [
            TextureFormat::Rgba8Unorm,
            TextureFormat::Rgba8UnormSrgb,
            TextureFormat::Bgra8Unorm,
            TextureFormat::Bgra8UnormSrgb,
        ] {
            let (texture, view) = target(&gpu, 64, format);
            scene(&gpu, &view, 64, format);
            let before = gpu.read_texture(&texture);
            let mut overlay = GpuOverlay::new(&gpu, format);
            let mut frame = output(id, 1.0);
            frame
                .textures_delta
                .push(id, image(Color32::from_gray(128)));
            overlay
                .paint_checked(&gpu, &view, (64, 64), &ctx, frame)
                .unwrap();
            let after = gpu.read_texture(&texture);
            near(pixel(&after, 64, 10, 10), [128, 128, 128, 255]);
            assert_eq!(
                pixel(&after, 64, 20, 10),
                pixel(&before, 64, 20, 10),
                "clip preserved shape"
            );
            assert_eq!(
                pixel(&after, 64, 40, 40),
                pixel(&before, 64, 40, 40),
                "uncovered shape preserved"
            );
            assert_eq!(
                pixel(&after, 64, 1, 1),
                pixel(&before, 64, 1, 1),
                "background preserved"
            );

            // Premultiplied alpha preserves the game beneath the overlay. sRGB
            // attachments blend in linear space, UNORM attachments in gamma space.
            scene(&gpu, &view, 64, format);
            let mut translucent = output(id, 1.0);
            translucent
                .textures_delta
                .push(id, image(Color32::from_white_alpha(128)));
            overlay
                .paint_checked(&gpu, &view, (64, 64), &ctx, translucent)
                .unwrap();
            let blended = gpu.read_texture(&texture);
            let other = if matches!(
                format,
                TextureFormat::Rgba8UnormSrgb | TextureFormat::Bgra8UnormSrgb
            ) {
                219
            } else {
                255
            };
            let expected = if matches!(
                format,
                TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
            ) {
                [other, other, 128, 255]
            } else {
                [128, other, other, 255]
            };
            near(pixel(&blended, 64, 10, 10), expected);
            // Restore the opaque reference before failure-atomicity checks.
            let mut opaque = output(id, 1.0);
            opaque
                .textures_delta
                .push(id, image(Color32::from_gray(128)));
            overlay
                .paint_checked(&gpu, &view, (64, 64), &ctx, opaque)
                .unwrap();

            // Callback plus a texture replacement must fail atomically, including nested callbacks.
            let mut frame = output(id, 1.0);
            frame.textures_delta.push(id, image(Color32::BLACK));
            frame.shapes[0].shape =
                egui::Shape::Vec(vec![egui::Shape::Callback(egui::PaintCallback {
                    rect: Rect::EVERYTHING,
                    callback: std::sync::Arc::new(()),
                })]);
            assert_eq!(
                overlay.paint_checked(&gpu, &view, (64, 64), &ctx, frame),
                Err(OverlayError::CallbackUnsupported)
            );
            assert_eq!(gpu.read_texture(&texture), after);
            overlay
                .paint_checked(&gpu, &view, (64, 64), &ctx, output(id, 1.0))
                .unwrap();
            near(
                pixel(&gpu.read_texture(&texture), 64, 10, 10),
                [128, 128, 128, 255],
            );
            let mut bad = output(id, f32::NAN);
            assert_eq!(
                overlay.paint_checked(&gpu, &view, (64, 64), &ctx, bad.clone()),
                Err(OverlayError::InvalidScale)
            );
            bad.pixels_per_point = 1.0;
            let mut delta = image(Color32::WHITE);
            delta.pos = Some([2, 0]);
            bad.textures_delta.push(id, delta);
            assert_eq!(
                overlay.paint_checked(&gpu, &view, (64, 64), &ctx, bad),
                Err(OverlayError::InvalidTexture(id))
            );

            // A partial update received while minimized survives resize and DPI change.
            let mut frame = output(id, 2.0);
            frame.shapes.clear();
            let mut delta = image(Color32::WHITE);
            delta.pos = Some([0, 0]);
            frame.textures_delta.push(id, delta);
            overlay
                .paint_checked(&gpu, &view, (0, 0), &ctx, frame)
                .unwrap();
            let (resized, resized_view) = target(&gpu, 128, format);
            scene(&gpu, &resized_view, 128, format);
            let baseline = gpu.read_texture(&resized);
            let mut frame = output(id, 2.0);
            frame.textures_delta.free(id);
            overlay
                .paint_checked(&gpu, &resized_view, (128, 128), &ctx, frame)
                .unwrap();
            let pixels = gpu.read_texture(&resized);
            near(pixel(&pixels, 128, 20, 20), [255; 4]);
            assert_eq!(pixel(&pixels, 128, 40, 20), pixel(&baseline, 128, 40, 20));
            assert!(!overlay.textures.contains_key(&id));
            assert_eq!(
                overlay.paint_checked(&gpu, &resized_view, (128, 128), &ctx, output(id, 2.0)),
                Err(OverlayError::MissingTexture(id))
            );
            // Reusing a freed id with a complete delta is valid.
            let mut frame = output(id, 2.0);
            frame.textures_delta.push(id, image(Color32::from_gray(64)));
            overlay
                .paint_checked(&gpu, &resized_view, (128, 128), &ctx, frame)
                .unwrap();
            near(
                pixel(&gpu.read_texture(&resized), 128, 20, 20),
                [64, 64, 64, 255],
            );
        }
        pending_epochs(&gpu);
        korean_glyphs(&gpu);
        #[cfg(feature = "sprites")]
        preserves_sprite(&gpu);
        if let Some(path) = std::env::var_os("ORR_GAME_UI_SCREENSHOT") {
            screenshot(&gpu, std::path::Path::new(&path));
        }
    }

    #[test]
    fn pending_queue_is_bounded_and_rejects_callbacks_without_mutation() {
        let id = TextureId::Managed(42);
        let mut pending = PendingOverlay::default();
        for _ in 0..1000 {
            pending.push(output(id, 1.0)).unwrap();
        }
        assert_eq!(pending.frames.len(), 1);
        assert_eq!(pending.texture_bytes, 0);
        let mut first = output(id, 1.0);
        first.textures_delta.push(id, image(Color32::WHITE));
        pending.push_with_limits(first, 1, 16).unwrap();
        assert_eq!(pending.frames.len(), 1);
        assert_eq!(pending.texture_bytes, 16);
        let mut second = output(id, 1.0);
        second.textures_delta.push(id, image(Color32::BLACK));
        assert_eq!(
            pending.push_with_limits(second.clone(), 1, 64),
            Err(OverlayError::PendingCapacityExceeded)
        );
        assert_eq!(
            pending.push_with_limits(second, 10, 16),
            Err(OverlayError::PendingCapacityExceeded)
        );
        let mut callback = output(id, 1.0);
        callback.textures_delta.push(id, image(Color32::BLACK));
        callback.shapes[0].shape = egui::Shape::Callback(egui::PaintCallback {
            rect: Rect::EVERYTHING,
            callback: std::sync::Arc::new(()),
        });
        assert_eq!(
            pending.push_with_limits(callback, 10, 64),
            Err(OverlayError::CallbackUnsupported)
        );
        assert_eq!(pending.frames.len(), 1);
        assert_eq!(pending.texture_bytes, 16);
        assert_eq!(pending.frames[0].0.shapes.len(), 1);
        // A free-only epoch has zero payload but still counts toward the bound.
        let mut free = output(id, 1.0);
        free.shapes.clear();
        free.textures_delta.free(id);
        pending.push_with_limits(free, 2, 16).unwrap();
        assert!(pending.frames[0].0.shapes.is_empty());
        assert_eq!(pending.frames.len(), 2);
        assert_eq!(
            pending.push_with_limits(output(id, 1.0), 2, 16),
            Err(OverlayError::PendingCapacityExceeded)
        );
    }

    fn pending_epochs(gpu: &Wgpu) {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |_| {})
            .textures_delta
            .clear();
        let (texture, view) = target(gpu, 64, TextureFormat::Rgba8Unorm);
        scene(gpu, &view, 64, TextureFormat::Rgba8Unorm);
        let mut overlay = GpuOverlay::new(gpu, TextureFormat::Rgba8Unorm);
        let mut pending = PendingOverlay::default();
        let id = TextureId::Managed(42);
        let mut initial = output(id, 1.0);
        initial.textures_delta.push(id, image(Color32::BLACK));
        pending.push(initial).unwrap();
        let mut freed = output(id, 1.0);
        freed.shapes.clear();
        freed.textures_delta.free(id);
        pending.push(freed).unwrap();
        let mut reused = output(id, 1.0);
        reused.textures_delta.push(id, image(Color32::WHITE));
        pending.push(reused).unwrap();
        pending
            .paint(&mut overlay, gpu, &view, (64, 64), &ctx)
            .unwrap();
        near(pixel(&gpu.read_texture(&texture), 64, 10, 10), [255; 4]);
        assert!(pending.frames.is_empty());
        assert_eq!(pending.texture_bytes, 0);
        pending.push(output(id, 1.0)).unwrap();
        pending
            .paint(&mut overlay, gpu, &view, (64, 64), &ctx)
            .unwrap();
        near(pixel(&gpu.read_texture(&texture), 64, 10, 10), [255; 4]);
        // The original upload is needed even though a newer frame frees its id.
        let id = TextureId::Managed(43);
        let mut initial = output(id, 1.0);
        initial
            .textures_delta
            .push(id, image(Color32::from_gray(96)));
        pending.push(initial).unwrap();
        let mut final_draw = output(id, 1.0);
        final_draw.textures_delta.free(id);
        pending.push(final_draw).unwrap();
        pending
            .paint(&mut overlay, gpu, &view, (64, 64), &ctx)
            .unwrap();
        near(
            pixel(&gpu.read_texture(&texture), 64, 10, 10),
            [96, 96, 96, 255],
        );
        assert_eq!(
            overlay.paint_checked(gpu, &view, (64, 64), &ctx, output(id, 1.0)),
            Err(OverlayError::MissingTexture(id))
        );
    }

    fn korean_glyphs(gpu: &Wgpu) {
        let (texture, view) = target(gpu, 256, TextureFormat::Rgba8Unorm);
        scene(gpu, &view, 256, TextureFormat::Rgba8Unorm);
        korean_glyphs_on(gpu, &texture, &view);
    }

    fn korean_glyphs_on(gpu: &Wgpu, texture: &wgpu::Texture, view: &wgpu::TextureView) {
        let font = include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec();
        let ui = crate::game_ui::GameUi::from_font(font, true).unwrap();
        let before = gpu.read_texture(texture);
        let mut glyph_widths = Vec::new();
        let frame = ui.context.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    egui::vec2(256.0, 256.0),
                )),
                ..Default::default()
            },
            |root| {
                let painter = root.ctx().layer_painter(egui::LayerId::background());
                for (i, ch) in ["오", "러", "리", "메", "뉴"].iter().enumerate() {
                    let rect = painter.text(
                        pos2(10.0 + i as f32 * 42.0, 50.0),
                        egui::Align2::LEFT_TOP,
                        ch,
                        egui::FontId::proportional(30.0),
                        Color32::WHITE,
                    );
                    glyph_widths.push(rect.width());
                }
            },
        );
        assert!(glyph_widths.iter().all(|w| *w > 10.0));
        let mut overlay = GpuOverlay::new(gpu, TextureFormat::Rgba8Unorm);
        overlay
            .paint(gpu, view, (256, 256), &ui.context, frame)
            .unwrap();
        let after = gpu.read_texture(texture);
        let masks: Vec<Vec<bool>> = (0..5)
            .map(|i| {
                (0..40)
                    .flat_map(|y| (0..40).map(move |x| (x, y)))
                    .map(|(x, y)| {
                        pixel(&after, 256, 10 + i * 42 + x, 50 + y)
                            != pixel(&before, 256, 10 + i * 42 + x, 50 + y)
                    })
                    .collect()
            })
            .collect();
        assert!(
            masks
                .iter()
                .all(|mask| mask.iter().filter(|v| **v).count() > 40),
            "each Korean glyph rendered"
        );
        assert!(
            masks.windows(2).all(|pair| pair[0] != pair[1]),
            "distinct Korean glyphs, not repeated tofu"
        );
        assert_eq!(pixel(&after, 256, 128, 128), pixel(&before, 256, 128, 128));
    }

    #[cfg(feature = "sprites")]
    fn preserves_sprite(gpu: &Wgpu) {
        use orr_render::{SpriteDrawList, SpriteInstance, SpriteRenderer};
        let doc = orr_sprite::SpriteDocument::from_json(r#"{"format":"orr_sprite","version":1,"atlas":{"image":"test.rgba","width":1,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1}],"clips":[]}"#).unwrap();
        let mut renderer = SpriteRenderer::new(
            gpu.clone(),
            TextureFormat::Rgba8Unorm,
            doc,
            &[255, 0, 255, 255],
        )
        .unwrap();
        renderer.clear = Some([0.0, 0.0, 0.0, 1.0]);
        let (texture, view) = target(gpu, 64, TextureFormat::Rgba8Unorm);
        renderer
            .draw(
                &view,
                (64, 64),
                &SpriteDrawList {
                    sprites: vec![SpriteInstance {
                        size: [1.5, 1.5],
                        ..Default::default()
                    }],
                },
                &orr_render::Camera::new([0.0, 0.0], 1.0),
            )
            .unwrap();
        let before = gpu.read_texture(&texture);
        near(pixel(&before, 64, 40, 40), [255, 0, 255, 255]);
        let mut overlay = GpuOverlay::new(gpu, TextureFormat::Rgba8Unorm);
        let mut frame = output(TextureId::Managed(2), 1.0);
        frame
            .textures_delta
            .push(TextureId::Managed(2), image(Color32::WHITE));
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |_| {})
            .textures_delta
            .clear();
        overlay.paint(gpu, &view, (64, 64), &ctx, frame).unwrap();
        let after = gpu.read_texture(&texture);
        assert_eq!(pixel(&before, 64, 40, 40), pixel(&after, 64, 40, 40));
        near(pixel(&after, 64, 10, 10), [255; 4]);
        let (texture, view) = target(gpu, 256, TextureFormat::Rgba8Unorm);
        renderer
            .draw(
                &view,
                (256, 256),
                &SpriteDrawList {
                    sprites: vec![SpriteInstance {
                        size: [1.5, 1.5],
                        ..Default::default()
                    }],
                },
                &orr_render::Camera::new([0.0, 0.0], 1.0),
            )
            .unwrap();
        near(
            pixel(&gpu.read_texture(&texture), 256, 128, 128),
            [255, 0, 255, 255],
        );
        korean_glyphs_on(gpu, &texture, &view);
    }

    /// Optional offscreen visual evidence. These are not native-window screenshots.
    fn screenshot(gpu: &Wgpu, path: &std::path::Path) {
        let mut ui = crate::game_ui::GameUi::from_font(
            include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec(),
            true,
        )
        .unwrap();
        let (texture, view) = target(gpu, 900, TextureFormat::Rgba8Unorm);
        let mut overlay = GpuOverlay::new(gpu, TextureFormat::Rgba8Unorm);
        for (screen, time) in [
            (crate::game_ui::Screen::Title, 1.0),
            (crate::game_ui::Screen::Menu, 3.0),
        ] {
            if screen == crate::game_ui::Screen::Menu {
                ui.apply(crate::game_ui::Action::Menu);
            }
            let mut pending = PendingOverlay::default();
            for offset in [0.0, 0.5, 1.0] {
                let (output, _) = ui.show(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            pos2(0.0, 0.0),
                            egui::vec2(900.0, 900.0),
                        )),
                        time: Some(time + offset),
                        ..Default::default()
                    },
                    crate::game_ui::Hud {
                        tick: 420,
                        verified_tick: 418,
                        rollbacks: 2,
                    },
                );
                pending.push(output).unwrap();
            }
            let mut renderer = orr_render::Renderer::new(gpu.clone(), TextureFormat::Rgba8Unorm);
            let mut list = orr_render::RenderList::new();
            list.quad([0.0, 0.0], [0.92, 0.92], 0.0, [0.045, 0.065, 0.10, 1.0]);
            list.circle([-0.6, -0.55], 0.10, [0.2, 0.8, 0.9, 1.0]);
            list.circle([0.6, -0.5], 0.10, [0.9, 0.3, 0.4, 1.0]);
            list.quad([-0.45, 0.5], [0.15, 0.04], 0.2, [0.4, 0.6, 0.9, 1.0]);
            renderer.draw(
                &view,
                (900, 900),
                &list,
                &orr_render::Camera::new([0.0, 0.0], 1.0),
            );
            pending
                .paint(&mut overlay, gpu, &view, (900, 900), &ui.context)
                .unwrap();
            let destination = if screen == crate::game_ui::Screen::Title {
                path.to_path_buf()
            } else {
                path.with_file_name(format!(
                    "{}-menu.png",
                    path.file_stem().unwrap().to_string_lossy()
                ))
            };
            let file = std::fs::File::create(&destination).unwrap();
            let mut encoder = png::Encoder::new(file, 900, 900);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&gpu.read_texture(&texture))
                .unwrap();
            eprintln!(
                "offscreen game UI visual evidence: {}",
                destination.display()
            );
        }
    }
}
