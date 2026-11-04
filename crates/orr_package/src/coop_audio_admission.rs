//! Shared pure admission for authored CoopDefense audio.
use orr_asset_cook::authoring::{validate_payload, AssetType};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
pub const CUES: [&str; 10] = [
    "attack-light",
    "attack-heavy",
    "attack-magic",
    "impact-light",
    "impact-heavy",
    "summon",
    "merge",
    "ui-confirm",
    "ui-reject",
    "death",
];
/// An entirely validated bank. Records preserve manifest order.
#[derive(Clone, Debug)]
pub struct AdmittedBank {
    records: Vec<AdmittedCue>,
}
impl AdmittedBank {
    pub fn records(&self) -> &[AdmittedCue] {
        &self.records
    }
}
/// A validated 48 kHz mono PCM16 object; fields cannot be forged by callers.
#[derive(Clone, Debug)]
pub struct AdmittedCue {
    cue: &'static str,
    pcm: Vec<u8>,
    frames: u32,
}
impl AdmittedCue {
    pub fn cue(&self) -> &str {
        self.cue
    }
    /// Full cooked bytes, including the sample-rate and frame-count header.
    pub fn pcm_bytes(&self) -> &[u8] {
        &self.pcm
    }
    pub fn frames(&self) -> u32 {
        self.frames
    }
}
/// Admit a complete fixed bank without IO or presentation dependencies.
/// Unknown JSON fields and unrelated supplied objects retain the web admission policy.
pub fn admit(
    manifest_bytes: &[u8],
    objects: &BTreeMap<String, Vec<u8>>,
) -> Result<AdmittedBank, String> {
    // Shared strict visitor rejects duplicates at every level, nonfinite and excessive depth.
    let manifest = crate::strict_json::parse_bounded(manifest_bytes, 128 * 1024)?;
    if manifest.get("version").and_then(|v| v.as_u64()) != Some(1)
        || manifest.get("sampleRate").and_then(|v| v.as_u64()) != Some(48000)
    {
        return Err("Unsupported audio bank".into());
    }
    let records = manifest
        .get("cues")
        .and_then(|v| v.as_array())
        .filter(|r| r.len() == CUES.len())
        .ok_or("Unsupported audio bank")?;
    let mut seen = BTreeSet::new();
    let mut cooked = 0u64;
    let mut decoded = 0u64;
    let mut admitted = Vec::new();
    for record in records {
        let cue = record
            .get("cue")
            .and_then(|v| v.as_str())
            .ok_or("Invalid audio cue")?;
        let index = CUES
            .iter()
            .position(|c| *c == cue)
            .ok_or("Invalid audio cue")?;
        let frames = record
            .get("frames")
            .and_then(|v| v.as_u64())
            .filter(|f| (1..=48000).contains(f))
            .ok_or("Invalid audio frame count")?;
        let file = format!("{cue}.pcm");
        let id = format!("a_000000005444{:04x}", index + 1);
        let size = 8 + frames * 2;
        let hash = record
            .get("sha256")
            .and_then(|v| v.as_str())
            .ok_or("Invalid audio digest")?;
        if !seen.insert(cue)
            || record.get("file").and_then(|v| v.as_str()) != Some(file.as_str())
            || record.get("assetId").and_then(|v| v.as_str()) != Some(id.as_str())
            || record.get("bytes").and_then(|v| v.as_u64()) != Some(size)
            || hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("Invalid audio record".into());
        }
        cooked += size;
        decoded += frames * 8;
        if cooked > 2 * 1024 * 1024 || decoded > 4 * 1024 * 1024 {
            return Err("Audio bank exceeds budget".into());
        }
        let bytes = objects.get(&file).ok_or("Missing PCM object")?;
        if bytes.len() as u64 != size {
            return Err("Invalid PCM byte count".into());
        }
        let actual_hash: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if actual_hash != hash
            || bytes[..4] != 48000u32.to_le_bytes()
            || bytes[4..8] != (frames as u32).to_le_bytes()
        {
            return Err("Invalid PCM object".into());
        }
        validate_payload(AssetType::Impact, bytes)
            .map_err(|e| format!("Invalid PCM object: {e}"))?;
        admitted.push(AdmittedCue {
            cue: CUES[index],
            pcm: bytes.clone(),
            frames: frames as u32,
        });
    }
    Ok(AdmittedBank { records: admitted })
}

/// Snapshot and admit a bank directory on Linux without following symlinks.
/// Requires an absolute path and trusted, stable ancestry. Only `bank.json`
/// and the ten fixed PCM names are read; no manifest-directed paths are opened.
#[cfg(all(feature = "coop-audio-load", target_os = "linux"))]
pub fn load_bank(path: &std::path::Path) -> Result<AdmittedBank, String> {
    use crate::authoring_io::{directory, read_at, recheck_directory};
    use std::{ffi::OsStr, path::Component};
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().len() > 4096
        || path
            .components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
    {
        return Err("audio bank requires a bounded absolute directory path".into());
    }
    let dir = directory(path)?;
    let manifest = read_at(&dir, OsStr::new("bank.json"), 128 * 1024)?;
    let mut objects = BTreeMap::new();
    for cue in CUES {
        let name = format!("{cue}.pcm");
        let bytes = read_at(&dir, OsStr::new(&name), 96008)?;
        objects.insert(name, bytes);
    }
    recheck_directory(path, &dir)?;
    admit(&manifest, &objects)
}

#[cfg(test)]
#[path = "coop_audio_admission_tests.rs"]
mod tests;
