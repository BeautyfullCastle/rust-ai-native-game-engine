//! Exact authored Scene bytes admitted by the existing trusted WASM validator.
use super::{
    prepared::{digest, snapshot},
    publish,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, ffi::OsString, path::PathBuf, process::Command, time::Duration};
const VALIDATOR: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/validate_coop_defense_scene.mjs"
));
const USAGE:&str="orr_pkg create-coop-scene --wasm TRUSTED_LOCAL_WASM --output NEW_SCENE [--starting-gold 0..10000] [--reactor-hp 1..1000] [--last-wave 1..10] [--spawn-interval-ticks 35..350]\nExecutes trusted local WASM in trusted Node on PATH; not a sandbox. No fetch or build. Integers accept ASCII decimal digits, optional sign, whitespace and single underscores between digits; Unicode digits are not accepted. Duplicate options are refused.";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Receipt {
    version: u32,
    scene_bytes: usize,
    scene_sha256: String,
    wasm_sha256: String,
    initial_checksum: String,
}
pub(super) struct Verified {
    bytes: Vec<u8>,
    receipt: Receipt,
}
impl Verified {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(super) fn receipt(&self) -> &Receipt {
        &self.receipt
    }
}
#[derive(Debug)]
struct Options {
    wasm: PathBuf,
    output: PathBuf,
    gold: i64,
    hp: i64,
    wave: i64,
    pacing: Option<i64>,
}
fn integer(value: &OsString) -> Result<i64, String> {
    let raw = value
        .to_str()
        .ok_or("integer must be UTF-8 ASCII decimal")?;
    let raw = raw.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}'));
    let digits = raw
        .strip_prefix('+')
        .or_else(|| raw.strip_prefix('-'))
        .unwrap_or(raw);
    if digits.is_empty()
        || digits
            .split('_')
            .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err("invalid ASCII decimal integer".into());
    }
    raw.replace('_', "")
        .parse()
        .map_err(|_| "integer outside supported range".into())
}
fn options(args: &[OsString]) -> Result<Options, String> {
    if args.len() > 12
        || args.len() % 2 != 0
        || args.iter().any(|s| {
            s.is_empty() || s.as_encoded_bytes().len() > 4096 || s.as_encoded_bytes().contains(&0)
        })
    {
        return Err("invalid bounded option/value arguments".into());
    }
    let (mut wasm, mut output) = (None, None);
    let (mut gold, mut hp, mut wave, mut pacing) = (150, 30, 10, None);
    let mut seen = BTreeSet::new();
    for pair in args.chunks_exact(2) {
        let key = pair[0].to_str().ok_or("invalid option")?;
        if !seen.insert(key) {
            return Err("duplicate option".into());
        }
        match key {
            "--wasm" => wasm = Some(PathBuf::from(&pair[1])),
            "--output" => output = Some(PathBuf::from(&pair[1])),
            "--starting-gold" => gold = integer(&pair[1])?,
            "--reactor-hp" => hp = integer(&pair[1])?,
            "--last-wave" => wave = integer(&pair[1])?,
            "--spawn-interval-ticks" => pacing = Some(integer(&pair[1])?),
            _ => return Err("unknown option".into()),
        }
    }
    let o = Options {
        wasm: wasm.ok_or("--wasm required")?,
        output: output.ok_or("--output required")?,
        gold,
        hp,
        wave,
        pacing,
    };
    scene_text(&o)?;
    Ok(o)
}
fn scene_text(o: &Options) -> Result<Vec<u8>, String> {
    if !(0..=10000).contains(&o.gold)
        || !(1..=1000).contains(&o.hp)
        || !(1..=10).contains(&o.wave)
        || o.pacing.is_some_and(|p| !(35..=350).contains(&p))
    {
        return Err("scene integer outside authored bounds".into());
    }
    let mut text=format!("schema: orr.scene/1\nentities: {{}}\nsingletons:\n  CoopDefense::Scenario:\n    starting_gold: {}\n    reactor_hp: {}\n  CoopDefense::Campaign:\n    last_wave: {}\n",o.gold,o.hp,o.wave);
    if let Some(pacing) = o.pacing {
        text.push_str(&format!(
            "  CoopDefense::WavePacing:\n    spawn_interval_ticks: {pacing}\n"
        ));
    }
    Ok(text.into_bytes())
}
fn admit(bytes: Vec<u8>, wasm: &[u8], output: &[u8]) -> Result<Verified, String> {
    if bytes.len() > 65536 || output.len() > 4096 {
        return Err("scene or receipt exceeds bound".into());
    }
    let receipt: Receipt = serde_json::from_slice(output).map_err(|e| e.to_string())?;
    if receipt.version != 1
        || receipt.scene_bytes != bytes.len()
        || receipt.scene_sha256 != digest(&bytes)
        || receipt.wasm_sha256 != digest(wasm)
        || receipt.initial_checksum.len() != 16
        || !receipt
            .initial_checksum
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("scene receipt does not match exact admitted inputs".into());
    }
    Ok(Verified { bytes, receipt })
}
fn validate_node(wasm: &[u8], scene: &[u8]) -> Result<Vec<u8>, String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let script = temp.path().join("validate.mjs");
    let wasm_path = temp.path().join("game.wasm");
    let scene_path = temp.path().join("scene.yaml");
    for (path, bytes) in [
        (&script, VALIDATOR),
        (&wasm_path, wasm),
        (&scene_path, scene),
    ] {
        std::fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    let mut command = Command::new("node");
    command.arg(&script).arg(&wasm_path).arg(&scene_path);
    super::node_gate::execute(command, Duration::from_secs(15))
}
pub(super) fn verify_bytes(bytes: Vec<u8>, wasm: &[u8]) -> Result<Verified, String> {
    if bytes.len() > 65536 || wasm.len() > 64 * 1024 * 1024 {
        return Err("scene/WASM exceeds bounds".into());
    }
    std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    let result = validate_node(wasm, &bytes)?;
    admit(bytes, wasm, &result)
}
fn create_with(
    o: &Options,
    gate: impl FnOnce(&[u8], &[u8]) -> Result<Vec<u8>, String>,
) -> Result<(Verified, Option<String>), String> {
    let bytes = scene_text(o)?;
    publish::preflight(&o.output)?;
    let wasm = snapshot(&o.wasm, 64 * 1024 * 1024)?;
    let output = gate(&wasm, &bytes)?;
    let verified = admit(bytes, &wasm, &output)?;
    if snapshot(&o.wasm, 64 * 1024 * 1024)? != wasm {
        return Err("trusted WASM changed during verification".into());
    }
    let warning = publish::publish_scene(&o.output, &verified)?;
    Ok((verified, warning))
}
pub(super) fn run_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    let args: Vec<_> = args.into_iter().take(14).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return 0;
    }
    let o = match options(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return 2;
        }
    };
    match create_with(&o, validate_node) {
        Ok((verified, warning)) => {
            if let Some(w) = warning {
                eprintln!("warning: published; {w}; do not retry or remove published output");
            }
            // Receipt fields are bounded and serializable by construction.
            println!(
                "{}",
                serde_json::to_string(verified.receipt()).expect("validated receipt serialization")
            );
            0
        }
        Err(e) => {
            eprintln!("Scene creation failed: {e}");
            1
        }
    }
}
#[cfg(test)]
#[path = "scene_tests.rs"]
mod tests;
