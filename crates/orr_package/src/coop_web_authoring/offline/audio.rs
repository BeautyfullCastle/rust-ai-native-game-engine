//! Pure admission of snapshotted browser PCM. Unknown JSON fields are retained.
pub(super) use crate::coop_audio_admission::CUES;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::collections::BTreeMap;
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
    let bank = crate::coop_audio_admission::admit(manifest_bytes, objects)?;
    let mut encoded_objects = Vec::new();
    for record in bank.records() {
        let file = format!("{}.pcm", record.cue());
        encoded_objects.push(format!(
            "{}:{}",
            serde_json::to_string(&file).map_err(|e| e.to_string())?,
            serde_json::to_string(&STANDARD.encode(record.pcm_bytes()))
                .map_err(|e| e.to_string())?
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
