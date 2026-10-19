//! Independent original pixels and hostile-input admission tests.
use super::*;
use orr_sample::project_sprites::decode_atlas;
use orr_sprite::SpriteDocument;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};

const LICENSE: &[u8] = b"Original synthetic test artwork.\r\nCC0-1.0\r\n";
const PIXELS: [[u8; 8]; 3] = [
    [255, 0, 0, 255, 0, 255, 0, 128],
    [0, 0, 255, 64, 7, 11, 13, 0],
    [31, 41, 59, 255, 26, 53, 58, 127],
];
struct Fixture {
    temp: tempfile::TempDir,
    request: Request,
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
        for (index, pixels) in PIXELS.iter().enumerate() {
            fs::write(
                root.join(format!("source/{index}.png")),
                png(2, 1, pixels, false),
            )
            .unwrap();
        }
        fs::write(root.join("source/LICENSE.txt"), LICENSE).unwrap();
        fs::write(
            &request.recipe,
            serde_json::to_vec_pretty(&recipe()).unwrap(),
        )
        .unwrap();
        Self { temp, request }
    }
    fn source(&self) -> &Path {
        self.request.recipe.parent().unwrap()
    }
    fn edit(&self, edit: impl FnOnce(&mut Value)) {
        let mut value = recipe();
        edit(&mut value);
        fs::write(&self.request.recipe, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn reject(&self) -> String {
        let before = tree(self.source());
        let error = import_package(&self.request).unwrap_err();
        assert!(
            !self.request.out.exists(),
            "published rejected output: {error}"
        );
        assert_eq!(tree(self.source()), before, "rejection changed source");
        error
    }
}
fn recipe() -> Value {
    json!({"schema":1,"name":"original-sequence","version":"1.2.3","license":"LICENSE.txt","clip":"run","mode":"loop","columns":2,
        "frames":[{"png":"2.png","duration_ms":30},{"png":"0.png","duration_ms":70},{"png":"1.png","duration_ms":20}]})
}
fn png(width: u32, height: u32, pixels: &[u8], animated: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png17::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        if animated {
            encoder.set_animated(1, 0).unwrap();
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
        writer.finish().unwrap();
    }
    bytes
}
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, result: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                walk(root, &entry.path(), result);
            } else if kind.is_file() {
                result.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}
