#![cfg(feature = "room-project")]

use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[path = "../windows_room/authoring.rs"]
mod room_authoring;

fn fake_pe() -> Vec<u8> {
    fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    let mut bytes = vec![0; 1024];
    bytes[..2].copy_from_slice(b"MZ");
    write_u32(&mut bytes, 0x3c, 128);
    bytes[128..132].copy_from_slice(b"PE\0\0");
    let file_header = 132;
    let optional = file_header + 20;
    let section = optional + 112;
    write_u16(&mut bytes, file_header, 0x8664);
    write_u16(&mut bytes, file_header + 2, 1);
    write_u16(&mut bytes, file_header + 16, 112);
    write_u16(&mut bytes, file_header + 18, 0x0002);
    write_u16(&mut bytes, optional, 0x20b);
    write_u32(&mut bytes, optional + 16, 0x1000);
    write_u32(&mut bytes, optional + 32, 0x1000);
    write_u32(&mut bytes, optional + 36, 0x200);
    write_u32(&mut bytes, optional + 56, 0x2000);
    write_u32(&mut bytes, optional + 60, 0x200);
    write_u32(&mut bytes, optional + 108, 0);
    write_u32(&mut bytes, section + 8, 0x1000);
    write_u32(&mut bytes, section + 12, 0x1000);
    write_u32(&mut bytes, section + 16, 0x200);
    write_u32(&mut bytes, section + 20, 0x200);
    write_u32(&mut bytes, section + 36, 0x2000_0000);
    super::io::validate_pe(&bytes).unwrap();
    bytes
}

struct Fixture {
    temp: tempfile::TempDir,
    options: ExportOptions,
    runtime_bytes: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("Authored Room Ω");
        let authoring = room_authoring::Options::parse(vec![
            OsString::from("--output"),
            project.as_os_str().to_owned(),
            OsString::from("--seed"),
            OsString::from("export-test"),
        ])
        .unwrap();
        let creation = room_authoring::create(&authoring).unwrap();
        let authored = Prepared::open(&project, Profile::Room).unwrap();
        assert_eq!(creation.output, project.canonicalize().unwrap());
        assert_eq!(creation.entity_guids.len(), 7);
        assert_eq!(creation.initial_checksum, authored.checksum());
        fs::write(project.join("private-note.txt"), b"must not be exported").unwrap();
        let runtime = temp.path().join("trusted-room.exe");
        let runtime_bytes = fake_pe();
        fs::write(&runtime, &runtime_bytes).unwrap();
        let runtime_sha256 = hash(&runtime_bytes);
        let output = temp.path().join("Room Export Ω");
        Self {
            temp,
            options: ExportOptions {
                project,
                runtime,
                runtime_sha256,
                output,
                trusted_runtime: true,
                source_revision: Some("windows-unit-fixture".into()),
            },
            runtime_bytes,
        }
    }

    fn expected_smoke(&self) -> Vec<u8> {
        Prepared::open(&self.options.project, Profile::Room)
            .unwrap()
            .smoke()
            .unwrap()
    }

    fn assert_not_published(&self) {
        assert!(!self.options.output.exists());
        assert!(fs::read_dir(self.temp.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".orr-win-export-")
        }));
    }
}

fn smoke_command_project(command: &Command) -> PathBuf {
    let arguments: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    assert_eq!(arguments.first().map(String::as_str), Some("--project"));
    assert_eq!(arguments.get(2).map(String::as_str), Some("--headless"));
    assert_eq!(arguments.get(3).map(String::as_str), Some("--ticks"));
    assert_eq!(arguments.get(4).map(String::as_str), Some("0"));
    PathBuf::from(arguments[1].as_str())
}

fn tree_files(root: &Path) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                pending.push(path);
            } else {
                assert!(metadata.is_file());
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/"),
                );
            }
        }
    }
    files
}

