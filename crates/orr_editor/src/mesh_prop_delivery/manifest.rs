//! Independent closed headless profile; hashes provide integrity, not provenance.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
pub const MANIFEST_NAME: &str = "mesh.prop.bundle.json";
pub const RUNTIME_NAME: &str = "orr_mesh_prop_player";
pub const CAPABILITY: &str = "ORR_MESH_PROP_HEADLESS_V1\n";
pub const MAX_MANIFEST: usize = 16 * 1024;
pub const MAX_SCENE: usize = 1024 * 1024;
pub const MAX_PROPS: usize = 16 * 1024 * 1024;
pub const MAX_SETTINGS: usize = 64 * 1024;
pub const MAX_RUNTIME: usize = 256 * 1024 * 1024;
pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn scene_name(name: &str) -> Result<(), String> {
    if name.len() > 180
        || !name.ends_with(".scene.yaml")
        || name.starts_with('.')
        || name.chars().any(char::is_control)
        || name.contains('/')
    {
        return Err(
            "scene must retain a safe basename ending .scene.yaml (at most 180 bytes)".into(),
        );
    }
    super::io::checked_relative(name)?;
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub name: String,
    pub bytes: usize,
    pub sha256: String,
    pub mode: u32,
}
impl Descriptor {
    pub(crate) fn new(name: String, bytes: &[u8], mode: u32) -> Self {
        Self {
            name,
            bytes: bytes.len(),
            sha256: hash(bytes),
            mode,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub profile: String,
    pub target: String,
    pub capability: String,
    pub scene: String,
    pub checksum: u64,
    pub graph_revision: [u8; 32],
    pub files: Vec<Descriptor>,
}
impl Manifest {
    pub(crate) fn new(
        scene: String,
        checksum: u64,
        graph_revision: [u8; 32],
        files: Vec<Descriptor>,
    ) -> Self {
        Self {
            schema: 1,
            profile: "MeshNavigationShadedPropsHeadlessV1".into(),
            target: "linux-x86_64".into(),
            capability: CAPABILITY.trim_end().into(),
            scene,
            checksum,
            graph_revision,
            files,
        }
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_MANIFEST {
            return Err("manifest exceeds 16 KiB".into());
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        value.validate()?;
        Ok(value)
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MANIFEST {
            return Err("manifest exceeds 16 KiB".into());
        }
        Ok(bytes)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || self.profile != "MeshNavigationShadedPropsHeadlessV1"
            || self.target != "linux-x86_64"
            || self.capability != CAPABILITY.trim_end()
        {
            return Err("unsupported mesh prop delivery profile".into());
        }
        scene_name(&self.scene)?;
        if !(3..=5).contains(&self.files.len()) {
            return Err("invalid descriptor count".into());
        }
        let mut names = BTreeSet::new();
        for f in &self.files {
            let (max, mode) = self.limit(&f.name)?;
            if !names.insert(f.name.as_str())
                || f.bytes == 0
                || f.bytes > max
                || f.mode != mode
                || !valid_hash(&f.sha256)
            {
                return Err("invalid or duplicate descriptor".into());
            }
        }
        for name in [
            self.scene.as_str(),
            format!("{}.mesh-props.json", self.scene).as_str(),
            RUNTIME_NAME,
        ] {
            if !names.contains(name) {
                return Err("required bundle member missing".into());
            }
        }
        Ok(())
    }
    pub(crate) fn limit(&self, name: &str) -> Result<(usize, u32), String> {
        if name == self.scene {
            Ok((MAX_SCENE, 0o644))
        } else if name == format!("{}.mesh-props.json", self.scene) {
            Ok((MAX_PROPS, 0o644))
        } else if name == format!("{}.mesh-camera.json", self.scene)
            || name == format!("{}.mesh-lighting.json", self.scene)
        {
            Ok((MAX_SETTINGS, 0o644))
        } else if name == RUNTIME_NAME {
            Ok((MAX_RUNTIME, 0o755))
        } else {
            Err("unsupported bundle member".into())
        }
    }
}
