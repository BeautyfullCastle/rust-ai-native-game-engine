//! Read-only package asset viewer. It never borrows the editor mutably and uses
//! its own camera, texture, and one-model GPU cache.
#![allow(clippy::float_arithmetic)]

use super::EditorApp;
use crate::{
    package_model_preview::{bounded_size, PreviewCamera, PreviewRequest, State},
    viewport3d::{GpuViewport3d, ModelPlacement},
};

#[derive(Default)]
pub(super) struct PreviewWindow {
    state: State,
    camera: PreviewCamera,
    gpu: Option<GpuViewport3d>,
    texture: Option<egui::TextureId>,
    rendered_size: (u32, u32),
    render_error: Option<String>,
}

impl PreviewWindow {
    fn release_gpu(&mut self) {
        // GpuViewport3d::drop unregisters the egui texture as well as dropping
        // its model cache and offscreen target.
        self.texture = None;
        self.gpu = None;
        self.render_error = None;
    }

    fn close(&mut self) {
        self.state.close();
        self.release_gpu();
        self.camera.reset();
    }

    fn request(&mut self, request: PreviewRequest) {
        self.release_gpu();
        self.camera.reset();
        self.state.request(request);
    }

    fn show(&mut self, ctx: &egui::Context, render_state: Option<&egui_wgpu::RenderState>) {
        let Some(request) = self.state.active_request().cloned() else {
            self.release_gpu();
            return;
        };
        if self.state.ready().is_none() {
            self.release_gpu();
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Static package model preview")
            .open(&mut open)
            .default_pos(egui::pos2(1000.0, 70.0))
            .resizable(false)
            .fade_out(false)
            .show(ctx, |ui| {
                ui.label(format!("{}/{}", request.package_id, request.asset_path));
                ui.label("Read-only asset view. No entity selection or assignment is needed.");
                ui.label(&self.state.status);
                if self.state.busy() {
                    ui.spinner();
                    ctx.request_repaint_after(std::time::Duration::from_millis(40));
                }
                let mut changed = false;
                ui.horizontal(|ui| {
                    if ui.button("Reset preview view").clicked() {
                        changed |= self.camera.reset();
                    }
                    close |= ui.button("Close static preview").clicked();
                });
                if close {
                    return;
                }
                let Some(ready) = self.state.ready() else {
                    return;
                };
                let Some(render_state) = render_state else {
                    ui.label("A GPU is required to draw the static preview.");
                    return;
                };
                let Some(model) = ready.loaded.static_model() else {
                    return;
                };
                let bounds = ready.bounds;
                let model = model.clone();
                let display_size = egui::vec2(448.0, 336.0);
                let scale = ctx.pixels_per_point();
                let size = bounded_size((
                    (display_size.x * scale) as u32,
                    (display_size.y * scale) as u32,
                ));
                if changed || self.texture.is_none() || size != self.rendered_size {
                    let gpu = self
                        .gpu
                        .get_or_insert_with(|| GpuViewport3d::new(render_state, size));
                    let placement = ModelPlacement {
                        // Sentinel only: this empty preview scene has no host entity.
                        entity: orr_ecs::Entity::NONE,
                        model,
                        instance: orr_render::StaticInstance::default(),
                    };
                    match gpu.render(
                        size,
                        &orr_render::RenderList3D::default(),
                        &self.camera.camera(bounds, size),
                        &[placement],
                    ) {
                        Ok(texture) => {
                            self.texture = Some(texture);
                            self.rendered_size = size;
                            self.render_error = None;
                        }
                        Err(error) => {
                            self.texture = None;
                            self.render_error = Some(error);
                        }
                    }
                }
                if let Some(error) = &self.render_error {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("Static preview failed: {error}"),
                    );
                }
                if let Some(texture) = self.texture {
                    let response = ui.add(
                        egui::Image::new((texture, display_size))
                            .sense(egui::Sense::drag())
                            .alt_text("Static package model preview canvas"),
                    );
                    let mut moved = false;
                    if response.dragged_by(egui::PointerButton::Primary) {
                        let delta = ctx.input(|input| input.pointer.delta());
                        moved |= self.camera.orbit([delta.x, delta.y]);
                    }
                    if response.hovered() {
                        let scroll = ctx.input(|input| input.smooth_scroll_delta.y);
                        if scroll != 0.0 {
                            moved |= self.camera.zoom(scroll / 50.0);
                        }
                    }
                    if moved {
                        self.texture = None;
                        ctx.request_repaint();
                    }
                    ui.label("Drag to orbit · Scroll to zoom · Reset restores the asset view");
                }
            });
        if close {
            self.close();
        } else if !open {
            // The title-bar X is processed before our contents. This frame may
            // already reference the texture, so unregister it on the next UI
            // frame rather than invalidating an in-flight egui paint primitive.
            self.state.close();
            self.camera.reset();
            ctx.request_repaint();
        }
    }
}

impl EditorApp {
    pub(super) fn sync_package_model_preview(&mut self) {
        let root = self.package_manager_root();
        self.package_model_preview.state.poll(root.as_deref());
        if self
            .package_model_preview
            .state
            .active_request()
            .is_some_and(|request| !self.package_manager.static_preview_is_current(request))
        {
            self.package_model_preview.close();
        }
        if self.package_model_preview.state.ready().is_none() {
            self.package_model_preview.release_gpu();
        }
    }

    pub(super) fn show_package_model_preview(&mut self, ctx: &egui::Context) {
        if let Some(request) = self.package_manager.take_static_preview_request() {
            if self.package_manager_blocker().is_none() {
                self.package_model_preview.request(request);
            }
        }
        self.sync_package_model_preview();
        self.package_model_preview
            .show(ctx, self.render_state.as_ref());
    }
}
