//! Optional in-process retextured model package authoring, not installation.
use crate::assembled_package::{create_kind, PackageKind};
pub use crate::assembled_package::{Metadata, Options, Report};
use std::ffi::OsString;

pub const USAGE: &str = "orr_pkg create-retextured-model --recipe FILE --asset-id ID.retexture.json --license FILE --name NAME --version EXACT --engine REQUIREMENT --output NEW_DIRECTORY\nLinux model-authoring feature required. Uses built-in Rust texture transformer/package APIs; no --transformer or --package-source executable. User-supplied license is preserved, not verified. Trusted stable, symlink-free input/output ancestry; no overwrite/install. Exit0: published (check durability warning);1: creation failed;2: usage error.";

/// Validates and replaces the material texture with the existing bounded decoder, then publishes
/// exactly model.model.json, LICENSE.txt and orr.package.json atomically.
pub fn create(options: &Options) -> Result<Report, String> {
    create_kind(options, PackageKind::StaticRetexture)
}
pub fn run_cli(arguments: impl IntoIterator<Item = OsString>) -> u8 {
    let args: Vec<_> = arguments.into_iter().take(16).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("{USAGE}");
        return 0;
    }
    let options = match Options::parse(args) {
        Ok(value) => value,
        Err(error) => {
            eprintln!(
                "orr_pkg create-retextured-model: {}",
                error.replace(crate::assembled_package::USAGE, USAGE)
            );
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
            eprintln!("retextured model package creation failed: {error}");
            1
        }
    }
}
