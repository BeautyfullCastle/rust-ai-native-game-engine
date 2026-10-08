//! Genuine installed-package creator coverage; no device, cooker or runtime executable.
use super::*;

fn temporary() -> tempfile::TempDir {
    tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
}
fn options(root: &Path, name: &str, template: &str) -> CreateOptions {
    CreateOptions {
        output: root.join(name),
        template: template.into(),
        seed: "audio-test-seed".into(),
    }
}
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, all: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            assert!(!entry.file_type().unwrap().is_symlink());
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), all);
            } else {
                all.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut all = BTreeMap::new();
    visit(root, root, &mut all);
    all
}
fn no_stages(root: &Path) {
    assert!(!fs::read_dir(root).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".orr-create-")));
}
#[cfg(feature = "collect-dodge")]
const ID: &str = "12345678-1234-4234-8234-123456789abc";

#[test]
fn original_arena_and_collect_templates_do_not_gain_audio_implicitly() {
    let temp = temporary();
    let report = create(&options(temp.path(), "arena", TEMPLATE)).unwrap();
    let project = orr_package::Project::open(&report.output, compiled_runtime()).unwrap();
    let lock = project.verify().unwrap();
    assert!(project
        .manifest()
        .unwrap()
        .entry
        .as_ref()
        .unwrap()
        .audio
        .is_none());
    assert_eq!(lock.packages.len(), 1);
    assert!(!lock.packages.contains_key("collect-audio-v1"));
    assert!(!files(&report.output)
        .keys()
        .any(|path| path.ends_with(".audio.json")));
    #[cfg(feature = "collect-dodge")]
    {
        let report =
            create_collect(&options(temp.path(), "collect", COLLECT_TEMPLATE), ID).unwrap();
        let project = orr_package::Project::open(&report.output, compiled_runtime()).unwrap();
        let lock = project.verify().unwrap();
        assert!(project
            .manifest()
            .unwrap()
            .entry
            .as_ref()
            .unwrap()
            .audio
            .is_none());
        assert_eq!(lock.packages.len(), 1);
        assert!(!lock.packages.contains_key("collect-audio-v1"));
        assert!(!files(&report.output)
            .keys()
            .any(|path| path.ends_with(".audio.json")));
    }
    no_stages(temp.path());
}

#[cfg(not(feature = "collect-audio"))]
#[test]
fn missing_audio_feature_rejects_template_before_any_stage_exists() {
    let temp = temporary();
    let option = options(temp.path(), "unsupported", COLLECT_AUDIO_TEMPLATE);
    assert!(create(&option)
        .unwrap_err()
        .contains("collect-audio feature"));
    #[cfg(feature = "collect-dodge")]
    assert!(create_collect(&option, ID)
        .unwrap_err()
        .contains("collect-audio feature"));
    assert!(!option.output.exists());
    no_stages(temp.path());
}

#[cfg(feature = "collect-audio")]
mod enabled {
    use super::*;
    use crate::collect_audio::{Document, DEFAULT_PACKAGE};
    use crate::collect_project::{PreparedProject, ProgressSupport, SpriteSupport};

    fn progress() -> orr_package::ProjectProgress {
        orr_package::ProjectProgress {
            schema: 1,
            game_id: ID.into(),
            profile: orr_package::ProgressProfile::CollectDodgeHighscoreV1,
        }
    }
    fn prepared(root: &Path) -> PreparedProject {
        PreparedProject::open_with_audio(
            root,
            ProgressSupport::MetadataOnly,
            SpriteSupport::Supported,
            false,
            true,
        )
        .unwrap()
    }
    fn audio_options(root: &Path, name: &str) -> CreateOptions {
        options(root, name, COLLECT_AUDIO_TEMPLATE)
    }

    #[test]
    fn audio_template_is_deterministic_with_real_install_reopen_and_exact_owned_assets() {
        let temp = temporary();
        let a = create_collect(&audio_options(temp.path(), "first"), ID).unwrap();
        let b = create_collect(&audio_options(temp.path(), "second"), ID).unwrap();
        assert_eq!(files(&a.output), files(&b.output));
        assert_eq!(a.initial_checksum, b.initial_checksum);
        assert_eq!(a.entity_guids, b.entity_guids);
        let first = prepared(&a.output);
        let second = prepared(&b.output);
        assert_eq!(
            first.scene().frame().to_bytes(),
            second.scene().frame().to_bytes()
        );
        assert_eq!(first.progress().unwrap().game_id, ID);
        let audio = first.audio().unwrap();
        assert_eq!(audio.path, a.output.join("level.audio.json"));
        assert_eq!(audio.document, Document::default_collect());
        assert_eq!(audio.available.len(), 2);
        assert_eq!(audio.stats.records, 2);
        let mut runtime = compiled_runtime();
        runtime.capabilities.insert("collect-audio".into());
        let package = orr_package::Project::open(&a.output, runtime).unwrap();
        let lock = package.verify().unwrap();
        assert_eq!(lock.direct.len(), 2);
        assert_eq!(lock.packages.len(), 2);
        assert_eq!(lock.direct[DEFAULT_PACKAGE], "1.0.0");
        assert_eq!(audio.package.locked, lock.packages[DEFAULT_PACKAGE]);
        for (name, bytes) in audio_template::SOURCE.iter().skip(1) {
            assert_eq!(
                package.read_asset(DEFAULT_PACKAGE, name).unwrap(),
                *bytes,
                "{name}"
            );
            assert_eq!(audio.package.files[*name], *bytes, "owned {name}");
        }
        assert!(std::str::from_utf8(&audio.package.files["LICENSE.txt"])
            .unwrap()
            .contains("CC0-1.0"));
        let readme = fs::read_to_string(a.output.join("README.md")).unwrap();
        assert!(readme.contains(COLLECT_AUDIO_TEMPLATE));
        assert!(PreparedProject::open_with_presentation(
            &a.output,
            ProgressSupport::MetadataOnly,
            SpriteSupport::Supported
        )
        .is_err());
        no_stages(temp.path());
    }

