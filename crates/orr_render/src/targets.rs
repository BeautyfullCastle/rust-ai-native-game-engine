//! Where the renderer draws: a window surface or an offscreen texture.

use std::sync::Arc;

use orr_rhi::{Acquire, Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions, WindowHandle};

use crate::camera::Camera;
use crate::camera3d::Camera3D;
use crate::list::RenderList;
use crate::list3d::RenderList3D;
use crate::renderer::Renderer;
use crate::renderer3d::{Renderer3D, Settings3D};

/// An offscreen color texture. For the editor viewport: draw into it with
/// [`OffscreenTarget::render`], then show [`OffscreenTarget::sample_view`]
/// in egui. When [`OffscreenTarget::generation`] changes (after `resize`)
/// the views are new and must be registered again.
pub struct OffscreenTarget<B: Rhi> {
    rhi: B,
    texture: B::Texture,
    render_view: B::TextureView,
    sample_view: B::TextureView,
    size: (u32, u32),
    format: TextureFormat,
    generation: u64,
}

impl<B: Rhi> OffscreenTarget<B> {
    /// `format` is what the renderer writes. With an sRGB format the
    /// shader output is encoded on write, exactly like a window surface.
    pub fn new(rhi: &B, width: u32, height: u32, format: TextureFormat) -> Self {
        let (texture, render_view, sample_view) = Self::create(rhi, width, height, format);
        Self { rhi: rhi.clone(), texture, render_view, sample_view, size: (width.max(1), height.max(1)), format, generation: 0 }
    }

    fn create(rhi: &B, width: u32, height: u32, format: TextureFormat) -> (B::Texture, B::TextureView, B::TextureView) {
        let raw = format.with_srgb(false);
        let view_formats: Vec<TextureFormat> = if raw != format { vec![raw] } else { Vec::new() };
        let texture = rhi.create_texture(&TextureDesc {
            label: "offscreen target",
            width,
            height,
            format,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::TEXTURE_BINDING | TextureUsage::COPY_SRC,
            sample_count: 1,
            view_formats: &view_formats,
        });
        let render_view = rhi.create_texture_view(&texture, None);
        let sample_view = rhi.create_texture_view(&texture, Some(raw));
        (texture, render_view, sample_view)
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn format(&self) -> TextureFormat {
        self.format
    }

    pub fn texture(&self) -> &B::Texture {
        &self.texture
    }

    /// The view the renderer writes to (the target's own format).
    pub fn render_view(&self) -> &B::TextureView {
        &self.render_view
    }

    /// A view of the same texels in the non-sRGB format: what a UI toolkit
    /// that blends in gamma space (egui) should sample, so the picture looks
    /// the same as in a window.
    pub fn sample_view(&self) -> &B::TextureView {
        &self.sample_view
    }

    /// Bumps each time the texture is recreated by `resize`.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Recreates the texture when the size changed. Returns `true` if it did.
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        let size = (width.max(1), height.max(1));
        if size == self.size {
            return false;
        }
        let (texture, render_view, sample_view) = Self::create(&self.rhi, size.0, size.1, self.format);
        self.texture = texture;
        self.render_view = render_view;
        self.sample_view = sample_view;
        self.size = size;
        self.generation += 1;
        true
    }

    /// Draws `list` into the texture.
    pub fn render(&self, renderer: &mut Renderer<B>, list: &RenderList, camera: &Camera) {
        renderer.draw(&self.render_view, self.size, list, camera);
    }

    /// Draws a 3D `list` into the texture (multisampled, resolved into it).
    pub fn render3d(&self, renderer: &mut Renderer3D<B>, list: &RenderList3D, camera: &Camera3D) {
        renderer.draw(&self.render_view, self.size, list, camera);
    }

    /// Draws a 3D `list`, then a 2D `overlay` (screen space text and gizmos, see
    /// [`crate::text`]) on top of it with `overlay_renderer`, which keeps the 3D picture.
    pub fn render3d_overlay(
        &self,
        renderer: &mut Renderer3D<B>,
        list: &RenderList3D,
        camera: &Camera3D,
        overlay_renderer: &mut Renderer<B>,
        overlay: &RenderList,
    ) {
        renderer.draw(&self.render_view, self.size, list, camera);
        let clear = overlay_renderer.clear.take();
        overlay_renderer.draw(&self.render_view, self.size, overlay, &crate::text::pixel_camera(self.size));
        overlay_renderer.clear = clear;
    }

    /// Waits for the GPU and reads the pixels back as tightly packed RGBA8
    /// (stored bytes, so sRGB targets return encoded values), top row first.
    pub fn read_rgba8(&self) -> Vec<u8> {
        let mut bytes = self.rhi.read_texture(&self.texture);
        if self.format.is_bgra() {
            for px in bytes.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        bytes
    }
}

/// A window surface and the renderer that draws to it.
pub struct WindowRenderer<B: Rhi> {
    pub renderer: Renderer<B>,
    surface: B::Surface,
    size: (u32, u32),
}

impl<B: Rhi> WindowRenderer<B> {
    pub fn from_parts(rhi: B, surface: B::Surface, size: (u32, u32)) -> Self {
        let format = rhi.surface_format(&surface);
        Self { renderer: Renderer::new(rhi, format), surface, size: (size.0.max(1), size.1.max(1)) }
    }

