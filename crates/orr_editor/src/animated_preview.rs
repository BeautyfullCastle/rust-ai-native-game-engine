//! Presentation-only animated model preview state and GPU pane.
//!
//! This module does not add entities to the simulation or alter the editor's
//! 2D scene viewport. It keeps one transient [`AnimationPlayer`] per persistent
//! entity GUID and renders the currently selected model to a private offscreen
//! texture that a panel can show with `ui.image((texture_id, size))`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use egui::TextureId;
use orr_model::animation::{AnimatedModel, AnimationPlayer, PlaybackMode, PlaybackState, Pose};
use orr_reflect::Guid;
use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu};
use orr_render::{
    Camera3D, Lighting, OffscreenTarget, SkinnedInstance, SkinnedModelRenderer, SkinnedRenderError,
};

/// Playback speeds match the persisted animated-binding format.
pub const MIN_PLAYBACK_SPEED: f32 = 0.05;
/// Playback speeds match the persisted animated-binding format.
pub const MAX_PLAYBACK_SPEED: f32 = 4.0;
// Rgba8Unorm avoids requesting an sRGB alternate texture view on adapters
// that do not support VIEW_FORMATS (including headless llvmpipe). The skinned
// renderer explicitly encodes linear output to sRGB for UNORM targets before
// the native egui texture samples the bytes.
const TARGET_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
const INITIAL_SIZE: (u32, u32) = (1, 1);
const MAX_TARGET_SIZE: u32 = 8192;

struct BoundPlayer {
    model: Arc<AnimatedModel>,
    clip_index: u32,
    mode: PlaybackMode,
    speed: f32,
    player: AnimationPlayer,
}

/// Independent, transient animation state keyed by scene GUID.
///
/// Bindings are idempotent: the same GUID, exact model `Arc`, clip, mode and
/// speed keep the current playback position. Replacing the `Arc` starts a fresh
/// player, because `AnimationPlayer` deliberately rejects poses from another
/// model identity. None of this state is serialized into the scene or binding
/// document.
#[derive(Default)]
pub struct PreviewPlayers {
    players: BTreeMap<Guid, BoundPlayer>,
}

impl PreviewPlayers {
    /// Assigns or updates a GUID's transient player.
    pub fn bind(
        &mut self,
        guid: &Guid,
        model: Arc<AnimatedModel>,
        clip_index: u32,
        mode: PlaybackMode,
        speed: f32,
    ) -> Result<(), String> {
        if !speed.is_finite() || !(MIN_PLAYBACK_SPEED..=MAX_PLAYBACK_SPEED).contains(&speed) {
            return Err(format!(
                "preview speed must be finite and in {MIN_PLAYBACK_SPEED}..={MAX_PLAYBACK_SPEED}"
            ));
        }
        if model.source().clips.get(clip_index as usize).is_none() {
            return Err("animation clip index is out of range".to_owned());
        }
        if self.players.get(guid).is_some_and(|bound| {
            Arc::ptr_eq(&bound.model, &model)
                && bound.clip_index == clip_index
                && bound.mode == mode
                && bound.speed == speed
        }) {
            return Ok(());
        }

        let mut player = AnimationPlayer::new();
        player
            .play(&model, clip_index, mode)
            .map_err(|error| error.to_string())?;
        // Binding/reload establishes a deterministic time-zero preview. Playback
        // begins only after the user presses Play in the dedicated pane.
        player.pause();
        self.players.insert(
            guid.clone(),
            BoundPlayer {
                model,
                clip_index,
                mode,
                speed,
                player,
            },
        );
        Ok(())
    }

    /// Drops transient state for one entity, for example when a binding is removed.
    pub fn remove(&mut self, guid: &Guid) -> bool {
        self.players.remove(guid).is_some()
    }

    /// Drops players whose GUIDs are no longer present in the current scene.
    pub fn retain(&mut self, guids: &BTreeSet<Guid>) {
        self.players.retain(|guid, _| guids.contains(guid));
    }

    /// Clears all transient playback state, suitable for closing a preview panel.
    pub fn clear(&mut self) {
        self.players.clear();
    }

    /// Number of active per-GUID players.
    pub fn len(&self) -> usize {
        self.players.len()
    }

