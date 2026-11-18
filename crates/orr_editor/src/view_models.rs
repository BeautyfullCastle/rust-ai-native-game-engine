//! The models of entities that have an `orr_gfx::ViewRef`. The id names a view asset (a
//! `*.orrview` file in the open scene's folder), and the asset names a model file next to it.
//! The viewport draws that model at the entity's `Transform3`, instead of its marker box.
//!
//! A frame holds only what the draw limits allow, like the `orr_app` window: at most
//! [`MAX_STATIC_INSTANCES`] instances and [`MAX_STATIC_INSTANCE_DRAWS`] primitive draws of one
//! model, the draw budget for all models together, and the models the viewport accepts.
#![allow(clippy::disallowed_types)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use orr_gfx_view::ViewAssets;
use orr_render::model_renderer::{MAX_STATIC_INSTANCES, MAX_STATIC_INSTANCE_DRAWS};
use orr_render::StaticInstance;

use crate::viewport3d::{ModelPlacement, ViewRefPlace};

/// How often the view asset files are looked at again.
const CHECK_INTERVAL: Duration = Duration::from_millis(500);

/// Distinct models one viewport frame accepts (see `static_groups` in the viewport).
pub const MAX_VIEWPORT_MODELS: usize = 8;

/// What a model file read gave, and the file's time and size when it was read.
type Loaded = (Option<(SystemTime, u64)>, Result<Arc<orr_model::StaticModel>, String>);

/// One placement that wants a model: the index of its model in the frame's model list, and how many
/// primitive draws each instance of that model takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Wanted {
    model: usize,
    primitives: usize,
}

/// Which wanted placements the frame keeps, in order. A placement is left out when its model is
/// full ([`MAX_STATIC_INSTANCES`] instances or [`MAX_STATIC_INSTANCE_DRAWS`] draws), when it would
/// pass `draw_budget` draws for all models together, or when it needs a model beyond `max_models`
/// distinct ones. The first placements are the ones kept, so the same input gives the same result.
fn fit(wanted: &[Wanted], draw_budget: usize, max_models: usize) -> Vec<bool> {
    let mut left = draw_budget;
    let mut models: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    wanted
        .iter()
        .map(|w| {
            let (instances, draws) = models.get(&w.model).copied().unwrap_or((0, 0));
            let fits = instances < MAX_STATIC_INSTANCES
                && draws + w.primitives <= MAX_STATIC_INSTANCE_DRAWS
                && w.primitives <= left
                && (models.contains_key(&w.model) || models.len() < max_models);
            if fits {
                models.insert(w.model, (instances + 1, draws + w.primitives));
                left -= w.primitives;
            }
            fits
        })
        .collect()
}

/// The view assets and models of the open scene's folder, read when needed.
#[derive(Default)]
pub struct ViewModels {
    root: Option<PathBuf>,
    assets: Option<Result<ViewAssets, String>>,
    checked: Option<Instant>,
    /// Model files by their path in the view asset, with the file time and size they were read at.
    models: BTreeMap<String, Loaded>,
    problem: Option<String>,
    /// Whether the previous frame left placements out.
    was_dropping: bool,
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

impl ViewModels {
    /// The first problem of the last [`ViewModels::placements`]: a folder whose view assets cannot
    /// be read, an id that names no asset, or a model file that cannot be loaded.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Looks at the folder again when it is another one, or when `CHECK_INTERVAL` has passed.
    fn refresh(&mut self, root: &Path, now: Instant) {
        let same = self.root.as_deref() == Some(root);
        if same && self.checked.is_some_and(|at| now.duration_since(at) < CHECK_INTERVAL) {
            return;
        }
        if !same {
            self.models.clear();
        }
        self.root = Some(root.to_path_buf());
        self.checked = Some(now);
        self.assets = Some(ViewAssets::load_dir(root).map_err(|e| e.to_string()));
    }

    fn model(&mut self, root: &Path, path: &str) -> Result<Arc<orr_model::StaticModel>, String> {
        let current = stamp(&root.join(path));
        if let Some((stamped, model)) = self.models.get(path) {
            if *stamped == current {
                return model.clone();
            }
        }
        let model = orr_model::import::import_path(root, path).map(Arc::new).map_err(|e| format!("{path}: {e}"));
        self.models.insert(path.to_string(), (current, model.clone()));
        model
    }

