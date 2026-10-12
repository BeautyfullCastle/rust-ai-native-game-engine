//! Presentation-only terrain course view. Terrain is uploaded once per live
//! scene; moving actors share its depth attachment through the public composer.
#![allow(clippy::float_arithmetic)]

use crate::{Course, Game, State};
use orr_render::{
    orr_rhi::{Rhi, TextureFormat},
    Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, Material,
    ModelRenderer, OrbitCamera, PointLightSettings, RenderList3D, Renderer3D, Settings3D,
    IDENTITY_ROT,
};

pub const EXPLORER_RADIUS: f32 = 0.24;
pub const EXPLORER_HALF_LENGTH: f32 = 0.32;
pub const MIN_PITCH: f32 = 0.25;
pub const MAX_PITCH: f32 = 1.3;
pub const MIN_DISTANCE: f32 = 3.0;
pub const MAX_DISTANCE: f32 = 24.0;
pub const CLEAR: [f64; 4] = [0.055, 0.085, 0.12, 1.0];

/// The capsule's lowest point sits at the exact accepted terrain query height.
/// This is artwork for a point anchor, not a collision capsule.
pub fn explorer_center(state: &State) -> [f32; 3] {
    let [x, y, z] = state.anchor.map(|v| v.to_f32());
    [x, y + EXPLORER_RADIUS + EXPLORER_HALF_LENGTH, z]
}

/// Follow all three accepted coordinates without changing the orbit or zoom.
pub fn follow_target(camera: &mut OrbitCamera, state: &State) {
    let [x, y, z] = state.anchor.map(|v| v.to_f32());
    camera.target = [x, y + 0.7, z];
}

pub fn follow_camera(state: &State) -> OrbitCamera {
    // Keep the authored next goal and its full numeral in the default framing,
    // including the narrower 4:3 viewport. User orbit/zoom remain unchanged.
    let mut camera = OrbitCamera::new([0.0; 3], 0.35, 0.68, 14.0);
    follow_target(&mut camera, state);
    camera
}

/// Reject nonfinite pointer input, bound finite spikes, and keep the eye above
/// its target. This does not promise camera/terrain collision or visibility.
pub fn orbit(camera: &mut OrbitCamera, delta: [f32; 2]) {
    if !delta.into_iter().all(f32::is_finite) {
        return;
    }
    camera.orbit(delta.map(|v| v.clamp(-2048.0, 2048.0)));
    camera.pitch = camera.pitch.clamp(MIN_PITCH, MAX_PITCH);
    camera.yaw = camera.yaw.rem_euclid(std::f32::consts::TAU);
}

pub fn zoom(camera: &mut OrbitCamera, steps: f32) {
    if !steps.is_finite() {
        return;
    }
    camera.zoom(steps.clamp(-12.0, 12.0));
    camera.distance = camera.distance.clamp(MIN_DISTANCE, MAX_DISTANCE);
}

pub fn lighting() -> Lighting {
    Lighting {
        intensity: 1.25,
        ambient: 0.65,
        sky: [0.8, 0.88, 1.0],
        ground: [0.46, 0.5, 0.42],
        shadows: false,
        ..Lighting::default()
    }
}

/// Build only the small moving/progress view; never rebuild the terrain mesh.
pub fn actors(game: &Game) -> RenderList3D {
    let mut list = RenderList3D::new();
    fill_actors(&mut list, game);
    list
}

fn fill_actors(list: &mut RenderList3D, game: &Game) {
    list.clear();
    let state = game.state();
    let blue = Material::new([0.025, 0.22, 0.95]).rough(0.65);
    list.capsule(
        explorer_center(&state),
        IDENTITY_ROT,
        EXPLORER_HALF_LENGTH,
        EXPLORER_RADIUS,
        &blue,
    );
    // A small white cap makes the blue explorer readable against dark slopes.
    let [x, y, z] = explorer_center(&state);
    list.sphere(
        [x, y + EXPLORER_HALF_LENGTH + 0.12, z],
        IDENTITY_ROT,
        0.13,
        &Material::new([0.8, 0.9, 1.0]),
    );
    for (index, anchor) in game.course().beacons().iter().enumerate() {
        let [x, y, z] = anchor.map(|v| v.to_f32());
        let reached = index < usize::from(state.collected);
        let active = index == usize::from(state.collected);
        let material = if reached {
            Material::new([0.12, 0.8, 0.46]).glow(0.12)
        } else {
            Material::new([1.0, 0.57, 0.035]).glow(if active { 0.4 } else { 0.08 })
        };
        list.cuboid(
            [x, y + 0.45, z],
            IDENTITY_ROT,
            [0.055, 0.45, 0.055],
            &material,
        );
        list.sphere(
            [x, y + 0.85, z],
            IDENTITY_ROT,
            if active { 0.28 } else { 0.19 },
            &material,
        );
        digit(list, index + 1, [x, y + 1.42, z], &material);
    }
    let [x, y, z] = game.course().finish().map(|v| v.to_f32());
    let finish = if state.finished {
        Material::new([0.2, 1.0, 0.6]).glow(0.55)
    } else if state.collected == 4 {
        Material::new([1.0, 0.72, 0.12]).glow(0.4)
    } else {
        Material::new([0.62, 0.68, 0.76])
    };
    // A flag, visibly different from the numbered checkpoint posts.
    list.cuboid([x, y + 0.8, z], IDENTITY_ROT, [0.06, 0.8, 0.06], &finish);
    list.cuboid(
        [x + 0.35, y + 1.4, z],
        IDENTITY_ROT,
        [0.3, 0.2, 0.055],
        &finish,
    );
}