#[test]
fn room_export_copies_only_admitted_closure_and_validates_staged_project() {
    let fixture = Fixture::new();
    let source = ProjectSnapshot::open_profile(&fixture.options.project, Profile::Room).unwrap();
    let source_prepared = Prepared::open(&fixture.options.project, Profile::Room).unwrap();
    let expected_checksum = source_prepared.checksum();
    let expected_smoke = source_prepared.smoke().unwrap();
    let expected_files: BTreeMap<_, _> = source
        .files
        .iter()
        .map(|file| (format!("project/{}", file.relative), file))
        .collect();
    assert_eq!(source.package_identities.len(), 1);
    assert!(expected_files
        .keys()
        .any(|path| path.ends_with("/LICENSE.txt")));
    let mut smoke_called = false;

    let report = export_with(
        &fixture.options,
        |_| Ok(()),
        |command| {
            smoke_called = true;
            let staged_project = smoke_command_project(command);
            assert!(staged_project.starts_with(
                fixture
                    .options
                    .output
                    .parent()
                    .unwrap()
                    .canonicalize()
                    .unwrap()
            ));
            let staged = Prepared::open(&staged_project, Profile::Room).unwrap();
            assert_eq!(staged.checksum(), expected_checksum);
            assert_eq!(staged.smoke().unwrap(), expected_smoke);
            Ok(expected_smoke.clone())
        },
    )
    .unwrap();

    assert!(smoke_called);
    assert_eq!(
        report.output,
        fixture.options.output.canonicalize().unwrap()
    );
    assert_eq!(report.project_files, expected_files.len());
    assert_eq!(
        report.manifest.payload.initial_checksum,
        format!("0x{expected_checksum:016x}")
    );
    assert_eq!(report.manifest.payload.entry, source.entry);
    assert_eq!(report.manifest.payload.packages, source.package_identities);
    assert_eq!(report.manifest.payload.declared.target, "windows-x86_64");
    assert_eq!(
        report.manifest.payload.declared.source_revision.as_deref(),
        Some("windows-unit-fixture")
    );
    assert_eq!(
        report.manifest.payload.runtime.sha256,
        fixture.options.runtime_sha256
    );

    let manifest_paths: BTreeSet<_> = report
        .manifest
        .payload
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    let expected_paths: BTreeSet<_> = expected_files.keys().map(String::as_str).collect();
    let mut expected_paths = expected_paths;
    let runtime_path = report
        .manifest
        .payload
        .files
        .iter()
        .find(|file| file.role == "runtime")
        .expect("one runtime is recorded");
    assert_eq!(
        usize::try_from(runtime_path.bytes).unwrap(),
        fixture.runtime_bytes.len()
    );
    assert_eq!(runtime_path.sha256, fixture.options.runtime_sha256);
    let runtime_relative = runtime_path.path.clone();
    assert!(manifest_paths.contains(runtime_relative.as_str()));
    expected_paths.insert(runtime_relative.as_str());
    let launcher_path = report
        .manifest
        .payload
        .files
        .iter()
        .find(|file| file.role == "launcher")
        .expect("one launcher is recorded");
    assert!(launcher_path.path.ends_with(".cmd"));
    let launcher = fs::read(report.output.join(&launcher_path.path)).unwrap();
    assert!(launcher
        .windows(b"setlocal DisableDelayedExpansion".len())
        .any(|bytes| bytes == b"setlocal DisableDelayedExpansion"));
    assert!(!launcher.windows(b"%*".len()).any(|bytes| bytes == b"%*"));
    expected_paths.insert(launcher_path.path.as_str());
    assert_eq!(manifest_paths, expected_paths);
    let mut expected_tree: BTreeSet<_> = manifest_paths
        .iter()
        .map(|path| (*path).to_owned())
        .collect();
    expected_tree.insert(MANIFEST_NAME.into());
    assert_eq!(tree_files(&report.output), expected_tree);

    for file in &report.manifest.payload.files {
        let staged_path = report.output.join(&file.path);
        let bytes = fs::read(&staged_path).unwrap();
        assert_eq!(bytes.len() as u64, file.bytes, "{}", file.path);
        assert_eq!(hash(&bytes), file.sha256, "{}", file.path);
        if let Some(source_file) = expected_files.get(&file.path) {
            assert_eq!(bytes, source_file.bytes, "{}", file.path);
        }
    }
    assert!(!report.output.join("project/private-note.txt").exists());

    let manifest_bytes = fs::read(report.output.join(MANIFEST_NAME)).unwrap();
    let manifest_text = String::from_utf8(manifest_bytes.clone()).unwrap();
    assert!(!manifest_text.contains(&fixture.options.project.display().to_string()));
    let disk_manifest: ExportManifest = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(disk_manifest, report.manifest);
    let mut published_bytes: BTreeMap<_, _> = report
        .manifest
        .payload
        .files
        .iter()
        .map(|file| {
            (
                file.path.clone(),
                fs::read(report.output.join(&file.path)).unwrap(),
            )
        })
        .collect();
    published_bytes.insert(MANIFEST_NAME.into(), manifest_bytes.clone());
    let copied = Prepared::open(&report.output.join("project"), Profile::Room).unwrap();
    assert_eq!(copied.checksum(), expected_checksum);
    assert_eq!(copied.scene_text(), source_prepared.scene_text());

    let second = export_with(
        &fixture.options,
        |_| Ok(()),
        |_| panic!("an existing export must reject before smoke"),
    );
    assert!(second.is_err());
    for (path, bytes) in published_bytes {
        assert_eq!(fs::read(report.output.join(path)).unwrap(), bytes);
    }
}

