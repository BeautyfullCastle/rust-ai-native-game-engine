use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use std::collections::BTreeMap;

pub(crate) const DIGEST: &str = "b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0";

/// Writes one installed object file where the package store keeps it.
pub(crate) fn stage(root: &Path, digest: &str, asset: &str, bytes: &[u8]) {
    let path = root.join(".orr/packages/objects").join(digest).join(asset);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

pub(crate) const STATIC_HEADER: &[u8] = br#"{"format":"orr_static_model","version":1,"asset_id":"a"}"#;
pub(crate) const ANIMATED_HEADER: &[u8] =
    br#"{"format":"orr_animated_model","version":1,"asset_id":"a"}"#;

pub(crate) fn lock_of(package: &str, capabilities: &[&str], files: &[&str]) -> Lock {
    let manifest = orr_package::Manifest {
        schema: 1,
        name: package.into(),
        version: "1.0.0".into(),
        engine: "*".into(),
        capabilities: capabilities.iter().map(|name| (*name).into()).collect(),
        dependencies: BTreeMap::new(),
        files: files.iter().map(|name| (*name).into()).collect(),
    };
    Lock {
        schema: 1,
        direct: BTreeMap::new(),
        packages: BTreeMap::from([(
            package.into(),
            orr_package::LockedPackage {
                files: manifest
                    .files
                    .iter()
                    .map(|path| (path.clone(), "a".repeat(64)))
                    .collect(),
                manifest,
                digest: DIGEST.into(),
            },
        )]),
    }
}

pub(crate) fn browser_harness(root: &Path, lock: Lock) -> Harness<'static, Panel> {
    let panel = Panel {
        root: Some(root.to_owned()),
        source: "/source".into(),
        source_identity: "/source".into(),
        verified: Some(lock),
        ..Default::default()
    };
    Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(
            |ui, panel: &mut Panel| {
                model_asset_buttons(
                    ui,
                    ModelBrowser {
                        lock: panel.verified.as_ref().unwrap(),
                        filter: &mut panel.model_filter,
                        preview_request: &mut panel.preview_request,
                    },
                    true,
                    &mut panel.model_request,
                    panel.root.as_ref(),
                    &panel.source,
                    (panel.project_generation, panel.source_generation),
                );
            },
            panel,
        )
}

#[test]
fn invalid_cooked_json_is_not_offered_and_valid_json_is() {
    let root = tempfile::tempdir().unwrap();
    for (asset, bytes) in [
        ("good.model.json", STATIC_HEADER),
        ("bad.model.json", b"this is not json".as_slice()),
        ("other.model.json", br#"{"hello":"world"}"#.as_slice()),
        (
            "future.model.json",
            br#"{"format":"orr_static_model","version":2}"#.as_slice(),
        ),
        ("swapped.model.json", ANIMATED_HEADER),
    ] {
        stage(root.path(), DIGEST, asset, bytes);
    }
    // "missing.model.json" is declared by the lock but has no installed file.
    let lock = lock_of(
        "pack",
        &["models"],
        &[
            "good.model.json",
            "bad.model.json",
            "other.model.json",
            "future.model.json",
            "swapped.model.json",
            "missing.model.json",
        ],
    );
    let mut harness = browser_harness(root.path(), lock.clone());
    harness.run_steps(3);
    assert!(harness
        .query_by_label("Showing 1 of 1 verified model assets")
        .is_some());
    assert!(harness
        .query_by_label("Preview static model pack/good.model.json")
        .is_some());
    assert!(harness
        .query_by_label("Use model pack/good.model.json")
        .is_some());
    for rejected in [
        "bad.model.json",
        "other.model.json",
        "future.model.json",
        "swapped.model.json",
        "missing.model.json",
    ] {
        assert!(
            harness
                .query_by_label(&format!("Preview static model pack/{rejected}"))
                .is_none(),
            "{rejected} must not be offered"
        );
        assert!(
            harness
                .query_by_label(&format!("Use model pack/{rejected}"))
                .is_none(),
            "{rejected} must not be offered"
        );
    }
    assert!(harness
        .query_by_label("Not offered: 5 cooked JSON files are not valid models.")
        .is_some());
    assert!(harness
        .query_by_label_contains("Not offered pack/bad.model.json:")
        .is_some());
    assert!(harness
        .query_by_label_contains("Not offered pack/swapped.model.json: the file name is for a static model but the content is an animated model")
        .is_some());
    assert_eq!(harness.state().verified.as_ref(), Some(&lock));
}

#[test]
fn animated_json_is_checked_against_its_own_format() {
    let root = tempfile::tempdir().unwrap();
    stage(root.path(), DIGEST, "good.animated.json", ANIMATED_HEADER);
    stage(root.path(), DIGEST, "bad.animated.json", b"{");
    let lock = lock_of(
        "clips",
        &["models", "animation"],
        &["good.animated.json", "bad.animated.json"],
    );
    let mut harness = browser_harness(root.path(), lock);
    harness.run_steps(3);
    assert!(harness
        .query_by_label_contains("Not offered clips/bad.animated.json:")
        .is_some());
    assert!(harness
        .query_by_label("Preview animated model clips/bad.animated.json")
        .is_none());
    if cfg!(feature = "room-animated-models") {
        assert!(harness
            .query_by_label("Showing 1 of 1 verified model assets")
            .is_some());
        assert!(harness
            .query_by_label("Preview animated model clips/good.animated.json")
            .is_some());
        assert!(harness
            .query_by_label("Use animated model clips/good.animated.json")
            .is_some());
    }
}

#[test]
fn a_header_beyond_the_probe_limit_is_not_offered() {
    let root = tempfile::tempdir().unwrap();
    let mut late = br#"{"padding":""#.to_vec();
    late.extend(std::iter::repeat_n(b'x', 64 * 1024));
    late.extend_from_slice(br#"","format":"orr_static_model","version":1}"#);
    stage(root.path(), DIGEST, "late.model.json", &late);
    let mut harness = browser_harness(
        root.path(),
        lock_of("pack", &["models"], &["late.model.json"]),
    );
    harness.run_steps(3);
    assert!(harness
        .query_by_label("Use model pack/late.model.json")
        .is_none());
    assert!(harness
        .query_by_label_contains("Not offered pack/late.model.json:")
        .is_some());
}
