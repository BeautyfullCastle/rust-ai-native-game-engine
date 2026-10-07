#![allow(dead_code)]
use serde_json::{json, Value};
use std::{fs, path::Path};
/// Resolve only the trusted OS temp root before creating fixture content.
/// Product validation still rejects symlinks in every supplied project ancestor.
pub fn tempdir() -> tempfile::TempDir {
    let root = std::env::temp_dir();
    #[cfg(unix)]
    let root = fs::canonicalize(root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

pub fn copy(source: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        assert!(!entry.file_type().unwrap().is_symlink());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &dest.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
        }
    }
}
pub fn write_json(path: impl AsRef<Path>, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
pub fn change(path: impl AsRef<Path>, f: impl FnOnce(&mut Value)) {
    let path = path.as_ref();
    let mut value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    f(&mut value);
    write_json(path, &value);
}
pub fn fixture(root: &Path) {
    // Copy the genuine installed, hash-verified MIT art package; no fabricated lock.
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/saved_arena_project");
    copy(&source, root);
    fs::remove_file(root.join("arena.scene.yaml")).unwrap();
    fs::remove_file(root.join("arena.sprites.json")).unwrap();
    fs::write(
        root.join("level.scene.yaml"),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scenes/collect_dodge_v1.scene.yaml"
        )),
    )
    .unwrap();
    write_json(
        root.join("orr.project.json"),
        &json!({"schema":2,"engine":"*","entry":{"game":"collect-dodge-v1","scene":"level.scene.yaml","sprites":"view.json"}}),
    );
    write_json(
        root.join("view.json"),
        &json!({"version":2,"scene":"level.scene.yaml","project":".","bindings":{
            "e_00000001":{"package":"sample-sprites","document":"sprites.json","source":{"Clip":"idle"},"units_per_pixel":0.5},
            "e_00000002":{"package":"sample-sprites","document":"sprites.json","source":{"Region":20},"units_per_pixel":0.5}
        }}),
    );
}
