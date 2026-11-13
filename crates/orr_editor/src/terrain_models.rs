//! Terrain entities of the open scene as drawable models. Each
//! `orr_gfx::Terrain` names a heightmap PNG next to the scene file; the mesh
//! is built once and again only when the file or the terrain's numbers change.
#![allow(clippy::disallowed_types)]

use orr_ecs::Entity;
use orr_gfx_view::TerrainRef;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// How often a heightmap file is looked at again.
const CHECK_INTERVAL: Duration = Duration::from_millis(500);

struct Entry {
    /// What the mesh was built from: the terrain's numbers and the file's time and size.
    key: (String, [u32; 2], u32, u32, Option<(SystemTime, u64)>),
    model: Result<Arc<orr_model::StaticModel>, String>,
    checked: Instant,
}

/// Built terrain meshes, by entity.
#[derive(Default)]
pub struct TerrainModels {
    entries: BTreeMap<(u32, u32), Entry>,
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn build(path: &Path, terrain: &TerrainRef) -> Result<Arc<orr_model::StaticModel>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", terrain.heightmap))?;
    let (width, height, heights) = orr_gfx_view::terrain_mesh::read_heightmap(&bytes).map_err(|e| format!("{}: {e}", terrain.heightmap))?;
    orr_gfx_view::terrain_mesh::terrain_model(terrain, width, height, &heights).map(Arc::new)
}

impl TerrainModels {
    /// The model of every terrain with a heightmap, by entity, and the first
    /// problem found. `scene` is the open scene file; paths start from its folder.
    pub fn refresh(&mut self, scene: Option<&Path>, terrains: &[TerrainRef]) -> (Vec<(Entity, [f32; 3], Arc<orr_model::StaticModel>)>, Option<String>) {
        let now = Instant::now();
        let root: Option<PathBuf> = scene.and_then(Path::parent).map(Path::to_path_buf);
        let mut out = Vec::new();
        let mut problem = None;
        let mut seen = Vec::new();
        for terrain in terrains.iter().filter(|t| !t.heightmap.is_empty()) {
            let id = (terrain.entity.index, terrain.entity.version);
            seen.push(id);
            let Some(root) = &root else {
                problem.get_or_insert_with(|| "Save the scene to a file first: heightmaps are read from its folder".to_string());
                continue;
            };
            let path = root.join(&terrain.heightmap);
            let numbers = (terrain.heightmap.clone(), terrain.size.map(f32::to_bits), terrain.height.to_bits(), terrain.resolution);
            let fresh = self.entries.get(&id).is_some_and(|e| (e.key.0.clone(), e.key.1, e.key.2, e.key.3) == numbers && now.duration_since(e.checked) < CHECK_INTERVAL);
            if !fresh {
                let current = stamp(&path);
                let key = (numbers.0, numbers.1, numbers.2, numbers.3, current);
                match self.entries.get_mut(&id) {
                    Some(entry) if entry.key == key => entry.checked = now,
                    _ => {
                        let model = build(&path, terrain);
                        self.entries.insert(id, Entry { key, model, checked: now });
                    }
                }
            }
            match &self.entries[&id].model {
                Ok(model) => out.push((terrain.entity, terrain.center, model.clone())),
                Err(error) => {
                    problem.get_or_insert_with(|| error.clone());
                }
            }
        }
        self.entries.retain(|id, _| seen.contains(id));
        (out, problem)
    }
}