#[test]
fn original_pixels_order_regions_license_and_relocated_bytes() {
    let a = Fixture::new();
    let b = Fixture::new();
    let before = tree(a.source());
    let report = import_package(&a.request).unwrap();
    import_package(&b.request).unwrap();
    assert_eq!(
        (
            report.width,
            report.height,
            report.frames,
            report.rgba_bytes
        ),
        (4, 2, 3, 32)
    );
    assert_eq!(tree(a.source()), before);
    let files = tree(&a.request.out);
    assert_eq!(files, tree(&b.request.out));
    assert_eq!(
        files.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "LICENSE.txt",
            "atlas.png",
            "orr.package.json",
            "sprites.json"
        ]
    );
    assert_eq!(files["LICENSE.txt"], LICENSE);
    let expected = [
        PIXELS[2].as_slice(),
        PIXELS[0].as_slice(),
        PIXELS[1].as_slice(),
        &[0; 8],
    ]
    .concat();
    assert_eq!(decode_atlas(&files["atlas.png"], 4, 2).unwrap(), expected);
    let document: Value = serde_json::from_slice(&files["sprites.json"]).unwrap();
    assert_eq!(
        document,
        json!({"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":4,"height":2},"regions":[{"id":0,"x":0,"y":0,"width":2,"height":1},{"id":1,"x":2,"y":0,"width":2,"height":1},{"id":2,"x":0,"y":1,"width":2,"height":1}],"clips":[{"id":"run","mode":"loop","frames":[{"region":0,"duration_ms":30},{"region":1,"duration_ms":70},{"region":2,"duration_ms":20}]}]})
    );
    let manifest: orr_package::Manifest =
        serde_json::from_slice(&files["orr.package.json"]).unwrap();
    assert_eq!(manifest.name, "original-sequence");
    assert_eq!(manifest.version, "1.2.3");
    assert_eq!(manifest.capabilities, ["sprite".to_string()].into());
    assert!(manifest.dependencies.is_empty());
    assert_eq!(
        manifest.files,
        [
            "sprites.json".to_string(),
            "atlas.png".to_string(),
            "LICENSE.txt".to_string()
        ]
        .into()
    );
}
#[test]
fn every_loop_and_once_boundary_is_exact() {
    for mode in ["loop", "once"] {
        let fixture = Fixture::new();
        fixture.edit(|v| v["mode"] = mode.into());
        import_package(&fixture.request).unwrap();
        let document = SpriteDocument::from_json(
            &fs::read_to_string(fixture.request.out.join("sprites.json")).unwrap(),
        )
        .unwrap();
        let clip = document.clip("run").unwrap();
        assert_eq!(clip.duration_ms(), 120);
        for (time, region) in [(0, 0), (29, 0), (30, 1), (99, 1), (100, 2), (119, 2)] {
            let frame = clip.sample(time);
            assert_eq!(frame.region, region);
            assert!(!frame.finished);
        }
        for time in [120, 149, 150, 219, 220, 239, 240, u64::MAX] {
            let sample = clip.sample(time);
            let expected = if mode == "once" {
                2
            } else {
                match time % 120 {
                    0..=29 => 0,
                    30..=99 => 1,
                    _ => 2,
                }
            };
            assert_eq!(sample.region, expected);
            assert_eq!(sample.finished, mode == "once");
        }
    }
}
#[test]
fn strict_recipe_schema_and_duration_overflow_rejected() {
    for kind in 0..13 {
        let fixture = Fixture::new();
        fixture.edit(|v| match kind {
            0 => v["schema"] = 2.into(),
            1 => v["extra"] = true.into(),
            2 => v["frames"][0]["extra"] = true.into(),
            3 => v["mode"] = "pingpong".into(),
            4 => v["frames"][0]["duration_ms"] = 0.into(),
            5 => v["frames"][0]["duration_ms"] = u64::MAX.into(),
            6 => v["columns"] = 0.into(),
            7 => v["columns"] = u64::MAX.into(),
            8 => v["frames"] = json!([]),
            9 => v["frames"] = json!(vec![v["frames"][0].clone(); MAX_FRAMES + 1]),
            10 => v["name"] = "../bad".into(),
            11 => v["version"] = "bad".into(),
            _ => v["clip"] = "".into(),
        });
        fixture.reject();
    }
    for bytes in [
        b"{".as_slice(),
        b"\xff",
        b"{}",
        b"{\"schema\":1,\"schema\":1}",
    ] {
        let fixture = Fixture::new();
        fs::write(&fixture.request.recipe, bytes).unwrap();
        fixture.reject();
    }
}
#[test]
fn unequal_malformed_truncated_and_apng_frames_rejected() {
    for bytes in [
        png(1, 2, &PIXELS[0], false),
        b"not PNG".to_vec(),
        png(2, 1, &PIXELS[0], false)[..30].to_vec(),
        png(2, 1, &PIXELS[0], true),
    ] {
        let fixture = Fixture::new();
        fs::write(fixture.source().join("0.png"), bytes).unwrap();
        fixture.reject();
    }
}
#[test]
fn encoded_and_text_caps_and_invalid_license_rejected() {
    for (name, cap) in [
        ("recipe.json", MAX_RECIPE_BYTES),
        ("0.png", MAX_PNG_BYTES),
        ("LICENSE.txt", MAX_LICENSE_BYTES),
    ] {
        let fixture = Fixture::new();
        fs::OpenOptions::new()
            .write(true)
            .open(fixture.source().join(name))
            .unwrap()
            .set_len(cap as u64 + 1)
            .unwrap();
        fixture.reject();
    }
    for bytes in [b"".as_slice(), b" \r\n", b"\xff"] {
        let fixture = Fixture::new();
        fs::write(fixture.source().join("LICENSE.txt"), bytes).unwrap();
        fixture.reject();
    }
}
#[test]
fn aggregate_encoded_budget_rejected() {
    let fixture = Fixture::new();
    for index in 0..3 {
        fs::OpenOptions::new()
            .write(true)
            .open(fixture.source().join(format!("{index}.png")))
            .unwrap()
            .set_len((MAX_TOTAL_PNG_BYTES / 3 + 1) as u64)
            .unwrap();
    }
    fixture.reject();
}
#[test]
fn atlas_dimensions_and_decoded_budget_rejected() {
    let fixture = Fixture::new();
    let pixels = vec![0; 1025 * 4];
    for index in 0..3 {
        fs::write(
            fixture.source().join(format!("{index}.png")),
            png(1025, 1, &pixels, false),
        )
        .unwrap();
    }
    fixture.reject();
    let fixture = Fixture::new();
    let pixels = vec![0; 1024 * 1024 * 4];
    for index in 0..3 {
        fs::write(
            fixture.source().join(format!("{index}.png")),
            png(1024, 1024, &pixels, false),
        )
        .unwrap();
    }
    fixture.edit(|v| {
        v["columns"] = 1.into();
        v["frames"] = json!(vec![v["frames"][0].clone(); 5]);
    });
    fixture.reject();
}
#[test]
fn unsafe_relative_paths_rejected() {
    for path in [
        "../0.png",
        "/0.png",
        "C:/0.png",
        "a/../../0.png",
        "a\\0.png",
    ] {
        for license in [false, true] {
            let fixture = Fixture::new();
            fixture.edit(|v| {
                if license {
                    v["license"] = path.into()
                } else {
                    v["frames"][0]["png"] = path.into()
                }
            });
            fixture.reject();
        }
    }
}
#[test]
fn existing_outputs_preserved_and_no_staging_leaks() {
    let fixture = Fixture::new();
    import_package(&fixture.request).unwrap();
    let before = tree(&fixture.request.out);
    assert!(import_package(&fixture.request).is_err());
    assert_eq!(tree(&fixture.request.out), before);
    let mut fixture = Fixture::new();
    fixture.request.out = fixture.request.recipe.clone();
    let before = tree(fixture.source());
    assert!(import_package(&fixture.request).is_err());
    assert_eq!(tree(fixture.source()), before);
    let fixture = Fixture::new();
    fixture.edit(|v| v["columns"] = 0.into());
    fixture.reject();
    assert_eq!(fs::read_dir(fixture.temp.path()).unwrap().count(), 1);
}
#[test]
fn missing_directory_and_fifo_sources_rejected() {
    for name in ["recipe.json", "LICENSE.txt", "0.png"] {
        let fixture = Fixture::new();
        let path = fixture.source().join(name);
        fs::remove_file(&path).unwrap();
        assert!(import_package(&fixture.request).is_err());
        fs::create_dir(&path).unwrap();
        assert!(import_package(&fixture.request).is_err());
        fs::remove_dir(&path).unwrap();
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &path,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            0,
        )
        .unwrap();
        assert!(import_package(&fixture.request).is_err());
        assert!(!fixture.request.out.exists());
    }
}
#[test]
fn symlink_sources_and_output_ancestors_rejected() {
    use std::os::unix::fs::symlink;
    for name in ["recipe.json", "LICENSE.txt", "0.png"] {
        let fixture = Fixture::new();
        let path = fixture.source().join(name);
        let original = path.with_extension("original");
        fs::rename(&path, &original).unwrap();
        symlink(original, &path).unwrap();
        assert!(import_package(&fixture.request).is_err());
        assert!(!fixture.request.out.exists());
    }
    let mut fixture = Fixture::new();
    let real = fixture.temp.path().join("real");
    fs::create_dir(&real).unwrap();
    let alias = fixture.temp.path().join("alias");
    symlink(&real, &alias).unwrap();
    fixture.request.out = alias.join("package");
    fixture.reject();
    assert_eq!(fs::read_dir(real).unwrap().count(), 0);
}

