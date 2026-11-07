//! Closed point-navigation bundle descriptor. A source revision is a user claim,
//! not verified provenance. Content SHA-256 is integrity, not a signature.
use super::{
    io::{checked_relative, Snapshot},
    LAUNCHER, LAUNCHER_PATH, MAX_LAUNCHER, MAX_RUNTIME, MAX_SCENE, RUNTIME_PATH, SCENE_PATH,
};
use crate::authored::AuthoredPlayer;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(super) payload: Payload,
    content_sha256: String,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    version: u32,
    profile: Profile,
    target: Target,
    source_revision: Option<String>,
    pub(super) initial_checksum: u64,
    pub(super) terrain_asset_id: String,
    pub(super) terrain_revision: String,
    pub(super) graph_revision: String,
    pub(super) files: Vec<Descriptor>,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
enum Profile {
    NavigationYard3DPointV1,
    NavigationYard3DPatrolV1,
}
impl Profile {
    fn compiled()->Result<Self,String>{match super::compiled_profile()? {
        "NavigationYard3DPointV1"=>Ok(Self::NavigationYard3DPointV1),
        "NavigationYard3DPatrolV1"=>Ok(Self::NavigationYard3DPatrolV1),
        _=>Err("unsupported compiled navigation bundle profile".into()),
    }}
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
enum Target {
    #[serde(rename = "linux-x86_64")]
    LinuxX86_64,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Role {
    Scene,
    Terrain,
    Runtime,
    Launcher,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Descriptor {
    pub(super) path: String,
    pub(super) bytes: usize,
    pub(super) mode: u32,
    pub(super) sha256: String,
    role: Role,
}
pub(crate) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn digest(payload: &Payload) -> Result<String, String> {
    let bytes = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    hash.update(b"orrery.navigation.bundle.content.v1\0");
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Manifest {
    pub(crate) fn from_snapshots(
        scene: &Snapshot,
        terrain_relative: &str,
        terrain: &Snapshot,
        runtime: &Snapshot,
        source_revision: Option<String>,
        player: &AuthoredPlayer,
    ) -> Result<Self, String> {
        let profile=Profile::compiled()?;
        checked_relative(terrain_relative)?;
        if terrain_relative.len() > 256 || !terrain_relative.ends_with(".orrt") {
            return Err("terrain pin must be confined .orrt path".into());
        }
        let pin = player
            .frame()
            .singleton::<orr_remote::navigation_yard3d::NavigationScenePin>();
        if pin.source()? != terrain_relative || player.tick() != 0 {
            return Err("bundle requires admitted initial scene and unchanged terrain pin".into());
        }
        #[cfg(feature="terrain-patrol-bundle")]
        {
            use orr_games::navigation_yard3d_game::patrol;
            if patrol::read_run(player.frame())? != patrol::TerrainPatrolRun::default() {
                return Err("patrol bundle requires canonical zero initial runtime progress".into());
            }
            patrol::validate_frame(player.frame(),*pin)?;
        }
        let view = player.view()?;
        let loaded = orr_terrain::Terrain::load(&terrain.bytes).map_err(|e| e.to_string())?;
        if loaded.asset_id() != view.terrain.asset_id()
            || loaded.revision() != view.terrain.revision()
        {
            return Err("snapshot terrain differs from admitted player".into());
        }
        let descriptor = |path: &str, data: &[u8], mode, role| Descriptor {
            path: path.into(),
            bytes: data.len(),
            mode,
            sha256: hash(data),
            role,
        };
        let source_revision = source_revision.map(|s| s.trim().to_owned());
        let payload = Payload {
            version: 1,
            profile,
            target: Target::LinuxX86_64,
            source_revision,
            initial_checksum: player.checksum(),
            terrain_asset_id: view.terrain.asset_id().into(),
            terrain_revision: hex_revision(view.terrain.revision()),
            graph_revision: hex_revision(view.graph.revision()),
            files: vec![
                descriptor(SCENE_PATH, &scene.bytes, 0o644, Role::Scene),
                descriptor(
                    &format!("project/{terrain_relative}"),
                    &terrain.bytes,
                    0o644,
                    Role::Terrain,
                ),
                descriptor(RUNTIME_PATH, &runtime.bytes, 0o755, Role::Runtime),
                descriptor(LAUNCHER_PATH, LAUNCHER, 0o755, Role::Launcher),
            ],
        };
        let result = Self {
            content_sha256: digest(&payload)?,
            payload,
        };
        result.validate()?;
        Ok(result)
    }
    pub(crate) fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if bytes.len() > super::MAX_MANIFEST {
            return Err("bundle manifest exceeds64KiB".into());
        }
        Ok(bytes)
    }
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > super::MAX_MANIFEST {
            return Err("bundle manifest exceeds64KiB".into());
        }
        // Shape check only; decode the original bytes again below so duplicate
        // fields cannot be hidden by the intermediate Value's map semantics.
        let shape: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let object = shape.as_object().ok_or("manifest must be an object")?;
        let payload = object
            .get("payload")
            .and_then(|v| v.as_object())
            .ok_or("payload must be an object")?;
        if payload.get("profile").and_then(|v| v.as_str()) != Some(super::compiled_profile()?)
            || payload.get("target").and_then(|v| v.as_str()) != Some("linux-x86_64")
            || !payload
                .get("files")
                .and_then(|v| v.as_array())
                .is_some_and(|items| {
                    items.len() == 4
                        && items
                            .iter()
                            .all(|v| v.is_object() && v.get("role").is_some_and(|r| r.is_string()))
                })
        {
            return Err("manifest profile, target or descriptor shape is invalid".into());
        }
        let result: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        result.validate()?;
        Ok(result)
    }
    fn validate(&self) -> Result<(), String> {
        let p = &self.payload;
        if p.profile != Profile::compiled()?
            || p.version != 1
            || p.files.len() != 4
            || p.terrain_asset_id.is_empty()
            || p.terrain_asset_id.len() > 256
            || !valid_hash(&p.terrain_revision)
            || !valid_hash(&p.graph_revision)
            || !valid_hash(&self.content_sha256)
        {
            return Err("unsupported or malformed bundle contract".into());
        }
        if p.source_revision
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 4096 || s.trim() != s)
        {
            return Err("source revision claim must be trimmed and1..4096UTF8bytes".into());
        }
        let mut paths = BTreeSet::new();
        for (index, file) in p.files.iter().enumerate() {
            let path = checked_relative(&file.path)?;
            let (role, mode, max) = match index {
                0 => (Role::Scene, 0o644, MAX_SCENE),
                1 => (Role::Terrain, 0o644, orr_terrain::MAX_FILE_BYTES),
                2 => (Role::Runtime, 0o755, MAX_RUNTIME),
                _ => (Role::Launcher, 0o755, MAX_LAUNCHER),
            };
            if file.role != role
                || file.mode != mode
                || file.bytes == 0
                || file.bytes > max
                || !valid_hash(&file.sha256)
                || !paths.insert(path.clone())
            {
                return Err("invalid bundle file descriptor".into());
            }
            match index {
                0 if file.path != SCENE_PATH => return Err("unexpected scene path".into()),
                1 => {
                    let relative = file
                        .path
                        .strip_prefix("project/")
                        .ok_or("terrain must be underproject")?;
                    checked_relative(relative)?;
                    if relative.len() > 256 || !relative.ends_with(".orrt") {
                        return Err("unexpected terrain path".into());
                    }
                }
                2 if file.path != RUNTIME_PATH => return Err("unexpected runtime path".into()),
                3 if file.path != LAUNCHER_PATH
                    || file.bytes != LAUNCHER.len()
                    || file.sha256 != hash(LAUNCHER) =>
                {
                    return Err("unexpected launcher bytes".into())
                }
                _ => {}
            }
        }
        for path in &paths {
            if paths
                .iter()
                .any(|other| path != other && other.starts_with(path))
            {
                return Err("bundle path prefix collision".into());
            }
        }
        if digest(p)? != self.content_sha256 {
            return Err("bundle canonical content digest mismatch".into());
        }
        Ok(())
    }
}
pub(super) fn hex_revision(value: [u8; 32]) -> String {
    value.iter().map(|b| format!("{b:02x}")).collect()
}
#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "profile_tests.rs"]
mod profile_tests;
