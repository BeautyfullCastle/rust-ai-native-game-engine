//! Original tiny PNGs exercise the package producer and the actual Arena consumer.
use super::*;
use orr_sample::{project::PreparedProject, project_runtime, project_sprites::decode_atlas};
use orr_sprite::SpriteDocument;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};

const PIXELS: [u8; 16] = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 7, 11, 13, 0];
const LICENSE: &[u8] =
    b"Original test fixture pixels, authored for this repository.\r\nCC0-1.0\r\n";

struct Fixture {
    temp: tempfile::TempDir,
    request: Request,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let request = Request {
            png: root.join("input.png"),
            json: root.join("input.json"),
            license: root.join("license.txt"),
            name: "original-test-sprites".into(),
            version: "1.2.3".into(),
            out: root.join("package"),
        };
        fs::write(&request.png, png(false)).unwrap();
        fs::write(&request.json, serde_json::to_vec_pretty(&source()).unwrap()).unwrap();
        fs::write(&request.license, LICENSE).unwrap();
        Self { temp, request }
    }
    fn edit(&self, edit: impl FnOnce(&mut Value)) {
        let mut value = source();
        edit(&mut value);
        fs::write(&self.request.json, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn reject_unchanged(&self) -> String {
        let before = [&self.request.png, &self.request.json, &self.request.license]
            .map(|path| fs::read(path).unwrap());
        let error = import_package(&self.request).unwrap_err();
        assert!(
            !self.request.out.exists(),
            "rejected import published output: {error}"
        );
        let after = [&self.request.png, &self.request.json, &self.request.license]
            .map(|path| fs::read(path).unwrap());
        assert_eq!(before, after, "failure changed sources");
        error
    }
}
fn source() -> Value {
    json!({
        "format":"orr_sprite", "version":1,
        "atlas":{"image":"atlas.png", "width":2, "height":2},
        "regions":[
            {"id":7,"x":0,"y":0,"width":1,"height":2},
            {"id":9,"x":1,"y":0,"width":1,"height":2}
        ],
        "clips":[
            {"id":"walk","mode":"loop","frames":[{"region":7,"duration_ms":30},{"region":9,"duration_ms":70}]},
            {"id":"once","mode":"once","frames":[{"region":9,"duration_ms":20},{"region":7,"duration_ms":40}]}
        ]
    })
}
fn png(animated: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png17::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        if animated {
            encoder.set_animated(1, 0).unwrap();
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&PIXELS).unwrap();
        writer.finish().unwrap();
    }
    bytes
}
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            assert!(!entry.file_type().unwrap().is_symlink());
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), files);
            } else {
                files.insert(
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
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}

#[test]
fn package_preserves_original_bytes_and_is_deterministic() {
    let a = Fixture::new();
    let b = Fixture::new();
    #[cfg(windows)]
    let b = {
        let mut b = b;
        // Exercise both ordinary drive paths and the verbatim paths in `a`.
        b.request.png = b.temp.path().join("input.png");
        b.request.json = b.temp.path().join("input.json");
        b.request.license = b.temp.path().join("license.txt");
        b.request.out = b.temp.path().join("package");
        for directory in [a.request.png.parent().unwrap(), b.temp.path()] {
            let root: std::path::PathBuf = directory.components().take(2).collect();
            assert_eq!(checked_path(&root, false).unwrap(), root);
            assert_eq!(checked_path(directory, false).unwrap(), directory);
            assert!(checked_path(&directory.join("input.png/child"), true).is_err());
            // Do not let PathBuf::push normalize traversal in a verbatim path.
            let mut traversal = directory.as_os_str().to_owned();
            traversal.push("\\..\\escape");
            assert!(checked_path(Path::new(&traversal), true).is_err());
        }
        b
    };
    let sources_before =
        [&a.request.png, &a.request.json, &a.request.license].map(|path| fs::read(path).unwrap());
    let report = import_package(&a.request).unwrap();
    assert_eq!(
        sources_before,
        [&a.request.png, &a.request.json, &a.request.license].map(|path| fs::read(path).unwrap())
    );
    assert_eq!((report.width, report.height, report.rgba_bytes), (2, 2, 16));
    assert_eq!(report.image, "atlas.png");
    assert_eq!(report, import_package(&b.request).unwrap());
    let files = tree(&a.request.out);
    assert_eq!(files, tree(&b.request.out));
    assert_eq!(files["sprites.json"], fs::read(&a.request.json).unwrap());
    assert_eq!(files["atlas.png"], fs::read(&a.request.png).unwrap());
    let manifest: orr_package::Manifest =
        serde_json::from_slice(&files["orr.package.json"]).unwrap();
    assert_eq!(manifest.name, a.request.name);
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
    assert_eq!(manifest.files.len(), 3);
    let license = manifest
        .files
        .iter()
        .find(|path| path.as_str() != "sprites.json" && path.as_str() != "atlas.png")
        .unwrap();
    assert_eq!(files[license], LICENSE);
    assert_eq!(files.len(), 4, "no extra generated content");
    let document =
        SpriteDocument::from_json(std::str::from_utf8(&files["sprites.json"]).unwrap()).unwrap();
    assert_eq!(document.regions().len(), 2);
    assert_eq!(decode_atlas(&files["atlas.png"], 2, 2).unwrap(), PIXELS);
    assert_eq!(fs::read(&a.request.license).unwrap(), LICENSE);
}

#[test]
fn installed_package_is_admitted_by_real_arena_without_sources() {
    let fixture = Fixture::new();
    import_package(&fixture.request).unwrap();
    let project = fixture
        .temp
        .path()
        .canonicalize()
        .unwrap()
        .join("arena-project");
    fs::create_dir(&project).unwrap();
    let scene = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/saved_arena_project/arena.scene.yaml"),
    )
    .unwrap();
    fs::write(project.join("arena.scene.yaml"), &scene).unwrap();
    fs::write(project.join("orr.project.json"), serde_json::to_vec(&json!({"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"arena.scene.yaml","sprites":"arena.sprites.json"}})).unwrap()).unwrap();
    fs::write(project.join("arena.sprites.json"), serde_json::to_vec(&json!({"version":1,"scene":"arena.scene.yaml","project":".","bindings":{"e_00000001":{"package":fixture.request.name,"document":"sprites.json","source":{"Clip":"walk"},"units_per_pixel":1.0}}})).unwrap()).unwrap();
    let installer = orr_package::Project::open_for_install(
        &project,
        project_runtime::compiled_runtime().engine_version,
    )
    .unwrap();
    let lock = installer
        .install(std::slice::from_ref(&fixture.request.out))
        .unwrap();
    assert_eq!(lock.packages.len(), 1);
    let admitted = PreparedProject::open(&project, project_runtime::compiled_runtime()).unwrap();
    let checksum = project_runtime::PreparedRuntime::open(&project)
        .unwrap()
        .initial_frame()
        .checksum();
    let snapshot = tree(&project);
    for path in [
        &fixture.request.png,
        &fixture.request.json,
        &fixture.request.license,
        &fixture.request.out,
    ] {
        let hidden = path.with_file_name(format!(
            "hidden-{}",
            path.file_name().unwrap().to_string_lossy()
        ));
        fs::rename(path, hidden).unwrap();
        assert!(!path.exists());
    }
    let reopened = PreparedProject::open(&project, project_runtime::compiled_runtime()).unwrap();
    for prepared in [&admitted, &reopened] {
        assert_eq!(prepared.scene().text().as_bytes(), scene);
        let sprites = prepared.sprites().unwrap();
        assert_eq!(sprites.assets.len(), 1);
        let asset = &sprites.assets[&(fixture.request.name.clone(), "sprites.json".into())];
        assert_eq!(asset.rgba, PIXELS);
        assert_eq!(asset.document.atlas().image, "atlas.png");
        assert_eq!(asset.document.regions().len(), 2);
        assert_eq!(asset.document.region(9).unwrap().x, 1);
        assert_eq!(asset.document.clips().len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(&asset.document.to_json().unwrap()).unwrap(),
            source(),
            "exact atlas, region and clip metadata survived installation"
        );
        assert_eq!(asset.document.clip("walk").unwrap().duration_ms(), 100);
        let binding = &sprites.document.bindings["e_00000001"];
        for (time, region) in [
            (0, 7),
            (29, 7),
            (30, 9),
            (99, 9),
            (100, 7),
            (130, 9),
            (u64::MAX, 7),
        ] {
            assert_eq!(
                binding.region(&asset.document, time).unwrap(),
                region,
                "time={time}"
            );
        }
        let once = asset.document.clip("once").unwrap();
        assert_eq!(once.duration_ms(), 60);
        for (time, region) in [(0, 9), (19, 9), (20, 7), (59, 7), (60, 7), (u64::MAX, 7)] {
            assert_eq!(once.sample(time).region, region);
            assert_eq!(once.sample(time).finished, time >= 60);
        }
    }
    assert_eq!(
        project_runtime::PreparedRuntime::open(&project)
            .unwrap()
            .initial_frame()
            .checksum(),
        checksum
    );
    assert_eq!(
        tree(&project),
        snapshot,
        "read-only admission changed installed project"
    );
}

#[test]
fn malformed_json_and_unknown_fields_are_rejected() {
    for bytes in [b"{".as_slice(), b"\xff", b"{}"] {
        let fixture = Fixture::new();
        fs::write(&fixture.request.json, bytes).unwrap();
        fixture.reject_unchanged();
    }
    let fixture = Fixture::new();
    fixture.edit(|v| v["unexpected"] = true.into());
    fixture.reject_unchanged();
}
#[test]
fn malformed_truncated_and_animated_png_are_rejected() {
    for bytes in [b"not png".to_vec(), png(false)[..30].to_vec(), png(true)] {
        let fixture = Fixture::new();
        fs::write(&fixture.request.png, bytes).unwrap();
        fixture.reject_unchanged();
    }
}
#[test]
fn atlas_dimension_mismatch_and_dimension_bounds_are_rejected() {
    for width in [0, 1, 3, 2049, u32::MAX] {
        let fixture = Fixture::new();
        fixture.edit(|v| v["atlas"]["width"] = width.into());
        fixture.reject_unchanged();
    }
}
#[test]
fn invalid_regions_and_clips_are_rejected() {
    for kind in 0..8 {
        let fixture = Fixture::new();
        fixture.edit(|v| match kind {
            0 => v["regions"][0]["width"] = 0.into(),
            1 => v["regions"][0]["x"] = 2.into(),
            2 => v["regions"][1]["id"] = 7.into(),
            3 => v["clips"][0]["frames"][0]["region"] = 999.into(),
            4 => v["clips"][0]["frames"][0]["duration_ms"] = 0.into(),
            5 => v["clips"][0]["frames"] = json!([]),
            6 => v["clips"][1]["id"] = "walk".into(),
            _ => v["clips"][0]["frames"][0]["duration_ms"] = u64::MAX.into(),
        });
        fixture.reject_unchanged();
    }
}
#[test]
fn encoded_input_byte_caps_are_enforced() {
    for (kind, cap) in [
        (0, MAX_PNG_BYTES),
        (1, MAX_JSON_BYTES),
        (2, MAX_LICENSE_BYTES),
    ] {
        let fixture = Fixture::new();
        let path = match kind {
            0 => &fixture.request.png,
            1 => &fixture.request.json,
            _ => &fixture.request.license,
        };
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(cap as u64 + 1)
            .unwrap();
        fixture.reject_unchanged();
    }
}
#[test]
fn unsafe_and_reserved_atlas_paths_are_rejected() {
    for path in [
        "../atlas.png",
        "/atlas.png",
        "C:/atlas.png",
        "images\\atlas.png",
        "images/../atlas.png",
        "./atlas.png",
        "images//atlas.png",
        "images/atlas.png",
        "sprites.json",
        "orr.package.json",
        "LICENSE.txt",
        "CON.png",
        "LPT1.png",
        "atlas.png.",
        "atlas.png ",
        "atlas:stream.png",
    ] {
        let fixture = Fixture::new();
        fixture.edit(|v| v["atlas"]["image"] = path.into());
        fixture.reject_unchanged();
    }
}
#[test]
fn invalid_package_identity_is_rejected() {
    for (name, version) in [
        ("../escape", "1.2.3"),
        ("valid-name", "not-semver"),
        ("", "1.2.3"),
    ] {
        let mut fixture = Fixture::new();
        fixture.request.name = name.into();
        fixture.request.version = version.into();
        fixture.reject_unchanged();
    }
}
#[test]
fn existing_output_is_preserved_without_partial_publication() {
    let fixture = Fixture::new();
    import_package(&fixture.request).unwrap();
    let before = tree(&fixture.request.out);
    assert!(import_package(&fixture.request).is_err());
    assert_eq!(tree(&fixture.request.out), before);
    let mut fixture = Fixture::new();
    fixture.request.out = fixture.request.png.clone();
    let before = fs::read(&fixture.request.png).unwrap();
    assert!(import_package(&fixture.request).is_err());
    assert_eq!(fs::read(&fixture.request.png).unwrap(), before);
}
#[test]
fn missing_or_nonregular_sources_are_rejected() {
    for kind in 0..3 {
        let fixture = Fixture::new();
        let path = match kind {
            0 => &fixture.request.png,
            1 => &fixture.request.json,
            _ => &fixture.request.license,
        };
        fs::remove_file(path).unwrap();
        assert!(import_package(&fixture.request).is_err());
        fs::create_dir(path).unwrap();
        assert!(import_package(&fixture.request).is_err());
        assert!(!fixture.request.out.exists());
    }
}
#[cfg(unix)]
#[test]
fn static_symlink_inputs_and_output_ancestors_are_rejected() {
    use std::os::unix::fs::symlink;
    for kind in 0..3 {
        let fixture = Fixture::new();
        let path = match kind {
            0 => &fixture.request.png,
            1 => &fixture.request.json,
            _ => &fixture.request.license,
        };
        let original = path.with_extension("original");
        fs::rename(path, &original).unwrap();
        symlink(&original, path).unwrap();
        assert!(import_package(&fixture.request).is_err());
        assert!(!fixture.request.out.exists());
        assert!(original.is_file());
    }
    let mut fixture = Fixture::new();
    let real = fixture.temp.path().join("real");
    fs::create_dir(&real).unwrap();
    let alias = fixture.temp.path().join("alias");
    symlink(&real, &alias).unwrap();
    fixture.request.out = alias.join("package");
    fixture.reject_unchanged();
    assert_eq!(fs::read_dir(real).unwrap().count(), 0);
}
#[cfg(target_os = "linux")]
#[test]
fn fifo_sources_are_rejected_without_opening_them() {
    for kind in 0..3 {
        let fixture = Fixture::new();
        let path = match kind {
            0 => &fixture.request.png,
            1 => &fixture.request.json,
            _ => &fixture.request.license,
        };
        fs::remove_file(path).unwrap();
        rustix::fs::mknodat(
            rustix::fs::CWD,
            path,
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
fn invalid_license_text_is_rejected_without_output() {
    for bytes in [b"".as_slice(), b" \r\n", b"\xff"] {
        let fixture = Fixture::new();
        fs::write(&fixture.request.license, bytes).unwrap();
        fixture.reject_unchanged();
    }
}
#[test]
fn sprite_collection_caps_are_enforced() {
    for kind in 0..3 {
        let fixture = Fixture::new();
        fixture.edit(|value| match kind {
            0 => {
                value["regions"] = Value::Array(vec![
                    value["regions"][0].clone();
                    orr_sprite::MAX_REGIONS + 1
                ])
            }
            1 => {
                value["clips"] =
                    Value::Array(vec![value["clips"][0].clone(); orr_sprite::MAX_CLIPS + 1])
            }
            _ => {
                value["clips"][0]["frames"] = Value::Array(vec![
                    value["clips"][0]["frames"][0]
                        .clone();
                    orr_sprite::MAX_FRAMES_PER_CLIP + 1
                ])
            }
        });
        fixture.reject_unchanged();
    }
}
