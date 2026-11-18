//! The GPU side of view assets: one model renderer per model file, drawn together with the 3D
//! shapes of the look in one picture (shared depth, one sun, one shadow map).
//!
//! The window-free part (which model, where) is [`crate::ModelDraw`]; this file only turns those
//! draws into GPU work. A run without view assets never creates any of it.

use std::collections::BTreeMap;

use orr_gfx_view::ViewAssets;
use orr_render::imported_scene::{model_draw_budget, MAX_IMPORTED_BATCHES};
use orr_render::model_renderer::{MAX_STATIC_INSTANCES, MAX_STATIC_INSTANCE_DRAWS};
use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu};
use orr_render::{Camera3D, OffscreenTarget, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, ModelRenderer, PointLightSettings, RenderList3D, Renderer3D, Settings3D, StaticInstance};

use crate::view3d::ModelDraw;

/// Shapes and models of a frame, drawn by one [`ImportedSceneRenderer`].
pub(crate) struct ModelScene {
    composer: ImportedSceneRenderer<Wgpu>,
    /// The shapes of the look. The composed pass has one sample per pixel, so this renderer has no MSAA.
    shapes: Renderer3D<Wgpu>,
    rhi: Wgpu,
    format: TextureFormat,
    /// One renderer per model file; `None` for a file that could not be loaded (reported once).
    renderers: BTreeMap<String, Option<ModelRenderer<Wgpu>>>,
    last_error: Option<String>,
    /// Instances left out of frames because they did not fit the draw budget, since the start.
    dropped_instances: u64,
    /// Whether the previous frame left instances out (the next such frame is then not reported again).
    was_dropping: bool,
}

/// What one model file asks of a frame: how many instances, and how many primitive draws each takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ModelRequest {
    instances: usize,
    primitives: usize,
}

/// How many instances of each request the renderer takes, so that the frame stays inside its limits:
/// [`MAX_STATIC_INSTANCES`] and [`MAX_STATIC_INSTANCE_DRAWS`] per model, `draw_budget` primitive
/// draws for all models together, and `max_models` models. The requests are served in order, and the
/// first instances of a request are the ones kept, so the same input always gives the same result.
fn fit_models(requests: &[ModelRequest], draw_budget: usize, max_models: usize) -> Vec<usize> {
    let mut left = draw_budget;
    let mut models = 0;
    requests
        .iter()
        .map(|request| {
            let mut keep = request.instances.min(MAX_STATIC_INSTANCES);
            if let Some(fit) = MAX_STATIC_INSTANCE_DRAWS.min(left).checked_div(request.primitives) {
                keep = keep.min(fit);
            }
            if keep == 0 || models >= max_models {
                return 0;
            }
            models += 1;
            left -= keep * request.primitives;
            keep
        })
        .collect()
}

/// Cuts `groups` with [`fit_models`] for the models in `primitives` (path and primitive draws per
/// instance) and returns how many instances were cut. A cut group stays in `groups`, possibly empty.
fn cut_groups<'a>(groups: &mut BTreeMap<&'a str, Vec<StaticInstance>>, primitives: &[(&'a str, usize)], draw_budget: usize) -> usize {
    let requests: Vec<ModelRequest> = primitives
        .iter()
        .map(|(path, primitives)| ModelRequest { instances: groups[path].len(), primitives: *primitives })
        .collect();
    let keep = fit_models(&requests, draw_budget, MAX_IMPORTED_BATCHES - 1);
    let mut dropped = 0usize;
    for ((path, _), keep) in primitives.iter().zip(keep) {
        let instances = groups.get_mut(path).expect("grouped above");
        dropped += instances.len() - keep;
        instances.truncate(keep);
    }
    dropped
}

/// The models that get a batch in the frame: the groups that are loaded and still hold an instance,
/// in path order. A group cut to nothing is left out, because every batch counts against
/// [`MAX_IMPORTED_BATCHES`] with the shapes of the look.
fn model_batch_paths<'a>(groups: &BTreeMap<&'a str, Vec<StaticInstance>>, is_loaded: impl Fn(&str) -> bool) -> Vec<&'a str> {
    groups.iter().filter(|(path, instances)| !instances.is_empty() && is_loaded(path)).map(|(path, _)| *path).collect()
}