    /// Whether there are no transient players.
    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }

    /// Whether any entity's transient player is currently advancing.
    pub fn is_playing(&self) -> bool {
        self.players
            .values()
            .any(|bound| bound.player.state() == PlaybackState::Playing)
    }

    /// Current player for a GUID, if it has an active preview binding.
    pub fn player(&self, guid: &Guid) -> Option<&AnimationPlayer> {
        self.players.get(guid).map(|bound| &bound.player)
    }

    /// Current model for a GUID, if it has an active preview binding.
    pub fn model(&self, guid: &Guid) -> Option<&Arc<AnimatedModel>> {
        self.players.get(guid).map(|bound| &bound.model)
    }

    /// Sample a GUID's current pose. A stopped player yields the model rest pose.
    pub fn pose(&self, guid: &Guid) -> Result<Option<Pose>, String> {
        let Some(bound) = self.players.get(guid) else {
            return Ok(None);
        };
        bound
            .player
            .pose(&bound.model)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    /// Starts playback, resumes a paused player, or restarts a stopped/finished one.
    pub fn play(&mut self, guid: &Guid) -> Result<(), String> {
        let bound = self.get_mut(guid)?;
        match bound.player.state() {
            PlaybackState::Playing => Ok(()),
            PlaybackState::Paused => {
                bound.player.resume();
                Ok(())
            }
            PlaybackState::Stopped | PlaybackState::Finished => bound
                .player
                .play(&bound.model, bound.clip_index, bound.mode)
                .map_err(|error| error.to_string()),
        }
    }

    /// Pauses playback without changing the selected clip or time.
    pub fn pause(&mut self, guid: &Guid) -> Result<(), String> {
        self.get_mut(guid)?.player.pause();
        Ok(())
    }

    /// Resumes a paused player. Playing, stopped and finished players are unchanged.
    pub fn resume(&mut self, guid: &Guid) -> Result<(), String> {
        self.get_mut(guid)?.player.resume();
        Ok(())
    }

    /// Stops and returns the preview to the rest pose, retaining its binding config.
    pub fn stop(&mut self, guid: &Guid) -> Result<(), String> {
        self.get_mut(guid)?.player.stop();
        Ok(())
    }

    /// Seeks in seconds. Seeking a stopped player selects its configured clip and
    /// remains paused; seeking never implicitly resumes an existing paused player.
    pub fn seek(&mut self, guid: &Guid, time_seconds: f32) -> Result<(), String> {
        if !time_seconds.is_finite() || time_seconds < 0.0 {
            return Err("invalid animation seek time".to_owned());
        }
        let bound = self.get_mut(guid)?;
        if bound.player.state() == PlaybackState::Stopped {
            bound
                .player
                .play(&bound.model, bound.clip_index, bound.mode)
                .map_err(|error| error.to_string())?;
            bound.player.pause();
        }
        bound
            .player
            .seek(&bound.model, time_seconds)
            .map_err(|error| error.to_string())
    }

    /// Advances all playing players by wall-clock seconds, scaled by each binding's speed.
    pub fn tick(&mut self, delta_seconds: f32) -> Result<(), String> {
        if !delta_seconds.is_finite() || delta_seconds < 0.0 {
            return Err("preview delta must be finite and nonnegative".to_owned());
        }
        if self
            .players
            .values()
            .any(|bound| !(delta_seconds * bound.speed).is_finite())
        {
            return Err("scaled preview delta is not finite".to_owned());
        }
        for bound in self.players.values_mut() {
            let scaled = delta_seconds * bound.speed;
            bound
                .player
                .advance(&bound.model, scaled)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn get_mut(&mut self, guid: &Guid) -> Result<&mut BoundPlayer, String> {
        self.players
            .get_mut(guid)
            .ok_or_else(|| format!("no animated preview is bound to {guid:?}"))
    }
}

/// The GPU-side error surfaced to a panel that renders the preview.
pub type PreviewRenderError = SkinnedRenderError;

/// Headless/offscreen renderer for the dedicated 3D presentation pane.
///
/// This shares its target, camera fitting, asset reload and skinned draw path
/// with [`GpuAnimatedPreview`], and also exposes readback for GPU acceptance tests.
pub struct AnimatedPreviewGpu {
    rhi: Wgpu,
    target: OffscreenTarget<Wgpu>,
    renderer: Option<SkinnedModelRenderer<Wgpu>>,
    model: Option<Arc<AnimatedModel>>,
    center: [f32; 3],
    radius: f32,
}

impl AnimatedPreviewGpu {
    /// Creates a preview target on an existing RHI device.
    pub fn new(rhi: &Wgpu, size: (u32, u32)) -> Self {
        let rhi = rhi.clone();
        let size = bounded_size(size);
        let target = OffscreenTarget::new(&rhi, size.0, size.1, TARGET_FORMAT);
        Self {
            rhi,
            target,
            renderer: None,
            model: None,
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        }
    }

    /// Renders one pose into the offscreen target, resizing when necessary.
    pub fn draw(
        &mut self,
        model: &Arc<AnimatedModel>,
        pose: &Pose,
        size: (u32, u32),
    ) -> Result<(), PreviewRenderError> {
        let size = bounded_size(size);
        self.target.resize(size.0, size.1);
        if self
            .model
            .as_ref()
            .is_none_or(|loaded| !Arc::ptr_eq(loaded, model))
        {
            self.renderer = Some(SkinnedModelRenderer::new(
                self.rhi.clone(),
                TARGET_FORMAT,
                model.as_ref().clone(),
            )?);
            self.model = Some(model.clone());
            (self.center, self.radius) = rest_frame(model);
        }
        let camera = fit_camera(self.center, self.radius, size);
        let lighting = Lighting {
            shadows: false,
            ..Lighting::default()
        };
        self.renderer
            .as_mut()
            .expect("renderer created for current model")
            .draw(
                self.target.render_view(),
                size,
                &camera,
                &lighting,
                &[SkinnedInstance::new(pose)],
            )
    }

    /// Pixel size of the current offscreen target.
    pub fn size(&self) -> (u32, u32) {
        self.target.size()
    }

    /// Generation changes whenever resize replaces the native texture.
    pub fn generation(&self) -> u64 {
        self.target.generation()
    }

    /// Waits for the GPU and returns the rendered image in top-row-first RGBA8.
    pub fn read_rgba8(&self) -> Vec<u8> {
        self.target.read_rgba8()
    }

    /// The adapter name, useful in tests and diagnostics.
    pub fn adapter_name(&self) -> String {
        self.rhi.adapter_name()
    }
}

/// Offscreen skinned preview registered as a native texture on eframe's device.
///
/// One texture ID is retained across resizes by updating its backing wgpu view.
/// Dropping this value frees that native texture from egui's renderer.
pub struct GpuAnimatedPreview {
    gpu: AnimatedPreviewGpu,
    state: egui_wgpu::RenderState,
    id: TextureId,
    generation: u64,
}

impl GpuAnimatedPreview {
    /// Wraps eframe's device and queue, like the existing `GpuViewport`.
    pub fn new(state: &egui_wgpu::RenderState) -> Self {
        let rhi = Wgpu::from_parts(
            state.instance.clone(),
            state.adapter.clone(),
            state.device.clone(),
            state.queue.clone(),
        );
        let gpu = AnimatedPreviewGpu::new(&rhi, INITIAL_SIZE);
        let id = state.renderer.write().register_native_texture(
            &state.device,
            gpu.target.sample_view(),
            egui_wgpu::wgpu::FilterMode::Linear,
        );
        let generation = gpu.generation();
        Self {
            gpu,
            state: state.clone(),
            id,
            generation,
        }
    }

    /// Draws a pose at the requested physical-pixel size and returns its egui texture ID.
    pub fn draw(
        &mut self,
        model: &Arc<AnimatedModel>,
        pose: &Pose,
        size: (u32, u32),
    ) -> Result<TextureId, PreviewRenderError> {
        self.gpu.draw(model, pose, size)?;
        let generation = self.gpu.generation();
        if generation != self.generation {
            self.state
                .renderer
                .write()
                .update_egui_texture_from_wgpu_texture(
                    &self.state.device,
                    self.gpu.target.sample_view(),
                    egui_wgpu::wgpu::FilterMode::Linear,
                    self.id,
                );
            self.generation = generation;
        }
        Ok(self.id)
    }

    /// Pixel size of the current target.
    pub fn size(&self) -> (u32, u32) {
        self.gpu.size()
    }

    /// Native egui texture ID, stable across target resizes.
    pub fn texture_id(&self) -> TextureId {
        self.id
    }
}

impl Drop for GpuAnimatedPreview {
    fn drop(&mut self) {
        self.state.renderer.write().free_texture(&self.id);
    }
}

fn bounded_size(size: (u32, u32)) -> (u32, u32) {
    (
        size.0.clamp(1, MAX_TARGET_SIZE),
        size.1.clamp(1, MAX_TARGET_SIZE),
    )
}

fn rest_frame(model: &AnimatedModel) -> ([f32; 3], f32) {
    let bounds = model
        .rest_pose()
        .ok()
        .and_then(|pose| model.bounds(&pose).ok());
    let Some(bounds) = bounds else {
        return ([0.0, 0.0, 0.0], 1.0);
    };
    let center = std::array::from_fn(|axis| (bounds.min[axis] + bounds.max[axis]) * 0.5);
    let extents = std::array::from_fn::<_, 3, _>(|axis| bounds.max[axis] - bounds.min[axis]);
    let radius = extents.into_iter().fold(0.0_f32, f32::max).max(0.25) * 0.5;
    if center.iter().all(|value| value.is_finite()) && radius.is_finite() {
        (center, radius)
    } else {
        ([0.0, 0.0, 0.0], 1.0)
    }
}

fn fit_camera(center: [f32; 3], radius: f32, size: (u32, u32)) -> Camera3D {
    let aspect = size.0 as f32 / size.1 as f32;
    // Leave margin around the widest horizontal or vertical extent. The
    // orthographic view stays stable while the clip animates.
    let half_height = (radius * 1.5 / aspect.min(1.0)).max(0.25);
    let depth = (radius * 4.0).max(1.0);
    Camera3D::orthographic(
        [center[0], center[1], center[2] + depth],
        center,
        half_height,
    )
}
