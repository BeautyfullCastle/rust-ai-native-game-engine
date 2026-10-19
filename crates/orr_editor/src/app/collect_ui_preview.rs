//! Isolated authored Collect screen preview. It shares the editor GPU device,
//! but owns its egui context, renderer, texture and pointer gesture.
use super::collect_ui_alignment::{AlignmentControls, Command as AlignmentCommand};
use super::collect_ui_position::{Command, PositionControls};
use super::collect_ui_resize::{Command as ResizeCommand, ResizeControls};
use super::EditorApp;
use crate::collect_ui_position::Context;
use crate::collect_ui_preview::{Frame, State, MAX_SCORE};
use egui_wgpu::wgpu;
use orr_sample::{authored_ui::Screen, collect_project::PreparedCollectUi};
use std::path::{Path, PathBuf};

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
    position: PositionControls,
    resize: ResizeControls,
    alignment: AlignmentControls,
}
impl PreviewWindow {
    fn release_gpu(&mut self) {
        self.gpu = None;
        self.canvas = None;
        self.pointer_down = false;
    }
    fn close(&mut self) {
        self.position
            .cancel(&mut self.state, "Position edit cancelled");
        self.resize.cancel(&mut self.state, "Size edit cancelled");
        self.alignment
            .cancel(&mut self.state, "Alignment cancelled");
        self.state.close();
        self.release_gpu();
    }
    fn controls(
        &mut self,
        ui: &mut egui::Ui,
        document: &orr_sample::authored_ui::Document,
        editable: bool,
    ) -> (bool, Command, ResizeCommand, AlignmentCommand) {
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
            self.position.cancel(
                &mut self.state,
                "Preview settings changed; position edit cancelled",
            );
            self.resize.cancel(
                &mut self.state,
                "Preview settings changed; size edit cancelled",
            );
            self.alignment.cancel(
                &mut self.state,
                "Preview settings changed; alignment cancelled",
            );
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
        // Reserve even an empty error row so a drag never changes canvas origin.
        ui.add_sized(
            egui::vec2(660.0, 20.0),
            egui::Label::new(
                egui::RichText::new(self.error.as_deref().or(self.state.error()).unwrap_or(" "))
                    .color(egui::Color32::YELLOW),
            )
            .truncate(),
        );
        let was_moving = self.position.moving;
        let was_resizing = self.resize.resizing;
        let was_aligning = self.alignment.aligning;
        let command = self
            .position
            .show(ui, document, self.state.settings().screen, editable);
        let resize_command = self
            .resize
            .show(ui, document, self.state.settings().screen, editable);
        let alignment_command =
            self.alignment
                .show(ui, document, self.state.settings().screen, editable);
        if self.position.moving && !was_moving {
            self.alignment.aligning = false;
            self.alignment
                .cancel(&mut self.state, "Switched to Move mode");
            self.resize.resizing = false;
            self.resize.cancel(&mut self.state, "Switched to Move mode");
        } else if self.resize.resizing && !was_resizing {
            self.alignment.aligning = false;
            self.alignment
                .cancel(&mut self.state, "Switched to Resize mode");
            self.position.moving = false;
            self.position
                .cancel(&mut self.state, "Switched to Resize mode");
        }
        if self.alignment.aligning && !was_aligning {
            self.position.moving = false;
            self.resize.resizing = false;
            self.position
                .cancel(&mut self.state, "Switched to Align mode");
            self.resize
                .cancel(&mut self.state, "Switched to Align mode");
        }
        (
            ui.button("Close screen preview").clicked(),
            command,
            resize_command,
            alignment_command,
        )
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
        if self.position.moving || self.resize.resizing || self.alignment.aligning {
            // Moving never forwards authored button presses or releases.
            self.pointer_down = false;
            self.canvas = Some(rect);
            raw.events.push(egui::Event::PointerGone);
            return raw;
        }
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
    fn position_context(&self, scene: &Path, epoch: u64, canvas: egui::Rect) -> Context {
        Context {
            scene: scene.to_owned(),
            epoch,
            screen: self.state.settings().screen,
            viewport: self.state.settings().viewport,
            canvas: [canvas.width(), canvas.height()],
        }
    }
    fn resize_context(
        &self,
        scene: &Path,
        epoch: u64,
        canvas: egui::Rect,
    ) -> crate::collect_ui_resize::Context {
        crate::collect_ui_resize::Context {
            scene: scene.to_owned(),
            epoch,
            screen: self.state.settings().screen,
            viewport: self.state.settings().viewport,
            canvas: [canvas.width(), canvas.height()],
        }
    }
    fn alignment_context(
        &self,
        scene: &Path,
        epoch: u64,
        canvas: egui::Rect,
    ) -> crate::collect_ui_alignment::Context {
        crate::collect_ui_alignment::Context {
            scene: scene.to_owned(),
            epoch,
            screen: self.state.settings().screen,
            viewport: self.state.settings().viewport,
            canvas: [canvas.width(), canvas.height()],
        }
    }
    fn show(
        &mut self,
        ctx: &egui::Context,
        host: Option<&egui_wgpu::RenderState>,
        panel: Option<&mut crate::collect_ui_panel::Panel>,
        scene: Option<PathBuf>,
        epoch: u64,
        editable: bool,
    ) {
        if !self.state.is_open() {
            self.release_gpu();
            return;
        }
        let (Some(panel), Some(scene)) = (panel, scene) else {
            self.close();
            return;
        };
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
                let command;
                let resize_command;
                let alignment_command;
                (close, command, resize_command, alignment_command) = self.controls(ui, panel.document(), editable);
                if close {
                    return;
                }
                let Some(host) = host else {
                    ui.label("A GPU is required to draw the authored-screen preview.");
                    return;
                };
                let size = self.state.settings().viewport;
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(size[0] as f32, size[1] as f32) * self.position.scale,
                    egui::Sense::click_and_drag(),
                );
                let context = self.position_context(&scene, epoch, rect);
                self.position
                    .validate(&context, panel.document(), editable, &mut self.state);
                let resize_context = self.resize_context(&scene, epoch, rect);
                self.resize.validate(&resize_context, panel.document(), editable, &mut self.state);
                let alignment_context = self.alignment_context(&scene, epoch, rect);
                self.alignment.validate(&alignment_context, panel.document(), editable, &mut self.state);
                let interrupted = self.canvas != Some(rect)
                    || ctx.input(|input| {
                        !input.focused || input.key_pressed(egui::Key::Escape)
                            // A focus-loss event cancels even when an integration
                            // has not yet updated its RawInput.focused snapshot.
                            || input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false)))
                    });
                if interrupted {
                    self.position.cancel(
                        &mut self.state,
                        "Position edit cancelled after layout or focus change",
                    );
                    self.resize.cancel(&mut self.state, "Size edit cancelled after layout or focus change");
                    self.alignment.cancel(&mut self.state, "Alignment cancelled after layout or focus change");
                } else {
                    match alignment_command {
                        AlignmentCommand::Stage(mode) if editable && self.alignment.aligning => self.alignment.stage(mode, &alignment_context, panel.document(), &mut self.state),
                        AlignmentCommand::Apply if editable => self.alignment.apply(&alignment_context, panel, &mut self.state),
                        AlignmentCommand::Cancel => self.alignment.cancel(&mut self.state, "Alignment cancelled; accepted document restored"),
                        _ => {}
                    }
                    match resize_command {
                        ResizeCommand::Apply if editable => self.resize.apply(&resize_context, panel, &mut self.state),
                        ResizeCommand::Cancel => self.resize.cancel(&mut self.state, "Size edit cancelled; accepted document restored"),
                        _ => {}
                    }
                    match command {
                        Command::Apply if editable => {
                            self.position.apply(&context, panel, &mut self.state)
                        }
                        Command::Cancel if self.position.moving || self.position.state.is_dragging() || self.position.state.is_ready() => self.position.cancel(
                            &mut self.state,
                            "Position edit cancelled; accepted document restored",
                        ),
                        _ => {}
                    }
                    if editable {
                        self.resize.input(ctx, rect, response.layer_id, &resize_context, panel.document(), &mut self.state);
                        self.position.input(
                            ctx,
                            rect,
                            response.layer_id,
                            &context,
                            panel.document(),
                            &mut self.state,
                        );
                    }
                }
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
                        self.position.paint_ghost(ui, rect, panel.document(), size);
                        self.resize.paint_ghost(ui, rect, panel.document(), size);
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
            self.position
                .cancel(&mut self.state, "Position edit cancelled");
            self.resize.cancel(&mut self.state, "Size edit cancelled");
            self.alignment
                .cancel(&mut self.state, "Alignment cancelled");
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
        if let (Some(scene), Some(panel), Some(canvas)) = (
            scene.as_deref(),
            self.collect_ui.as_ref(),
            self.collect_ui_preview.canvas,
        ) {
            let context = self.collect_ui_preview.position_context(
                scene,
                self.editor.scene_load_epoch(),
                canvas,
            );
            let resize_context = self.collect_ui_preview.resize_context(
                scene,
                self.editor.scene_load_epoch(),
                canvas,
            );
            let alignment_context = self.collect_ui_preview.alignment_context(
                scene,
                self.editor.scene_load_epoch(),
                canvas,
            );
            let preview = &mut self.collect_ui_preview;
            preview.alignment.validate(
                &alignment_context,
                panel.document(),
                !self.editor.is_playing_mode(),
                &mut preview.state,
            );
            preview.resize.validate(
                &resize_context,
                panel.document(),
                !self.editor.is_playing_mode(),
                &mut preview.state,
            );
            preview.position.validate(
                &context,
                panel.document(),
                !self.editor.is_playing_mode(),
                &mut preview.state,
            );
        } else {
            let preview = &mut self.collect_ui_preview;
            preview
                .alignment
                .cancel(&mut preview.state, "Alignment unavailable");
            preview
                .resize
                .cancel(&mut preview.state, "Size edit unavailable");
            preview
                .position
                .cancel(&mut preview.state, "Position edit unavailable");
        }
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
        self.collect_ui_preview.show(
            ctx,
            self.render_state.as_ref(),
            self.collect_ui.as_mut(),
            self.editor.path(),
            self.editor.scene_load_epoch(),
            !self.editor.is_playing_mode(),
        );
    }
}

#[cfg(test)]
#[path = "collect_ui_preview_tests.rs"]
mod tests;
