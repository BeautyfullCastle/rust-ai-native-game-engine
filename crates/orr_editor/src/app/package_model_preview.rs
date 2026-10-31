//! Read-only package asset viewer. It never borrows the editor mutably and uses
//! its own camera, texture, and one-model GPU cache.
#![allow(clippy::float_arithmetic)]

#[cfg(feature = "room-animated-models")]
#[path = "package_model_preview/transport.rs"]
mod transport;

use super::EditorApp;
use crate::{
    package_model_preview::{PreviewCamera, PreviewRequest, State, bounded_size},
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
    #[cfg(feature = "room-animated-models")]
    clip_index: String,
    #[cfg(feature = "room-animated-models")]
    seconds: String,
    #[cfg(feature = "room-animated-models")]
    sample_error: Option<String>,
    #[cfg(feature = "room-animated-models")]
    transport: transport::Transport,
}

impl PreviewWindow {
    fn release_gpu(&mut self) {
        // GpuViewport3d::drop unregisters the egui texture as well as dropping
        // its model cache and offscreen target.
        self.texture = None;
        self.gpu = None;
        self.render_error = None;
    }

    #[cfg(feature = "room-animated-models")]
    fn reset_animation_controls(&mut self) {
        self.clip_index = "0".into();
        self.seconds = "0".into();
        self.sample_error = None;
        self.transport = transport::Transport::default();
    }

    fn close(&mut self) {
        self.state.close();
        #[cfg(feature = "room-animated-models")]
        self.reset_animation_controls();
        self.release_gpu();
        self.camera.reset();
    }

    fn request(&mut self, request: PreviewRequest) {
        #[cfg(feature = "room-animated-models")]
        self.reset_animation_controls();
        self.release_gpu();
        self.camera.reset();
        self.state.request(request);
    }