#[test]
fn cli_options_are_strict_and_help_standalone() {
    use std::ffi::OsString;
    assert!(crate::cli::parse_args([OsString::from("--help")])
        .unwrap()
        .is_none());
    for args in [
        vec![],
        vec!["--recipe"],
        vec!["--recipe", "a", "--recipe", "b"],
        vec!["--out", "a", "--out", "b"],
        vec!["--help", "--out", "b"],
        vec!["--unknown", "a", "--out", "b"],
        vec!["--recipe", "", "--out", "b"],
    ] {
        assert!(crate::cli::parse_args(args.into_iter().map(OsString::from)).is_err());
    }
    assert!(crate::cli::parse_args(
        ["--out", "new", "--recipe", "recipe.json"].map(OsString::from)
    )
    .unwrap()
    .is_some());
}

#[test]
fn aggregate_preflight_precedes_decode_and_counts_references() {
    let fixture = Fixture::new();
    let mut header = png(2, 1, &PIXELS[0], false)[..24].to_vec();
    header[16..20].copy_from_slice(&2048u32.to_be_bytes());
    header[20..24].copy_from_slice(&2048u32.to_be_bytes());
    for index in 0..3 {
        fs::write(fixture.source().join(format!("{index}.png")), &header).unwrap();
    }
    assert!(fixture.reject().contains("aggregate decoded RGBA"));
    let fixture = Fixture::new();
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.source().join("0.png"))
        .unwrap()
        .set_len((MAX_TOTAL_PNG_BYTES / 3 + 1) as u64)
        .unwrap();
    fixture.edit(|v| {
        for frame in v["frames"].as_array_mut().unwrap() {
            frame["png"] = "0.png".into();
        }
    });
    assert!(fixture.reject().contains("aggregate compressed PNG"));
}

