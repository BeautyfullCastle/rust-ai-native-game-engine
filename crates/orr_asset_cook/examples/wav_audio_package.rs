//! Offline, deliberately narrow WAV-to-Collect-SFX package authoring example.
#![forbid(unsafe_code)]

#[path = "wav_audio_package/package.rs"]
mod package;
#[path = "wav_audio_package/wav.rs"]
mod wav;

use orr_asset_cook::{CookError, Result};
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

const USAGE: &str = "wav_audio_package --wav PATH --asset-id a_0123456789abcdef --name NAME --version VERSION --license PATH --out NEW_DIR\n\nOffline PCM16 mono 48000 Hz only, 1..48000 frames, samples -8192..8192.\nAll six options are required exactly once. Output parent must already exist.\nCreates package bytes only; does not install, play, preview, or modify a project.";

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<package::Options>> {
    let args: Vec<_> = args.into_iter().collect();
    if args.len() == 1 && args[0] == "--help" {
        return Ok(None);
    }
    let mut values = BTreeMap::new();
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let flag = flag
            .to_str()
            .ok_or_else(|| CookError::invalid("invalid option"))?;
        if ![
            "--wav",
            "--asset-id",
            "--name",
            "--version",
            "--license",
            "--out",
        ]
        .contains(&flag)
        {
            return Err(CookError::invalid(format!("unknown option: {flag}")));
        }
        let value = args
            .next()
            .ok_or_else(|| CookError::invalid("missing option value"))?;
        if value.is_empty() || values.insert(flag.to_owned(), value).is_some() {
            return Err(CookError::invalid("empty or duplicate option"));
        }
    }
    let mut take = |key: &str| {
        values
            .remove(key)
            .ok_or_else(|| CookError::invalid(format!("missing {key}")))
    };
    let wav = PathBuf::from(take("--wav")?);
    let id = take("--asset-id")?
        .into_string()
        .map_err(|_| CookError::invalid("invalid asset ID"))?;
    let name = take("--name")?
        .into_string()
        .map_err(|_| CookError::invalid("invalid name"))?;
    let version = take("--version")?
        .into_string()
        .map_err(|_| CookError::invalid("invalid version"))?;
    let license = PathBuf::from(take("--license")?);
    let out = PathBuf::from(take("--out")?);
    Ok(Some(package::Options {
        wav,
        id: id.parse()?,
        name,
        version,
        license,
        out,
    }))
}

fn run() -> Result<()> {
    let Some(options) = parse_args(std::env::args_os().skip(1))? else {
        println!("{USAGE}");
        return Ok(());
    };
    let digest = package::create(&options)?;
    println!(
        "Created {} (PCM SHA-256 {digest}). Package bytes only; playback is not verified.",
        options.out.display()
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}\n\n{USAGE}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    #[test]
    fn strict_options_and_help() {
        assert!(parse_args([OsString::from("--help")]).unwrap().is_none());
        for args in [
            vec![],
            vec!["--unknown"],
            vec!["--wav"],
            vec!["--wav", "a", "--wav", "b"],
            vec!["--help", "--wav", "a"],
        ] {
            assert!(parse_args(args.into_iter().map(OsString::from)).is_err());
        }
    }
}
