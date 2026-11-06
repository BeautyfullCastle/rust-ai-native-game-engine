//! Native-only execution path. Shared YAML authoring, strict native admission,
//! and typed no-replace publication; no child process or WASM input.
use super::{prepared::digest, publish, scene};
use orr_games::{coop_defense_game::SCENARIO_BUILD_ID, coop_defense_scene::parse_config};
use serde::Serialize;
use std::ffi::OsString;

const USAGE: &str = "orr_pkg create-coop-native-scene --output NEW_SCENE [--starting-gold 0..10000] [--reactor-hp 1..1000] [--last-wave 1..10] [--spawn-interval-ticks 35..350] [--wave-pairs N1,N2,N3,N4,N5,N6,N7,N8,N9,N10] [--enemy-health-multiplier 1..4] [--wave-kinds ROW1;ROW2;...;ROW10] [--opening-bosses B1,B2,...,B10]\nNative admission only; no Node, WASM, fetch or build. Output bytes match create-coop-scene with the same authored settings. Wave pairs: ten integers in 1..28. Wave kinds: ten semicolon-separated rows of four integers in 0..3. Opening bosses: ten flags in 0..1. Supplying either composition option writes both fields with the historical default for the other. Integers accept ASCII decimal digits, optional sign, whitespace and single underscores between digits. Duplicate flags and --wasm are refused. Omitted settings retain existing native defaults; omitted optional singletons are not written.";

/// Deliberately distinct from the web receipt: no WASM hash or claimed WASM execution.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeReceipt {
    kind: &'static str,
    scene_bytes: usize,
    scene_sha256: String,
    native_build_id: String,
    admitted: AdmittedConfig,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdmittedConfig {
    starting_gold: u32,
    reactor_hp: u32,
    last_wave: u32,
    spawn_interval_ticks: u32,
    wave_pairs: [u32; 10],
    enemy_health_multiplier: u32,
    wave_kinds: [[u32; 4]; 10],
    opening_bosses: [u32; 10],
}
pub(super) struct Verified {
    bytes: Vec<u8>,
    receipt: NativeReceipt,
}
impl Verified {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
fn admit(bytes: Vec<u8>) -> Result<Verified, String> {
    let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    // This is the exact bounded reflected parser/bake used before native startup.
    let config = parse_config(text)?;
    let receipt = NativeReceipt {
        kind: "orr.coop-native-scene/1",
        scene_bytes: bytes.len(),
        scene_sha256: digest(&bytes),
        native_build_id: format!("{SCENARIO_BUILD_ID:016x}"),
        admitted: AdmittedConfig {
            starting_gold: config.scenario.starting_gold,
            reactor_hp: config.scenario.reactor_hp,
            last_wave: config.campaign.last_wave,
            spawn_interval_ticks: config.wave_pacing.spawn_interval_ticks,
            wave_pairs: config.wave_counts.pair_counts,
            enemy_health_multiplier: config.enemy_health.multiplier,
            wave_kinds: config.wave_composition.regular_kinds,
            opening_bosses: config.wave_composition.opening_boss,
        },
    };
    Ok(Verified { bytes, receipt })
}
fn create(options: &scene::Options) -> Result<(Verified, Option<String>), String> {
    let bytes = scene::scene_text(options)?;
    publish::preflight(&options.output)?;
    let verified = admit(bytes)?;
    let warning = publish::publish_native_scene(&options.output, &verified)?;
    Ok((verified, warning))
}
pub(super) fn run_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    let args: Vec<_> = args.into_iter().take(22).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return 0;
    }
    let options = match scene::options_for(&args, true) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            return 2;
        }
    };
    match create(&options) {
        Ok((verified, warning)) => {
            if let Some(warning) = warning {
                eprintln!("warning: published; {warning}; do not retry or remove published output");
            }
            println!(
                "{}",
                serde_json::to_string(&verified.receipt).expect("native receipt serialization")
            );
            0
        }
        Err(error) => {
            eprintln!("Native Scene creation failed: {error}");
            1
        }
    }
}

#[cfg(test)]
#[path = "native_scene_tests.rs"]
mod tests;
