//! The JavaScript-facing side (wasm32 only): `GpuView`.

use orr_render::{
    Camera, Camera3D, Lighting, Material, RenderList, RenderList3D, Settings3D, SphereLod3D, WindowRenderer,
    WindowRenderer3D, IDENTITY_ROT,
};
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

/// An isolated 3D sphere fixture for browser correctness checks.
#[wasm_bindgen]
pub struct SphereLodFixture {
    win: WindowRenderer3D<Wgpu>,
    backend: String,
    adapter: String,
    scene: String,
    lod_enabled: bool,
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

/// Creates a 3D fixture surface with an explicit backend. This intentionally has no `auto` mode:
/// the returned backend must match the requested browser API.
#[wasm_bindgen]
pub async fn create_sphere_lod_fixture(
    canvas: HtmlCanvasElement,
    backend: String,
    preset: String,
    scene: String,
    lod_enabled: bool,
) -> Result<SphereLodFixture, JsValue> {
    let which = match backend.as_str() {
        "webgpu" => WebBackend::WebGpu,
        "webgl" => WebBackend::WebGl,
        _ => return Err(JsValue::from_str("sphere fixture backend must be webgpu or webgl")),
    };
    if !matches!(scene.as_str(), "near" | "far" | "mixed") {
        return Err(JsValue::from_str("sphere fixture scene must be near, far, or mixed"));
    }
    let settings = match preset.as_str() {
        "default" => Settings3D::default(),
        "low" => Settings3D::LOW,
        _ => return Err(JsValue::from_str("sphere fixture preset must be default or low")),
    };
    let size = (canvas.width().max(1), canvas.height().max(1));
    let (rhi, surface) = Wgpu::for_canvas(canvas, size, which).await.map_err(|e| JsValue::from_str(&e))?;
    let actual_backend = rhi.backend_name();
    let expected_backend = if backend == "webgpu" { "BrowserWebGpu" } else { "Gl" };
    if actual_backend != expected_backend {
        return Err(JsValue::from_str(&format!("requested {expected_backend}, got {actual_backend}")));
    }
    let adapter = rhi.adapter_name();
    let mut win = if lod_enabled {
        let policy = if preset == "low" { SphereLod3D::LOW } else { SphereLod3D::default() };
        WindowRenderer3D::from_parts_with_sphere_lod(rhi, surface, size, settings, policy)
            .map_err(|e| JsValue::from_str(&e.to_string()))?
    } else {
        WindowRenderer3D::from_parts(rhi, surface, size, settings)
    };
    win.renderer.clear = [0.0, 0.0, 0.0, 1.0];
    Ok(SphereLodFixture { win, backend: actual_backend, adapter, scene, lod_enabled })
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

#[wasm_bindgen]
impl SphereLodFixture {
    pub fn backend(&self) -> String {
        self.backend.clone()
    }

    pub fn adapter(&self) -> String {
        self.adapter.clone()
    }

    pub fn format(&self) -> String {
        format!("{:?}", self.win.renderer.format())
    }

    pub fn width(&self) -> u32 {
        let size = self.win.size();
        size.0
    }

    pub fn height(&self) -> u32 {
        let size = self.win.size();
        size.1
    }

    pub fn msaa_samples(&self) -> u32 {
        self.win.renderer.last_frame_stats().msaa_samples
    }

    pub fn near_spheres(&self) -> u32 {
        self.win.renderer.last_sphere_lod_stats().near_instances
    }

    pub fn far_spheres(&self) -> u32 {
        self.win.renderer.last_sphere_lod_stats().far_instances
    }

    pub fn draw_fixture(&mut self) -> bool {
        let mut list = RenderList3D::new();
        list.lighting = Lighting {
            intensity: 0.0,
            ambient: 0.0,
            shadows: false,
            tonemap: false,
            ..Lighting::default()
        };
        let distance = if self.scene == "far" { 10.0 } else { 4.0 };
        let near_material = Material::new([0.8, 0.24, 0.08]).glow(1.0);
        let far_material = Material::new([0.08, 0.3, 0.9]).glow(1.0);
        match self.scene.as_str() {
            "near" => list.sphere([0.0, 0.0, 0.0], IDENTITY_ROT, 0.8, &near_material),
            "far" => list.sphere([0.0, 0.0, 0.0], IDENTITY_ROT, 0.13, &near_material),
            // Deliberately submit far before near. LOD stable-partitioning moves the far sphere
            // to instance index 1, exercising the WebGL first-instance emulation path.
            "mixed" => {
                list.sphere([1.6, 0.0, -6.0], IDENTITY_ROT, 0.13, &far_material);
                list.sphere([-0.25, 0.0, 0.0], IDENTITY_ROT, 0.45, &near_material);
            }
            _ => return false,
        }
        let camera = Camera3D::perspective([0.0, 0.0, distance], [0.0, 0.0, 0.0], 55.0);
        let presented = self.win.render(&list, &camera);
        if presented {
            let stats = self.win.renderer.last_sphere_lod_stats();
            if self.lod_enabled && self.scene == "far" && stats.far_instances != 1 {
                return false;
            }
            if self.lod_enabled && self.scene == "near" && (stats.near_instances != 1 || stats.far_instances != 0) {
                return false;
            }
            if self.lod_enabled && self.scene == "mixed" && (stats.near_instances != 1 || stats.far_instances != 1) {
                return false;
            }
        }
        presented
    }
}
