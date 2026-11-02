//! Fixed trusted Node gate; no user script/executable input, no structural-only fallback.
use super::node_gate::execute;
use super::prepared::{digest, Prepared};
use serde::{Deserialize, Serialize};
use std::{process::Command, time::Duration};
const VERIFIER: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/validate_coop_defense_animation.mjs"
));
const SAMPLER: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../orr_coop_defense_web/web/combat-presentation.mjs"
));
const CAP: usize = 64 * 1024;
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u32,
    #[serde(rename = "manifestBytes")]
    manifest_bytes: usize,
    #[serde(rename = "manifestSha256")]
    manifest_sha256: String,
}
pub(super) struct Verified {
    prepared: Prepared,
}
impl Verified {
    pub(super) fn prepared(&self) -> &Prepared {
        &self.prepared
    }
}
fn admit(prepared: Prepared, output: &[u8]) -> Result<Verified, String> {
    if output.len() > CAP {
        return Err("sampler receipt exceeds bound".into());
    }
    let receipt: Receipt = serde_json::from_slice(output).map_err(|e| e.to_string())?;
    let expected = Receipt {
        version: 1,
        manifest_bytes: prepared.bytes().len(),
        manifest_sha256: digest(prepared.bytes()),
    };
    if receipt != expected {
        return Err("sampler receipt differs from immutable manifest".into());
    }
    Ok(Verified { prepared })
}
pub(super) fn verify(prepared: Prepared) -> Result<Verified, String> {
    let temp = tempfile::Builder::new()
        .prefix("orr-web-animation-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let manifest = temp.path().join("manifest.json");
    let verifier = temp.path().join("verify.mjs");
    let sampler = temp.path().join("combat-presentation.mjs");
    for (p, bytes) in [
        (&manifest, prepared.bytes()),
        (&verifier, VERIFIER),
        (&sampler, SAMPLER),
    ] {
        std::fs::write(p, bytes).map_err(|e| e.to_string())?;
    }
    let mut command = Command::new("node");
    command.arg(&verifier).arg(&manifest).arg(&sampler);
    let output = execute(command, Duration::from_secs(15))?;
    admit(prepared, &output)
}
#[cfg(test)]
#[path = "sampler_tests.rs"]
mod tests;
