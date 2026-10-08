//! Read-only production terrain point-route presentation. The immutable Frame
//! owns every dependency; terrain and the terrain-free overlay use one shared
//! ImportedSceneRenderer depth pass, for both window play and PNG capture.
use orr_bridge::FrameView;
use orr_games::navigation_yard3d_game::{NavigationScenePin, NavigationStepStatus};
use orr_model::StaticModel;
use orr_navigation::{Navigator, TerrainGraph};
use orr_navigation_runtime::{AgentSpec, RuntimeAgent, RuntimeState};
use orr_render::orr_rhi::{TextureFormat, Wgpu};
use orr_render::{
    Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, ModelRenderer,
    OffscreenTarget, OrbitCamera, PointLightSettings, Projection, RenderList3D, StaticInstance,
};
use orr_terrain::Terrain;

pub const TARGET_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;
pub const MAX_TARGET_SIDE: u32 = 4096;

pub struct NavigationReadback {
    pub terrain: Terrain,
    pub graph: TerrainGraph,
    pub navigator: Navigator,
    pub spec: AgentSpec,
}

/// Decode the same Frame used by play, without reopening a scene or terrain file.
/// The pin and complete derived-agent closure must still match admission.
pub fn read_navigation(frame: FrameView<'_>) -> Result<NavigationReadback, String> {
    let failed = frame.singleton::<NavigationStepStatus>();
    if failed.failed != 0 {
        return Err(format!(
            "navigation stopped at tick {}: {}",
            failed.failed_tick,
            failed.message()
        ));
    }
    let state = *frame.singleton::<RuntimeState>();
    if !state.is_active()
        || !frame.exists(state.agent)
        || frame.alive_count() != 1
        || frame.count::<RuntimeAgent>() != 1
        || frame.count::<orr_physics3d::Body>() != 0
        || frame.count::<orr_physics3d::Collider>() != 0
    {
        return Err(
            "navigation Frame requires exactly one derived point agent and no physics".into(),
        );
    }
    let agent = frame
        .get::<RuntimeAgent>(state.agent)
        .ok_or("navigation Frame agent is missing")?;
    let (terrain, graph, navigator, spec) = orr_navigation_runtime::decode_scene(
        &state,
        agent,
        frame.list(state.terrain_bytes),
        frame.list(state.graph_bytes),
        frame.list(state.navigator_bytes),
    )
    .map_err(|e| e.to_string())?;
    let pin = frame.singleton::<NavigationScenePin>();
    if terrain.asset_id() != pin.identity()?
        || terrain.revision() != pin.terrain_revision
        || graph.revision() != pin.graph_revision
        || spec != pin.agent.to_runtime()
    {
        return Err("navigation Frame differs from its admitted scene pin".into());
    }
    for point in graph.vertices() {
        orr_navigation_view::overlay::validate_point(*point)?;
    }
    orr_navigation_view::overlay::validate_point(navigator.position())?;
    Ok(NavigationReadback {
        terrain,
        graph,
        navigator,
        spec,
    })
}

fn overlay(view: &NavigationReadback) -> Result<StaticModel, String> {
    orr_navigation_view::overlay::build_overlay(
        &view.terrain,
        &view.graph,
        &view.navigator,
        view.spec.start,
        view.spec.goal,
        false,
    )
}
fn terrain_model(view: &NavigationReadback) -> Result<StaticModel, String> {
    orr_terrain_view::to_static_model(&view.terrain, &[])
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "navigation requires visible terrain triangles".into())
}
/// CPU-only admission of the exact renderer workload.
pub fn admit_presentation(frame: FrameView<'_>) -> Result<(), String> {
    let view = read_navigation(frame)?;
    terrain_model(&view)?;
    overlay(&view)?;
    Ok(())
}
pub fn validate_size(size: (u32, u32)) -> Result<(), String> {
    if size.0 == 0 || size.1 == 0 || size.0 > MAX_TARGET_SIDE || size.1 > MAX_TARGET_SIDE {
        return Err("navigation target must be 1..=4096 pixels per dimension".into());
    }
    Ok(())
}
fn lighting() -> Lighting {
    Lighting {
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient: 1.0,
        intensity: 0.0,
        shadows: false,
        tonemap: false,
        exposure: 1.0,
        ..Default::default()
    }
}

