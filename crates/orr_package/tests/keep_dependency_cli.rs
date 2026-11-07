#![cfg(target_os="linux")]
mod common_keep;
use common_keep::*;
use orr_package::{Lock,PackagePreview};
use std::{fs,process::Command};
fn tool()->Command {Command::new(env!("CARGO_BIN_EXE_orr_pkg"))}
#[test]
fn cli_dry_run_keep_and_parent_removal_work_without_original_sources() {
    let f=Fixture::new();fs::rename(&f.dep,f.temp.path().join("hidden dep")).unwrap();fs::rename(&f.parent,f.temp.path().join("hidden parent")).unwrap();let before=files(f.temp.path());let output=tool().arg("keep").arg(f.project.root()).args(["dep","--dry-run"]).output().unwrap();assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));let preview:PackagePreview=serde_json::from_slice(&output.stdout).unwrap();assert_eq!(preview.previous,f.old);assert_eq!(preview.candidate.packages,f.old.packages);assert_eq!(files(f.temp.path()),before);
    let output=tool().arg("keep").arg(f.project.root()).arg("dep").output().unwrap();assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));let lock:Lock=serde_json::from_slice(&output.stdout).unwrap();assert_eq!(lock,preview.candidate);assert_eq!(f.project.verify().unwrap(),lock);let output=tool().arg("remove").arg(f.project.root()).arg("parent").output().unwrap();assert!(output.status.success());let lock:Lock=serde_json::from_slice(&output.stdout).unwrap();assert!(lock.direct.contains_key("dep"));assert!(!lock.packages.contains_key("parent"));assert_eq!(f.project.read_asset("dep","asset.txt").unwrap(),b"dep");
}
#[test]
fn cli_invalid_arguments_and_non_dependencies_produce_no_success_json_or_writes() {
    let f=Fixture::new();let before=files(f.temp.path());for tail in [vec!["dep","--dry-run","--dry-run"],vec!["dep","--unknown"],vec!["parent"],vec!["missing"],vec!["../dep"],vec![]] {let output=tool().arg("keep").arg(f.project.root()).args(tail).output().unwrap();assert!(!output.status.success());assert!(output.stdout.is_empty());assert_eq!(files(f.temp.path()),before);}
}
#[test]
fn cli_refuses_damaged_active_bytes_without_repairing_or_publishing() {
    let f=Fixture::new();let damaged=f.project.root().join(".orr/packages/objects").join(&f.old.packages["parent"].digest).join("asset.txt");fs::write(damaged,"corrupt").unwrap();let before=files(f.temp.path());for dry in [false,true] {let mut cmd=tool();cmd.arg("keep").arg(f.project.root()).arg("dep");if dry {cmd.arg("--dry-run");}let output=cmd.output().unwrap();assert!(!output.status.success());assert!(output.stdout.is_empty());assert_eq!(files(f.temp.path()),before);}
}
