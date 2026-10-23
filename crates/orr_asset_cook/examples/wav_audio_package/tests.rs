use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "orr-wav-package-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn options(&self) -> Options {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&38u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&48000u32.to_le_bytes());
        wav.extend_from_slice(&96000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&2u32.to_le_bytes());
        wav.extend_from_slice(&8192i16.to_le_bytes());
        fs::write(self.0.join("tone.wav"), wav).unwrap();
        fs::write(self.0.join("license"), b"Test license\n").unwrap();
        Options {
            wav: self.0.join("tone.wav"),
            id: AssetRef::from_raw(7),
            name: "collect-sfx".into(),
            version: "1.0.0".into(),
            license: self.0.join("license"),
            out: self.0.join("package"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn canonical_names_and_versions() {
    for name in ["collect-sfx", "test_2", "0"] {
        assert!(validate_name(name));
    }
    for name in ["", "Upper", "../x", "a.b", "a/b", "é"] {
        assert!(!validate_name(name));
    }
    assert!(!validate_name(&"x".repeat(65)));
    for version in [
        "0.0.1",
        "1.2.3",
        "1.2.3-alpha.1+build.01",
        "18446744073709551615.0.0",
    ] {
        assert!(validate_version(version), "{version}");
    }
    for version in [
        "",
        "v1.2.3",
        "1.2",
        "01.2.3",
        "1.2.3-01",
        "1.2.3+",
        "1.2.3-",
        "1.2.3+a+b",
        "1.2.3-ä",
        "18446744073709551616.0.0",
        " 1.2.3",
    ] {
        assert!(!validate_version(version), "{version}");
    }
}

#[test]
fn deterministic_package_matches_public_codec_and_preserves_inputs() {
    let fixture = Fixture::new();
    let mut options = fixture.options();
    let before_wav = fs::read(&options.wav).unwrap();
    let before_license = fs::read(&options.license).unwrap();
    let digest = create(&options).unwrap();
    let first = options.out.clone();
    options.out = fixture.0.join("second");
    assert_eq!(create(&options).unwrap(), digest);
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(first.join("orr.package.json")).unwrap()).unwrap();
    assert_eq!(metadata["schema"], 1);
    assert_eq!(metadata["engine"], "^0.0.1");
    assert_eq!(
        metadata["capabilities"],
        serde_json::json!(["collect-audio"])
    );
    assert_eq!(metadata["dependencies"], serde_json::json!({}));
    let files = metadata["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    for name in files
        .iter()
        .map(|v| v.as_str().unwrap())
        .chain(["orr.package.json"])
    {
        assert_eq!(
            fs::read(first.join(name)).unwrap(),
            fs::read(options.out.join(name)).unwrap()
        );
    }
    assert!(!files.contains(&serde_json::json!("orr.package.json")));
    let view = fs::read(first.join("cooked/view.manifest.bin")).unwrap();
    let manifest = orr_asset::Manifest::decode(&view, Domain::View).unwrap();
    let entries: Vec<_> = manifest.entries().collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, options.id);
    assert_eq!(entries[0].type_id, 2);
    assert_eq!(entries[0].schema_version, 1);
    let payload = fs::read(first.join(format!("cooked/objects/{digest}.bin"))).unwrap();
    assert_eq!(sha256(&payload), entries[0].payload_sha256);
    assert_eq!(payload.len() as u64, entries[0].payload_len);
    orr_asset_cook::authoring::validate_payload(
        orr_asset_cook::authoring::AssetType::Impact,
        &payload,
    )
    .unwrap();
    assert_eq!(fs::read(options.wav).unwrap(), before_wav);
    assert_eq!(fs::read(options.license).unwrap(), before_license);
}

#[test]
fn rejects_existing_output_and_invalid_inputs_without_output() {
    let fixture = Fixture::new();
    let mut options = fixture.options();
    fs::create_dir(&options.out).unwrap();
    fs::write(options.out.join("sentinel"), b"keep").unwrap();
    assert!(create(&options).is_err());
    assert_eq!(fs::read(options.out.join("sentinel")).unwrap(), b"keep");
    options.out = fixture.0.join("new");
    options.id = AssetRef::NULL;
    assert!(create(&options).is_err());
    options.id = AssetRef::from_raw(1);
    fs::write(&options.license, []).unwrap();
    assert!(create(&options).is_err());
    fs::write(&options.license, [255]).unwrap();
    assert!(create(&options).is_err());
    fs::write(&options.license, vec![b'a'; MAX_LICENSE_BYTES + 1]).unwrap();
    assert!(create(&options).is_err());
    fs::write(&options.license, b"license").unwrap();
    fs::write(&options.wav, b"invalid").unwrap();
    assert!(create(&options).is_err());
    assert!(!options.out.exists());
}

#[test]
fn rejects_parent_traversal_missing_parent_and_directory_input() {
    let fixture = Fixture::new();
    let mut options = fixture.options();
    options.out = fixture.0.join("missing/package");
    assert!(create(&options).is_err());
    assert!(!fixture.0.join("missing").exists());
    options.out = fixture.0.join("../package");
    assert!(create(&options).is_err());
    options.out = fixture.0.join("new");
    options.wav = fixture.0.clone();
    assert!(create(&options).is_err());
    assert!(!options.out.exists());
}

#[test]
fn read_budget_and_failed_publication_cleanup() {
    let fixture = Fixture::new();
    let options = fixture.options();
    assert!(read_regular(&options.wav, 1).is_err());
    let files = vec![("missing/file".to_owned(), vec![1])];
    assert!(publish(&options.out, &files).is_err());
    assert!(!options.out.exists());
    fs::write(&options.out, b"sentinel").unwrap();
    assert!(publish(&options.out, &[]).is_err());
    assert_eq!(fs::read(&options.out).unwrap(), b"sentinel");
}

#[cfg(unix)]
#[test]
fn rejects_symlink_inputs_ancestors_and_output() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let mut options = fixture.options();
    symlink(&options.wav, fixture.0.join("link.wav")).unwrap();
    options.wav = fixture.0.join("link.wav");
    assert!(create(&options).is_err());
    options.wav = fixture.0.join("tone.wav");
    symlink(&fixture.0, fixture.0.join("alias")).unwrap();
    options.out = fixture.0.join("alias/new");
    assert!(create(&options).is_err());
    symlink(fixture.0.join("absent"), fixture.0.join("dangling")).unwrap();
    options.out = fixture.0.join("dangling");
    assert!(create(&options).is_err());
    options.out = fixture.0.join("new");
    options.license = fixture.0.join("alias/license");
    assert!(create(&options).is_err());
    assert!(!options.out.exists());
}