/// View-only orbit, pan and zoom, initially fitted to every admitted terrain vertex.
/// Orthographic framing preserves the complete scene in landscape and portrait.
#[derive(Clone, Copy, Debug)]
pub struct NavigationCamera {
    pub orbit: OrbitCamera,
    half_height: f32,
    initial_distance: f32,
}
impl NavigationCamera {
    pub fn new(frame: FrameView<'_>) -> Result<Self, String> {
        let view = read_navigation(frame)?;
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for point in view.graph.vertices() {
            for (axis, value) in point.iter().enumerate() {
                min[axis] = min[axis].min(value.to_f32());
                max[axis] = max[axis].max(value.to_f32());
            }
        }
        let target = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
        let radius = (0..3)
            .map(|i| ((max[i] - min[i]) * 0.5).powi(2))
            .sum::<f32>()
            .sqrt()
            .max(view.terrain.spacing().to_f32());
        let orbit = OrbitCamera::new(target, 0.45, 0.75, (radius * 3.8).max(1.0));
        Ok(Self {
            orbit,
            half_height: radius * 1.05,
            initial_distance: orbit.distance,
        })
    }
    pub fn orbit(&self) -> OrbitCamera {
        self.orbit
    }
    pub fn camera(&self, size: (u32, u32)) -> Camera3D {
        let aspect = size.0.max(1) as f32 / size.1.max(1) as f32;
        let half_height =
            self.half_height * self.orbit.distance / self.initial_distance / aspect.min(1.0);
        let mut camera = Camera3D::orthographic(self.orbit.eye(), self.orbit.target, half_height);
        // The full ±256 XYZ envelope can exceed the generic camera's 500-unit depth.
        camera.projection = Projection::Orthographic {
            half_height,
            near: -2048.0,
            far: 2048.0,
        };
        camera
    }
}