#[test]
fn atomic_publication_rejects_raced_existing_directory() {
    let fixture = Fixture::new();
    fs::create_dir(&fixture.request.out).unwrap();
    fs::write(fixture.request.out.join("keep"), b"sentinel").unwrap();
    let empty = fixture.temp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    assert!(publish(&empty, &[("new", b"payload")]).is_err());
    assert_eq!(fs::read_dir(&empty).unwrap().count(), 0);
    fs::remove_dir(&empty).unwrap();
    let before = tree(&fixture.request.out);
    assert!(publish(&fixture.request.out, &[("new", b"payload")])
        .unwrap_err()
        .contains("no-replace"));
    assert_eq!(tree(&fixture.request.out), before);
    assert_eq!(
        fs::read_dir(fixture.temp.path()).unwrap().count(),
        2,
        "staging leak"
    );
}

fn canvas_recipe() -> Value {
    let mut value = recipe();
    value["schema"] = 2.into();
    value["canvas"] = json!({"width":3,"height":3});
    for frame in value["frames"].as_array_mut().unwrap() {
        frame["offset"] = json!({"x":0,"y":0});
    }
    value
}
fn write_recipe(fixture: &Fixture, value: &Value) {
    fs::write(&fixture.request.recipe, serde_json::to_vec(value).unwrap()).unwrap();
}
fn canvas_fixture() -> Fixture {
    let fixture = Fixture::new();
    write_recipe(&fixture, &canvas_recipe());
    fixture
}

