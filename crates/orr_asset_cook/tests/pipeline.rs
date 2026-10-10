use orr_asset::{AssetRef, Domain, Manifest};
use orr_asset_cook::authoring::{AssetType, MAX_SOURCE_BYTES};
use orr_asset_cook::{
    cache_key, cook, hex, inspect_bundle, CookOptions, SIM_MANIFEST, VIEW_MANIFEST,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(0);
struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        for _ in 0..128 {
            let path = std::env::temp_dir().join(format!(
                "orr-asset-cook-test-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => panic!("test temp dir: {e}"),
            }
        }
        panic!("no test directory")
    }
    fn fixture(&self, name: &str) -> PathBuf {
        let root = self.0.join(name);
        fs::create_dir_all(root.join("source")).unwrap();
        fs::write(
            root.join("index.json"),
            include_bytes!("../../../assets/fixture_v1/index.json"),
        )
        .unwrap();
        fs::write(
            root.join("source/motion.json"),
            include_bytes!("../../../assets/fixture_v1/source/motion.json"),
        )
        .unwrap();
        fs::write(
            root.join("source/impact.json"),
            include_bytes!("../../../assets/fixture_v1/source/impact.json"),
        )
        .unwrap();
        root
    }
    fn options(&self, root: &Path, out: &str) -> CookOptions {
        CookOptions {
            index: root.join("index.json"),
            out: self.0.join(out),
            cache: Some(self.0.join("cache")),
            roots: vec![AssetRef::from_raw(0x1001), AssetRef::from_raw(0x2001)],
            check: false,
        }
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn files(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            for object in fs::read_dir(entry.path()).unwrap() {
                let object = object.unwrap();
                result.insert(
                    format!("objects/{}", object.file_name().to_str().unwrap()),
                    fs::read(object.path()).unwrap(),
                );
            }
        } else {
            result.insert(
                entry.file_name().to_str().unwrap().to_string(),
                fs::read(entry.path()).unwrap(),
            );
        }
    }
    result
}
fn runtime_files(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut result = files(path);
    result.remove("report.json");
    result
}

#[test]
fn clean_cold_warm_and_read_only_check_are_exact() {
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let first = temp.options(&root, "first");
    let cold = cook(&first).unwrap();
    assert_eq!(cold.cache_hits, 0);
    let second = temp.options(&root, "second");
    let warm = cook(&second).unwrap();
    assert_eq!(warm.cache_hits, 2);
    assert_eq!(files(&first.out), files(&second.out));
    assert_eq!(
        inspect_bundle(&first.out).unwrap(),
        fs::read(first.out.join("inspect.json")).unwrap()
    );
    let mut check = first.clone();
    check.check = true;
    check.cache = Some(temp.0.join("must-not-create-cache"));
    assert_eq!(cook(&check).unwrap().cache_hits, 0);
    assert!(!check.cache.unwrap().exists());
    assert_eq!(files(&first.out), files(&second.out));
}

#[test]
fn reordered_ids_and_relocated_sources_preserve_runtime_bytes() {
    let temp = TestDir::new();
    let original = temp.fixture("original");
    let relocated = temp.fixture("relocated");
    let mut index: serde_json::Value =
        serde_json::from_slice(&fs::read(relocated.join("index.json")).unwrap()).unwrap();
    let entries = index["entries"].as_array_mut().unwrap();
    entries.reverse();
    entries[1]["source"] = "source/renamed.json".into();
    fs::rename(
        relocated.join("source/motion.json"),
        relocated.join("source/renamed.json"),
    )
    .unwrap();
    fs::write(
        relocated.join("index.json"),
        serde_json::to_vec(&index).unwrap(),
    )
    .unwrap();
    let first = temp.options(&original, "first");
    let second = temp.options(&relocated, "second");
    cook(&first).unwrap();
    cook(&second).unwrap();
    assert_eq!(runtime_files(&first.out), runtime_files(&second.out));
    assert_ne!(
        fs::read(first.out.join("report.json")).unwrap(),
        fs::read(second.out.join("report.json")).unwrap()
    );
}

