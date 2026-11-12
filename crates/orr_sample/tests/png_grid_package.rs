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
    rgba_png(4, 2, &PIXELS)
}
fn rgba_png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png17::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(pixels)
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
fn padded_sheet_preserves_bytes_and_literal_row_major_regions() {
    let f = Fixture::new();
    f.edit(|r| {
        r["margin_x"] = 1.into();
        r["margin_y"] = 2.into();
        r["spacing_x"] = 3.into();
        r["spacing_y"] = 1.into();
    });
    // 9 x 7, with distinct texels in cells, margins, and gaps.
    let pixels: Vec<u8> = (0..63).flat_map(|i| [i, 255 - i, i * 3, 255]).collect();
    let png = rgba_png(9, 7, &pixels);
    fs::write(f.source().join("sheet.png"), &png).unwrap();
    let before = snapshot(f.source());
    let report = import_package(&f.request).unwrap();
    assert_eq!(
        (report.width, report.height, report.cells, report.rgba_bytes),
        (9, 7, 4, 252)
    );
    assert_eq!(snapshot(f.source()), before);
    assert_eq!(fs::read(f.request.out.join("atlas.png")).unwrap(), png);
    assert_eq!(
        fs::read(f.request.out.join("LICENSE.txt")).unwrap(),
        LICENSE
    );
    assert_eq!(
        orr_sample::project_sprites::decode_atlas(&png, 9, 7).unwrap(),
        pixels
    );
    let d = f.document();
    let cells: Vec<_> = d
        .regions()
        .iter()
        .map(|r| (r.id, r.x, r.y, r.width, r.height))
        .collect();
    assert_eq!(
        cells,
        [
            (0, 1, 2, 2, 1),
            (1, 6, 2, 2, 1),
            (2, 1, 4, 2, 1),
            (3, 6, 4, 2, 1)
        ]
    );
    assert_eq!(d.clip("idle").unwrap().sample(30).region, 0);
    assert_eq!(d.clip("walk").unwrap().sample(40).region, 2);
    assert!(d.clip("walk").unwrap().sample(120).finished);
    let output_before = snapshot(&f.request.out);
    assert!(import_package(&f.request).is_err());
    assert_eq!(snapshot(&f.request.out), output_before);
    assert_eq!(snapshot(f.source()), before);
}
#[test]
fn omitted_and_explicit_zero_padding_have_identical_entire_packages() {
    let omitted = Fixture::new();
    let explicit = Fixture::new();
    explicit.edit(|r| {
        for key in ["margin_x", "margin_y", "spacing_x", "spacing_y"] {
            r[key] = 0.into();
        }
    });
    import_package(&omitted.request).unwrap();
    import_package(&explicit.request).unwrap();
    assert_eq!(
        snapshot(&omitted.request.out),
        snapshot(&explicit.request.out)
    );
}
#[test]
fn singleton_axes_ignore_unused_maximum_spacing() {
    for vertical in [false, true] {
        let f = Fixture::new();
        f.edit(|r| {
            r["columns"] = if vertical { 1 } else { 4 }.into();
            r["rows"] = if vertical { 4 } else { 1 }.into();
            r["margin_x"] = 1.into();
            r["margin_y"] = 2.into();
            r["spacing_x"] = if vertical { u32::MAX } else { 3 }.into();
            r["spacing_y"] = if vertical { 2 } else { u32::MAX }.into();
        });
        let (width, height) = if vertical { (4, 14) } else { (19, 5) };
        fs::write(
            f.source().join("sheet.png"),
            rgba_png(width, height, &vec![127; (width * height * 4) as usize]),
        )
        .unwrap();
        import_package(&f.request).unwrap();
        let d = f.document();
        let coords: Vec<_> = d.regions().iter().map(|r| (r.x, r.y)).collect();
        let expected = if vertical {
            vec![(1, 2), (1, 5), (1, 8), (1, 11)]
        } else {
            vec![(1, 2), (6, 2), (11, 2), (16, 2)]
        };
        assert_eq!(coords, expected);
    }
}
#[test]
fn padded_dimension_limit_includes_margins_and_gaps() {
    for axis in ["x", "y"] {
        let f = Fixture::new();
        f.edit(|r| {
            r["cell_width"] = 1.into();
            r["cell_height"] = 1.into();
            r[format!("margin_{axis}")] = 1.into();
            r[format!("spacing_{axis}")] = 2044.into();
        });
        let (width, height) = if axis == "x" { (2048, 2) } else { (2, 2048) };
        fs::write(
            f.source().join("sheet.png"),
            rgba_png(width, height, &vec![255; (width * height * 4) as usize]),
        )
        .unwrap();
        import_package(&f.request).unwrap();
        assert_eq!(f.document().regions().len(), 4);
        let rejected = Fixture::new();
        rejected.edit(|r| {
            r["cell_width"] = 1.into();
            r["cell_height"] = 1.into();
            r[format!("margin_{axis}")] = 1.into();
            r[format!("spacing_{axis}")] = 2045.into();
        });
        rejected.reject();
    }
}
#[test]
fn padding_fields_reject_invalid_types_duplicates_and_unknown_names() {
    for key in ["margin_x", "margin_y", "spacing_x", "spacing_y"] {
        for value in [
            json!(-1),
            json!(1.5),
            json!(4294967296u64),
            Value::Null,
            json!("1"),
            json!(true),
        ] {
            let f = Fixture::new();
            f.edit(|r| r[key] = value);
            f.reject();
        }
        let f = Fixture::new();
        let original = serde_json::to_string(&recipe()).unwrap();
        let duplicate = format!("{{\"{key}\":0,\"{key}\":1,{}", &original[1..]);
        fs::write(&f.request.recipe, duplicate).unwrap();
        f.reject();
    }
    let f = Fixture::new();
    f.edit(|r| r["margin"] = 1.into());
    f.reject();
}
#[test]
fn padded_overflow_and_nonexact_dimensions_preserve_sources() {
    for key in ["margin_x", "margin_y", "spacing_x", "spacing_y"] {
        for value in [1, u32::MAX] {
            let f = Fixture::new();
            f.edit(|r| r[key] = value.into());
            f.reject();
        }
    }
    // Gap multiplication overflows, separately from adding a single huge gap.
    for axis in ["x", "y"] {
        let f = Fixture::new();
        f.edit(|r| {
            r[if axis == "x" { "columns" } else { "rows" }] = 3.into();
            r[format!("spacing_{axis}")] = (u32::MAX / 2 + 1).into();
        });
        f.reject();
    }
    // Exact symmetric margins are required: neither extra tail nor truncation.
    for (width, height) in [(8, 7), (10, 7), (9, 6), (9, 8)] {
        let f = Fixture::new();
        f.edit(|r| {
            r["margin_x"] = 1.into();
            r["margin_y"] = 2.into();
            r["spacing_x"] = 3.into();
            r["spacing_y"] = 1.into();
        });
        fs::write(
            f.source().join("sheet.png"),
            rgba_png(width, height, &vec![255; (width * height * 4) as usize]),
        )
        .unwrap();
        f.reject();
    }
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
