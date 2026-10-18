//! Isolated authored Collect screen preview. It shares the editor GPU device,
//! but owns its egui context, renderer, texture and pointer gesture.
use super::EditorApp;
use crate::collect_ui_preview::{Frame, State, MAX_SCORE};
use egui_wgpu::wgpu;
use orr_sample::{authored_ui::Screen, collect_project::PreparedCollectUi};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const LANDSCAPE: [u32; 2] = [640, 360];
const PORTRAIT: [u32; 2] = [360, 640];
const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.035,
    g: 0.045,
    b: 0.065,
    a: 1.0,
};

struct PreviewGpu {
    host: egui_wgpu::RenderState,
    renderer: egui_wgpu::Renderer,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    id: egui::TextureId,
    size: [u32; 2],
    generation: u64,
}
impl PreviewGpu {
    fn new(host: &egui_wgpu::RenderState, size: [u32; 2], generation: u64) -> Self {
        let texture = host.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Collect authored-screen preview"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let id = host.renderer.write().register_native_texture(
            &host.device,
            &view,
            wgpu::FilterMode::Nearest,
        );
        let renderer = egui_wgpu::Renderer::new(
            &host.device,
            FORMAT,
            egui_wgpu::RendererOptions::PREDICTABLE,
        );
        Self {
            host: host.clone(),
            renderer,
            texture,
            view,
            id,
            size,
            generation,
        }
    }
    fn resize(&mut self, size: [u32; 2]) {
        if self.size == size {
            return;
        }
        let texture = self.host.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Collect authored-screen preview"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.host
            .renderer
            .write()
            .update_egui_texture_from_wgpu_texture(
                &self.host.device,
                &view,
                wgpu::FilterMode::Nearest,
                self.id,
            );
        self.texture.destroy();
        self.texture = texture;
        self.view = view;
        self.size = size;
        // Keep the renderer's already-uploaded font atlas on viewport resize.
    }
    fn render(&mut self, mut frame: Frame) {
        // Drain only as each upload is handled; texture IDs are independent.
        #[expect(clippy::iter_over_hash_type)]
        for (id, deltas) in frame.output.textures_delta.set.drain() {
            for delta in deltas {
                self.renderer
                    .update_texture(&self.host.device, &self.host.queue, id, &delta);
            }
        }
        let jobs = frame
            .context
            .tessellate(frame.output.shapes, frame.output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: self.size,
            pixels_per_point: frame.output.pixels_per_point,
        };
        let mut encoder =
            self.host
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Collect preview encoder"),
                });
        let callbacks = self.renderer.update_buffers(
            &self.host.device,
            &self.host.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Collect preview pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(CLEAR),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &jobs, &screen);
        }
        self.host.queue.submit(
            callbacks
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );
        #[expect(clippy::iter_over_hash_type)]
        for id in frame.output.textures_delta.free.drain() {
            self.renderer.free_texture(&id);
        }
    }
}
impl Drop for PreviewGpu {
    fn drop(&mut self) {
        self.host.renderer.write().free_texture(&self.id);
        self.texture.destroy();
    }
}

