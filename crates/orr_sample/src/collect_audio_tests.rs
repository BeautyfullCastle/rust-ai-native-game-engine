use super::*;
use std::fs;

fn runtime() -> orr_package::Runtime {
    let mut runtime = orr_package::Runtime::content_only();
    runtime.capabilities.insert("collect-audio".into());
    runtime
}
fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/packages/collect-audio-v1")
        .canonicalize()
        .unwrap()
}
pub(crate) fn fixture() -> tempfile::TempDir {
    let root = std::env::temp_dir().canonicalize().unwrap();
    let temp = tempfile::tempdir_in(root).unwrap();
    fs::write(temp.path().join("orr.project.json"), br#"{"schema":2,"engine":"*","entry":{"game":"collect-dodge-v1","scene":"scene.yaml","audio":"collect.audio.json"}}"#).unwrap();
    fs::write(
        temp.path().join("collect.audio.json"),
        Document::default_collect().to_bytes().unwrap(),
    )
    .unwrap();
    let project = orr_package::Project::open(temp.path(), runtime()).unwrap();
    project.install(&[source()]).unwrap();
    temp
}
pub(crate) fn prepared(root: &Path) -> Result<PreparedAudio, String> {
    let project = orr_package::Project::open(root, runtime()).map_err(|e| e.to_string())?;
    PreparedAudio::load(&project, "collect.audio.json")
}
fn copy_package(destination: &Path) {
    let package: orr_package::Manifest =
        serde_json::from_slice(&fs::read(source().join("orr.package.json")).unwrap()).unwrap();
    fs::create_dir_all(destination).unwrap();
    for path in package
        .files
        .iter()
        .map(String::as_str)
        .chain(std::iter::once("orr.package.json"))
    {
        let target = destination.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(source().join(path), target).unwrap();
    }
}
fn replace_package(root: &Path, source: &Path) {
    // Immutable package versions cannot be rewritten even in adversarial fixtures.
    let path = source.join("orr.package.json");
    let mut manifest: orr_package::Manifest =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest.version = "1.0.1".into();
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let project = orr_package::Project::open(root, runtime()).unwrap();
    project.install(&[source.to_path_buf()]).unwrap();
}
fn rewrite_package(source: &Path, payloads: &[(u64, Vec<u8>)]) {
    let mut entries = Vec::new();
    let mut files = std::collections::BTreeSet::new();
    for (id, payload) in payloads {
        let digest: [u8; 32] = Sha256::digest(payload).into();
        let path = format!("cooked/objects/{}.bin", hex(&digest));
        fs::create_dir_all(source.join("cooked/objects")).unwrap();
        fs::write(source.join(&path), payload).unwrap();
        files.insert(path);
        entries.push(orr_asset::ManifestEntry {
            id: AssetRef::from_raw(*id),
            type_id: orr_asset::IMPACT_PCM16_TYPE_ID,
            schema_version: 1,
            payload_len: payload.len() as u64,
            payload_sha256: digest,
        });
    }
    let mut bytes = vec![0; orr_asset::manifest_encoded_len(Domain::View, entries.len()).unwrap()];
    orr_asset::encode_manifest(Domain::View, &entries, &mut bytes).unwrap();
    fs::write(source.join(DEFAULT_MANIFEST), bytes).unwrap();
    files.insert(DEFAULT_MANIFEST.into());
    let manifest = orr_package::Manifest {
        schema: 1,
        name: DEFAULT_PACKAGE.into(),
        version: "1.0.0".into(),
        engine: "*".into(),
        capabilities: ["collect-audio".into()].into(),
        dependencies: Default::default(),
        files,
    };
    fs::write(
        source.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}
fn pcm(frames: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(&frames.to_le_bytes());
    bytes.resize(8 + frames as usize * 2, 0);
    bytes
}

#[test]
fn strict_closed_json_rejects_all_ambiguous_shapes() {
    let document = Document::default_collect();
    let bytes = document.to_bytes().unwrap();
    assert_eq!(Document::parse(&bytes).unwrap(), document);
    let value = serde_json::to_value(&document).unwrap();
    for field in ["version", "pickup", "gain", "mute"] {
        let mut bad = value.clone();
        bad.as_object_mut().unwrap().remove(field);
        assert!(
            Document::parse(bad.to_string().as_bytes()).is_err(),
            "missing {field}"
        );
        let mut bad = value.clone();
        bad[field] = serde_json::Value::Null;
        assert!(
            Document::parse(bad.to_string().as_bytes()).is_err(),
            "null {field}"
        );
        let duplicate = format!("{{\"{field}\":{},{}", value[field], &value.to_string()[1..]);
        assert!(
            Document::parse(duplicate.as_bytes()).is_err(),
            "duplicate {field}"
        );
    }
    for field in ["package", "manifest", "asset"] {
        let mut bad = value.clone();
        bad["pickup"].as_object_mut().unwrap().remove(field);
        assert!(Document::parse(bad.to_string().as_bytes()).is_err());
        let mut bad = value.clone();
        bad["pickup"][field] = serde_json::Value::Null;
        assert!(Document::parse(bad.to_string().as_bytes()).is_err());
        let pickup = &value["pickup"];
        let duplicate = format!(
            "{{\"{field}\":{},{}",
            pickup[field],
            &pickup.to_string()[1..]
        );
        let bad = format!(r#"{{"version":1,"pickup":{duplicate},"gain":700,"mute":false}}"#);
        assert!(Document::parse(bad.as_bytes()).is_err());
    }
    for bad in [
        "null",
        "[]",
        "true",
        "1",
        "\"audio\"",
        "{}",
        r#"[1,{},700,false]"#,
        r#"{"version":1,"pickup":["collect-audio-v1","cooked/view.manifest.bin","a_0000000000003001"],"gain":700,"mute":false}"#,
    ] {
        assert!(Document::parse(bad.as_bytes()).is_err(), "{bad}");
    }
    for bad in [
        serde_json::json!(true),
        serde_json::json!("700"),
        serde_json::json!(0.5),
        serde_json::json!(-1),
        serde_json::json!(1001),
        serde_json::json!(65536),
    ] {
        let mut value = value.clone();
        value["gain"] = bad;
        assert!(Document::parse(value.to_string().as_bytes()).is_err());
    }
    for bad in ["1.0", "1e2", "0e0"] {
        let encoded = format!(
            r#"{{"version":1,"pickup":{},"gain":{bad},"mute":false}}"#,
            value["pickup"]
        );
        assert!(Document::parse(encoded.as_bytes()).is_err());
    }
    let mut extra = value.clone();
    extra["extra"] = true.into();
    assert!(Document::parse(extra.to_string().as_bytes()).is_err());
    let mut extra = value;
    extra["pickup"]["extra"] = true.into();
    assert!(Document::parse(extra.to_string().as_bytes()).is_err());
    assert!(Document::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
    for gain in [0, 1, 1000] {
        let mut d = document.clone();
        d.gain = gain;
        assert!(d.to_bytes().is_ok());
    }
}

#[test]
fn strict_versions_ids_and_paths_are_checked_before_package_lookup() {
    for version in [0, 2, u32::MAX] {
        let mut d = Document::default_collect();
        d.version = version;
        assert!(d.validate().is_err());
    }
    for path in [
        "",
        "../view.manifest.bin",
        "/view.manifest.bin",
        "C:/view.manifest.bin",
        "cooked\\view.bin",
        "a//b",
        "a/./b",
        "a/../b",
        "NUL.bin",
        "café.bin",
    ] {
        let mut d = Document::default_collect();
        d.pickup.manifest = path.into();
        assert!(d.validate().is_err(), "{path}");
    }
    for id in [
        "",
        "1",
        "a_0000000000000000",
        "a_000000000000300A",
        "a_123",
        "A_0000000000003001",
    ] {
        let mut d = Document::default_collect();
        d.pickup.asset = id.into();
        assert!(d.validate().is_err(), "{id}");
    }
    for name in ["", "../package", "Collect", "a/b", "a b"] {
        let mut d = Document::default_collect();
        d.pickup.package = name.into();
        assert!(d.validate().is_err());
    }
}

#[test]
fn real_owned_bank_admits_two_distinct_clips_and_survives_source_removal() {
    let temp = fixture();
    let first = prepared(temp.path()).unwrap();
    assert_eq!(first.stats.records, 2);
    assert_eq!(first.stats.manifest_bytes, 128);
    assert_eq!(first.stats.cooked_bytes, 24_016);
    assert_eq!(first.stats.decoded_bytes, 96_000);
    assert_eq!(
        first
            .available
            .iter()
            .map(|c| c.asset.as_str())
            .collect::<Vec<_>>(),
        [DEFAULT_ASSET, ALTERNATE_ASSET]
    );
    assert_ne!(
        first.available[0].payload_path,
        first.available[1].payload_path
    );
    let mut next = first.document.clone();
    next.pickup.asset = ALTERNATE_ASSET.into();
    next.gain = 250;
    next.mute = true;
    let second = first.with_document(next).unwrap();
    assert_ne!(first.document, second.document);
    assert_eq!(first.package, second.package);
    assert_eq!(second.bytes, second.document.to_bytes().unwrap());
    fs::remove_file(&first.path).unwrap();
    fs::remove_dir_all(temp.path().join(".orr")).unwrap();
    assert!(first.pickup_clip().is_ok());
    assert!(second.pickup_clip().is_ok());
    assert!(first.with_document(second.document.clone()).is_ok());
    for role in [0, 1, 2] {
        let mut invalid = first.document.clone();
        match role {
            0 => invalid.pickup.package = "other".into(),
            1 => invalid.pickup.manifest = "other.bin".into(),
            _ => invalid.pickup.asset = "a_0000000000003999".into(),
        }
        assert!(first.with_document(invalid).is_err());
    }
}

#[test]
fn compiled_capability_and_matching_collect_entry_are_required() {
    let temp = fixture();
    let project =
        orr_package::Project::open(temp.path(), orr_package::Runtime::content_only()).unwrap();
    assert!(PreparedAudio::load(&project, "collect.audio.json").is_err());
    let project = orr_package::Project::open(temp.path(), runtime()).unwrap();
    assert!(PreparedAudio::load(&project, "../collect.audio.json").is_err());
    assert!(PreparedAudio::from_bytes(
        &project,
        "other.json",
        &Document::default_collect().to_bytes().unwrap()
    )
    .is_err());
    fs::write(temp.path().join("orr.project.json"), br#"{"schema":2,"engine":"*","entry":{"game":"arena","scene":"scene.yaml","audio":"collect.audio.json"}}"#).unwrap();
    assert!(orr_package::Project::open(temp.path(), runtime()).is_err());
}

#[test]
fn every_package_byte_and_manifest_identity_are_verified() {
    for target in ["source/pickup.json", DEFAULT_MANIFEST, "orr.package.json"] {
        let temp = fixture();
        let project = orr_package::Project::open(temp.path(), runtime()).unwrap();
        let lock = project.list().unwrap();
        let object = temp
            .path()
            .join(".orr/packages/objects")
            .join(&lock.packages[DEFAULT_PACKAGE].digest);
        fs::write(object.join(target), b"tampered").unwrap();
        assert!(prepared(temp.path()).is_err(), "{target}");
    }
}

#[test]
fn malformed_oram_and_payload_fail_even_with_fresh_valid_package_hashes() {
    for offset in [0, 4, 8, 12, 24, 28, 32, 40] {
        let temp = fixture();
        let package = temp.path().join("edited");
        copy_package(&package);
        let manifest = package.join(DEFAULT_MANIFEST);
        let mut bytes = fs::read(&manifest).unwrap();
        bytes[offset] ^= 0xff;
        fs::write(manifest, bytes).unwrap();
        replace_package(temp.path(), &package);
        assert!(prepared(temp.path()).is_err(), "offset {offset}");
    }
    for failure in [0, 1, 2, 3] {
        let temp = fixture();
        let package = temp.path().join("edited");
        let mut payload = pcm(4);
        match failure {
            0 => payload[..4].copy_from_slice(&44_100u32.to_le_bytes()),
            1 => payload[4..8].copy_from_slice(&3u32.to_le_bytes()),
            2 => payload[8..10].copy_from_slice(&8_193i16.to_le_bytes()),
            _ => payload[8..10].copy_from_slice(&i16::MIN.to_le_bytes()),
        }
        rewrite_package(&package, &[(0x3001, payload)]);
        replace_package(temp.path(), &package);
        assert!(prepared(temp.path()).is_err(), "PCM failure {failure}");
    }
}

#[test]
fn cumulative_pcm_budget_is_admitted_before_conversion() {
    let temp = fixture();
    let package = temp.path().join("edited");
    let payload = pcm(48_000);
    let clips: Vec<_> = (0x3001..=0x300b).map(|id| (id, payload.clone())).collect();
    rewrite_package(&package, &clips);
    replace_package(temp.path(), &package);
    let error = prepared(temp.path()).err().unwrap();
    assert!(error.contains("preload budget"), "{error}");
}

#[test]
fn sidecar_file_and_owned_package_reads_respect_tighter_bounds() {
    let temp = fixture();
    let project = orr_package::Project::open(temp.path(), runtime()).unwrap();
    assert!(project
        .read_package_bounded(DEFAULT_PACKAGE, 1, MAX_PACKAGE_BYTES, MAX_PACKAGE_BYTES)
        .is_err());
    assert!(project
        .read_package_bounded(DEFAULT_PACKAGE, MAX_PACKAGE_FILES, 64, MAX_PACKAGE_BYTES)
        .is_err());
    assert!(project
        .read_package_bounded(DEFAULT_PACKAGE, MAX_PACKAGE_FILES, MAX_PACKAGE_BYTES, 1000)
        .is_err());
    fs::write(
        temp.path().join("collect.audio.json"),
        vec![b' '; MAX_BYTES + 1],
    )
    .unwrap();
    assert!(PreparedAudio::load(&project, "collect.audio.json").is_err());
}

#[cfg(unix)]
#[test]
fn symlink_and_directory_inputs_are_never_followed() {
    use std::os::unix::fs::symlink;
    for package_file in [false, true] {
        let temp = fixture();
        let project = orr_package::Project::open(temp.path(), runtime()).unwrap();
        let target = if package_file {
            let lock = project.list().unwrap();
            temp.path()
                .join(".orr/packages/objects")
                .join(&lock.packages[DEFAULT_PACKAGE].digest)
                .join(DEFAULT_MANIFEST)
        } else {
            temp.path().join("collect.audio.json")
        };
        let backup = temp.path().join("backup");
        fs::rename(&target, &backup).unwrap();
        symlink(&backup, &target).unwrap();
        assert!(PreparedAudio::load(&project, "collect.audio.json").is_err());
        fs::remove_file(&target).unwrap();
        fs::create_dir(&target).unwrap();
        assert!(PreparedAudio::load(&project, "collect.audio.json").is_err());
    }
}

#[cfg(unix)]
#[test]
#[expect(
    clippy::disallowed_types,
    reason = "wall-clock timeout only for isolated FIFO regression"
)]
fn fifo_sidecars_and_package_files_fail_without_blocking() {
    use std::{
        process::Command,
        thread,
        time::{Duration, Instant},
    };
    const CHILD_ROOT: &str = "ORR_COLLECT_AUDIO_FIFO_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let error = prepared(&root).err().expect("FIFO must be rejected");
        assert!(error.contains("regular file"), "{error}");
        return;
    }
    for package_file in [false, true] {
        let temp = fixture();
        let target = if package_file {
            let project = orr_package::Project::open(temp.path(), runtime()).unwrap();
            let lock = project.list().unwrap();
            temp.path()
                .join(".orr/packages/objects")
                .join(&lock.packages[DEFAULT_PACKAGE].digest)
                .join(DEFAULT_MANIFEST)
        } else {
            temp.path().join("collect.audio.json")
        };
        fs::remove_file(&target).unwrap();
        assert!(Command::new("mkfifo")
            .arg(&target)
            .status()
            .unwrap()
            .success());
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "collect_audio::tests::fifo_sidecars_and_package_files_fail_without_blocking",
                "--nocapture",
            ])
            .env(CHILD_ROOT, temp.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                let output = child.wait_with_output().unwrap();
                assert!(
                    status.success(),
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("FIFO read blocked Collect audio admission");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
