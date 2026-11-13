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

/// Width, depth and 0..1 heights of a heightmap.
pub type Heights = (u32, u32, Arc<Vec<f32>>);

struct Entry {
    /// What the mesh was built from: the terrain's numbers and the file's time and size.
    key: (String, [u32; 2], u32, u32, Option<(SystemTime, u64)>),
    model: Result<(Arc<orr_model::StaticModel>, Heights), String>,
    checked: Instant,
}

/// A decoded heightmap and the file time and size it was read at.
type Decoded = (Option<(SystemTime, u64)>, Result<(u32, u32, Arc<Vec<f32>>), String>);

/// One drawable terrain: its entity, center, mesh and heights.
pub struct TerrainModel {
    pub entity: Entity,
    pub center: [f32; 3],
    pub model: Arc<orr_model::StaticModel>,
    pub heights: Heights,
}

/// Built terrain meshes, by entity.
#[derive(Default)]
pub struct TerrainModels {
    entries: BTreeMap<(u32, u32), Entry>,
    /// Decoded heightmaps by path, so changing a terrain's numbers does not read the file again.
    decoded: BTreeMap<PathBuf, Decoded>,
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn decode(path: &Path, shown: &str) -> Result<(u32, u32, Arc<Vec<f32>>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{shown}: {e}"))?;
    let (width, height, heights) = orr_gfx_view::terrain_mesh::read_heightmap(&bytes).map_err(|e| format!("{shown}: {e}"))?;
    Ok((width, height, Arc::new(heights)))
}

/// The mesh and the drawn grid heights of `terrain` from a decoded heightmap.
fn build(decoded: &(u32, u32, Arc<Vec<f32>>), terrain: &TerrainRef) -> Result<(Arc<orr_model::StaticModel>, Heights), String> {
    let (width, height, heights) = decoded;
    let model = orr_gfx_view::terrain_mesh::terrain_model(terrain, *width, *height, heights)?;
    let (grid, samples) = orr_gfx_view::terrain_mesh::terrain_grid(terrain, *width, *height, heights);
    Ok((Arc::new(model), (grid, grid, Arc::new(samples))))
}

impl TerrainModels {
    /// The model of every terrain with a heightmap, by entity, and the first
    /// problem found. `scene` is the open scene file; paths start from its folder.
    pub fn refresh(&mut self, scene: Option<&Path>, terrains: &[TerrainRef]) -> (Vec<TerrainModel>, Option<String>) {
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
            // The path came from the frame; simulation systems can write any bytes there.
            if orr_reflect::TypeDesc::asset(orr_gfx::TERRAIN_PATH_BYTES, "heightmap").check(&orr_reflect::Value::Text(terrain.heightmap.clone()), true).is_err() {
                problem.get_or_insert_with(|| format!("'{}' is not a heightmap path next to the scene", terrain.heightmap));
                continue;
            }
            let path = root.join(&terrain.heightmap);
            let numbers = (terrain.heightmap.clone(), terrain.size.map(f32::to_bits), terrain.height.to_bits(), terrain.resolution);
            let fresh = self.entries.get(&id).is_some_and(|e| (e.key.0.clone(), e.key.1, e.key.2, e.key.3) == numbers && now.duration_since(e.checked) < CHECK_INTERVAL);
            if !fresh {
                let current = stamp(&path);
                let key = (numbers.0, numbers.1, numbers.2, numbers.3, current);
                match self.entries.get_mut(&id) {
                    Some(entry) if entry.key == key => entry.checked = now,
                    _ => {
                        let fresh = self.decoded.get(&path).is_some_and(|(stamp, _)| *stamp == current);
                        if !fresh {
                            self.decoded.insert(path.clone(), (current, decode(&path, &terrain.heightmap)));
                        }
                        let model = self.decoded[&path].1.clone().and_then(|d| build(&d, terrain));
                        self.entries.insert(id, Entry { key, model, checked: now });
                    }
                }
            }
            match &self.entries[&id].model {
                Ok((model, heights)) => out.push(TerrainModel { entity: terrain.entity, center: terrain.center, model: model.clone(), heights: heights.clone() }),
                Err(error) => {
                    problem.get_or_insert_with(|| error.clone());
                }
            }
        }
        self.entries.retain(|id, _| seen.contains(id));
        (out, problem)
    }
}
