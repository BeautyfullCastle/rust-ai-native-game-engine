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
