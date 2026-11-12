//! Opt-in deployment helper, not game startup or a settings-file editor.
use std::{ffi::OsString, path::PathBuf};
const HELP: &str = "orr_prepare_stormglass_settings --directory ABSOLUTE_NEW_DIRECTORY\norr_prepare_stormglass_settings --check ABSOLUTE_EXISTING_DIRECTORY\nWindows local NTFS only. Create one new private directory, or read-only admit both settings profiles.\nNever repairs existing ACLs/data; no game window or audio device.";

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Help,
    Create(PathBuf),
    Check(PathBuf),
}
fn parse(args: Vec<OsString>) -> Result<Action, &'static str> {
    if args.as_slice() == [OsString::from("--help")] {
        return Ok(Action::Help);
    }
    if args.len() != 2 || args[1].is_empty() {
        return Err(
            "expected exactly --directory NEW_DIRECTORY, --check EXISTING_DIRECTORY, or --help",
        );
    }
    match args[0].to_str() {
        Some("--directory") => Ok(Action::Create(PathBuf::from(&args[1]))),
        Some("--check") => Ok(Action::Check(PathBuf::from(&args[1]))),
        _ => Err("unknown mode; use --directory, --check, or --help"),
    }
}
fn run() -> Result<(), String> {
    let action = parse(std::env::args_os().skip(1).collect()).map_err(str::to_owned)?;
    if action == Action::Help {
        println!("{HELP}");
        return Ok(());
    }
    #[cfg(windows)]
    {
        match action {
            Action::Create(path) => {
                orr_sample::prepare_stormglass_settings_directory(&path)?;
                println!("Created new private settings directory: {}", path.display());
                println!("Pass it explicitly to run-game.ps1 -UserSettingsDirectory (or per-profile options). Runtime admission still applies.");
            }
            Action::Check(path) => {
                let status = orr_sample::check_stormglass_settings_directory(&path)?;
                let label = |saved| if saved { "valid-file" } else { "defaults" };
                println!(
                    "Settings admission: audio={} controls={}",
                    label(status.audio_saved),
                    label(status.controls_saved)
                );
                println!("Read-only admission only; the game rechecks settings at launch. This does not guarantee a future Save.");
            }
            Action::Help => unreachable!(),
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = action;
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
    fn help_and_explicit_literal_modes_are_the_only_forms() {
        assert_eq!(parse(vec!["--help".into()]).unwrap(), Action::Help);
        let path = OsString::from(r"C:\Private & [keys]\new");
        assert_eq!(
            parse(vec!["--directory".into(), path.clone()]).unwrap(),
            Action::Create(PathBuf::from(&path))
        );
        assert_eq!(
            parse(vec!["--check".into(), path.clone()]).unwrap(),
            Action::Check(PathBuf::from(path))
        );
    }
    #[test]
    fn invalid_arguments_have_no_creation_or_check_action() {
        for args in [
            vec![],
            vec!["--directory"],
            vec!["--check"],
            vec!["--directory", ""],
            vec!["--check", ""],
            vec!["relative"],
            vec!["--help", "extra"],
            vec!["--directory", "one", "--check", "two"],
            vec!["--unknown", "one"],
        ] {
            assert!(parse(args.into_iter().map(OsString::from).collect()).is_err());
        }
    }
}