    pub fn adapter_name(&self) -> String {
        self.renderer.rhi().adapter_name()
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.size = (width, height);
        self.renderer.rhi().resize_surface(&mut self.surface, width, height);
    }

    /// Draws and presents. Returns `false` if the frame was skipped (window
    /// hidden, surface being reconfigured).
    pub fn render(&mut self, list: &RenderList, camera: &Camera) -> bool {
        let rhi = self.renderer.rhi().clone();
        let Acquire::Frame(frame) = rhi.acquire_frame(&mut self.surface) else { return false };
        self.renderer.draw(rhi.frame_view(&frame), self.size, list, camera);
        rhi.present(frame);
        true
    }
}

impl WindowRenderer<Wgpu> {
    /// Creates the GPU state for `window`. `vsync` off asks for a present
    /// mode without waiting for the display (to measure raw speed).
    pub fn new<W: WindowHandle>(window: Arc<W>, size: (u32, u32), vsync: bool) -> Result<Self, String> {
        let (rhi, surface) = Wgpu::for_window(window, size, vsync, WgpuOptions { allow_software_fallback: false, ..Default::default() })?;
        Ok(Self::from_parts(rhi, surface, size))
    }
}

/// A window surface and the 3D renderer that draws to it.
pub struct WindowRenderer3D<B: Rhi> {
    pub renderer: Renderer3D<B>,
    /// Draws the 2D overlay of [`WindowRenderer3D::render_with_overlay`] over the 3D picture.
    overlay: Renderer<B>,
    surface: B::Surface,
    size: (u32, u32),
}

impl<B: Rhi> WindowRenderer3D<B> {
    pub fn from_parts(rhi: B, surface: B::Surface, size: (u32, u32), settings: Settings3D) -> Self {
        let format = rhi.surface_format(&surface);
        let mut overlay = Renderer::new(rhi.clone(), format);
        overlay.clear = None;
        Self {
            overlay,
            renderer: Renderer3D::with_settings(rhi, format, settings),
            surface,
            size: (size.0.max(1), size.1.max(1)),
        }
    }

    pub fn adapter_name(&self) -> String {
        self.renderer.rhi().adapter_name()
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.size = (width, height);
        self.renderer.rhi().resize_surface(&mut self.surface, width, height);
    }

    /// Draws and presents. Returns `false` if the frame was skipped.
    pub fn render(&mut self, list: &RenderList3D, camera: &Camera3D) -> bool {
        self.render_with_overlay(list, camera, &RenderList::new())
    }

    /// Draws the 3D `list`, then the 2D `overlay` in pixels (see [`crate::text`]), and presents.
    pub fn render_with_overlay(&mut self, list: &RenderList3D, camera: &Camera3D, overlay: &RenderList) -> bool {
        self.render_capture(list, camera, overlay, false).0
    }

    /// Like [`render_with_overlay`](Self::render_with_overlay), and with `capture` also
    /// reads the presented frame back: `(presented, Some((width, height, RGBA8 bytes)))`
    /// of the real framebuffer (stored encoding, so sRGB surfaces give encoded values).
    /// The pixels are `None` when the frame was skipped or the surface cannot be
    /// copied from.
    pub fn render_capture(
        &mut self,
        list: &RenderList3D,
        camera: &Camera3D,
        overlay: &RenderList,
        capture: bool,
    ) -> (bool, Option<(u32, u32, Vec<u8>)>) {
        let rhi = self.renderer.rhi().clone();
        let Acquire::Frame(frame) = rhi.acquire_frame(&mut self.surface) else { return (false, None) };
        let view = rhi.frame_view(&frame);
        self.renderer.draw(view, self.size, list, camera);
        if !overlay.is_empty() {
            self.overlay.draw(view, self.size, overlay, &crate::text::pixel_camera(self.size));
        }
        let shot = if capture {
            rhi.read_frame(&frame).map(|mut bytes| {
                if rhi.surface_format(&self.surface).is_bgra() {
                    for px in bytes.chunks_exact_mut(4) {
                        px.swap(0, 2);
                    }
                }
                (self.size.0, self.size.1, bytes)
            })
        } else {
            None
        };
        rhi.present(frame);
        (true, shot)
    }
}

impl WindowRenderer3D<Wgpu> {
    /// Creates the GPU state for `window`.
    pub fn new<W: WindowHandle>(window: Arc<W>, size: (u32, u32), vsync: bool, settings: Settings3D) -> Result<Self, String> {
        let (rhi, surface) =
            Wgpu::for_window(window, size, vsync, WgpuOptions { allow_software_fallback: false, ..Default::default() })?;
        Ok(Self::from_parts(rhi, surface, size, settings))
    }
}