#[test]
fn canvas_asymmetric_pixels_literal_atlas_and_relocated_repeat_bytes() {
    fn setup() -> Fixture {
        let fixture = canvas_fixture();
        // Independent hand-authored unequal sources, including alpha-zero RGB.
        fs::write(
            fixture.source().join("2.png"),
            png(
                2,
                2,
                &[255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 7, 11, 13, 0],
                false,
            ),
        )
        .unwrap();
        fs::write(
            fixture.source().join("0.png"),
            png(
                1,
                3,
                &[31, 41, 59, 255, 26, 53, 58, 127, 91, 92, 93, 19],
                false,
            ),
        )
        .unwrap();
        fs::write(
            fixture.source().join("1.png"),
            png(1, 1, &[0, 0, 0, 0], false),
        )
        .unwrap();
        let mut value = canvas_recipe();
        value["frames"][0]["offset"] = json!({"x":1,"y":1});
        value["frames"][2]["offset"] = json!({"x":2,"y":0});
        write_recipe(&fixture, &value);
        fixture
    }
    let a = setup();
    let b = setup();
    let before = tree(a.source());
    let report = import_package(&a.request).unwrap();
    assert_eq!(
        (
            report.width,
            report.height,
            report.frames,
            report.rgba_bytes
        ),
        (6, 6, 3, 144)
    );
    let files = tree(&a.request.out);
    import_package(&b.request).unwrap();
    assert_eq!(files, tree(&b.request.out));
    assert_eq!(tree(a.source()), before);
    fs::remove_dir_all(&a.request.out).unwrap();
    import_package(&a.request).unwrap();
    assert_eq!(files, tree(&a.request.out));
    assert_eq!(tree(a.source()), before);
    let expected: [[u8; 4]; 36] = [
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [31, 41, 59, 255],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [255, 0, 0, 255],
        [0, 255, 0, 128],
        [26, 53, 58, 127],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 255, 64],
        [7, 11, 13, 0],
        [91, 92, 93, 19],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
    ];
    assert_eq!(
        decode_atlas(&files["atlas.png"], 6, 6).unwrap(),
        expected.concat()
    );
    let document: Value = serde_json::from_slice(&files["sprites.json"]).unwrap();
    assert_eq!(
        document,
        json!({"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":6,"height":6},"regions":[{"id":0,"x":0,"y":0,"width":3,"height":3},{"id":1,"x":3,"y":0,"width":3,"height":3},{"id":2,"x":0,"y":3,"width":3,"height":3}],"clips":[{"id":"run","mode":"loop","frames":[{"region":0,"duration_ms":30},{"region":1,"duration_ms":70},{"region":2,"duration_ms":20}]}]})
    );
    assert_eq!(files["LICENSE.txt"], LICENSE);
}

#[test]
fn canvas_and_offsets_require_strict_complete_integer_objects() {
    for field in ["width", "height", "x", "y"] {
        for bad in [
            json!(-1),
            json!(1.0),
            json!("1"),
            json!(null),
            json!(true),
            json!(4294967296u64),
        ] {
            let fixture = canvas_fixture();
            let mut value = canvas_recipe();
            let object = if matches!(field, "width" | "height") {
                &mut value["canvas"]
            } else {
                &mut value["frames"][1]["offset"]
            };
            object[field] = bad;
            write_recipe(&fixture, &value);
            fixture.reject();
        }
        let fixture = canvas_fixture();
        let mut value = canvas_recipe();
        let object = if matches!(field, "width" | "height") {
            &mut value["canvas"]
        } else {
            &mut value["frames"][1]["offset"]
        };
        object.as_object_mut().unwrap().remove(field);
        write_recipe(&fixture, &value);
        fixture.reject();
    }
    for kind in 0..12 {
        let fixture = canvas_fixture();
        let mut value = canvas_recipe();
        match kind {
            0 => {
                value.as_object_mut().unwrap().remove("canvas");
            }
            1 => {
                value["frames"][1].as_object_mut().unwrap().remove("offset");
            }
            2 => value["canvas"] = Value::Null,
            3 => value["frames"][1]["offset"] = Value::Null,
            4 => value["canvas"] = json!([3, 3]),
            5 => value["frames"][1]["offset"] = json!([0, 0]),
            6 => value["canvas"]["extra"] = true.into(),
            7 => value["frames"][1]["offset"]["extra"] = true.into(),
            8 => value["schema"] = 3.into(),
            9 => value["canvas"]["width"] = 0.into(),
            10 => value["canvas"]["height"] = 0.into(),
            _ => value["schema"] = 1.into(),
        }
        write_recipe(&fixture, &value);
        fixture.reject();
    }
    for (needle, replacement) in [
        ("\"width\":3", "\"width\":3,\"width\":3"),
        ("\"height\":3", "\"height\":3,\"height\":3"),
        ("\"x\":0", "\"x\":0,\"x\":0"),
        ("\"y\":0", "\"y\":0,\"y\":0"),
        (
            "\"canvas\":",
            "\"canvas\":{\"width\":3,\"height\":3},\"canvas\":",
        ),
        ("\"offset\":", "\"offset\":{\"x\":0,\"y\":0},\"offset\":"),
    ] {
        let fixture = canvas_fixture();
        let text = serde_json::to_string(&canvas_recipe()).unwrap();
        assert!(text.contains(needle));
        fs::write(
            &fixture.request.recipe,
            text.replacen(needle, replacement, 1),
        )
        .unwrap();
        fixture.reject();
    }
    for field in ["canvas", "offset"] {
        let fixture = Fixture::new();
        fixture.edit(|value| {
            if field == "canvas" {
                value["canvas"] = Value::Null;
            } else {
                value["frames"][0]["offset"] = Value::Null;
            }
        });
        fixture.reject();
    }
    // Schema 1 continues rejecting each schema-2-only field independently.
    for field in ["canvas", "offset"] {
        let fixture = Fixture::new();
        fixture.edit(|value| {
            if field == "canvas" {
                value["canvas"] = json!({"width":3,"height":3});
            } else {
                value["frames"][0]["offset"] = json!({"x":0,"y":0});
            }
        });
        fixture.reject();
    }
}