pub struct NavigationRenderer {
    target: OffscreenTarget<Wgpu>,
    core: RenderCore,
}
struct RenderCore {
    pin: NavigationScenePin,
    navigator_checksum: [u8; 32],
    format: TextureFormat,
    terrain: ModelRenderer<Wgpu>,
    overlay: ModelRenderer<Wgpu>,
    scene: ImportedSceneRenderer<Wgpu>,
}
impl NavigationRenderer {
    pub fn new(rhi: &Wgpu, size: (u32, u32), frame: FrameView<'_>) -> Result<Self, String> {
        Self::new_with_format(rhi, size, frame, TARGET_FORMAT)
    }
    pub fn new_with_format(
        rhi: &Wgpu,
        size: (u32, u32),
        frame: FrameView<'_>,
        format: TextureFormat,
    ) -> Result<Self, String> {
        validate_size(size)?;
        let view = read_navigation(frame)?;
        let terrain = terrain_model(&view)?;
        let overlay = overlay(&view)?;
        let mut scene =
            ImportedSceneRenderer::new(rhi.clone(), format).map_err(|e| e.to_string())?;
        scene.clear = [0.025, 0.035, 0.06, 1.0];
        preflight(
            &scene,
            size,
            &NavigationCamera::new(frame)?.camera(size),
            &terrain,
            &overlay,
        )?;
        // Only validated complete geometry reaches GPU allocation.
        let terrain =
            ModelRenderer::new(rhi.clone(), format, terrain).map_err(|e| e.to_string())?;
        let overlay =
            ModelRenderer::new(rhi.clone(), format, overlay).map_err(|e| e.to_string())?;
        Ok(Self {
            target: OffscreenTarget::new(rhi, size.0, size.1, format),
            core: RenderCore {
                pin: *frame.singleton::<NavigationScenePin>(),
                navigator_checksum: view.navigator.checksum(),
                format,
                terrain,
                overlay,
                scene,
            },
        })
    }
    pub fn target(&self) -> &OffscreenTarget<Wgpu> {
        &self.target
    }
    pub fn read_rgba8(&self) -> Vec<u8> {
        self.target.read_rgba8()
    }
    pub fn resize(&mut self, size: (u32, u32)) -> Result<(), String> {
        validate_size(size)?;
        self.target.resize(size.0, size.1);
        Ok(())
    }
    pub fn render(&mut self, frame: FrameView<'_>, camera: &Camera3D) -> Result<(), String> {
        self.core.render(
            frame,
            camera,
            ImportedSceneTarget {
                view: self.target.render_view(),
                size: self.target.size(),
                format: self.core.format,
                sample_count: 1,
            },
        )
    }
    /// Window and captures share this exact render core; no readback/blit in window play.
    pub fn render_to(
        &mut self,
        frame: FrameView<'_>,
        camera: &Camera3D,
        target: ImportedSceneTarget<'_, Wgpu>,
    ) -> Result<(), String> {
        self.core.render(frame, camera, target)
    }
}
fn preflight(
    scene: &ImportedSceneRenderer<Wgpu>,
    size: (u32, u32),
    camera: &Camera3D,
    terrain: &StaticModel,
    overlay: &StaticModel,
) -> Result<(), String> {
    let instance = [StaticInstance::default()];
    let models = [
        (terrain, instance.as_slice()),
        (overlay, instance.as_slice()),
    ];
    scene
        .preflight(
            size,
            camera,
            &lighting(),
            &PointLightSettings::default(),
            &RenderList3D::new(),
            &models,
        )
        .map_err(|e| e.to_string())
}
impl RenderCore {
    fn render(
        &mut self,
        frame: FrameView<'_>,
        camera: &Camera3D,
        target: ImportedSceneTarget<'_, Wgpu>,
    ) -> Result<(), String> {
        validate_size(target.size)?;
        if target.format != self.format || target.sample_count != 1 {
            return Err(
                "navigation target format/sample count differs from admitted renderer".into(),
            );
        }
        if *frame.singleton::<NavigationScenePin>() != self.pin {
            return Err("navigation renderer rejects a replacement scene pin".into());
        }
        let view = read_navigation(frame)?;
        let navigator_checksum = view.navigator.checksum();
        let candidate = (navigator_checksum != self.navigator_checksum)
            .then(|| overlay(&view))
            .transpose()?;
        preflight(
            &self.scene,
            target.size,
            camera,
            self.terrain.model(),
            candidate.as_ref().unwrap_or_else(|| self.overlay.model()),
        )?;
        // Invalid targets, frames and cameras retain the last successful image/cache.
        if let Some(model) = candidate {
            let replacement = ModelRenderer::new(self.terrain.rhi().clone(), self.format, model)
                .map_err(|e| e.to_string())?;
            self.overlay = replacement;
            self.navigator_checksum = navigator_checksum;
        }
        let mut batches = [
            ImportedBatch::Static(&mut self.terrain),
            ImportedBatch::Static(&mut self.overlay),
        ];
        self.scene
            .draw(
                target,
                camera,
                &lighting(),
                &PointLightSettings::default(),
                &mut batches,
            )
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation_project::PreparedScene;

    #[test]
    fn presentation_is_frame_owned_read_only_and_bounded() {
        let scene = PreparedScene::parse(
            include_str!("../../../scenes/navigation_point.scene.yaml"),
            include_bytes!("../../../scenes/navigation/point_demo.orrt"),
        )
        .unwrap();
        let before = scene.frame().to_bytes();
        let frame = FrameView::of(scene.frame());
        admit_presentation(frame).unwrap();
        let view = read_navigation(frame).unwrap();
        let terrain = terrain_model(&view).unwrap();
        let overlay = overlay(&view).unwrap();
        assert_eq!(terrain.source().primitives.len(), 1);
        assert!(overlay
            .source()
            .primitives
            .iter()
            .all(|p| p.id.starts_with("navigation/editor_overlay.orrmodel#")));
        assert_eq!(before, scene.frame().to_bytes());
        assert!(validate_size((0, 10)).is_err());
        assert!(validate_size((4097, 10)).is_err());
        assert!(validate_size((4096, 1)).is_ok());
    }
}