    #[cfg(feature = "room-animated-models")]
    fn animation_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(ready) = self.state.ready() else {
            self.transport.pause();
            return false;
        };
        let Some(model) = ready.loaded.animated_model() else {
            self.transport.pause();
            return false;
        };
        let clips: Vec<_> = model
            .source()
            .clips
            .iter()
            .enumerate()
            .map(|(index, clip)| (index, clip.name.clone(), clip.duration()))
            .collect();
        let label = self
            .clip_index
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|index| clips.get(index))
            .map_or_else(
                || "Choose a valid clip".to_owned(),
                |(index, name, _)| format!("{index}: {name}"),
            );
        let prior_draft = (self.clip_index.clone(), self.seconds.clone());
        egui::ComboBox::from_label("Preview animation clip")
            .selected_text(label)
            .show_ui(ui, |ui| {
                for (index, name, duration) in &clips {
                    ui.selectable_value(
                        &mut self.clip_index,
                        index.to_string(),
                        format!("{index}: {name} ({duration:.2}s)"),
                    );
                }
            });
        ui.horizontal(|ui| {
            let index_label = ui.label("Preview clip index");
            ui.add(
                egui::TextEdit::singleline(&mut self.clip_index)
                    .desired_width(60.0)
                    .char_limit(10),
            )
            .labelled_by(index_label.id);
            let time_label = ui.label("Preview seconds");
            ui.add(
                egui::TextEdit::singleline(&mut self.seconds)
                    .desired_width(90.0)
                    .char_limit(40),
            )
            .labelled_by(time_label.id);
        });
        if prior_draft != (self.clip_index.clone(), self.seconds.clone()) {
            self.transport.pause();
        }
        let now = ui.input(|input| input.time);
        let frame = ui.ctx().cumulative_frame_nr();
        let mut changed = false;
        let mut manual_action = false;
        if ui.button("Sample preview pose").clicked() {
            manual_action = true;
            self.transport.pause();
            changed |= self.sample_draft(false, &clips, now, frame);
        }
        ui.horizontal(|ui| {
            if ui.button("Play preview once").clicked() {
                manual_action = true;
                self.transport.pause();
                changed |= self.sample_draft(true, &clips, now, frame);
            }
            if ui.button("Pause preview").clicked() {
                manual_action = true;
                self.transport.pause();
            }
            if ui.button("Restart preview").clicked() {
                manual_action = true;
                let running = self.transport.running;
                self.transport.pause();
                // Restart the accepted clip, never silently accept an invalid draft.
                if let Some(accepted) = self
                    .state
                    .ready()
                    .and_then(|ready| ready.animation.as_ref())
                {
                    let index = accepted.clip_index;
                    let duration = clips[index as usize].2;
                    match self.state.sample(index, 0.0) {
                        Ok(()) => {
                            self.clip_index = index.to_string();
                            self.seconds = "0".into();
                            self.sample_error = None;
                            if running {
                                self.transport.start(0.0, duration, now, frame);
                            }
                            changed = true;
                        }
                        Err(error) => self.sample_error = Some(error),
                    }
                }
            }
        });
        if !manual_action {
            if let Some(accepted) = self
                .state
                .ready()
                .and_then(|ready| ready.animation.as_ref())
            {
                let index = accepted.clip_index;
                let duration = clips[index as usize].2;
                if let Some(seconds) =
                    self.transport
                        .advance(now, frame, ui.input(|input| input.focused), duration)
                {
                    match self.state.sample(index, seconds) {
                        Ok(()) => {
                            self.transport.accept(seconds, duration);
                            self.seconds = seconds.to_string();
                            self.sample_error = None;
                            changed = true;
                        }
                        Err(error) => {
                            self.transport.pause();
                            self.sample_error = Some(error);
                        }
                    }
                }
            }
        }
        if self.transport.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 30.0));
        }
        ui.label(if self.transport.running {
            "Preview playback: playing"
        } else {
            "Preview playback: paused"
        });
        if let Some(error) = &self.sample_error {
            ui.colored_label(
                egui::Color32::YELLOW,
                format!("Preview sample refused: {error}"),
            );
        }
        if let Some(accepted) = self
            .state
            .ready()
            .and_then(|ready| ready.animation.as_ref())
        {
            ui.label(format!(
                "Accepted preview pose: clip {} at {:.3}s",
                accepted.clip_index, accepted.seconds
            ));
        }
        ui.label("Preview-only playback. No game time or model bindings are changed.");
        changed
    }

    #[cfg(feature = "room-animated-models")]
    fn sample_draft(
        &mut self,
        play: bool,
        clips: &[(usize, String, f32)],
        now: f64,
        frame: u64,
    ) -> bool {
        let result = (|| {
            let index = self
                .clip_index
                .trim()
                .parse::<u32>()
                .map_err(|_| "Clip index must be a nonnegative integer".to_owned())?;
            let seconds = self
                .seconds
                .trim()
                .parse::<f32>()
                .map_err(|_| "Preview seconds must be a finite number".to_owned())?;
            let duration = clips
                .get(index as usize)
                .ok_or("Preview clip index is out of range")?
                .2;
            if !seconds.is_finite() || seconds < 0.0 || seconds > duration {
                return Err(format!(
                    "sample time must be finite and between 0 and {duration} seconds"
                ));
            }
            let seconds = if play && seconds == duration {
                0.0
            } else {
                seconds
            };
            self.state.sample(index, seconds)?;
            Ok((seconds, duration))
        })();
        match result {
            Ok((seconds, duration)) => {
                self.sample_error = None;
                self.seconds = seconds.to_string();
                if play {
                    self.transport.start(seconds, duration, now, frame);
                }
                true
            }
            Err(error) => {
                self.sample_error = Some(error);
                false
            }
        }
    }

    fn show(&mut self, ctx: &egui::Context, render_state: Option<&egui_wgpu::RenderState>) {
        let Some(request) = self.state.active_request().cloned() else {
            self.release_gpu();
            return;
        };
        if self.state.ready().is_none() {
            #[cfg(feature = "room-animated-models")]
            self.transport.pause();
            self.release_gpu();
        }
        let animated = request.kind == crate::model_bindings::ModelKind::Animated;
        let title = if animated {
            "Animated package model preview"
        } else {
            "Static package model preview"
        };
        let close_label = if animated {
            "Close animated preview"
        } else {
            "Close static preview"
        };
        let canvas_label = if animated {
            "Animated package model preview canvas"
        } else {
            "Static package model preview canvas"
        };
        let mut open = true;
        let mut close = false;
        egui::Window::new(title)
            .id(egui::Id::new("package-model-preview-window"))
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
                    close |= ui.button(close_label).clicked();
                });
                if close {
                    return;
                }
                #[cfg(feature = "room-animated-models")]
                {
                    changed |= self.animation_controls(ui);
                }
                let Some(ready) = self.state.ready() else {
                    return;
                };
                let Some(render_state) = render_state else {
                    ui.label("A GPU is required to draw the model preview.");
                    return;
                };
                let bounds = ready.bounds;
                let static_model = ready.loaded.static_model().cloned();
                #[cfg(feature = "room-animated-models")]
                let animated_model = ready.loaded.animated_model().cloned();
                #[cfg(feature = "room-animated-models")]
                let accepted_pose = ready
                    .animation
                    .as_ref()
                    .map(|accepted| accepted.pose.clone());
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
                    let list = orr_render::RenderList3D::default();
                    let camera = self.camera.camera(bounds, size);
                    let result = if let Some(model) = static_model {
                        let placement = ModelPlacement {
                            // Sentinel only: this empty preview scene has no host entity.
                            entity: orr_ecs::Entity::NONE,
                            model,
                            instance: orr_render::StaticInstance::default(),
                        };
                        gpu.render(size, &list, &camera, &[placement])
                    } else {
                        #[cfg(feature = "room-animated-models")]
                        {
                            match (animated_model, accepted_pose) {
                                (Some(model), Some(pose)) => gpu.render_mixed(
                                    size,
                                    &list,
                                    &camera,
                                    &[],
                                    &[crate::viewport3d::AnimatedPlacement {
                                        entity: orr_ecs::Entity::NONE,
                                        model,
                                        instance: orr_render::math3::Mat4::IDENTITY.0,
                                        pose,
                                    }],
                                ),
                                _ => Err("No accepted animation pose is available".into()),
                            }
                        }
                        #[cfg(not(feature = "room-animated-models"))]
                        {
                            Err("Animated preview requires room-animated-models".into())
                        }
                    };
                    match result {
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
                        format!("Model preview failed: {error}"),
                    );
                }
                if let Some(texture) = self.texture {
                    let response = ui.add(
                        egui::Image::new((texture, display_size))
                            .sense(egui::Sense::drag())
                            .alt_text(canvas_label),
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
            #[cfg(feature = "room-animated-models")]
            self.reset_animation_controls();
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
            .is_some_and(|request| !self.package_manager.preview_is_current(request))
        {
            self.package_model_preview.close();
        }
        if self.package_model_preview.state.ready().is_none() {
            #[cfg(feature = "room-animated-models")]
            self.package_model_preview.transport.pause();
            self.package_model_preview.release_gpu();
        }
    }

    pub(super) fn show_package_model_preview(&mut self, ctx: &egui::Context) {
        if let Some(request) = self.package_manager.take_preview_request() {
            if self.package_manager_blocker().is_none() {
                self.package_model_preview.request(request);
            }
        }
        self.sync_package_model_preview();
        self.package_model_preview
            .show(ctx, self.render_state.as_ref());
    }
}
