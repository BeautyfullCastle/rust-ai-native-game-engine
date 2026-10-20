//! Focused CPU acceptance. This file is also used by the example test harness.
#![cfg(all(feature = "project-create", target_os = "linux"))]
#[path = "../examples/png_grid_package/authoring.rs"]
mod grid;
use grid::{import_package, Request};
use orr_sprite::SpriteDocument;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};

const LICENSE: &[u8] = b"Original grid fixture. CC0-1.0\r\n";
const PIXELS: [u8; 32] = [
    255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 17, 29, 41, 0, 43, 59, 71, 255, 83, 97, 101,
    64, 113, 127, 139, 255, 149, 157, 163, 127,
];
struct Fixture {
    _temp: tempfile::TempDir,
    request: Request,
}
fn recipe() -> Value {
    json!({"schema":1,"name":"grid-art","version":"1.2.3","license":"LICENSE.txt","png":"sheet.png", "cell_width":2,"cell_height":1,"columns":2,"rows":2,
        "clips":[{"id":"idle","mode":"loop","frames":[{"cell":3,"duration_ms":30},{"cell":0,"duration_ms":70}]},
        {"id":"walk","mode":"once","frames":[{"cell":1,"duration_ms":40},{"cell":2,"duration_ms":60},{"cell":1,"duration_ms":20}]}]})
}
fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png17::Encoder::new(&mut bytes, 4, 2);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&PIXELS)
            .unwrap();
    }
    bytes
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("source")).unwrap();
        let request = Request {
            recipe: root.join("source/recipe.json"),
            out: root.join("package"),
        };
        fs::write(root.join("source/sheet.png"), png()).unwrap();
        fs::write(root.join("source/LICENSE.txt"), LICENSE).unwrap();
        fs::write(&request.recipe, serde_json::to_vec(&recipe()).unwrap()).unwrap();
        Self {
            _temp: temp,
            request,
        }
    }
    fn source(&self) -> &Path {
        self.request.recipe.parent().unwrap()
    }
    fn edit(&self, edit: impl FnOnce(&mut Value)) {
        let mut r = recipe();
        edit(&mut r);
        fs::write(&self.request.recipe, serde_json::to_vec(&r).unwrap()).unwrap();
    }
    fn reject(&self) {
        let before = snapshot(self.source());
        assert!(import_package(&self.request).is_err());
        assert!(!self.request.out.exists());
        assert_eq!(snapshot(self.source()), before);
    }
    fn document(&self) -> SpriteDocument {
        SpriteDocument::from_json(
            &fs::read_to_string(self.request.out.join("sprites.json")).unwrap(),
        )
        .unwrap()
    }
}
// Capture directory entries and symlink targets without following links. File
// byte comparisons below remain separate for exact content expectations.
fn snapshot(root: &Path) -> BTreeMap<String, (String, Vec<u8>)> {
    fn visit(root: &Path, at: &Path, result: &mut BTreeMap<String, (String, Vec<u8>)>) {
        for e in fs::read_dir(at).unwrap() {
            let e = e.unwrap();
            let path = e.path();
            let kind = e.file_type().unwrap();
            let key = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if kind.is_symlink() {
                result.insert(
                    key,
                    (
                        "symlink".into(),
                        fs::read_link(&path)
                            .unwrap()
                            .to_string_lossy()
                            .as_bytes()
                            .to_vec(),
                    ),
                );
            } else if kind.is_dir() {
                result.insert(key, ("directory".into(), Vec::new()));
                visit(root, &path, result);
            } else if kind.is_file() {
                result.insert(key, ("file".into(), fs::read(&path).unwrap()));
            } else {
                result.insert(key, ("special".into(), Vec::new()));
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    snapshot(root)
        .into_iter()
        .filter_map(|(path, (kind, bytes))| (kind == "file").then_some((path, bytes)))
        .collect()
}

#[test]
fn exact_original_png_cells_clips_and_deterministic_package() {
    let a = Fixture::new();
    let b = Fixture::new();
    let before = tree(a.source());
    let report = import_package(&a.request).unwrap();
    import_package(&b.request).unwrap();
    assert_eq!(
        (report.width, report.height, report.cells, report.rgba_bytes),
        (4, 2, 4, 32)
    );
    assert_eq!(tree(a.source()), before);
    assert_eq!(tree(&a.request.out), tree(&b.request.out));
    assert_eq!(
        fs::read(a.request.out.join("atlas.png")).unwrap(),
        before["sheet.png"]
    );
    assert_eq!(
        fs::read(a.request.out.join("LICENSE.txt")).unwrap(),
        LICENSE
    );
    assert_eq!(
        orr_sample::project_sprites::decode_atlas(&before["sheet.png"], 4, 2).unwrap(),
        PIXELS
    );
    let d = a.document();
    let cells: Vec<_> = d
        .regions()
        .iter()
        .map(|r| (r.id, r.x, r.y, r.width, r.height))
        .collect();
    assert_eq!(
        cells,
        vec![
            (0, 0, 0, 2, 1),
            (1, 2, 0, 2, 1),
            (2, 0, 1, 2, 1),
            (3, 2, 1, 2, 1)
        ]
    );
    let m: orr_package::Manifest =
        serde_json::from_slice(&fs::read(a.request.out.join("orr.package.json")).unwrap()).unwrap();
    assert_eq!(m.name, "grid-art");
    assert_eq!(m.capabilities, ["sprite".to_string()].into());
    assert!(m.dependencies.is_empty());
    assert_eq!(m.files.len(), 3);
}
#[test]
fn runtime_loop_once_and_repeated_cell_boundaries() {
    let f = Fixture::new();
    import_package(&f.request).unwrap();
    let d = f.document();
    let idle = d.clip("idle").unwrap();
    let walk = d.clip("walk").unwrap();
    for (t, id) in [(0, 3), (29, 3), (30, 0), (99, 0), (100, 3), (130, 0)] {
        assert_eq!(idle.sample(t).region, id);
        assert!(!idle.sample(t).finished);
    }
    for (t, id, end) in [
        (0, 1, false),
        (39, 1, false),
        (40, 2, false),
        (99, 2, false),
        (100, 1, false),
        (119, 1, false),
        (120, 1, true),
        (u64::MAX, 1, true),
    ] {
        assert_eq!((walk.sample(t).region, walk.sample(t).finished), (id, end));
    }
}
#[test]
fn invalid_grid_dimensions_and_nonexact_sheet_are_rejected() {
    for (key, value) in [
        ("cell_width", 0),
        ("cell_height", 0),
        ("columns", 0),
        ("rows", 0),
        ("columns", 3),
        ("rows", 3),
        ("cell_width", 3),
        ("cell_height", 2),
        ("columns", u32::MAX),
        ("rows", u32::MAX),
        ("cell_width", u32::MAX),
        ("cell_height", u32::MAX),
        ("columns", 4097),
    ] {
        let f = Fixture::new();
        f.edit(|r| r[key] = value.into());
        f.reject();
    }
}
#[test]
fn malformed_clip_contracts_fail_before_publication() {
    for case in 0..12 {
        let f = Fixture::new();
        f.edit(|r| match case {
            0 => r["clips"] = json!([]),
            1 => r["clips"][0]["id"] = "".into(),
            2 => r["clips"][1]["id"] = "idle".into(),
            3 => r["clips"][0]["mode"] = "pingpong".into(),
            4 => r["clips"][0]["frames"] = json!([]),
            5 => r["clips"][0]["frames"][0]["cell"] = 4.into(),
            6 => r["clips"][0]["frames"][0]["duration_ms"] = 0.into(),
            7 => r["clips"][0]["frames"][0]["duration_ms"] = u64::MAX.into(),
            8 => r["clips"][0]["frames"][0]["extra"] = true.into(),
            9 => r["clips"][0]["extra"] = true.into(),
            10 => r["clips"][0]["id"] = "a".repeat(129).into(),
            _ => r["clips"][0]["frames"][0]["cell"] = (-1).into(),
        });
        f.reject();
    }
}
#[test]
fn strict_records_duplicate_fields_and_unknown_schema() {
    for bytes in [
        "[]",
        "{}",
        "null",
        "{\"schema\":1,\"schema\":1}",
        "{\"schema\":1,\"clips\":null}",
    ] {
        let f = Fixture::new();
        fs::write(&f.request.recipe, bytes).unwrap();
        f.reject();
    }
    for case in 0..7 {
        let f = Fixture::new();
        f.edit(|r| match case {
            0 => r["schema"] = 2.into(),
            1 => r["extra"] = true.into(),
            2 => r["clips"][0] = json!(["idle", "loop", []]),
            3 => r["clips"][0]["frames"][0] = json!([0, 30]),
            4 => r["name"] = "../bad".into(),
            5 => r["version"] = "01.2.3".into(),
            _ => r["png"] = Value::Null,
        });
        f.reject();
    }
    let f = Fixture::new();
    let body = serde_json::to_string(&recipe())
        .unwrap()
        .replace("\"cell\":3", "\"cell\":3,\"cell\":0");
    fs::write(&f.request.recipe, body).unwrap();
    f.reject();
}
#[test]
fn bounded_recipe_clips_and_references() {
    for case in 0..4 {
        let f = Fixture::new();
        f.edit(|r| match case {
            0=>r["clips"]=json!(vec![r["clips"][0].clone();orr_sprite::MAX_CLIPS+1]),
            1=>r["clips"][0]["frames"]=json!(vec![r["clips"][0]["frames"][0].clone();orr_sprite::MAX_FRAMES_PER_CLIP+1]),
            2=> { let mut clips=Vec::new(); for n in 0..5 { clips.push(json!({"id":format!("clip{n}"),"mode":"loop","frames":vec![json!({"cell":0,"duration_ms":1});4096]})); } r["clips"]=json!(clips); },
            _=>r["name"]="a".repeat(grid::MAX_RECIPE_BYTES).into(),
        });
        f.reject();
    }
}
#[test]
fn input_and_output_failure_preserve_existing_bytes() {
    for case in 0..8 {
        let f = Fixture::new();
        match case {
            0 => fs::write(f.source().join("sheet.png"), b"not PNG").unwrap(),
            1 => fs::write(f.source().join("LICENSE.txt"), b" \n").unwrap(),
            2 => fs::write(f.source().join("LICENSE.txt"), [255]).unwrap(),
            3 => f.edit(|r| r["png"] = "../source/sheet.png".into()),
            4 => f.edit(|r| r["license"] = "/tmp/LICENSE.txt".into()),
            5 => f.edit(|r| r["png"] = "a\\sheet.png".into()),
            6 => f.edit(|r| r["png"] = "missing.png".into()),
            _ => {
                fs::remove_file(f.source().join("sheet.png")).unwrap();
                fs::create_dir(f.source().join("sheet.png")).unwrap();
            }
        }
        f.reject();
    }
    let f = Fixture::new();
    fs::create_dir(&f.request.out).unwrap();
    fs::write(f.request.out.join("keep"), b"unchanged").unwrap();
    assert!(import_package(&f.request).is_err());
    assert_eq!(
        tree(&f.request.out),
        BTreeMap::from([("keep".into(), b"unchanged".to_vec())])
    );
}
#[test]
fn symlink_inputs_and_output_are_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    fs::rename(f.source().join("sheet.png"), f.source().join("real.png")).unwrap();
    symlink("real.png", f.source().join("sheet.png")).unwrap();
    f.reject();
    let f = Fixture::new();
    symlink(f.source(), &f.request.out).unwrap();
    let before = tree(f.source());
    assert!(import_package(&f.request).is_err());
    assert_eq!(tree(f.source()), before);
}
#[test]
fn installed_snapshot_reopens_without_original_source() {
    let f = Fixture::new();
    import_package(&f.request).unwrap();
    let root = f.source().parent().unwrap().join("project");
    fs::create_dir(&root).unwrap();
    let runtime = orr_sample::project_runtime::compiled_runtime();
    let project = orr_package::Project::open(&root, runtime.clone()).unwrap();
    project
        .install(std::slice::from_ref(&f.request.out))
        .unwrap();
    project.verify().unwrap();
    let before = project.list().unwrap();
    fs::rename(
        &f.request.out,
        f.request.out.with_file_name("hidden-package"),
    )
    .unwrap();
    fs::rename(f.source(), f.source().with_file_name("hidden-source")).unwrap();
    let reopened = orr_package::Project::open(&root, runtime).unwrap();
    assert_eq!(reopened.verify().unwrap(), before);
    let json = reopened.read_asset("grid-art", "sprites.json").unwrap();
    let d = SpriteDocument::from_json(std::str::from_utf8(&json).unwrap()).unwrap();
    let png = reopened.read_asset("grid-art", "atlas.png").unwrap();
    assert_eq!(png, png_original());
    assert_eq!(d.clip("walk").unwrap().sample(40).region, 2);
}
fn png_original() -> Vec<u8> {
    png()
}

#[test]
fn png_and_license_byte_budgets_and_corruption_are_rejected() {
    for case in 0..5 {
        let f = Fixture::new();
        match case {
            0 => {
                let file = fs::OpenOptions::new()
                    .write(true)
                    .open(f.source().join("sheet.png"))
                    .unwrap();
                file.set_len(16 * 1024 * 1024 + 1).unwrap();
            }
            1 => {
                let file = fs::OpenOptions::new()
                    .write(true)
                    .open(f.source().join("LICENSE.txt"))
                    .unwrap();
                file.set_len(64 * 1024 + 1).unwrap();
            }
            2 => {
                let bytes = png();
                fs::write(f.source().join("sheet.png"), &bytes[..bytes.len() / 2]).unwrap();
            }
            3 => {
                let mut bytes = Vec::new();
                {
                    let mut encoder = png17::Encoder::new(&mut bytes, 4, 2);
                    encoder.set_color(png17::ColorType::Rgba);
                    encoder.set_depth(png17::BitDepth::Eight);
                    encoder.set_animated(1, 0).unwrap();
                    encoder
                        .write_header()
                        .unwrap()
                        .write_image_data(&PIXELS)
                        .unwrap();
                }
                fs::write(f.source().join("sheet.png"), bytes).unwrap();
            }
            _ => {
                fs::remove_file(f.source().join("sheet.png")).unwrap();
                rustix::fs::mknodat(
                    rustix::fs::CWD,
                    f.source().join("sheet.png"),
                    rustix::fs::FileType::Fifo,
                    rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
                    0,
                )
                .unwrap();
            }
        }
        f.reject();
    }
}
#[test]
fn symlink_output_ancestor_is_rejected_without_changing_target() {
    use std::os::unix::fs::symlink;
    let mut f = Fixture::new();
    let link = f.source().parent().unwrap().join("linked-parent");
    symlink(f.source(), &link).unwrap();
    f.request.out = link.join("new-package");
    let before = snapshot(f.source());
    assert!(import_package(&f.request).is_err());
    assert_eq!(snapshot(f.source()), before);
    assert!(!f.source().join("new-package").exists());
}
