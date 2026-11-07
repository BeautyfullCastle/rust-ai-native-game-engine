//! Isolated, ephemeral view of a captured authoring result, never a package claim.
#![allow(clippy::float_arithmetic)]
use crate::{
    model_package_import_panel::PreviewFrame,
    package_model_preview::{bounded_size, PreviewBounds, PreviewCamera},
    viewport3d::GpuViewport3d,
};
use orr_model::animation::Pose;
struct Accepted {
    clip: u32,
    seconds: f32,
    pose: Pose,
    bounds: PreviewBounds,
}
#[derive(Default)]
pub(super) struct PreviewWindow {
    frame: Option<PreviewFrame>,
    accepted: Option<Accepted>,
    camera: PreviewCamera,
    clip: String,
    seconds: String,
    error: Option<String>,
    gpu: Option<GpuViewport3d>,
    texture: Option<egui::TextureId>,
    size: (u32, u32),
}
impl PreviewWindow {
    fn release_gpu(&mut self) {
        self.texture = None;
        self.gpu = None;
        self.size = (0, 0);
    }
    pub(super) fn close(&mut self) {
        self.release_gpu();
        self.frame = None;
        self.accepted = None;
        self.camera.reset();
        self.clip = "0".into();
        self.seconds = "0".into();
        self.error = None;
    }
    fn sync(&mut self, input: Option<PreviewFrame>) -> bool {
        let Some(input) = input else {
            self.close();
            return false;
        };
        if self.frame.as_ref().is_some_and(|old| {
            old.ticket == input.ticket && std::sync::Arc::ptr_eq(&old.prepared, &input.prepared)
        }) {
            return false;
        }
        self.close();
        self.frame = Some(input);
        if let Err(error) = self.sample(0, 0.0) {
            self.error = Some(error);
        }
        true
    }
    fn sample(&mut self, clip: u32, seconds: f32) -> Result<(), String> {
        let model = &self
            .frame
            .as_ref()
            .ok_or("No prepared variant")?
            .prepared
            .model;
        let duration = model
            .source()
            .clips
            .get(clip as usize)
            .ok_or("Choose a valid preview clip")?
            .duration();
        if !seconds.is_finite() || seconds < 0.0 || seconds > duration {
            return Err("Preview seconds must be finite and within the selected clip".into());
        }
        let pose = model
            .sample_clip(clip, seconds)
            .map_err(|e| e.to_string())?;
        let bounds =
            PreviewBounds::from_animation(model.bounds(&pose).map_err(|e| e.to_string())?)?;
        self.accepted = Some(Accepted {
            clip,
            seconds,
            pose,
            bounds,
        });
        self.error = None;
        Ok(())
    }
    fn sample_draft(&mut self) -> bool {
        let parsed = self
            .clip
            .trim()
            .parse::<u32>()
            .map_err(|_| "Choose a valid preview clip".to_string())
            .and_then(|clip| {
                self.seconds
                    .trim()
                    .parse::<f32>()
                    .map(|seconds| (clip, seconds))
                    .map_err(|_| "Enter finite preview seconds".to_string())
            });
        match parsed.and_then(|(clip, seconds)| self.sample(clip, seconds)) {
            Ok(()) => true,
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }
    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        render_state: Option<&egui_wgpu::RenderState>,
        input: Option<PreviewFrame>,
    ) -> bool {
        let mut changed = self.sync(input);
        if self.frame.is_none() {
            return false;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Animated variant preview").id(egui::Id::new("animated-variant-preview-window")).open(&mut open).default_pos(egui::pos2(40.0,50.0)).resizable(false).fade_out(false).show(ctx,|ui| {
            let frame=self.frame.as_ref().unwrap();
            ui.label("Captured authoring result. Not installed, assigned, or published.");
            ui.label(format!("Asset: {}",frame.prepared.model.source().asset_id));
            ui.label(format!("Asset SHA-256: {}",frame.prepared.asset_sha256));ui.label(format!("Source SHA-256: {}",frame.prepared.source_sha256));
            if let Some(hash)=&frame.prepared.png_sha256 {ui.label(format!("PNG SHA-256: {hash}"));}
            ui.horizontal(|ui| {if ui.button("Reset variant view").clicked() {changed|=self.camera.reset();}close|=ui.button("Close variant preview").clicked();});
            ui.horizontal(|ui| {for (index,label) in ["Variant front view","Variant right view","Variant back view","Variant left view"].into_iter().enumerate() {if ui.button(label).clicked() {changed|=self.camera.horizontal_view(index);}}});
            if close {return;}
            let clips:Vec<_>=self.frame.as_ref().unwrap().prepared.model.source().clips.iter().enumerate().map(|(i,c)|(i,c.name.clone(),c.duration())).collect();
            egui::ComboBox::from_label("Variant animation clip").selected_text(self.clip.clone()).show_ui(ui,|ui| {for (i,name,duration) in clips {ui.selectable_value(&mut self.clip,i.to_string(),format!("{i}: {name} ({duration:.3}s)"));}});
            ui.horizontal(|ui| {let label=ui.label("Variant preview clip index");ui.add(egui::TextEdit::singleline(&mut self.clip).desired_width(60.0).char_limit(10)).labelled_by(label.id);let label=ui.label("Variant preview seconds");ui.add(egui::TextEdit::singleline(&mut self.seconds).desired_width(90.0).char_limit(40)).labelled_by(label.id);});
            if ui.button("Sample variant pose").clicked() {changed|=self.sample_draft();}
            if let Some(accepted)=&self.accepted {ui.label(format!("Accepted clip {} at {:.3}s",accepted.clip,accepted.seconds));}
            if let Some(error)=&self.error {ui.colored_label(egui::Color32::YELLOW,error);}
            let Some(state)=render_state else {self.release_gpu();ui.label("A GPU is required to draw the variant preview. Pose controls remain available.");return;};
            let Some(accepted)=&self.accepted else {return;};
            let display=egui::vec2(448.0,336.0);let scale=ctx.pixels_per_point();let size=bounded_size(((display.x*scale) as u32,(display.y*scale) as u32));
            if changed || self.texture.is_none() || size!=self.size {
                let camera=self.camera.camera(accepted.bounds,size);let frame=self.frame.as_ref().unwrap();let gpu=self.gpu.get_or_insert_with(||GpuViewport3d::new(state,size));
                match gpu.render_mixed(size,&orr_render::RenderList3D::default(),&camera,&[],&[crate::viewport3d::AnimatedPlacement{entity:orr_ecs::Entity::NONE,model:frame.prepared.model.clone(),instance:orr_render::math3::Mat4::IDENTITY.0,pose:accepted.pose.clone()}]) {Ok(texture)=>{self.texture=Some(texture);self.size=size;},Err(error)=>{self.texture=None;self.error=Some(error);}}
            }
            if let Some(texture) = self.texture {
                let response = ui.add(
                    egui::Image::new((texture, display))
                        .sense(egui::Sense::drag())
                        .alt_text("Animated variant preview canvas"),
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
            }
        });
        if close || !open {
            self.close();
            true
        } else {
            false
        }
    }
}
#[cfg(test)]
#[path = "variant_model_preview_tests.rs"]
mod tests;
