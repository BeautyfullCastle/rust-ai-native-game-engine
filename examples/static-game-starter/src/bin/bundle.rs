//! Package one already-trusted concrete native starter executable.
use orr_static_starter::{
    bundle::{export, Options},
    delivery::DeliveryGame,
    switchboard::SwitchGame,
};
use std::{collections::BTreeMap, path::PathBuf};
fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let game = args.next().ok_or("usage: bundle <delivery|switchboard> --runtime PATH --runtime-sha256 HEX64 --scene PATH --output NEWPATH --source-sha HEX40 --trusted-runtime")?;
    let mut values = BTreeMap::new();
    let mut trusted_runtime = false;
    while let Some(key) = args.next() {
        if key == "--trusted-runtime" {
            if trusted_runtime {
                return Err("duplicate trusted-runtime flag".into());
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
            let value = args.next().ok_or("missing option value")?;
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
    let receipt = match game.as_str() {
        "delivery" => export::<DeliveryGame>(&options)?,
        "switchboard" => export::<SwitchGame>(&options)?,
        _ => return Err("unknown concrete game".into()),
    };
    println!(
        "{}",
        serde_json::to_string(&receipt).map_err(|e| e.to_string())?
    );
    Ok(())
}