    /// The model placements of `places`, for a scene at `scene`, next to the `existing` placements
    /// of the frame. The view asset files are read from the folder of the scene file.
    /// `draw_budget` is what the frame can still take (see `orr_render::imported_scene::model_draw_budget`)
    /// before the primitives of `existing`.
    pub fn placements(&mut self, scene: Option<&Path>, places: &[ViewRefPlace], existing: &[ModelPlacement], draw_budget: usize, now: Instant) -> Vec<ModelPlacement> {
        self.problem = None;
        if places.iter().all(|p| p.asset == 0) {
            return Vec::new();
        }
        let Some(root) = scene.and_then(Path::parent) else {
            self.problem = Some("Save the scene to a file first: view assets are read from its folder".to_string());
            return Vec::new();
        };
        self.refresh(root, now);
        let root = root.to_path_buf();
        let assets = match self.assets.take() {
            Some(Ok(assets)) => assets,
            other => {
                self.problem = other.and_then(Result::err);
                return Vec::new();
            }
        };
        let mut models: Vec<Arc<orr_model::StaticModel>> = Vec::new();
        let mut wanted = Vec::new();
        let mut drawn = Vec::new();
        for place in places.iter().filter(|p| p.asset != 0) {
            let Some(asset) = assets.get(place.asset) else {
                self.problem.get_or_insert_with(|| format!("{}: no .orrview file has the id {}", entity_text(place), place.asset));
                continue;
            };
            let scale = place.scale.map(|s| s * asset.scale);
            if !scale.iter().all(|s| s.is_finite() && *s > 0.0) {
                self.problem.get_or_insert_with(|| format!("{}: the scale of the entity and its view asset must be positive", entity_text(place)));
                continue;
            }
            let model = match self.model(&root, &asset.model) {
                Ok(model) => model,
                Err(error) => {
                    self.problem.get_or_insert(error);
                    continue;
                }
            };
            let index = models.iter().position(|m| Arc::ptr_eq(m, &model)).unwrap_or_else(|| {
                models.push(model.clone());
                models.len() - 1
            });
            wanted.push(Wanted { model: index, primitives: model.source().primitives.len() });
            drawn.push((place, model, scale));
        }
        self.assets = Some(Ok(assets));
        // The placements of other sources count against the same limits.
        let used_draws: usize = existing.iter().map(|p| p.model.source().primitives.len()).sum();
        let mut other_models: Vec<&Arc<orr_model::StaticModel>> = Vec::new();
        for p in existing {
            if !other_models.iter().any(|m| Arc::ptr_eq(m, &p.model)) {
                other_models.push(&p.model);
            }
        }
        let keep = fit(&wanted, draw_budget.saturating_sub(used_draws), MAX_VIEWPORT_MODELS.saturating_sub(other_models.len()));
        let dropped = keep.iter().filter(|k| !**k).count();
        if dropped > 0 && !self.was_dropping {
            eprintln!("viewport: {dropped} view asset models do not fit the draw limits and are left out (the first ones are kept)");
        }
        if dropped > 0 {
            self.problem.get_or_insert_with(|| format!("{dropped} view asset models are left out: they do not fit the draw limits"));
        }
        self.was_dropping = dropped > 0;
        drawn
            .into_iter()
            .zip(keep)
            .filter(|(_, keep)| *keep)
            .map(|((place, model, scale), _)| ModelPlacement {
                entity: place.entity,
                model,
                instance: StaticInstance { translation: place.translation, rotation: place.rotation, scale },
            })
            .collect()
    }
}

fn entity_text(place: &ViewRefPlace) -> String {
    format!("entity {}", place.entity.index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(model: usize, primitives: usize) -> Wanted {
        Wanted { model, primitives }
    }

    #[test]
    fn placements_that_fit_are_all_kept() {
        assert_eq!(fit(&[w(0, 2), w(1, 3), w(0, 2)], 4096, 8), [true, true, true]);
    }

    #[test]
    fn the_draw_budget_keeps_the_first_placements() {
        // 10 draws: 4 + 4 fit, the third placement of 4 does not, a later one of 2 does.
        assert_eq!(fit(&[w(0, 4), w(1, 4), w(2, 4), w(3, 2)], 10, 8), [true, true, false, true]);
        assert_eq!(fit(&[w(0, 1)], 0, 8), [false], "no budget keeps nothing");
    }

    #[test]
    fn one_model_is_cut_at_the_instance_and_draw_limits() {
        let many = vec![w(0, 1); MAX_STATIC_INSTANCES + 1];
        let kept = fit(&many, 4096, 8);
        assert_eq!(kept.iter().filter(|k| **k).count(), MAX_STATIC_INSTANCES);
        assert!(!kept[MAX_STATIC_INSTANCES]);

        let heavy = vec![w(0, 17); 300];
        let kept = fit(&heavy, usize::MAX, 8);
        assert_eq!(kept.iter().filter(|k| **k).count(), MAX_STATIC_INSTANCE_DRAWS / 17, "4096 / 17 draws");
    }

    #[test]
    fn only_the_given_number_of_distinct_models_is_kept_but_more_instances_of_those_are() {
        let wanted = [w(0, 1), w(1, 1), w(2, 1), w(0, 1), w(1, 1)];
        assert_eq!(fit(&wanted, 4096, 2), [true, true, false, true, true]);
        assert_eq!(fit(&wanted, 4096, 0), [false; 5]);
    }

    #[test]
    fn a_model_without_primitives_costs_no_draw() {
        assert_eq!(fit(&[w(0, 0)], 0, 8), [true]);
    }
}