#[test]
fn canvas_flush_edges_and_one_past_edges_and_addition_overflow() {
    for (x, y, accept) in [
        (1, 2, true),
        (2, 2, false),
        (1, 3, false),
        (u32::MAX, 0, false),
        (0, u32::MAX, false),
    ] {
        let fixture = canvas_fixture();
        let mut value = canvas_recipe();
        value["frames"][0]["offset"] = json!({"x":x,"y":y});
        write_recipe(&fixture, &value);
        if accept {
            import_package(&fixture.request).unwrap();
        } else {
            fixture.reject();
        }
    }
    // Alpha-zero pixels still occupy the whole source rectangle; never crop them.
    for pixels in [[0u8; 8], [7, 11, 13, 0, 17, 19, 23, 0]] {
        let fixture = canvas_fixture();
        fs::write(fixture.source().join("2.png"), png(2, 1, &pixels, false)).unwrap();
        let mut value = canvas_recipe();
        value["frames"][0]["offset"] = json!({"x":2,"y":0});
        write_recipe(&fixture, &value);
        fixture.reject();
    }
    // Exact existing atlas edge/budget succeeds; either +1 cell dimension fails.
    for (width, height, accept) in [(1024, 1024, true), (1025, 1024, false), (1024, 1025, false)] {
        let fixture = canvas_fixture();
        let mut value = canvas_recipe();
        value["canvas"] = json!({"width":width,"height":height});
        write_recipe(&fixture, &value);
        if accept {
            let report = import_package(&fixture.request).unwrap();
            assert_eq!(
                (report.width, report.height, report.rgba_bytes),
                (2048, 2048, MAX_RGBA_BYTES)
            );
        } else {
            fixture.reject();
        }
    }
    for (width, height) in [
        (1, 3),
        (3, 0),
        (0, 3),
        (2049, 1),
        (1, 2049),
        (u32::MAX, 1),
        (1, u32::MAX),
        (u32::MAX, u32::MAX),
        (1025, 1),
        (1, 1025),
    ] {
        let fixture = canvas_fixture();
        let mut value = canvas_recipe();
        value["canvas"] = json!({"width":width,"height":height});
        write_recipe(&fixture, &value);
        fixture.reject();
    }
}

