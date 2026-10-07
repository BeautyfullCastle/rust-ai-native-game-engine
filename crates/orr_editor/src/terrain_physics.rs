//! View-only cache derived from an admitted immutable host snapshot.
use orr_bridge::FrameView;
use orr_model::StaticModel;
use orr_remote::terrain_yard3d::{terrain_from_view, TerrainScenePin, TerrainStepStatus};
use std::sync::Arc;

#[derive(Default)]
pub struct TerrainPhysicsView {
    pub admitted: bool,
    pub source: String,
    pub identity: String,
    pub revision: Option<[u8; 32]>,
    pub model: Option<Arc<StaticModel>>,
    pub error: Option<String>,
}
impl TerrainPhysicsView {
    pub fn update(&mut self, frame: FrameView<'_>) {
        let pin = frame.singleton::<TerrainScenePin>();
        let status = frame.singleton::<TerrainStepStatus>();
        // Source metadata belongs to the current scene pin. Identical asset
        // bytes may be admitted from another path without changing mesh identity.
        self.source = pin.source().unwrap_or_default().to_string();
        let runtime_error = (status.failed != 0).then(|| {
            format!(
                "Terrain physics stopped at tick {}: {}. Stop play and reload a valid scene",
                status.failed_tick,
                status.message()
            )
        });
        if self.revision != Some(pin.revision) || !self.admitted {
            let result = (|| {
                let terrain = terrain_from_view(frame)?;
                let model = orr_terrain_view::to_static_model(&terrain, &[])
                    .map_err(|e| e.to_string())?
                    .map(Arc::new);
                Ok::<_, String>((terrain, model))
            })();
            match result {
                Ok((terrain, model)) => {
                    self.admitted = true;
                    self.identity = terrain.asset_id().to_string();
                    self.revision = Some(terrain.revision());
                    self.model = model;
                    self.error = runtime_error;
                }
                Err(error) => {
                    self.admitted = false;
                    self.revision = None;
                    self.model = None;
                    self.error = Some(error);
                }
            }
        } else {
            self.error = runtime_error;
        }
    }

    pub fn revision_text(&self) -> String {
        self.revision
            .map(|hash| hash.iter().map(|byte| format!("{byte:02x}")).collect())
            .unwrap_or_default()
    }
}
