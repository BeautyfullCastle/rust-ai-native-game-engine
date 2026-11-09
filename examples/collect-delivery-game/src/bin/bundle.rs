//! Export the one concrete Linux user-game runtime through the starter bundler.
use std::{collections::BTreeMap, path::PathBuf};

use orr_collect_delivery_game::game::CollectDeliveryGame;
use orr_static_starter::bundle::{Options, export};

fn main() -> Result<(), String> {
    let mut values = BTreeMap::new();
    let mut trusted_runtime = false;
    let mut args = std::env::args().skip(1);
    while let Some(key) = args.next() {
        if key == "--trusted-runtime" {
            if trusted_runtime {
                return Err("duplicate --trusted-runtime".into());
            }
            trusted_runtime = true;
        } else if [
            "--runtime",
            "--runtime-sha256",
            "--scene",
            "--output",
            "--source-sha",
        ]
        .contains(&key.as_str())
        {
            let value = args.next().ok_or("missing bundle option value")?;
            if values.insert(key, value).is_some() {
                return Err("duplicate bundle option".into());
            }
        } else {
            return Err("unknown bundle option".into());
        }
    }
    let mut required = |key: &str| values.remove(key).ok_or_else(|| format!("missing {key}"));
    let options = Options {
        runtime: PathBuf::from(required("--runtime")?),
        runtime_sha256: required("--runtime-sha256")?,
        scene: PathBuf::from(required("--scene")?),
        output: PathBuf::from(required("--output")?),
        source_sha: required("--source-sha")?,
        trusted_runtime,
    };
    let receipt = export::<CollectDeliveryGame>(&options)?;
    println!(
        "{}",
        serde_json::to_string(&receipt).map_err(|error| error.to_string())?
    );
    Ok(())
}
