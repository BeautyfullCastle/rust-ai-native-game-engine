//! Immutable saved-source binding for asynchronous export. Trusted quiescent
//! ancestry is required; snapshots do not constitute a filesystem sandbox.
use super::io::{checked_path, Snapshot};
use crate::authored::AuthoredPlayer;
use orr_remote::navigation_yard3d::NavigationScenePin;
use std::path::{Path, PathBuf};

pub struct SavedSource {
    path: PathBuf,
    scene: Snapshot,
    terrain: Snapshot,
    initial: Vec<u8>,
    checksum: u64,
}
impl SavedSource {
    pub fn capture(scene: &Path) -> Result<Self, String> {
        if orr_games::navigation_yard3d_game::TERRAIN_PATROL_ENABLED {
            return Err("legacy navigation bundle does not support terrain patrol profile".into());
        }
        let path = checked_path(scene)?;
        let scene = Snapshot::read(&path, super::MAX_SCENE)?;
        let player = AuthoredPlayer::open(&path)?;
        let relative = player.frame().singleton::<NavigationScenePin>().source()?;
        let terrain = Snapshot::read(
            &path.parent().ok_or("scene parent missing")?.join(relative),
            orr_terrain::MAX_FILE_BYTES,
        )?;
        let result = Self {
            path,
            scene,
            terrain,
            initial: player.frame().to_bytes(),
            checksum: player.checksum(),
        };
        result.recheck()?;
        // Bind the actual admission to the terrain snapshot captured afterwards.
        if AuthoredPlayer::open(&result.path)?.frame().to_bytes() != result.initial {
            return Err("saved navigation admission changed during capture".into());
        }
        result.recheck()?;
        Ok(result)
    }
    pub fn scene_path(&self) -> &Path {
        &self.path
    }
    pub fn scene_bytes(&self) -> &[u8] {
        &self.scene.bytes
    }
    pub fn initial_frame_bytes(&self) -> &[u8] {
        &self.initial
    }
    pub fn initial_checksum(&self) -> u64 {
        self.checksum
    }
    /// Revalidate the original file identities and bytes. This is not a lock,
    /// runtime trust token, or guarantee against later concurrent changes.
    pub fn recheck(&self) -> Result<(), String> {
        self.scene.recheck()?;
        self.terrain.recheck()
    }
}
#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
