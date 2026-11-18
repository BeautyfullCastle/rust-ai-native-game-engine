//! The GPU side of view assets: one model renderer per model file, drawn together with the 3D
//! shapes of the look in one picture (shared depth, one sun, one shadow map).
//!
//! The window-free part (which model, where) is [`crate::ModelDraw`]; this file only turns those
//! draws into GPU work. A run without view assets never creates any of it.

use std::collections::BTreeMap;

use orr_gfx_view::ViewAssets;
use orr_render::model_renderer::MAX_STATIC_INSTANCES;
use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu};
use orr_render::{Camera3D, OffscreenTarget, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, ModelRenderer, PointLightSettings, RenderList3D, Renderer3D, StaticInstance};

use crate::view3d::ModelDraw;

/// Shapes and models of a frame, drawn by one [`ImportedSceneRenderer`].
pub(crate) struct ModelScene {
    composer: ImportedSceneRenderer<Wgpu>,
    rhi: Wgpu,
    format: TextureFormat,
    /// One renderer per model file; `None` for a file that could not be loaded (reported once).
    renderers: BTreeMap<String, Option<ModelRenderer<Wgpu>>>,
    last_error: Option<String>,
}

impl ModelScene {
    pub(crate) fn new(rhi: Wgpu, format: TextureFormat) -> Result<Self, String> {
        let composer = ImportedSceneRenderer::new(rhi.clone(), format).map_err(|e| e.to_string())?;
        Ok(Self { composer, rhi, format, renderers: BTreeMap::new(), last_error: None })
    }

    /// Draws the shapes of `list` (with the renderer `shapes`) and the `draws` into `view`.
    /// The sun and the other lights of `list` light both. A model that cannot be loaded is
    /// reported on stderr once and left out.
    pub(crate) fn draw(
        &mut self,
        shapes: &mut Renderer3D<Wgpu>,
        view: &<Wgpu as Rhi>::TextureView,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        draws: &[ModelDraw],
        assets: &ViewAssets,
    ) {
        let mut groups: BTreeMap<&str, Vec<StaticInstance>> = BTreeMap::new();
        for draw in draws {
            let group = groups.entry(draw.model.as_str()).or_default();
            if group.len() < MAX_STATIC_INSTANCES {
                group.push(draw.instance);
            }
        }
        for path in groups.keys() {
            if !self.renderers.contains_key(*path) {
                let renderer = match self.load(assets, path) {
                    Ok(renderer) => Some(renderer),
                    Err(error) => {
                        eprintln!("view asset model {path}: {error}");
                        None
                    }
                };
                self.renderers.insert((*path).to_string(), renderer);
            }
        }
        self.composer.scene_lights = list.scene_lights;
        self.composer.decals.clone_from(&list.decals);
        let mut batches = vec![ImportedBatch::Procedural { renderer: shapes, list }];
        for (path, renderer) in &mut self.renderers {
            if let (Some(renderer), Some(instances)) = (renderer.as_mut(), groups.get(path.as_str())) {
                batches.push(ImportedBatch::StaticInstances { renderer, instances });
            }
        }
        let target = ImportedSceneTarget { view, size, format: self.format, sample_count: 1 };
        let result = self.composer.draw(target, camera, &list.lighting, &PointLightSettings::default(), &mut batches);
        match result {
            Ok(()) => self.last_error = None,
            Err(error) => {
                let text = error.to_string();
                if self.last_error.as_deref() != Some(text.as_str()) {
                    eprintln!("drawing models: {text}");
                }
                self.last_error = Some(text);
            }
        }
    }

    /// Draws like [`ModelScene::draw`] into a picture of exactly `size` pixels and reads it back as
    /// RGBA8, top row first. It does not depend on the window.
    pub(crate) fn capture(
        &mut self,
        shapes: &mut Renderer3D<Wgpu>,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        draws: &[ModelDraw],
        assets: &ViewAssets,
    ) -> Vec<u8> {
        let target = OffscreenTarget::new(&self.rhi.clone(), size.0, size.1, self.format);
        target.render_into(|_, view, size| self.draw(shapes, view, size, list, camera, draws, assets));
        target.read_rgba8()
    }

    fn load(&self, assets: &ViewAssets, path: &str) -> Result<ModelRenderer<Wgpu>, String> {
        let model = orr_model::import::import_path(assets.root(), path).map_err(|e| e.to_string())?;
        ModelRenderer::new(self.rhi.clone(), self.format, model).map_err(|e| e.to_string())
    }
}