#[test]
fn canvas_preflight_rejects_huge_canvas_before_png_decode() {
    let fixture = canvas_fixture();
    let mut value = canvas_recipe();
    value["canvas"] = json!({"width":u32::MAX,"height":u32::MAX});
    write_recipe(&fixture, &value);
    // Valid IHDR prefix but undecodable content proves dimension admission first.
    let header = png(2, 1, &PIXELS[0], false)[..24].to_vec();
    for index in 0..3 {
        fs::write(fixture.source().join(format!("{index}.png")), &header).unwrap();
    }
    let error = fixture.reject();
    assert!(
        error.contains("dimension") || error.contains("overflow"),
        "{error}"
    );
    assert!(!error.contains("decode"), "{error}");
}

#[test]
fn canvas_preserves_resource_limits_and_counts_repeated_sources() {
    assert_eq!(
        (
            MAX_RECIPE_BYTES,
            MAX_FRAMES,
            MAX_PNG_BYTES,
            MAX_TOTAL_PNG_BYTES,
            MAX_RGBA_BYTES,
            MAX_LICENSE_BYTES
        ),
        (
            1024 * 1024,
            256,
            16 * 1024 * 1024,
            32 * 1024 * 1024,
            16 * 1024 * 1024,
            64 * 1024
        )
    );
    let fixture = canvas_fixture();
    let mut value = canvas_recipe();
    for frame in value["frames"].as_array_mut().unwrap() {
        frame["png"] = "0.png".into();
    }
    write_recipe(&fixture, &value);
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.source().join("0.png"))
        .unwrap()
        .set_len((MAX_TOTAL_PNG_BYTES / 3 + 1) as u64)
        .unwrap();
    assert!(fixture.reject().contains("aggregate compressed PNG"));
    let fixture = canvas_fixture();
    let mut value = canvas_recipe();
    value["canvas"] = json!({"width":2048,"height":2048});
    value["columns"] = 1.into();
    for frame in value["frames"].as_array_mut().unwrap() {
        frame["png"] = "0.png".into();
    }
    write_recipe(&fixture, &value);
    let mut header = png(2, 1, &PIXELS[0], false)[..24].to_vec();
    header[16..20].copy_from_slice(&2048u32.to_be_bytes());
    header[20..24].copy_from_slice(&2048u32.to_be_bytes());
    fs::write(fixture.source().join("0.png"), header).unwrap();
    let error = fixture.reject();
    assert!(error.contains("aggregate decoded RGBA"), "{error}");
}

// Frozen before schema-2 implementation using e023ba60d242ece8f3050758d8853ab5aa6c5243.
// SHA256 literals are independent of the implementation under test.
#[test]
fn schema1_all_four_files_match_frozen_pre_canvas_bytes() {
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new();
    import_package(&fixture.request).unwrap();
    let files = tree(&fixture.request.out);
    for (name, expected, digest) in [
        (
            "LICENSE.txt",
            include_bytes!("fixtures/schema1/LICENSE.txt").as_slice(),
            "947c920c9c134b00e590c40861d1f925ea1fb3d5c444d59d273f37ba53502db8",
        ),
        (
            "atlas.png",
            include_bytes!("fixtures/schema1/atlas.png").as_slice(),
            "291746191733d5d55a4e73987b17dfe0cb764194a919ff749bfd24e5d1c17e3c",
        ),
        (
            "orr.package.json",
            include_bytes!("fixtures/schema1/orr.package.json").as_slice(),
            "ebe1b6b2f12223b1cfe60e7cf7efb251e1bc5da540955f4a8e1178cb1f3966c6",
        ),
        (
            "sprites.json",
            include_bytes!("fixtures/schema1/sprites.json").as_slice(),
            "2097787159dfa8c9fd7d9bdff72bedd5094e8dfc8e4aae0e04b727ee64e41fa8",
        ),
    ] {
        assert_eq!(files[name], expected, "frozen bytes changed: {name}");
        assert_eq!(
            Sha256::digest(&files[name])
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            digest,
            "frozen hash changed: {name}"
        );
    }
}