    #[test]
    fn new_audio_template_requires_explicit_valid_identity_and_preserves_existing_output() {
        let temp = temporary();
        let option = audio_options(temp.path(), "audio");
        assert!(create(&option).is_err());
        assert!(!option.output.exists());
        for invalid in [
            "",
            "bad",
            "00000000-0000-0000-0000-000000000000",
            "12345678-1234-5234-8234-123456789abc",
        ] {
            assert!(create_collect(&option, invalid).is_err());
            assert!(!option.output.exists());
        }
        create_collect(&option, ID).unwrap();
        let before = files(&option.output);
        assert!(create_collect(&option, ID).is_err());
        assert_eq!(files(&option.output), before);
        no_stages(temp.path());
    }

    #[test]
    fn every_audio_creator_checkpoint_and_publish_failure_preserves_other_content() {
        let temp = temporary();
        let previous = temp.path().join("previous");
        fs::create_dir(&previous).unwrap();
        fs::write(previous.join("keep"), b"prior project").unwrap();
        let prior = files(&previous);
        for (index, point) in [
            "stage-created",
            "project-file-written",
            "source-file-written",
            "before-install",
            "installed",
            "before-validation",
            "before-publish",
        ]
        .iter()
        .enumerate()
        {
            let option = audio_options(temp.path(), &format!("failed-{index}"));
            let result = create_transaction(
                &option,
                Some(&progress()),
                |stage, _| {
                    if stage == *point {
                        Err(format!("injected at {point}"))
                    } else {
                        Ok(())
                    }
                },
                publish_no_replace,
            );
            assert!(result.unwrap_err().contains("injected"), "{point}");
            assert!(!option.output.exists());
            assert_eq!(files(&previous), prior);
            no_stages(temp.path());
        }
        let option = audio_options(temp.path(), "publish-failed");
        assert!(create_transaction(
            &option,
            Some(&progress()),
            |_, _| Ok(()),
            |_, _| Err("publish failure".into())
        )
        .unwrap_err()
        .contains("publish failure"));
        assert!(!option.output.exists());
        assert_eq!(files(&previous), prior);
        no_stages(temp.path());
    }

    #[test]
    fn corrupted_audio_source_and_sidecar_cannot_be_published() {
        let temp = temporary();
        for (index, target) in [
            "LICENSE.txt",
            "source/pickup.json",
            "cooked/view.manifest.bin",
            "cooked/objects/46a3ffa90212574047f6ad40c51c5e81d5644bb4d1a232a73c42d246a78f3eb2.bin",
        ]
        .iter()
        .enumerate()
        {
            let option = audio_options(temp.path(), &format!("corrupt-{index}"));
            let result = create_transaction(
                &option,
                Some(&progress()),
                |stage, root| {
                    if stage == "before-install" {
                        fs::write(
                            root.join("source").join(DEFAULT_PACKAGE).join(target),
                            b"corrupt",
                        )
                        .unwrap();
                    }
                    Ok(())
                },
                publish_no_replace,
            );
            assert!(result.is_err(), "{target}");
            assert!(!option.output.exists());
            no_stages(temp.path());
        }
        for point in ["before-validation", "before-publish"] {
            let option = audio_options(temp.path(), point);
            let result = create_transaction(
                &option,
                Some(&progress()),
                |stage, root| {
                    if stage == point {
                        let mut doc = Document::default_collect();
                        doc.gain = 123;
                        fs::write(
                            root.join("project/level.audio.json"),
                            doc.to_bytes().unwrap(),
                        )
                        .unwrap();
                    }
                    Ok(())
                },
                publish_no_replace,
            );
            assert!(result.is_err());
            assert!(!option.output.exists());
            no_stages(temp.path());
        }
    }

    #[test]
    fn concurrent_destination_is_preserved_and_only_owned_transaction_is_cleaned() {
        let temp = temporary();
        let option = audio_options(temp.path(), "raced");
        let result = create_transaction(
            &option,
            Some(&progress()),
            |stage, _| {
                if stage == "before-publish" {
                    fs::create_dir(&option.output).unwrap();
                    fs::write(option.output.join("keep"), b"concurrent owner").unwrap();
                }
                Ok(())
            },
            publish_no_replace,
        );
        assert!(result.is_err());
        assert_eq!(
            fs::read(option.output.join("keep")).unwrap(),
            b"concurrent owner"
        );
        no_stages(temp.path());
    }
}