impl ModelScene {
    pub(crate) fn new(rhi: Wgpu, format: TextureFormat) -> Result<Self, String> {
        let composer = ImportedSceneRenderer::new(rhi.clone(), format).map_err(|e| e.to_string())?;
        let shapes = Renderer3D::with_settings(rhi.clone(), format, Settings3D { msaa: 1, ..Settings3D::default() });
        Ok(Self { composer, shapes, rhi, format, renderers: BTreeMap::new(), last_error: None, dropped_instances: 0, was_dropping: false })
    }

    /// Draws the shapes of `list` and the `draws` into `view`.
    /// The sun and the other lights of `list` light both. A model that cannot be loaded is
    /// reported on stderr once and left out.
    pub(crate) fn draw(
        &mut self,
        view: &<Wgpu as Rhi>::TextureView,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        draws: &[ModelDraw],
        assets: &ViewAssets,
    ) {
        let mut groups: BTreeMap<&str, Vec<StaticInstance>> = BTreeMap::new();
        for draw in draws {
            groups.entry(draw.model.as_str()).or_default().push(draw.instance);
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
        self.limit_to_budget(&mut groups, list);
        self.composer.scene_lights = list.scene_lights;
        self.composer.decals.clone_from(&list.decals);
        let planned = model_batch_paths(&groups, |path| matches!(self.renderers.get(path), Some(Some(_))));
        let mut batches = vec![ImportedBatch::Procedural { renderer: &mut self.shapes, list }];
        for (path, renderer) in &mut self.renderers {
            if let (Some(renderer), Some(instances)) = (renderer.as_mut(), groups.get(path.as_str())) {
                if planned.binary_search(&path.as_str()).is_ok() {
                    batches.push(ImportedBatch::StaticInstances { renderer, instances });
                }
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

    /// Cuts `groups` down to what the renderer accepts next to the shapes of `list`, so that the
    /// shapes and the models that fit are still drawn. The cut instances are counted, and the first
    /// frame of a series of frames with cut instances is reported on stderr.
    fn limit_to_budget(&mut self, groups: &mut BTreeMap<&str, Vec<StaticInstance>>, list: &RenderList3D) {
        let primitives: Vec<(&str, usize)> = groups
            .keys()
            .filter_map(|path| match self.renderers.get(*path) {
                Some(Some(renderer)) => Some((*path, renderer.model().source().primitives.len())),
                _ => None,
            })
            .collect();
        let dropped = cut_groups(groups, &primitives, model_draw_budget(list));
        self.dropped_instances += dropped as u64;
        if dropped > 0 && !self.was_dropping {
            eprintln!("drawing models: {dropped} model instances do not fit the draw budget and are left out (the first ones of each model are kept; {} left out so far)", self.dropped_instances);
        }
        self.was_dropping = dropped > 0;
    }

    /// Draws like [`ModelScene::draw`] into a picture of exactly `size` pixels, then `on_top` over it
    /// (the overlay text), and reads it back as RGBA8, top row first. It does not depend on the window.
    pub(crate) fn capture(
        &mut self,
        size: (u32, u32),
        list: &RenderList3D,
        camera: &Camera3D,
        draws: &[ModelDraw],
        assets: &ViewAssets,
        on_top: impl FnOnce(&<Wgpu as Rhi>::TextureView, (u32, u32)),
    ) -> Vec<u8> {
        let target = OffscreenTarget::new(&self.rhi.clone(), size.0, size.1, self.format);
        target.render_into(|_, view, size| {
            self.draw(view, size, list, camera, draws, assets);
            on_top(view, size);
        });
        target.read_rgba8()
    }

    fn load(&self, assets: &ViewAssets, path: &str) -> Result<ModelRenderer<Wgpu>, String> {
        let model = orr_model::import::import_path(assets.root(), path).map_err(|e| e.to_string())?;
        ModelRenderer::new(self.rhi.clone(), self.format, model).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_render::imported_scene::MAX_IMPORTED_DRAWS;
    use orr_render::Material;

    fn request(instances: usize, primitives: usize) -> ModelRequest {
        ModelRequest { instances, primitives }
    }

    fn draws(requests: &[ModelRequest], kept: &[usize]) -> usize {
        requests.iter().zip(kept).map(|(r, k)| r.primitives * k).sum()
    }

    #[test]
    fn a_model_of_17_primitives_with_256_instances_is_cut_to_what_the_draw_limit_takes() {
        let requests = [request(256, 17)];

        let kept = fit_models(&requests, MAX_IMPORTED_DRAWS, MAX_IMPORTED_BATCHES - 1);

        assert_eq!(kept, [240], "4096 / 17 draws, and the file limit of 256 instances is not what binds");
        assert!(draws(&requests, &kept) <= MAX_IMPORTED_DRAWS);
    }

    #[test]
    fn a_model_that_fits_is_kept_whole_at_the_boundary_and_cut_by_one_instance_beyond_it() {
        assert_eq!(fit_models(&[request(256, 16)], MAX_IMPORTED_DRAWS, 255), [256]);
        assert_eq!(fit_models(&[request(256, 16)], MAX_IMPORTED_DRAWS - 1, 255), [255]);
        assert_eq!(fit_models(&[request(300, 1)], MAX_IMPORTED_DRAWS, 255), [MAX_STATIC_INSTANCES]);
    }

    #[test]
    fn several_models_that_sum_past_the_limit_are_cut_in_order_and_the_first_ones_win() {
        let requests = [request(100, 16), request(100, 16), request(100, 16), request(100, 16)];

        let kept = fit_models(&requests, MAX_IMPORTED_DRAWS, 255);

        assert_eq!(kept, [100, 100, 56, 0]);
        assert!(draws(&requests, &kept) <= MAX_IMPORTED_DRAWS);
        assert_eq!(kept, fit_models(&requests, MAX_IMPORTED_DRAWS, 255), "the same input gives the same cut");
    }

    #[test]
    fn a_small_model_after_a_cut_one_still_takes_what_is_left() {
        let requests = [request(256, 17), request(10, 2)];

        let kept = fit_models(&requests, MAX_IMPORTED_DRAWS, 255);

        assert_eq!(kept, [240, 8], "4096 - 240 * 17 = 16 draws are left for 2 primitives each");
    }

    #[test]
    fn at_most_the_given_number_of_models_is_kept() {
        let requests = vec![request(1, 1); 10];

        let kept = fit_models(&requests, MAX_IMPORTED_DRAWS, 3);

        assert_eq!(kept, [1, 1, 1, 0, 0, 0, 0, 0, 0, 0]);
    }

    /// The batches a frame submits (the shapes of the look plus one per model) for `models` distinct
    /// models of one primitive each, drawn once each, after the cut of the real frame path.
    fn submitted_batches(models: usize) -> usize {
        let paths: Vec<String> = (0..models).map(|i| format!("m{i:04}.glb")).collect();
        let mut groups: BTreeMap<&str, Vec<StaticInstance>> = BTreeMap::new();
        for path in &paths {
            groups.insert(path.as_str(), vec![StaticInstance::default()]);
        }
        let primitives: Vec<(&str, usize)> = paths.iter().map(|p| (p.as_str(), 1)).collect();
        cut_groups(&mut groups, &primitives, MAX_IMPORTED_DRAWS);
        1 + model_batch_paths(&groups, |_| true).len()
    }

    #[test]
    fn an_empty_model_group_gets_no_batch_so_256_distinct_models_stay_inside_the_batch_limit() {
        assert!(submitted_batches(255) <= MAX_IMPORTED_BATCHES);
        assert!(submitted_batches(256) <= MAX_IMPORTED_BATCHES, "the 256th model is cut to nothing and must not be submitted");
        assert_eq!(submitted_batches(256), MAX_IMPORTED_BATCHES);
    }

    #[test]
    fn a_model_without_primitives_costs_no_draw() {
        assert_eq!(fit_models(&[request(256, 0)], 0, 255), [256]);
    }

    #[test]
    fn the_shapes_of_the_look_take_their_draws_out_of_the_budget_of_the_models() {
        let mut list = RenderList3D::new();
        assert_eq!(fit_models(&[request(256, 16)], model_draw_budget(&list), 255), [256]);

        let m = Material::new([1.0; 3]);
        list.sphere([0.0; 3], [0.0, 0.0, 0.0, 1.0], 0.5, &m);
        list.plane([0.0; 3], 10.0, 10.0, &m);

        let kept = fit_models(&[request(256, 16)], model_draw_budget(&list), 255);

        assert_eq!(kept, [255], "the sphere kind and the plane kind take 2 of the 4096 draws");
        assert_eq!(list.instance_count(), 2, "the shapes themselves are not cut");
    }
}