#[test]
fn room_export_requires_trust_matching_hash_and_a_valid_pe_before_staging() {
    let fixture = Fixture::new();
    let smoke = |_: &mut Command| panic!("invalid runtime admission must not invoke smoke");

    let mut untrusted = fixture.options.clone();
    untrusted.trusted_runtime = false;
    assert!(export_with(&untrusted, |_| Ok(()), smoke).is_err());
    fixture.assert_not_published();

    let mut wrong_hash = fixture.options.clone();
    wrong_hash.runtime_sha256 = "0".repeat(64);
    assert!(export_with(&wrong_hash, |_| Ok(()), smoke).is_err());
    fixture.assert_not_published();

    let invalid_runtime = fixture.temp.path().join("invalid.exe");
    fs::write(&invalid_runtime, b"not a PE image").unwrap();
    let mut invalid_pe = fixture.options.clone();
    invalid_pe.runtime = invalid_runtime.clone();
    invalid_pe.runtime_sha256 = hash(&fs::read(invalid_runtime).unwrap());
    assert!(export_with(&invalid_pe, |_| Ok(()), smoke).is_err());
    fixture.assert_not_published();
}

#[test]
fn room_export_rechecks_sources_after_staged_smoke_before_publication() {
    let fixture = Fixture::new();
    let scene = fixture.options.project.join("room.scene.yaml");
    let expected_smoke = fixture.expected_smoke();
    let result = export_with(
        &fixture.options,
        |point| {
            if point == "before-publish" {
                fs::write(&scene, b"source changed after admission").unwrap();
            }
            Ok(())
        },
        |_| Ok(expected_smoke.clone()),
    );
    assert!(result.is_err());
    fixture.assert_not_published();
}

#[test]
fn room_export_rejects_a_smoke_that_does_not_match_the_copied_project() {
    let fixture = Fixture::new();
    let result = export_with(
        &fixture.options,
        |_| Ok(()),
        |_| Ok(b"wrong smoke\n".to_vec()),
    );
    assert!(result.unwrap_err().contains("smoke"));
    fixture.assert_not_published();
}

#[test]
fn room_export_for_scene_requires_the_exact_admitted_path_and_checksum() {
    let fixture = Fixture::new();
    let scene = fixture.options.project.join("room.scene.yaml");
    let prepared = Prepared::open(&fixture.options.project, Profile::Room).unwrap();
    let checksum = prepared.checksum();
    let expected_smoke = prepared.smoke().unwrap();
    let mut smoke_called = false;
    let report = export_profile_with_scene(
        &fixture.options,
        Profile::Room,
        &scene,
        checksum,
        |_| Ok(()),
        |_| {
            smoke_called = true;
            Ok(expected_smoke.clone())
        },
    )
    .unwrap();
    assert!(smoke_called);
    assert_eq!(
        report.manifest.payload.initial_checksum,
        format!("0x{checksum:016x}")
    );

    let wrong_path_fixture = Fixture::new();
    let wrong_path = wrong_path_fixture.temp.path().join("different.scene.yaml");
    fs::write(&wrong_path, b"different but regular").unwrap();
    let result = export_profile_with_scene(
        &wrong_path_fixture.options,
        Profile::Room,
        &wrong_path,
        checksum,
        |_| Ok(()),
        |_| panic!("wrong scene path must reject before smoke"),
    );
    assert!(result.is_err());
    wrong_path_fixture.assert_not_published();

    let wrong_checksum_fixture = Fixture::new();
    let wrong_scene = wrong_checksum_fixture
        .options
        .project
        .join("room.scene.yaml");
    let result = export_profile_with_scene(
        &wrong_checksum_fixture.options,
        Profile::Room,
        &wrong_scene,
        checksum ^ 1,
        |_| Ok(()),
        |_| panic!("stale scene checksum must reject before smoke"),
    );
    assert!(result.is_err());
    wrong_checksum_fixture.assert_not_published();
}