fn digit(list: &mut RenderList3D, number: usize, center: [f32; 3], material: &Material) {
    // Seven-segment numbers are real depth-tested geometry, visible from both
    // sides. At edge-on angles the window title remains the progress fallback.
    const DIGITS: [u8; 4] = [0b000_0110, 0b101_1011, 0b100_1111, 0b110_0110];
    let segments = [
        ([0.0, 0.28], [0.14, 0.028]),
        ([0.16, 0.14], [0.028, 0.14]),
        ([0.16, -0.14], [0.028, 0.14]),
        ([0.0, -0.28], [0.14, 0.028]),
        ([-0.16, -0.14], [0.028, 0.14]),
        ([-0.16, 0.14], [0.028, 0.14]),
        ([0.0, 0.0], [0.14, 0.028]),
    ];
    for (index, (offset, half)) in segments.into_iter().enumerate() {
        if DIGITS[number - 1] & (1 << index) != 0 {
            list.cuboid(
                [center[0] + offset[0], center[1] + offset[1], center[2]],
                IDENTITY_ROT,
                [half[0], half[1], 0.035],
                material,
            );
        }
    }
}

/// Owns one immutable terrain upload for the admitted course snapshot. Call
/// `draw` with the Game that uses that same Course. Restart/motion/resize do not
/// construct another ModelRenderer or touch the canonical terrain bytes.
pub struct Scene<B: Rhi> {
    terrain_revision: [u8; 32],
    composer: ImportedSceneRenderer<B>,
    terrain: ModelRenderer<B>,
    actor_renderer: Renderer3D<B>,
    actors: RenderList3D,
}

impl<B: Rhi> Scene<B> {
    pub fn new(rhi: B, format: TextureFormat, course: &Course) -> Result<Self, String> {
        // Reject unsupported mesh coordinates before constructing GPU renderers.
        let model = orr_terrain_view::to_static_model(course.terrain(), &[])
            .map_err(|e| e.to_string())?
            .ok_or("course terrain has no visible triangles")?;
        let mut composer =
            ImportedSceneRenderer::new(rhi.clone(), format).map_err(|e| e.to_string())?;
        composer.clear = CLEAR;
        Ok(Self {
            terrain_revision: course.terrain_revision(),
            terrain: ModelRenderer::new(rhi.clone(), format, model).map_err(|e| e.to_string())?,
            actor_renderer: Renderer3D::with_settings(rhi, format, Settings3D::LOW),
            composer,
            actors: RenderList3D::new(),
        })
    }

    pub fn depth_size(&self) -> Option<(u32, u32)> {
        self.composer.depth_size()
    }

    pub fn depth_generation(&self) -> u64 {
        self.composer.depth_generation()
    }

    pub fn draw(
        &mut self,
        view: &B::TextureView,
        size: (u32, u32),
        game: &Game,
        camera: &Camera3D,
    ) -> Result<(), String> {
        self.draw_order(view, size, game, camera, false)
    }

    /// Identical production composition with actor/terrain submission reversed;
    /// useful for the explicit shared-depth acceptance oracle.
    pub fn draw_reversed(
        &mut self,
        view: &B::TextureView,
        size: (u32, u32),
        game: &Game,
        camera: &Camera3D,
    ) -> Result<(), String> {
        self.draw_order(view, size, game, camera, true)
    }

    fn draw_order(
        &mut self,
        view: &B::TextureView,
        size: (u32, u32),
        game: &Game,
        camera: &Camera3D,
        reversed: bool,
    ) -> Result<(), String> {
        if game.course().terrain_revision() != self.terrain_revision {
            return Err("game terrain differs from the admitted render snapshot".into());
        }
        if size.0 == 0 || size.1 == 0 || size.0 > 8192 || size.1 > 8192 {
            return Err("render size must be 1..8192 per axis".into());
        }
        fill_actors(&mut self.actors, game);
        let mut batches = [
            ImportedBatch::Static(&mut self.terrain),
            ImportedBatch::Procedural {
                renderer: &mut self.actor_renderer,
                list: &self.actors,
            },
        ];
        if reversed {
            batches.reverse();
        }
        self.composer
            .draw(
                ImportedSceneTarget {
                    view,
                    size,
                    format: self.composer.format(),
                    sample_count: 1,
                },
                camera,
                &lighting(),
                &PointLightSettings::default(),
                &mut batches,
            )
            .map_err(|e| e.to_string())
    }
}
