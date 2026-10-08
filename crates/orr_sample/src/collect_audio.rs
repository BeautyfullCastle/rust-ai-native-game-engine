//! Closed Collect pickup audio authoring and owned, device-independent admission.
//! No fixture gameplay, filesystem cooker, device or simulation state is involved.
use orr_asset::{AssetRef, Domain, Manifest};
use orr_audio::Clip;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const MAX_BYTES: usize = 4096;
pub const MAX_PACKAGE_FILES: usize = 64;
pub const MAX_PACKAGE_BYTES: u64 = orr_asset::MAX_VIEW_PAYLOAD_BYTES + 192 * 1024;
pub const MAX_DECODED_BYTES: usize = 4 * 1024 * 1024;
pub const DEFAULT_PACKAGE: &str = "collect-audio-v1";
pub const DEFAULT_MANIFEST: &str = "cooked/view.manifest.bin";
pub const DEFAULT_ASSET: &str = "a_0000000000003001";
pub const ALTERNATE_ASSET: &str = "a_0000000000003002";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Binding {
    pub package: String,
    pub manifest: String,
    pub asset: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Document {
    pub version: u32,
    pub pickup: Binding,
    /// Integer thousandths of unity; never a simulation value.
    pub gain: u16,
    pub mute: bool,
}

// Keep typed object decoding: Value would erase duplicate keys; ordinary derived
// structs also accept positional arrays. Neither is admitted by this document.
fn object<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    struct Visitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
        type Value = T;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a Collect audio JSON object")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<T, M::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    d.deserialize_map(Visitor(std::marker::PhantomData))
}
impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            package: String,
            manifest: String,
            asset: String,
        }
        let w: Wire = object(d)?;
        Ok(Self {
            package: w.package,
            manifest: w.manifest,
            asset: w.asset,
        })
    }
}
impl<'de> Deserialize<'de> for Document {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            version: u32,
            pickup: Binding,
            gain: u16,
            mute: bool,
        }
        let w: Wire = object(d)?;
        Ok(Self {
            version: w.version,
            pickup: w.pickup,
            gain: w.gain,
            mute: w.mute,
        })
    }
}
fn portable(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 240
        && path.is_ascii()
        && path.split('/').count() <= 16
        && path.split('/').all(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with('.')
                && part.len() <= 100
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
                && !["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit())
        })
}
impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Collect audio document exceeds 4096 bytes".into());
        }
        let document: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("Collect audio JSON: {e}"))?;
        document.validate()?;
        Ok(document)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        Self::parse(bytes)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if bytes.len() > MAX_BYTES {
            return Err("Collect audio document exceeds 4096 bytes".into());
        }
        Ok(bytes)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("Collect audio version must be 1".into());
        }
        if self.gain > 1000 {
            return Err("Collect audio gain must be an integer in 0..=1000".into());
        }
        let package = &self.pickup.package;
        if package.is_empty()
            || package.len() > 64
            || !package
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-_".contains(&c))
        {
            return Err("Collect audio package name is invalid".into());
        }
        if !portable(&self.pickup.manifest) {
            return Err("Collect audio manifest must be a portable package-relative path".into());
        }
        let id: AssetRef = self
            .pickup
            .asset
            .parse()
            .map_err(|_| "Collect audio asset must be a canonical a_ GUID")?;
        if id.is_null() {
            return Err("Collect audio asset cannot be null".into());
        }
        Ok(())
    }
    pub fn default_collect() -> Self {
        Self {
            version: 1,
            pickup: Binding {
                package: DEFAULT_PACKAGE.into(),
                manifest: DEFAULT_MANIFEST.into(),
                asset: DEFAULT_ASSET.into(),
            },
            gain: 700,
            mute: false,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreloadStats {
    pub records: usize,
    pub manifest_bytes: usize,
    pub cooked_bytes: usize,
    /// Retained stereo f32 PCM, excluding object, mixer and allocator overhead.
    pub decoded_bytes: usize,
    pub package_bytes: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableClip {
    pub asset: String,
    pub frames: u32,
    pub payload_path: String,
}
#[derive(Clone)]
pub struct PreparedAudio {
    pub path: PathBuf,
    /// Exact admitted sidecar bytes, retained for safe export reconciliation.
    pub bytes: Vec<u8>,
    pub document: Document,
    pub package: orr_package::PackageSnapshot,
    pub bank: BTreeMap<String, Clip>,
    pub stats: PreloadStats,
    pub available: Vec<AvailableClip>,
}
fn bounded(total: usize, extra: usize, limit: usize) -> Result<usize, String> {
    total
        .checked_add(extra)
        .filter(|n| *n <= limit)
        .ok_or_else(|| "Collect audio preload budget exceeded".into())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn validate_entry(project: &orr_package::Project, relative: &str) -> Result<(), String> {
    let manifest = project
        .manifest()
        .ok_or("Collect audio requires project metadata")?;
    let entry = manifest
        .entry
        .as_ref()
        .ok_or("Collect audio requires a project entry")?;
    if !matches!(manifest.schema, 2 | 3)
        || entry.game != orr_package::ProjectGame::CollectDodgeV1
        || entry.audio.as_deref() != Some(relative)
        || !portable(relative)
        || relative.contains('/')
        || relative.starts_with('.')
    {
        return Err("Collect audio requires the exact declared Collect-only sidecar".into());
    }
    Ok(())
}
impl PreparedAudio {
    pub fn open(root: impl AsRef<Path>, relative: &str) -> Result<Self, String> {
        let mut runtime = crate::collect_project::sprite_runtime(
            crate::collect_project::compiled_sprite_support(),
        );
        runtime.capabilities.insert("collect-audio".into());
        let project = orr_package::Project::open(root, runtime).map_err(|e| e.to_string())?;
        Self::load(&project, relative)
    }
    pub fn load(project: &orr_package::Project, relative: &str) -> Result<Self, String> {
        validate_entry(project, relative)?;
        let bytes = project
            .read_file_bounded(relative, MAX_BYTES as u64)
            .map_err(|e| e.to_string())?;
        Self::from_bytes(project, relative, &bytes)
    }
    /// Decode authored draft bytes against one owned installed-package snapshot.
    /// All object/header/digest/cumulative checks precede the first PCM allocation.
    pub fn from_bytes(
        project: &orr_package::Project,
        relative: &str,
        bytes: &[u8],
    ) -> Result<Self, String> {
        validate_entry(project, relative)?;
        let document = Document::parse(bytes)?;
        let package = project
            .read_package_bounded(
                &document.pickup.package,
                MAX_PACKAGE_FILES,
                orr_asset::MAX_VIEW_PAYLOAD_BYTES,
                MAX_PACKAGE_BYTES,
            )
            .map_err(|e| format!("Collect audio package: {e}"))?;
        if !package
            .locked
            .manifest
            .capabilities
            .contains("collect-audio")
        {
            return Err("Collect audio package must declare collect-audio capability".into());
        }
        let manifest_bytes = package
            .files
            .get(&document.pickup.manifest)
            .ok_or("Collect audio manifest absent from package")?;
        let manifest = Manifest::decode(manifest_bytes, Domain::View)
            .map_err(|e| format!("Collect audio manifest: {e:?}"))?;
        if manifest.entries().len() == 0 {
            return Err("Collect audio manifest must contain a clip".into());
        }
        let id = document
            .pickup
            .asset
            .parse::<AssetRef>()
            .map_err(|e| format!("Collect audio asset: {e:?}"))?;
        manifest
            .find(id)
            .map_err(|e| format!("Collect pickup asset: {e:?}"))?;
        let parent = document
            .pickup
            .manifest
            .rsplit_once('/')
            .map(|(parent, _)| format!("{parent}/"))
            .unwrap_or_default();
        let mut stats = PreloadStats {
            records: manifest.entries().len(),
            manifest_bytes: manifest_bytes.len(),
            package_bytes: package.manifest_bytes.len()
                + package.files.values().map(Vec::len).sum::<usize>(),
            ..PreloadStats::default()
        };
        let mut available = Vec::with_capacity(stats.records);
        for entry in manifest.entries() {
            let payload_path = format!("{parent}objects/{}.bin", hex(&entry.payload_sha256));
            let payload = package
                .files
                .get(&payload_path)
                .ok_or_else(|| format!("Collect audio object absent: {payload_path}"))?;
            if payload.len() as u64 != entry.payload_len
                || <[u8; 32]>::from(Sha256::digest(payload)) != entry.payload_sha256
            {
                return Err(format!(
                    "Collect audio payload length/hash mismatch: {}",
                    entry.id
                ));
            }
            let frames = orr_audio::pcm16::validate(payload)
                .map_err(|e| format!("Collect audio PCM: {e}"))?;
            stats.cooked_bytes = bounded(
                stats.cooked_bytes,
                payload.len(),
                orr_asset::MAX_VIEW_PAYLOAD_BYTES as usize,
            )?;
            stats.decoded_bytes =
                bounded(stats.decoded_bytes, frames as usize * 8, MAX_DECODED_BYTES)?;
            available.push(AvailableClip {
                asset: entry.id.to_string(),
                frames,
                payload_path,
            });
        }
        let mut bank = BTreeMap::new();
        for clip in &available {
            let pcm = orr_audio::pcm16::decode(&package.files[&clip.payload_path])
                .map_err(|e| e.to_string())?;
            bank.insert(clip.asset.clone(), pcm);
        }
        Ok(Self {
            path: project.root().join(relative),
            bytes: bytes.to_vec(),
            document,
            package,
            bank,
            stats,
            available,
        })
    }
    /// Change only the assignment/gain/mute using the existing validated owned bank.
    /// Changing package or manifest requires a fresh admission transaction.
    pub fn with_document(&self, next: Document) -> Result<Self, String> {
        next.validate()?;
        if next.pickup.package != self.document.pickup.package
            || next.pickup.manifest != self.document.pickup.manifest
            || !self.bank.contains_key(&next.pickup.asset)
        {
            return Err("Collect audio edit must select an already admitted clip".into());
        }
        let bytes = next.to_bytes()?;
        let mut prepared = self.clone();
        prepared.document = next;
        prepared.bytes = bytes;
        Ok(prepared)
    }
    pub fn pickup_clip(&self) -> Result<Clip, String> {
        self.bank
            .get(&self.document.pickup.asset)
            .ok_or_else(|| "Collect pickup clip is absent from the admitted bank".to_string())?
            .clone()
            .with_gain_milli(self.document.gain)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
#[path = "collect_audio_tests.rs"]
pub(crate) mod tests;
