//! The JavaScript-facing side (wasm32 only): `GpuView`.

use orr_render::{Camera, RenderList, WindowRenderer};
use orr_rhi::{Rhi, WebBackend, Wgpu};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::{arena_list, phys_list};

/// A canvas drawn by `orr_render` on WebGPU or WebGL2.
#[wasm_bindgen]
pub struct GpuView {
    win: WindowRenderer<Wgpu>,
    backend: String,
    adapter: String,
    srgb: bool,
}

/// Creates the view on `canvas`. `backend`: `"auto"` (WebGPU when the browser has it, else WebGL2),
/// `"webgpu"` or `"webgl"`. Rejects when the chosen API is not available.
#[wasm_bindgen]
pub async fn create_gpu_view(canvas: HtmlCanvasElement, backend: String) -> Result<GpuView, JsValue> {
    let size = (canvas.width().max(1), canvas.height().max(1));
    let which = match backend.as_str() {
        "webgpu" => WebBackend::WebGpu,
        "webgl" => WebBackend::WebGl,
        _ => WebBackend::Auto,
    };
    let (rhi, surface) = Wgpu::for_canvas(canvas, size, which).await.map_err(|e| JsValue::from_str(&e))?;
    let (backend, adapter) = (rhi.backend_name(), rhi.adapter_name());
    let win = WindowRenderer::from_parts(rhi, surface, size);
    let srgb = win.renderer.format().is_srgb();
    Ok(GpuView { win, backend, adapter, srgb })
}

#[wasm_bindgen]
impl GpuView {
    /// The graphics API in use (`BrowserWebGpu` or `Gl`).
    pub fn backend(&self) -> String {
        self.backend.clone()
    }

    pub fn adapter(&self) -> String {
        self.adapter.clone()
    }

    /// Reconfigures the surface after the canvas was resized.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.win.resize(width, height);
    }

    fn draw(&mut self, list: &RenderList, camera: &Camera) -> bool {
        self.win.render(list, camera)
    }

    /// Draws a physics frame (`PhysClient.render()` and `.scene_box()`). `false` when skipped.
    pub fn draw_phys(&mut self, data: &[i32], scene_box: &[i32]) -> bool {
        match phys_list(data, scene_box, self.srgb) {
            Some((list, camera)) => self.draw(&list, &camera),
            None => self.draw(&RenderList::new(), &Camera::new([0.0, 0.0], 1.0)),
        }
    }

    /// Draws an arena frame (`WebClient.render()`). `false` when skipped.
    pub fn draw_arena(&mut self, data: &[i32]) -> bool {
        let (list, camera) = arena_list(data, self.srgb);
        self.draw(&list, &camera)
    }
}
