//! Opt-in deployment helper, not game startup or a settings-file editor.
use std::{ffi::OsString, path::PathBuf};
const HELP: &str = "orr_prepare_stormglass_settings --directory ABSOLUTE_NEW_DIRECTORY\nWindows only. Creates one new private local NTFS directory; parents must exist.\nNever changes existing directories, ACLs or files. No game window or audio device.";

fn parse(args: Vec<OsString>) -> Result<Option<PathBuf>, &'static str> {
    if args.as_slice() == [OsString::from("--help")] {
        return Ok(None);
    }
    if args.len() != 2 || args[0] != "--directory" || args[1].is_empty() {
        return Err("expected exactly --directory ABSOLUTE_NEW_DIRECTORY (or --help)");
    }
    Ok(Some(PathBuf::from(&args[1])))
}
fn run() -> Result<(), String> {
    let Some(path) = parse(std::env::args_os().skip(1).collect()).map_err(str::to_owned)? else {
        println!("{HELP}");
        return Ok(());
    };
    #[cfg(windows)]
    {
        orr_sample::prepare_stormglass_settings_directory(&path)?;
        println!("Created new private settings directory: {}", path.display());
        println!("Pass it explicitly to run-game.ps1 -ControlsDirectory (and optionally -AudioSettingsDirectory). Runtime admission still applies.");
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("this helper supports Windows local NTFS only; no directory was created".into())
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("Stormglass settings setup: {e}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn help_and_explicit_literal_path_are_the_only_forms() {
        assert_eq!(parse(vec!["--help".into()]).unwrap(), None);
        let path = OsString::from(r"C:\Private & [keys]\new");
        assert_eq!(
            parse(vec!["--directory".into(), path.clone()]).unwrap(),
            Some(PathBuf::from(path))
        );
    }
    #[test]
    fn invalid_arguments_have_no_creation_action() {
        for args in [
            vec![],
            vec!["--directory"],
            vec!["--directory", ""],
            vec!["relative"],
            vec!["--help", "extra"],
            vec!["--directory", "one", "--directory", "two"],
            vec!["--unknown", "one"],
        ] {
            assert!(parse(args.into_iter().map(OsString::from).collect()).is_err());
        }
    }
}
