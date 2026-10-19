//! Offline PNG + existing Sprite JSON to a new sprite content package.
#![forbid(unsafe_code)]

#[cfg(feature = "project-create")]
#[path = "png_sprite_package/authoring.rs"]
mod authoring;

#[cfg(feature = "project-create")]
mod cli {
    use super::authoring::{import_package, Request};
    use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

    const USAGE: &str = "png_sprite_package --png PATH --json PATH --license PATH --name NAME --version VERSION --out NEW_DIR\n\nAll six options are required exactly once. Output parent must already exist.\nOffline package bytes only: no installation, project edit, preview or playback.\nRequires --features project-create. See docs/png-sprite-package.md for bounds and filesystem trust requirements.";

    fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<Request>, String> {
        let args: Vec<_> = args.into_iter().collect();
        if args.len() == 1 && args[0] == "--help" {
            return Ok(None);
        }
        let mut values = BTreeMap::new();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let flag = flag.to_str().ok_or("invalid option")?;
            if ![
                "--png",
                "--json",
                "--license",
                "--name",
                "--version",
                "--out",
            ]
            .contains(&flag)
            {
                return Err(format!("unknown option: {flag}"));
            }
            let value = args.next().ok_or("missing option value")?;
            if value.is_empty() || values.insert(flag.to_owned(), value).is_some() {
                return Err("empty or duplicate option".into());
            }
        }
        let mut take = |key: &str| values.remove(key).ok_or_else(|| format!("missing {key}"));
        Ok(Some(Request {
            png: PathBuf::from(take("--png")?),
            json: PathBuf::from(take("--json")?),
            license: PathBuf::from(take("--license")?),
            name: take("--name")?.into_string().map_err(|_| "invalid name")?,
            version: take("--version")?
                .into_string()
                .map_err(|_| "invalid version")?,
            out: PathBuf::from(take("--out")?),
        }))
    }

    pub fn run() -> Result<(), String> {
        let Some(request) = parse_args(std::env::args_os().skip(1))? else {
            println!("{USAGE}");
            return Ok(());
        };
        let report = import_package(&request)?;
        println!(
            "Created {}: {} ({}x{}, {} decoded RGBA bytes). Package bytes only; runtime rendering and playback are not verified.",
            request.out.display(), report.image, report.width, report.height, report.rgba_bytes
        );
        Ok(())
    }

    pub fn main() {
        if let Err(error) = run() {
            eprintln!("{error}\n\n{USAGE}");
            std::process::exit(1);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn strict_options_and_help() {
            assert!(parse_args([OsString::from("--help")]).unwrap().is_none());
            for args in [
                vec![],
                vec!["--unknown"],
                vec!["--png"],
                vec!["--png", "a", "--png", "b"],
                vec!["--help", "--png", "a"],
            ] {
                assert!(parse_args(args.into_iter().map(OsString::from)).is_err());
            }
        }
    }
}

#[cfg(feature = "project-create")]
fn main() {
    cli::main();
}

// Deliberately runs before argument processing or any source/output file access.
#[cfg(not(feature = "project-create"))]
fn main() {
    eprintln!(
        "png_sprite_package requires --features project-create; no files were read or written"
    );
    std::process::exit(1);
}
