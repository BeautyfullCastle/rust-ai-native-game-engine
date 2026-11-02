//! Fixed trusted Node gate; no user script/executable input, no structural-only fallback.
use super::prepared::{digest, Prepared};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::Duration,
};
// Monotonic process supervision outside simulation; required to terminate a stalled verifier.
#[allow(clippy::disallowed_types)]
use std::time::Instant;
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
fn drain(mut stream: impl Read + Send + 'static) -> thread::JoinHandle<Result<Vec<u8>, String>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        stream
            .by_ref()
            .take((CAP + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    })
}
// This clock bounds a trusted external authoring verifier, not deterministic game simulation.
#[allow(clippy::disallowed_types)]
fn execute(mut command: Command, timeout: Duration) -> Result<Vec<u8>, String> {
    command
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("trusted Node could not start: {e}"))?;
    let stdout = drain(child.stdout.take().ok_or("Node stdout unavailable")?);
    let stderr = drain(child.stderr.take().ok_or("Node stderr unavailable")?);
    let start = Instant::now();
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err("Node sampler timed out".to_owned());
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(e.to_string());
            }
        }
    };
    let out = stdout.join().map_err(|_| "Node stdout reader panicked")??;
    let err = stderr.join().map_err(|_| "Node stderr reader panicked")??;
    let status = outcome?;
    if out.len() > CAP || err.len() > CAP {
        return Err("Node output exceeds bound".into());
    }
    if !status.success() {
        return Err(format!(
            "Node sampler failed: {}",
            String::from_utf8_lossy(&err)
        ));
    }
    Ok(out)
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