#[test]
fn whitespace_recooks_without_changing_runtime_identity() {
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let first = temp.options(&root, "first");
    cook(&first).unwrap();
    fs::write(
        root.join("source/motion.json"),
        b" { \"speed_per_tick\" : \"0.0625\" } \n",
    )
    .unwrap();
    let second = temp.options(&root, "second");
    assert_eq!(cook(&second).unwrap().cache_hits, 1);
    assert_eq!(runtime_files(&first.out), runtime_files(&second.out));
}

#[test]
fn cache_corruption_and_partial_entries_are_rebuilt() {
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let first = temp.options(&root, "first");
    cook(&first).unwrap();
    let key = cache_key(
        AssetType::Motion,
        &fs::read(root.join("source/motion.json")).unwrap(),
    );
    let path = first
        .cache
        .as_ref()
        .unwrap()
        .join(format!("{}.bin", hex(&key)));
    fs::write(&path, b"ORAC").unwrap();
    let second = temp.options(&root, "second");
    assert_eq!(cook(&second).unwrap().cache_hits, 1);
    assert_eq!(runtime_files(&first.out), runtime_files(&second.out));
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(&path, bytes).unwrap();
    let third = temp.options(&root, "third");
    assert_eq!(cook(&third).unwrap().cache_hits, 1);
    let fourth = temp.options(&root, "fourth");
    assert_eq!(cook(&fourth).unwrap().cache_hits, 2);
    assert_eq!(runtime_files(&first.out), runtime_files(&fourth.out));
}

#[test]
fn failures_never_publish_or_replace_a_good_bundle() {
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let good = temp.options(&root, "good");
    cook(&good).unwrap();
    let expected = files(&good.out);
    assert!(cook(&good).is_err());
    assert_eq!(files(&good.out), expected);
    fs::write(
        root.join("source/impact.json"),
        b"{\"generator\":\"unknown\"}",
    )
    .unwrap();
    let bad = temp.options(&root, "bad");
    assert!(cook(&bad).is_err());
    assert!(!bad.out.exists());
    assert!(cook(&good).is_err());
    assert_eq!(files(&good.out), expected);
    assert!(!temp.0.join(".good.orr-asset-cook.lock").exists());
    assert!(!temp.0.join(".bad.orr-asset-cook.lock").exists());
}

