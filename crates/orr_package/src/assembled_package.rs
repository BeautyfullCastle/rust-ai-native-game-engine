//! Optional in-process static model package authoring. No child executable,
//! installation, runtime capability grant or rights verification is performed.
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::PathBuf,
};

mod publish;

pub const USAGE: &str = "orr_pkg create-assembled --recipe FILE --asset-id ID.assemble.json --license FILE --name NAME --version EXACT --engine REQUIREMENT --output NEW_DIRECTORY\nLinux model-authoring feature required. Uses built-in Rust assembler/package APIs; no --assembler or --package-source executable. User-supplied license is preserved, not verified. Trusted stable, symlink-free input/output ancestry; no overwrite/install. Exit0: published (check durability warning);1: creation failed;2: usage error.";
const MAX_RECIPE: usize = 16 * 1024;
const MAX_LICENSE: usize = 1024 * 1024;
const MAX_MODEL: usize = 64 * 1024 * 1024;

/// Closed internal policy: callers cannot choose a decoder, filename or grants.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PackageKind {
    Static,
    StaticUv,
    StaticRetexture,
    #[cfg(feature = "animation-authoring")]
    AnimatedSequence,
}
impl PackageKind {
    fn model_name(self) -> &'static str {
        match self {
            Self::Static | Self::StaticUv | Self::StaticRetexture => "model.model.json",
            #[cfg(feature = "animation-authoring")]
            Self::AnimatedSequence => "model.animated.json",
        }
    }
    fn capabilities(self) -> BTreeSet<String> {
        match self {
            Self::Static | Self::StaticUv | Self::StaticRetexture => ["models".into()].into(),
            #[cfg(feature = "animation-authoring")]
            Self::AnimatedSequence => ["models".into(), "animation".into()].into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Metadata {
    pub name: String,
    pub version: String,
    pub engine: String,
}
#[derive(Clone, Debug)]
pub struct Options {
    pub recipe: PathBuf,
    pub asset_id: String,
    pub license: PathBuf,
    pub metadata: Metadata,
    pub output: PathBuf,
}
#[derive(Debug)]
pub struct Report {
    pub output: PathBuf,
    /// Publication has happened. Do not retry or remove the published package.
    pub durability_warning: Option<String>,
}
impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let args: Vec<_> = arguments.into_iter().take(16).collect();
        if args.len() != 14 {
            return Err(format!(
                "expected exactly seven unique option/value pairs; {USAGE}"
            ));
        }
        let mut values = BTreeMap::new();
        for pair in args.chunks_exact(2) {
            let key = pair[0].to_str().ok_or("option names must be UTF-8")?;
            if ![
                "--recipe",
                "--asset-id",
                "--license",
                "--name",
                "--version",
                "--engine",
                "--output",
            ]
            .contains(&key)
            {
                return Err(format!("unknown or obsolete option {key}; external executable flags are not supported; {USAGE}"));
            }
            if values.insert(key, pair[1].clone()).is_some() {
                return Err(format!("duplicate option {key}"));
            }
        }
        let mut take = |key| values.remove(key).ok_or_else(|| format!("missing {key}"));
        let recipe = PathBuf::from(take("--recipe")?);
        let asset_id = take("--asset-id")?
            .into_string()
            .map_err(|_| "asset ID must be UTF-8")?;
        let license = PathBuf::from(take("--license")?);
        let name = take("--name")?
            .into_string()
            .map_err(|_| "name must be UTF-8")?;
        let version = take("--version")?
            .into_string()
            .map_err(|_| "version must be UTF-8")?;
        let engine = take("--engine")?
            .into_string()
            .map_err(|_| "engine must be UTF-8")?;
        Ok(Self {
            recipe,
            asset_id,
            license,
            metadata: Metadata {
                name,
                version,
                engine,
            },
            output: PathBuf::from(take("--output")?),
        })
    }
}
fn valid_metadata(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.as_bytes().contains(&0)
}
/// All decoding remains in the existing assembler and package-source libraries.
/// Bounded inputs replace the old child-process timeout, not a wall-time guarantee.
pub fn create(options: &Options) -> Result<Report, String> {
    create_kind(options, PackageKind::Static)
}
pub(crate) fn create_kind(options: &Options, kind: PackageKind) -> Result<Report, String> {
    for value in [
        &options.asset_id,
        &options.metadata.name,
        &options.metadata.version,
        &options.metadata.engine,
    ] {
        if !valid_metadata(value) {
            return Err("metadata requires 1..4096 UTF-8 bytes without NUL".into());
        }
    }
    for path in [&options.recipe, &options.license, &options.output] {
        let bytes = path.as_os_str().as_encoded_bytes();
        if bytes.is_empty() || bytes.len() > 4096 || bytes.contains(&0) {
            return Err("paths require 1..4096 bytes without NUL".into());
        }
    }
    publish::preflight_output(&options.output)?;
    let recipe = publish::snapshot(&options.recipe, MAX_RECIPE)?;
    let license = publish::snapshot(&options.license, MAX_LICENSE)?;
    if std::str::from_utf8(&license)
        .map_err(|_| "license must be UTF-8")?
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}'))
        .is_empty()
    {
        return Err("license must contain nonempty user-provided text".into());
    }
    let model = match kind {
        PackageKind::Static => {
            orr_model::static_assemble::assemble_recipe(&options.recipe, &options.asset_id)?
        }
        PackageKind::StaticUv => {
            orr_model::static_uv::transform_recipe(&options.recipe, &options.asset_id)?
        }
        PackageKind::StaticRetexture => {
            orr_model::static_retexture::transform_recipe(&options.recipe, &options.asset_id)?
        }
        #[cfg(feature = "animation-authoring")]
        PackageKind::AnimatedSequence => {
            orr_model::animation_sequence::sequence_recipe(&options.recipe, &options.asset_id)?
        }
    };
    if model.len() > MAX_MODEL {
        return Err("assembled model exceeds 64 MiB".into());
    }
    if publish::snapshot(&options.recipe, MAX_RECIPE)? != recipe {
        return Err("recipe changed during assembly".into());
    }
    match kind {
        PackageKind::Static | PackageKind::StaticUv | PackageKind::StaticRetexture => {
            publish::publish(&options.output, &model, &license, &options.metadata)
        }
        #[cfg(feature = "animation-authoring")]
        PackageKind::AnimatedSequence => {
            publish::publish_kind(&options.output, &model, &license, &options.metadata, kind)
        }
    }
}
/// Dedicated CLI entry point. Legacy external executable options are never run.
pub fn run_cli(arguments: impl IntoIterator<Item = OsString>) -> u8 {
    let args: Vec<_> = arguments.into_iter().take(16).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("{USAGE}");
        return 0;
    }
    let options = match Options::parse(args) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("orr_pkg create-assembled: {error}");
            return 2;
        }
    };
    match create(&options) {
        Ok(report) => {
            println!("{}", report.output.display());
            if let Some(warning) = report.durability_warning {
                eprintln!("warning: published; durability uncertain: {warning}; do not retry or remove the published package");
            }
            0
        }
        Err(error) => {
            eprintln!("Assembled package creation failed: {error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args() -> Vec<OsString> {
        [
            "--recipe",
            "a.json",
            "--asset-id",
            "a.assemble.json",
            "--license",
            "LICENSE",
            "--name",
            "model-package",
            "--version",
            "1.0.0",
            "--engine",
            "*",
            "--output",
            "new-output",
        ]
        .into_iter()
        .map(Into::into)
        .collect()
    }
    #[test]
    fn closed_arguments_reject_duplicates_unknown_obsolete_and_unbounded_values() {
        let parsed = Options::parse(args()).unwrap();
        assert_eq!(parsed.metadata.name, "model-package");
        for bad in ["--assembler", "--package-source", "--command", "--recipe"] {
            let mut input = args();
            input[2] = bad.into();
            assert!(Options::parse(input).is_err());
        }
        let mut input = args();
        input[3] = "x".repeat(4097).into();
        assert!(create(&Options::parse(input).unwrap()).is_err());
        let mut input = args();
        input[3] = "a\0b".into();
        assert!(create(&Options::parse(input).unwrap()).is_err());
        assert!(Options::parse(std::iter::repeat(OsString::from("--recipe"))).is_err());
        assert!(Options::parse(args().into_iter().take(13)).is_err());
    }
}
