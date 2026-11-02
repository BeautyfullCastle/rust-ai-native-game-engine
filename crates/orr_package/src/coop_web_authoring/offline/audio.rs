//! Pure admission of snapshotted browser PCM. Unknown JSON fields are retained.
use base64::{engine::general_purpose::STANDARD, Engine};
use orr_asset_cook::authoring::{validate_payload, AssetType};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
pub(super) const CUES: [&str; 10] = [
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
pub(super) struct PreparedAudio {
    embedded: String,
}
impl PreparedAudio {
    pub(super) fn embedded(&self) -> &str {
        &self.embedded
    }
}
pub(super) fn prepare(
    manifest_bytes: &[u8],
    objects: &BTreeMap<String, Vec<u8>>,
) -> Result<PreparedAudio, String> {
    // Shared strict visitor rejects duplicates at every level, nonfinite and excessive depth.
    let manifest = super::super::manifest::parse_bounded(manifest_bytes, 128 * 1024)?;
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
    let mut encoded_objects = Vec::new();
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
        encoded_objects.push(format!(
            "{}:{}",
            serde_json::to_string(&file).map_err(|e| e.to_string())?,
            serde_json::to_string(&STANDARD.encode(bytes)).map_err(|e| e.to_string())?
        ));
    }
    // Keep original ordered/unknown fields and number spelling, not Value reserialization.
    // Raw JSON is already strict UTF-8/finite/depth bounded; escape every literal '<'.
    let raw = std::str::from_utf8(manifest_bytes)
        .map_err(|e| e.to_string())?
        .replace('<', "\\u003c");
    let embedded = format!("<script type=\"application/json\" id=\"authored-audio-bank\">{{\"manifest\":{raw},\"objects\":{{{}}}}}</script>", encoded_objects.join(","));
    Ok(PreparedAudio { embedded })
}
#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;