#[test]
fn declared_tombstone_or_missing_root_is_rejected() {
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let mut options = temp.options(&root, "out");
    options.roots.push(AssetRef::from_raw(0x9999));
    assert!(cook(&options).is_err());
    fs::write(root.join("index.json"), br#"{"format":"orr.asset-index/1","entries":[{"id":"a_0000000000001001","tombstone":true}]}"#).unwrap();
    options.roots = vec![AssetRef::from_raw(0x1001)];
    assert!(cook(&options).is_err());
    assert!(!options.out.exists());
}

#[test]
fn check_detects_drift_extra_files_and_object_corruption() {
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let mut options = temp.options(&root, "out");
    cook(&options).unwrap();
    options.check = true;
    fs::write(options.out.join("extra"), b"untracked").unwrap();
    assert!(cook(&options).is_err());
    fs::remove_file(options.out.join("extra")).unwrap();
    let object = fs::read_dir(options.out.join("objects"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let old = fs::read(&object).unwrap();
    let mut bad = old.clone();
    bad[0] ^= 1;
    fs::write(&object, bad).unwrap();
    assert!(inspect_bundle(&options.out).is_err());
    assert!(cook(&options).is_err());
    fs::write(&object, old).unwrap();
    fs::write(options.out.join("generated_sim.rs"), b"wrong table").unwrap();
    assert!(inspect_bundle(&options.out).is_err());
    assert!(cook(&options).is_err());
}

#[test]
fn bounded_sources_and_total_input_fail_before_output() {
    let temp = TestDir::new();
    let root = temp.fixture("source");
    fs::write(
        root.join("source/motion.json"),
        vec![b' '; MAX_SOURCE_BYTES + 1],
    )
    .unwrap();
    let mut options = temp.options(&root, "out");
    assert!(cook(&options).is_err());
    let mut entries = Vec::new();
    for i in 1..=17 {
        let source = format!("source/{i}.json");
        let mut bytes = br#"{"speed_per_tick":"0.0625"}"#.to_vec();
        bytes.resize(MAX_SOURCE_BYTES, b' ');
        fs::write(root.join(&source), bytes).unwrap();
        entries.push(serde_json::json!({"id":format!("a_{i:016x}"),"type":"sim.motion_profile","schema_version":1,"source":source}));
    }
    fs::write(
        root.join("index.json"),
        serde_json::to_vec(&serde_json::json!({"format":"orr.asset-index/1","entries":entries}))
            .unwrap(),
    )
    .unwrap();
    options.roots.clear();
    assert!(cook(&options).is_err());
    assert!(!options.out.exists());
}

#[cfg(unix)]
#[test]
fn symlink_escape_and_source_aliases_are_rejected() {
    use std::os::unix::fs::symlink;
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let original = fs::read(root.join("source/motion.json")).unwrap();
    fs::write(temp.0.join("outside.json"), &original).unwrap();
    fs::remove_file(root.join("source/motion.json")).unwrap();
    symlink(temp.0.join("outside.json"), root.join("source/motion.json")).unwrap();
    assert!(cook(&temp.options(&root, "escape")).is_err());
    fs::remove_file(root.join("source/motion.json")).unwrap();
    fs::write(root.join("source/motion.json"), original).unwrap();
    symlink(
        root.join("source/motion.json"),
        root.join("source/alias.json"),
    )
    .unwrap();
    let mut index: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("index.json")).unwrap()).unwrap();
    index["entries"].as_array_mut().unwrap().push(serde_json::json!({"id":"a_0000000000003001","type":"sim.motion_profile","schema_version":1,"source":"source/alias.json"}));
    fs::write(root.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
    assert!(cook(&temp.options(&root, "alias")).is_err());
}

#[test]
fn fixed_vectors_bind_payloads_manifests_and_generated_raw_bits() {
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let options = temp.options(&root, "out");
    let summary = cook(&options).unwrap();
    let expected: serde_json::Value =
        serde_json::from_slice(include_bytes!("../../../assets/fixture_v1/vectors.json")).unwrap();
    assert_eq!(
        hex(&summary.sim_manifest_sha256),
        expected["sim_manifest_sha256"].as_str().unwrap()
    );
    assert_eq!(
        hex(&summary.view_manifest_sha256),
        expected["view_manifest_sha256"].as_str().unwrap()
    );
    for (name, domain, hash_key) in [
        (SIM_MANIFEST, Domain::Sim, "motion_payload_sha256"),
        (VIEW_MANIFEST, Domain::View, "impact_payload_sha256"),
    ] {
        let bytes = fs::read(options.out.join(name)).unwrap();
        let manifest = Manifest::decode(&bytes, domain).unwrap();
        assert_eq!(manifest.entries().len(), 1);
        let entry = manifest.entries().next().unwrap();
        assert_eq!(
            hex(&entry.payload_sha256),
            expected[hash_key].as_str().unwrap()
        );
    }
    let generated = fs::read_to_string(options.out.join("generated_sim.rs")).unwrap();
    assert!(generated.contains("generated_motion(4096)"));
}

#[cfg(unix)]
#[test]
fn cache_symlink_repair_does_not_modify_escape_target() {
    use std::os::unix::fs::symlink;
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let options = temp.options(&root, "out");
    let cache = options.cache.as_ref().unwrap();
    fs::create_dir(cache).unwrap();
    let external = temp.0.join("external.bin");
    fs::write(&external, b"preserve me").unwrap();
    let key = cache_key(
        AssetType::Motion,
        &fs::read(root.join("source/motion.json")).unwrap(),
    );
    let cache_entry = cache.join(format!("{}.bin", hex(&key)));
    symlink(&external, &cache_entry).unwrap();
    assert_eq!(cook(&options).unwrap().cache_hits, 0);
    assert_eq!(fs::read(&external).unwrap(), b"preserve me");
    assert!(!fs::symlink_metadata(cache_entry)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn cache_output_overlap_is_rejected_before_any_write() {
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let good = temp.options(&root, "good");
    cook(&good).unwrap();
    let before = files(&good.out);
    for cache in [
        good.out.clone(),
        good.out.join("objects"),
        good.out.join("new-cache"),
    ] {
        let mut bad = good.clone();
        bad.cache = Some(cache);
        assert!(cook(&bad).is_err());
        assert_eq!(files(&good.out), before);
    }
    for name in ["new-equal", "new-nested", "new-ancestor"] {
        let mut bad = temp.options(&root, name);
        bad.cache = Some(match name {
            "new-equal" => bad.out.clone(),
            "new-nested" => bad.out.join("cache"),
            _ => temp.0.clone(),
        });
        assert!(cook(&bad).is_err());
        assert!(!bad.out.exists());
    }
}

#[cfg(unix)]
#[test]
fn cache_output_alias_and_missing_suffix_overlap_are_rejected() {
    use std::os::unix::fs::symlink;
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let good = temp.options(&root, "good");
    cook(&good).unwrap();
    let before = files(&good.out);
    let alias = temp.0.join("alias");
    symlink(&temp.0, &alias).unwrap();
    let mut bad = good.clone();
    bad.cache = Some(alias.join("good/objects"));
    assert!(cook(&bad).is_err());
    assert_eq!(files(&good.out), before);
    let mut fresh = temp.options(&root, "fresh");
    fresh.cache = Some(alias.join("fresh/cache"));
    assert!(cook(&fresh).is_err());
    assert!(!fresh.out.exists());
    fresh.cache = Some(alias.join("missing/../fresh/cache"));
    assert!(cook(&fresh).is_err());
    assert!(!fresh.out.exists());
    assert!(!temp.0.join("missing").exists());
    fs::create_dir(temp.0.join("existing")).unwrap();
    fresh.cache = Some(temp.0.join("existing/../alias/fresh/cache"));
    assert!(cook(&fresh).is_err());
    assert!(!fresh.out.exists());
    // Resolve a symlink before consuming '..': alias/source/../fresh is the
    // original parent/fresh, rather than an unrelated lexical path.
    fresh.cache = Some(alias.join("source/../fresh/cache"));
    assert!(cook(&fresh).is_err());
    assert!(!fresh.out.exists());
}

#[test]
fn normalized_cache_path_cannot_create_missing_output_as_a_side_effect() {
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let mut options = temp.options(&root, "new-bundle");
    options.cache = Some(options.out.join("../actual-cache"));
    fs::write(root.join("source/impact.json"), b"{}").unwrap();
    assert!(cook(&options).is_err());
    assert!(!options.out.exists());
    assert!(temp.0.join("actual-cache").is_dir());
    fs::write(
        root.join("source/impact.json"),
        include_bytes!("../../../assets/fixture_v1/source/impact.json"),
    )
    .unwrap();
    cook(&options).unwrap();
    inspect_bundle(&options.out).unwrap();
}

#[test]
fn missing_cache_output_aliases_are_conservatively_case_insensitive() {
    let temp = TestDir::new();
    let root = temp.fixture("source");
    let mut options = temp.options(&root, "NewBundle");
    for alias in ["newbundle/cache", "newbundle./cache", "NEWBUNDLE /cache"] {
        options.cache = Some(temp.0.join(alias));
        assert!(cook(&options).is_err());
        assert!(!options.out.exists());
        assert!(!temp.0.join(alias).exists());
    }
}