#[derive(Default)]
pub(super) struct PreviewWindow {
    state: State,
    gpu: Option<PreviewGpu>,
    error: Option<String>,
    canvas: Option<egui::Rect>,
    pointer_down: bool,
}
impl PreviewWindow {
    fn release_gpu(&mut self) {
        self.gpu = None;
        self.canvas = None;
        self.pointer_down = false;
    }
    fn close(&mut self) {
        self.state.close();
        self.release_gpu();
    }
    fn controls(&mut self, ui: &mut egui::Ui) -> bool {
        let mut settings = self.state.settings().clone();
        ui.horizontal_wrapped(|ui| {
            ui.label("Screen");
            for screen in [
                Screen::Title,
                Screen::Playing,
                Screen::Menu,
                Screen::Terminal,
            ] {
                ui.selectable_value(&mut settings.screen, screen, format!("{screen:?}"));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Viewport");
            ui.selectable_value(&mut settings.viewport, LANDSCAPE, "Landscape 640 × 360");
            ui.selectable_value(&mut settings.viewport, PORTRAIT, "Portrait 360 × 640");
        });
        ui.horizontal_wrapped(|ui| {
            let score = ui.label("Sample score");
            ui.add(egui::DragValue::new(&mut settings.score).range(0..=MAX_SCORE))
                .labelled_by(score.id);
            egui::ComboBox::from_label("Sample result")
                .selected_text(match settings.phase {
                    2 => "Lost",
                    3 => "Timed out",
                    _ => "Won",
                })
                .show_ui(ui, |ui| {
                    for (phase, label) in [(1, "Won"), (2, "Lost"), (3, "Timed out")] {
                        ui.selectable_value(&mut settings.phase, phase, label);
                    }
                });
            let best = ui.label("Sample best");
            ui.add(
                egui::TextEdit::singleline(&mut settings.best)
                    .desired_width(80.0)
                    .char_limit(64),
            )
            .labelled_by(best.id);
        });
        if settings != *self.state.settings() {
            self.error = self.state.set_settings(settings).err();
            self.pointer_down = false;
        }
        ui.label("Accepted edits appear immediately. Preview buttons only report their action.");
        ui.label(format!(
            "Observed action: {}",
            self.state
                .last_action()
                .map_or_else(|| "none".into(), |action| format!("{action:?}"))
        ));
        if let Some(error) = self.error.as_deref().or(self.state.error()) {
            ui.colored_label(egui::Color32::YELLOW, error);
        }
        ui.button("Close screen preview").clicked()
    }
    fn input(
        &mut self,
        ctx: &egui::Context,
        rect: egui::Rect,
        layer: egui::LayerId,
    ) -> egui::RawInput {
        let mut raw = egui::RawInput {
            time: Some(ctx.input(|input| input.time)),
            focused: ctx.input(|input| input.focused),
            ..Default::default()
        };
        let cancelled = !raw.focused
            || ctx.input(|input| input.key_pressed(egui::Key::Escape))
            || self.canvas != Some(rect);
        if cancelled {
            self.state.cancel_pointer();
            self.pointer_down = false;
            raw.events.push(egui::Event::PointerGone);
        } else {
            let viewport = self.state.settings().viewport;
            let local = |point: egui::Pos2| {
                egui::pos2(
                    (point.x - rect.min.x) * viewport[0] as f32 / rect.width(),
                    (point.y - rect.min.y) * viewport[1] as f32 / rect.height(),
                )
            };
            for event in ctx.input(|input| input.events.clone()) {
                match event {
                    egui::Event::PointerMoved(point)
                        if rect.contains(point) && ctx.layer_id_at(point) == Some(layer) =>
                    {
                        raw.events.push(egui::Event::PointerMoved(local(point)))
                    }
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    } if rect.contains(pos) && ctx.layer_id_at(pos) == Some(layer) => {
                        if pressed || self.pointer_down {
                            raw.events.push(egui::Event::PointerButton {
                                pos: local(pos),
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers,
                            });
                        }
                        self.pointer_down = pressed;
                    }
                    egui::Event::PointerMoved(_)
                    | egui::Event::PointerButton { .. }
                    | egui::Event::PointerGone
                    | egui::Event::WindowFocused(false) => {
                        self.pointer_down = false;
                        // A release outside the canvas, or leaving it and returning,
                        // cannot resurrect a press from the preview.
                        raw.events.push(egui::Event::PointerGone);
                    }
                    _ => {}
                }
            }
        }
        self.canvas = Some(rect);
        raw
    }
    fn show(&mut self, ctx: &egui::Context, host: Option<&egui_wgpu::RenderState>) {
        if !self.state.is_open() {
            self.release_gpu();
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Collect authored-screen preview")
            .id(egui::Id::new("collect-authored-screen-preview"))
            .open(&mut open)
            .default_pos(egui::pos2(250.0, 70.0))
            .default_width(660.0)
            .resizable(false)
            .fade_out(false)
            .show(ctx, |ui| {
                ui.set_min_width(660.0);
                close = self.controls(ui);
                if close {
                    return;
                }
                let Some(host) = host else {
                    ui.label("A GPU is required to draw the authored-screen preview.");
                    return;
                };
                let size = self.state.settings().viewport;
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(size[0] as f32, size[1] as f32),
                    egui::Sense::click_and_drag(),
                );
                let raw = self.input(ctx, rect, response.layer_id);
                match self.state.render(raw) {
                    Ok(frame) => {
                        if self
                            .gpu
                            .as_ref()
                            .is_none_or(|gpu| gpu.generation != frame.generation)
                        {
                            self.gpu = Some(PreviewGpu::new(host, size, frame.generation));
                        }
                        let gpu = self.gpu.as_mut().expect("created preview GPU");
                        gpu.resize(size);
                        gpu.render(frame);
                        ui.painter().image(
                            gpu.id,
                            rect,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                        response.on_hover_text("Collect screen preview canvas");
                    }
                    Err(error) => {
                        self.error = Some(error);
                        self.release_gpu();
                    }
                }
            });
        if close {
            self.close();
        } else if !open {
            // The title-bar X may close after the texture was used this frame.
            // Release it at the next frame, before paint commands reference it.
            self.state.close();
            ctx.request_repaint();
        }
    }
}
impl EditorApp {
    pub(crate) fn install_collect_ui_preview(&mut self, prepared: &PreparedCollectUi) {
        self.collect_ui_preview.close();
        self.collect_ui_preview.error = match self.editor.path() {
            Some(scene) => self
                .collect_ui_preview
                .state
                .admit(prepared, &scene, self.editor.scene_load_epoch())
                .err(),
            None => Some("Open a saved Collect project to preview its authored screens".into()),
        };
    }
    pub(super) fn sync_collect_ui_preview(&mut self) {
        let scene = self.editor.path();
        let result = self.collect_ui_preview.state.synchronize(
            scene.as_deref(),
            self.editor.scene_load_epoch(),
            self.collect_ui.as_ref().map(|panel| panel.document()),
        );
        if let Err(error) = result {
            self.collect_ui_preview.error = Some(error);
        }
        if !self.collect_ui_preview.state.is_open() {
            self.collect_ui_preview.release_gpu();
        }
    }
    pub(super) fn collect_ui_preview_button(&mut self, ui: &mut egui::Ui) {
        self.sync_collect_ui_preview();
        if ui
            .add_enabled(
                self.collect_ui_preview.state.is_available(),
                egui::Button::new("Preview authored screens"),
            )
            .clicked()
        {
            self.collect_ui_preview.error = self.collect_ui_preview.state.open().err();
        }
    }
    pub(super) fn show_collect_ui_preview(&mut self, ctx: &egui::Context) {
        self.sync_collect_ui_preview();
        self.collect_ui_preview
            .show(ctx, self.render_state.as_ref());
    }
}

#[cfg(test)]
#[path = "collect_ui_preview_tests.rs"]
mod tests;
