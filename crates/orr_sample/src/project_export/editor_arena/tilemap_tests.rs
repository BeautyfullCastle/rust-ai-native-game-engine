//! Exact editor receipts against real installed content; no runtime executed.
use super::*;
use crate::project_export::admission::hash;
use std::fs;
#[path = "../../../tests/common/project.rs"]
mod fixture;
use fixture::ProjectFixture;
const MAP: &str = "arena.tilemap.json";
fn fixture() -> ProjectFixture {
    let f = ProjectFixture::new();
    let path = f.root.join(PROJECT_MANIFEST);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["entry"]["tilemap"] = MAP.into();
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    fs::write(f.root.join(MAP), serde_json::to_vec(&serde_json::json!({
        "version":1,"sprite":{"package":"sample-sprites","document":"sprites.json"},
        "width":2,"height":1,"tile_size":[16,16],"origin":[0,0],"units_per_pixel":1,"cells":[10,11]
    })).unwrap()).unwrap();
    f
}
fn capture(root: &Path) -> ArenaExportSource {
    let snapshot = ProjectSnapshot::open(root).unwrap();
    let inventory = [
        PROJECT_MANIFEST,
        PACKAGE_LOCK,
        "arena.scene.yaml",
        "arena.sprites.json",
        MAP,
    ]
    .into_iter()
    .map(|path| {
        let file = snapshot.files.iter().find(|f| f.relative == path).unwrap();
        ArenaSourceFile::new(
            path.into(),
            Some(ArenaSourceDigest::new(file.bytes.len() as u64, file.sha256.clone()).unwrap()),
        )
        .unwrap()
    })
    .collect();
    ArenaExportSource::new(
        root.join("arena.scene.yaml"),
        snapshot.initial_checksum,
        inventory,
    )
    .unwrap()
}
fn reject_before_runtime(root: &Path, source: &ArenaExportSource) -> String {
    let options = ExportOptions {
        project: root.into(),
        runtime: root.join("runtime-must-not-be-inspected"),
        runtime_sha256: "0".repeat(64),
        output: root.join("never-published"),
        trusted_runtime: true,
        source_revision: None,
    };
    let error = export_profile_for_source_with(
        &options,
        Profile::Arena,
        Some(source),
        |_| panic!("invalid source reached admitted checkpoint"),
        |_| panic!("invalid source reached smoke"),
    )
    .unwrap_err();
    assert!(!error.contains("runtime-must-not-be-inspected"), "{error}");
    assert!(!options.output.exists());
    assert!(fs::read_dir(root).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".orr-export-")));
    error
}
#[test]
fn five_file_tile_receipt_matches_exact_admitted_source() {
    let f = fixture();
    let source = capture(&f.root);
    assert_eq!(source.inventory().len(), 5);
    let tile = source.inventory().iter().find(|v| v.path() == MAP).unwrap();
    let bytes = fs::read(f.root.join(MAP)).unwrap();
    assert_eq!(tile.present().unwrap().bytes(), bytes.len() as u64);
    assert_eq!(tile.present().unwrap().sha256(), hash(&bytes));
    source
        .check_snapshot(&ProjectSnapshot::open(&f.root).unwrap())
        .unwrap();
}
#[test]
fn tile_receipt_missing_wrong_path_digest_and_length_refuse_before_runtime() {
    let f = fixture();
    for mode in 0..4 {
        let mut source = capture(&f.root);
        let tile = source.inventory.iter_mut().find(|v| v.path == MAP).unwrap();
        match mode {
            0 => source.inventory.retain(|v| v.path != MAP),
            1 => tile.path = "other.tilemap.json".into(),
            2 => tile.present.as_mut().unwrap().sha256 = "0".repeat(64),
            _ => tile.present.as_mut().unwrap().bytes += 1,
        }
        reject_before_runtime(&f.root, &source);
    }
}
#[test]
fn changed_or_deleted_saved_tile_refuses_before_runtime() {
    for remove in [false, true] {
        let f = fixture();
        let source = capture(&f.root);
        if remove {
            fs::remove_file(f.root.join(MAP)).unwrap();
        } else {
            let mut document: serde_json::Value =
                serde_json::from_slice(&fs::read(f.root.join(MAP)).unwrap()).unwrap();
            document["cells"][0] = 11.into();
            fs::write(f.root.join(MAP), serde_json::to_vec(&document).unwrap()).unwrap();
        }
        reject_before_runtime(&f.root, &source);
    }
}
#[test]
fn tile_receipt_preserves_absence_and_duplicate_guards() {
    let f = fixture();
    let source = capture(&f.root);
    for mode in 0..3 {
        let mut changed = source.clone();
        let tile = changed
            .inventory
            .iter_mut()
            .find(|v| v.path == MAP)
            .unwrap();
        match mode {
            0 => tile.present = None,
            1 => tile.path = PROJECT_MANIFEST.into(),
            _ => tile.path = "ARENA.SPRITES.JSON".into(),
        }
        assert!(changed.validate().is_err());
        reject_before_runtime(&f.root, &changed);
    }
}
