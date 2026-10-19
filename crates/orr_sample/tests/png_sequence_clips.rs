//! Black-box CPU contracts for the private schema-3 named-clip authoring recipe.
//! Build the actual example and set ORR_PNG_SEQUENCE_PACKAGE_BIN. The acceptance
//! gate sets ORR_REQUIRE_PNG_SEQUENCE_CLIPS=1 so a missing CLI cannot pass.
#![cfg(all(feature = "project-create", target_os = "linux"))]

use orr_sprite::{Playback, PlaybackMode, SpriteDocument};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const LICENSE: &[u8] = b"Original synthetic test artwork.\r\nCC0-1.0\r\n";
const MAX_RECIPE: usize = 1024 * 1024;
const MAX_PNG: usize = 16 * 1024 * 1024;
const MAX_ENCODED: usize = 32 * 1024 * 1024;
const MAX_LICENSE: usize = 64 * 1024;
const P0: [[u8; 4]; 4] = [
    [255, 0, 0, 255],
    [0, 255, 0, 128],
    [0, 0, 255, 64],
    [7, 11, 13, 0],
];
const P1: [[u8; 4]; 3] = [[31, 41, 59, 255], [26, 53, 58, 127], [91, 92, 93, 19]];
const P2: [[u8; 4]; 2] = [[181, 17, 99, 255], [71, 19, 83, 0]];
const P3: [[u8; 4]; 1] = [[39, 41, 43, 255]];

fn binary() -> Option<PathBuf> {
    let Some(path) = std::env::var_os("ORR_PNG_SEQUENCE_PACKAGE_BIN") else {
        assert!(
            std::env::var_os("ORR_REQUIRE_PNG_SEQUENCE_CLIPS").is_none(),
            "required named-clip CLI missing: set ORR_PNG_SEQUENCE_PACKAGE_BIN"
        );
        eprintln!("SKIP: ORR_PNG_SEQUENCE_PACKAGE_BIN not set");
        return None;
    };
    let path = Path::new(&path)
        .canonicalize()
        .expect("CLI binary must exist");
    assert!(path.is_file(), "CLI binary must be a regular file");
    Some(path)
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

fn recipe() -> Value {
    // Deliberately unsorted filenames, IDs, overlapping and repeated references.
    json!({"schema":3,"name":"named-sequence","version":"1.2.3","license":"LICENSE.txt",
        "columns":3,"canvas":{"width":4,"height":3},
        "frames":[
            {"png":"z-first.png","offset":{"x":1,"y":1}},
            {"png":"a-second.png","offset":{"x":3,"y":0}},
            {"png":"m-third.png","offset":{"x":0,"y":2}},
            {"png":"unused.png","offset":{"x":2,"y":0}}],
        "clips":[
            {"id":"z-run","mode":"loop","frames":[
                {"frame":2,"duration_ms":11},{"frame":0,"duration_ms":17},{"frame":2,"duration_ms":23}]},
            {"id":"a-land","mode":"once","frames":[
                {"frame":0,"duration_ms":13},{"frame":1,"duration_ms":19},{"frame":0,"duration_ms":5}]}]})
}

#[derive(Debug, PartialEq, Eq)]
enum Entry {
    Directory,
    File(u64, Vec<u8>),
    Link(PathBuf),
    Other,
}

// Never follow links or open FIFOs. Hashes keep hostile-budget snapshots bounded.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Entry>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let kind = entry.file_type().unwrap();
            let value = if kind.is_symlink() {
                Entry::Link(fs::read_link(&path).unwrap())
            } else if kind.is_dir() {
                visit(root, &path, out);
                Entry::Directory
            } else if kind.is_file() {
                Entry::File(
                    entry.metadata().unwrap().len(),
                    Sha256::digest(fs::read(path).unwrap()).to_vec(),
                )
            } else {
                Entry::Other
            };
            out.insert(relative, value);
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn package_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        assert!(entry.file_type().unwrap().is_file());
        files.insert(
            entry.file_name().into_string().unwrap(),
            fs::read(entry.path()).unwrap(),
        );
    }
    assert_eq!(
        files.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "LICENSE.txt",
            "atlas.png",
            "orr.package.json",
            "sprites.json"
        ]
    );
    files
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
    cwd: PathBuf,
    recipe: PathBuf,
    out: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("input images");
        let cwd = root.join("unrelated working directory");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&cwd).unwrap();
        fs::write(
            cwd.join("existing-project.json"),
            b"preserve unrelated project",
        )
        .unwrap();
        for (name, w, h, pixels) in [
            ("z-first.png", 2, 2, P0.concat()),
            ("a-second.png", 1, 3, P1.concat()),
            ("m-third.png", 2, 1, P2.concat()),
            ("unused.png", 1, 1, P3.concat()),
        ] {
            fs::write(source.join(name), png(w, h, &pixels, false)).unwrap();
        }
        fs::write(source.join("LICENSE.txt"), LICENSE).unwrap();
        let recipe = source.join("recipe.json");
        fs::write(&recipe, serde_json::to_vec(&crate::recipe()).unwrap()).unwrap();
        let out = root.join("new package");
        Self {
            _temp: temp,
            root,
            source,
            cwd,
            recipe,
            out,
        }
    }
    fn write(&self, value: &Value) {
        fs::write(&self.recipe, serde_json::to_vec(value).unwrap()).unwrap();
    }
    fn command(&self, binary: &Path) -> Command {
        let mut command = Command::new(binary);
        command
            .current_dir(&self.cwd)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        command
    }
    fn run(&self, binary: &Path) -> Output {
        self.command(binary)
            .arg("--recipe")
            .arg(&self.recipe)
            .arg("--out")
            .arg(&self.out)
            .output()
            .unwrap()
    }
    fn success(&self, binary: &Path) -> BTreeMap<String, Vec<u8>> {
        let before = snapshot(&self.root);
        let result = self.run(binary);
        assert!(
            result.status.success(),
            "CLI failed: {}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let mut after = snapshot(&self.root);
        let output = self.out.strip_prefix(&self.root).unwrap();
        after.retain(|path, _| !path.starts_with(output));
        assert_eq!(
            after, before,
            "success changed inputs or leaked staging data"
        );
        package_files(&self.out)
    }
    fn reject(&self, binary: &Path, label: &str) -> String {
        let before = snapshot(&self.root);
        let result = self.run(binary);
        assert!(!result.status.success(), "accepted invalid case: {label}");
        assert_eq!(
            snapshot(&self.root),
            before,
            "rejection changed files: {label}"
        );
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(!error.trim().is_empty(), "missing diagnostic: {label}");
        error
    }
}

fn document(files: &BTreeMap<String, Vec<u8>>) -> SpriteDocument {
    SpriteDocument::from_json(std::str::from_utf8(&files["sprites.json"]).unwrap()).unwrap()
}
fn rgba(files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let document = document(files);
    orr_sample::project_sprites::decode_atlas(
        &files["atlas.png"],
        document.atlas().width,
        document.atlas().height,
    )
    .unwrap()
}
fn sampled_pixels(files: &BTreeMap<String, Vec<u8>>, id: &str, time: u64) -> Vec<u8> {
    let document = document(files);
    let image = rgba(files);
    let region = document
        .region(document.clip(id).unwrap().sample(time).region)
        .unwrap();
    let mut pixels = Vec::new();
    for y in region.y..region.y + region.height {
        let offset = ((y * document.atlas().width + region.x) * 4) as usize;
        pixels.extend_from_slice(&image[offset..offset + region.width as usize * 4]);
    }
    pixels
}

#[test]
fn shared_source_table_preserves_literal_pixels_regions_clips_and_relocated_bytes() {
    let Some(binary) = binary() else { return };
    let a = Fixture::new();
    let b = Fixture::new();
    let files = a.success(&binary);
    assert_eq!(files, b.success(&binary));
    assert_eq!(files["LICENSE.txt"], LICENSE);
    let mut expected = vec![0; 12 * 6 * 4];
    // Literal atlas locations, independent of the generated regions/offset code.
    for (x, y, pixel) in [
        (1, 1, P0[0]),
        (2, 1, P0[1]),
        (1, 2, P0[2]),
        (2, 2, P0[3]),
        (7, 0, P1[0]),
        (7, 1, P1[1]),
        (7, 2, P1[2]),
        (8, 2, P2[0]),
        (9, 2, P2[1]),
        (2, 3, P3[0]),
    ] {
        expected[(y * 12 + x) * 4..(y * 12 + x + 1) * 4].copy_from_slice(&pixel);
    }
    assert_eq!(
        rgba(&files),
        expected,
        "padding, hidden RGB, or placement changed"
    );
    let generated: Value = serde_json::from_slice(&files["sprites.json"]).unwrap();
    assert_eq!(
        generated,
        json!({"format":"orr_sprite","version":1,
        "atlas":{"image":"atlas.png","width":12,"height":6},
        "regions":[
            {"id":0,"x":0,"y":0,"width":4,"height":3},
            {"id":1,"x":4,"y":0,"width":4,"height":3},
            {"id":2,"x":8,"y":0,"width":4,"height":3},
            {"id":3,"x":0,"y":3,"width":4,"height":3}],
        "clips":[
            {"id":"z-run","mode":"loop","frames":[{"region":2,"duration_ms":11},{"region":0,"duration_ms":17},{"region":2,"duration_ms":23}]},
            {"id":"a-land","mode":"once","frames":[{"region":0,"duration_ms":13},{"region":1,"duration_ms":19},{"region":0,"duration_ms":5}]}]})
    );
    let manifest: Value = serde_json::from_slice(&files["orr.package.json"]).unwrap();
    assert_eq!(
        manifest,
        json!({"schema":1,"name":"named-sequence","version":"1.2.3","engine":"^0.0.1","capabilities":["sprite"],"dependencies":{},"files":["sprites.json","atlas.png","LICENSE.txt"]})
    );
}

#[test]
fn overlapping_repeated_clips_have_independent_exact_loop_once_boundaries() {
    let Some(binary) = binary() else { return };
    let files = Fixture::new().success(&binary);
    let document = document(&files);
    let loop_clip = document.clip("z-run").unwrap();
    let once_clip = document.clip("a-land").unwrap();
    assert_eq!(
        (loop_clip.mode(), loop_clip.duration_ms()),
        (PlaybackMode::Loop, 51)
    );
    assert_eq!(
        (once_clip.mode(), once_clip.duration_ms()),
        (PlaybackMode::Once, 37)
    );
    for time in [
        0,
        10,
        11,
        12,
        13,
        27,
        28,
        31,
        32,
        36,
        37,
        50,
        51,
        52,
        62,
        79,
        101,
        102,
        10000,
        u64::MAX,
    ] {
        let reduced = time % 51;
        let (index, region) = if reduced < 11 {
            (0, 2)
        } else if reduced < 28 {
            (1, 0)
        } else {
            (2, 2)
        };
        let sample = loop_clip.sample(time);
        assert_eq!(
            (sample.frame_index, sample.region, sample.finished),
            (index, region, false),
            "loop at {time}"
        );
        let (index, region) = if time < 13 {
            (0, 0)
        } else if time < 32 {
            (1, 1)
        } else {
            (2, 0)
        };
        let sample = once_clip.sample(time);
        assert_eq!(
            (sample.frame_index, sample.region, sample.finished),
            (index, region, time >= 37),
            "once at {time}"
        );
    }
    let mut a = Playback::default();
    let mut b = Playback::default();
    a.seek(28);
    b.seek(13);
    assert_eq!(
        (
            a.sample(loop_clip).frame_index,
            b.sample(once_clip).frame_index
        ),
        (2, 1)
    );
    a.advance(u64::MAX);
    assert_eq!(b.elapsed_ms(), 13);
    b.advance(24);
    assert!(b.sample(once_clip).finished);
    assert_eq!(a.elapsed_ms(), u64::MAX);
    a.reset();
    assert_eq!(a.sample(loop_clip).frame_index, 0);
    assert_eq!(b.elapsed_ms(), 37);
}

#[test]
fn unused_source_pixels_are_packed_but_do_not_change_selected_clip_semantics() {
    let Some(binary) = binary() else { return };
    let original = Fixture::new().success(&binary);
    let changed = Fixture::new();
    fs::write(
        changed.source.join("unused.png"),
        png(1, 1, &[251, 3, 5, 1], false),
    )
    .unwrap();
    let changed = changed.success(&binary);
    assert_ne!(original["atlas.png"], changed["atlas.png"]);
    assert_eq!(original["sprites.json"], changed["sprites.json"]);
    for id in ["z-run", "a-land"] {
        for time in [0, 11, 13, 28, 32, 37, 51, u64::MAX] {
            assert_eq!(
                sampled_pixels(&original, id, time),
                sampled_pixels(&changed, id, time)
            );
        }
    }
    let mut selected = recipe();
    selected["clips"][0]["frames"][0]["frame"] = 3.into();
    let fixture = Fixture::new();
    fixture.write(&selected);
    let referenced = fixture.success(&binary);
    assert_eq!(
        referenced["atlas.png"], original["atlas.png"],
        "selection must not repack source table"
    );
    assert_ne!(
        sampled_pixels(&referenced, "z-run", 0),
        sampled_pixels(&original, "z-run", 0)
    );
}

#[test]
fn collection_caps_accept_exact_limits_and_reject_empty_or_one_past() {
    let Some(binary) = binary() else { return };
    for (kind, count) in [
        ("sources", 256),
        ("clips", 256),
        ("per_clip", 4096),
        ("total", 16384),
    ] {
        let fixture = Fixture::new();
        let mut value = recipe();
        match kind {
            "sources" => {
                value["frames"] = json!(vec![json!({"png":"unused.png","offset":{"x":0,"y":0}}); count]);
                value["columns"] = 16.into();
            }
            "clips" => value["clips"] = Value::Array((0..count).map(|i| json!({"id":format!("clip-{i}"),"mode":"loop","frames":[{"frame":0,"duration_ms":1}]})).collect()),
            "per_clip" => value["clips"] = json!([{"id":"long","mode":"loop","frames":vec![json!({"frame":0,"duration_ms":1}); count]}]),
            "total" => value["clips"] = Value::Array((0..4).map(|i| json!({"id":format!("clip-{i}"),"mode":"once","frames":vec![json!({"frame":0,"duration_ms":1}); count / 4]})).collect()),
            _ => unreachable!(),
        }
        fixture.write(&value);
        let files = fixture.success(&binary);
        let generated = document(&files);
        match kind {
            "sources" => assert_eq!(generated.regions().len(), count),
            "clips" => assert_eq!(generated.clips().len(), count),
            "per_clip" => assert_eq!(generated.clips()[0].frames().len(), count),
            "total" => assert_eq!(
                generated
                    .clips()
                    .iter()
                    .map(|clip| clip.frames().len())
                    .sum::<usize>(),
                count
            ),
            _ => unreachable!(),
        }
        fs::remove_dir_all(&fixture.out).unwrap();
        match kind {
            "sources" => {
                let source = value["frames"][0].clone();
                value["frames"].as_array_mut().unwrap().push(source);
            }
            "clips" => value["clips"].as_array_mut().unwrap().push(
                json!({"id":"one-too-many","mode":"loop","frames":[{"frame":0,"duration_ms":1}]}),
            ),
            "per_clip" => value["clips"][0]["frames"]
                .as_array_mut()
                .unwrap()
                .push(json!({"frame":0,"duration_ms":1})),
            "total" => value["clips"].as_array_mut().unwrap().push(
                json!({"id":"over-total","mode":"loop","frames":[{"frame":0,"duration_ms":1}]}),
            ),
            _ => unreachable!(),
        }
        fixture.write(&value);
        fixture.reject(&binary, kind);
    }
    for pointer in ["/frames", "/clips", "/clips/0/frames"] {
        let fixture = Fixture::new();
        let mut value = recipe();
        *value.pointer_mut(pointer).unwrap() = json!([]);
        fixture.write(&value);
        fixture.reject(&binary, pointer);
    }
}

#[test]
fn schema3_requires_its_exact_fields_and_rejects_legacy_or_unknown_fields() {
    let Some(binary) = binary() else { return };
    for (parent, fields) in [
        (
            "",
            &[
                "schema", "name", "version", "license", "columns", "canvas", "frames", "clips",
            ][..],
        ),
        ("/canvas", &["width", "height"][..]),
        ("/frames/0", &["png", "offset"][..]),
        ("/frames/0/offset", &["x", "y"][..]),
        ("/clips/0", &["id", "mode", "frames"][..]),
        ("/clips/0/frames/0", &["frame", "duration_ms"][..]),
    ] {
        for field in fields {
            let fixture = Fixture::new();
            let mut value = recipe();
            value
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(*field);
            fixture.write(&value);
            fixture.reject(&binary, &format!("missing {parent}/{field}"));
        }
        let fixture = Fixture::new();
        let mut value = recipe();
        value.pointer_mut(parent).unwrap()["unrecognized"] = true.into();
        fixture.write(&value);
        fixture.reject(&binary, &format!("unknown field in {parent}"));
    }
    for (parent, field, extra) in [
        ("", "clip", json!("legacy")),
        ("", "mode", json!("loop")),
        ("/frames/0", "duration_ms", json!(1)),
        ("/clips/0/frames/0", "png", json!("z-first.png")),
        ("/clips/0/frames/0", "region", json!(0)),
    ] {
        let fixture = Fixture::new();
        let mut value = recipe();
        value.pointer_mut(parent).unwrap()[field] = extra;
        fixture.write(&value);
        fixture.reject(&binary, &format!("legacy {parent}/{field}"));
    }
    for version in [
        json!(0),
        json!(1),
        json!(2),
        json!(4),
        json!(3.0),
        json!("3"),
        Value::Null,
    ] {
        let fixture = Fixture::new();
        let mut value = recipe();
        value["schema"] = version;
        fixture.write(&value);
        fixture.reject(&binary, "wrong schema discriminator");
    }
}

#[test]
fn every_schema3_object_rejects_duplicates_null_and_positional_arrays() {
    let Some(binary) = binary() else { return };
    let serialized = serde_json::to_string(&recipe()).unwrap();
    for field in [
        "\"schema\":3",
        "\"name\":\"named-sequence\"",
        "\"columns\":3",
        "\"width\":4",
        "\"height\":3",
        "\"png\":\"z-first.png\"",
        "\"x\":1",
        "\"y\":1",
        "\"id\":\"z-run\"",
        "\"mode\":\"loop\"",
        "\"frame\":2",
        "\"duration_ms\":11",
    ] {
        assert!(serialized.contains(field));
        let fixture = Fixture::new();
        fs::write(
            &fixture.recipe,
            serialized.replacen(field, &format!("{field},{field}"), 1),
        )
        .unwrap();
        fixture.reject(&binary, &format!("duplicate {field}"));
    }
    // Struct-shaped positional arrays can otherwise be accepted by derived serde.
    for (pointer, positional) in [
        (
            "",
            json!([3,"named-sequence","1.2.3","LICENSE.txt",3,{"width":4,"height":3},recipe()["frames"],recipe()["clips"]]),
        ),
        ("/canvas", json!([4, 3])),
        ("/frames/0", json!(["z-first.png",{"x":1,"y":1}])),
        ("/frames/0/offset", json!([1, 1])),
        (
            "/clips/0",
            json!(["z-run","loop",[{"frame":2,"duration_ms":11}]]),
        ),
        ("/clips/0/frames/0", json!([2, 11])),
    ] {
        for replacement in [Value::Null, positional] {
            let fixture = Fixture::new();
            let mut value = recipe();
            *value.pointer_mut(pointer).unwrap() = replacement;
            fixture.write(&value);
            fixture.reject(&binary, &format!("non-object {pointer}"));
        }
    }
    for pointer in ["/frames", "/clips", "/clips/0/frames"] {
        for replacement in [Value::Null, json!({}), json!("not an array")] {
            let fixture = Fixture::new();
            let mut value = recipe();
            *value.pointer_mut(pointer).unwrap() = replacement;
            fixture.write(&value);
            fixture.reject(&binary, &format!("non-array {pointer}"));
        }
    }
    for suffix in ["{}", " trailing"] {
        let fixture = Fixture::new();
        fs::write(&fixture.recipe, format!("{serialized}{suffix}")).unwrap();
        fixture.reject(&binary, "trailing JSON data");
    }
}

#[test]
fn clip_names_use_unique_nonblank_utf8_byte_bounded_identifiers() {
    let Some(binary) = binary() else { return };
    for id in ["a".repeat(128), "é".repeat(64)] {
        let fixture = Fixture::new();
        let mut value = recipe();
        value["clips"][0]["id"] = id.clone().into();
        fixture.write(&value);
        assert!(document(&fixture.success(&binary)).clip(&id).is_some());
    }
    for id in [
        String::new(),
        " \t\r\n".into(),
        "\u{2003}".into(),
        "a".repeat(129),
        format!("{}a", "é".repeat(64)),
        "a-land".into(),
    ] {
        let fixture = Fixture::new();
        let mut value = recipe();
        value["clips"][0]["id"] = id.into();
        fixture.write(&value);
        fixture.reject(&binary, "blank, duplicate or over-byte-limit ID");
    }
    for mode in [
        json!("Loop"),
        json!("Once"),
        json!("pingpong"),
        json!(""),
        Value::Null,
        json!(1),
    ] {
        let fixture = Fixture::new();
        let mut value = recipe();
        value["clips"][0]["mode"] = mode;
        fixture.write(&value);
        fixture.reject(&binary, "invalid clip mode");
    }
}

#[test]
fn references_and_durations_are_strict_and_duration_overflow_is_per_clip() {
    let Some(binary) = binary() else { return };
    for reference in [
        json!(4),
        json!(u32::MAX),
        json!(u64::MAX),
        json!(-1),
        json!(0.5),
        json!(0.0),
        json!("0"),
        Value::Null,
    ] {
        let fixture = Fixture::new();
        let mut value = recipe();
        value["clips"][0]["frames"][0]["frame"] = reference;
        fixture.write(&value);
        fixture.reject(&binary, "invalid source reference");
    }
    for duration in [
        json!(0),
        json!(-1),
        json!(0.5),
        json!(1.0),
        json!("1"),
        Value::Null,
    ] {
        let fixture = Fixture::new();
        let mut value = recipe();
        value["clips"][0]["frames"][0]["duration_ms"] = duration;
        fixture.write(&value);
        fixture.reject(&binary, "invalid duration");
    }
    let fixture = Fixture::new();
    let mut value = recipe();
    value["clips"] = json!([
        {"id":"max-loop","mode":"loop","frames":[{"frame":2,"duration_ms":u64::MAX-1},{"frame":0,"duration_ms":1}]},
        {"id":"max-once","mode":"once","frames":[{"frame":1,"duration_ms":u64::MAX}]}]);
    fixture.write(&value);
    let files = fixture.success(&binary);
    let document = document(&files);
    let looping = document.clip("max-loop").unwrap();
    let once = document.clip("max-once").unwrap();
    assert_eq!(looping.duration_ms(), u64::MAX);
    assert_eq!(once.duration_ms(), u64::MAX);
    assert_eq!(looping.sample(u64::MAX - 2).region, 2);
    assert_eq!(looping.sample(u64::MAX - 1).region, 0);
    assert_eq!(looping.sample(u64::MAX).region, 2);
    assert!(!once.sample(u64::MAX - 1).finished);
    assert!(once.sample(u64::MAX).finished);
    fs::remove_dir_all(&fixture.out).unwrap();
    value["clips"][0]["frames"][0]["duration_ms"] = u64::MAX.into();
    fixture.write(&value);
    assert!(fixture
        .reject(&binary, "per-clip duration overflow")
        .contains("overflow"));
}

#[test]
fn common_canvas_full_rectangles_reject_clipping_bad_numbers_and_overflow() {
    let Some(binary) = binary() else { return };
    // Existing fixture has bottom/right flush edges and must be accepted.
    Fixture::new().success(&binary);
    for (pointer, bad) in [
        ("/canvas/width", json!(0)),
        ("/canvas/height", json!(0)),
        ("/canvas/width", json!(2049)),
        ("/canvas/height", json!(2049)),
        ("/frames/0/offset/x", json!(3)),
        ("/frames/0/offset/y", json!(2)),
        ("/frames/0/offset/x", json!(u32::MAX)),
        ("/frames/0/offset/y", json!(u32::MAX)),
        ("/columns", json!(0)),
        ("/columns", json!(5)),
        ("/columns", json!(u32::MAX)),
    ] {
        let fixture = Fixture::new();
        let mut value = recipe();
        *value.pointer_mut(pointer).unwrap() = bad;
        fixture.write(&value);
        fixture.reject(&binary, pointer);
    }
    for pointer in [
        "/canvas/width",
        "/canvas/height",
        "/frames/0/offset/x",
        "/frames/0/offset/y",
        "/columns",
    ] {
        for bad in [
            json!(-1),
            json!(1.5),
            json!(1.0),
            json!(u64::MAX),
            json!("1"),
            Value::Null,
        ] {
            let fixture = Fixture::new();
            let mut value = recipe();
            *value.pointer_mut(pointer).unwrap() = bad;
            fixture.write(&value);
            fixture.reject(&binary, pointer);
        }
    }
    let fixture = Fixture::new();
    let mut value = recipe();
    value["frames"][3]["offset"] = json!({"x":4,"y":0});
    fixture.write(&value);
    fs::write(
        fixture.source.join("unused.png"),
        png(1, 1, &[37, 41, 43, 0], false),
    )
    .unwrap();
    fixture.reject(
        &binary,
        "unused transparent source still requires full containment",
    );
}

#[test]
fn byte_budgets_include_each_source_table_entry_but_not_clip_reuse() {
    let Some(binary) = binary() else { return };
    let fixture = Fixture::new();
    let mut value = recipe();
    value["frames"] = json!([{"png":"unused.png","offset":{"x":0,"y":0}}]);
    value["columns"] = 1.into();
    value["canvas"] = json!({"width":1,"height":1});
    value["clips"] = json!([{"id":"reuse","mode":"loop","frames":vec![json!({"frame":0,"duration_ms":1}); 4096]}]);
    fixture.write(&value);
    // Valid PNG with trailing zero padding. Repeated clip references must not
    // multiply this encoded source charge or create duplicate atlas regions.
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.source.join("unused.png"))
        .unwrap()
        .set_len(16 * 1024)
        .unwrap();
    let files = fixture.success(&binary);
    assert_eq!(document(&files).regions().len(), 1);
    assert_eq!(rgba(&files), P3.concat());
    fs::remove_dir_all(&fixture.out).unwrap();
    // Exact encoded caps are valid: 16 MiB per source, 32 MiB across two
    // table entries naming the same file. The reference count remains 4096.
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.source.join("unused.png"))
        .unwrap()
        .set_len(MAX_PNG as u64)
        .unwrap();
    let files = fixture.success(&binary);
    assert_eq!(rgba(&files), P3.concat());
    fs::remove_dir_all(&fixture.out).unwrap();
    value["frames"] = json!(vec![value["frames"][0].clone(); 2]);
    fixture.write(&value);
    let files = fixture.success(&binary);
    assert_eq!(document(&files).regions().len(), 2);
    assert_eq!(rgba(&files), [P3.concat(), P3.concat()].concat());
    fs::remove_dir_all(&fixture.out).unwrap();
    value["frames"] = json!(vec![value["frames"][0].clone(); 3]);
    fixture.write(&value);
    fs::OpenOptions::new()
        .write(true)
        .open(fixture.source.join("unused.png"))
        .unwrap()
        .set_len((MAX_ENCODED / 3 + 1) as u64)
        .unwrap();
    let error = fixture.reject(
        &binary,
        "repeated source table entries count toward encoded aggregate",
    );
    assert!(error.contains("aggregate compressed PNG"), "{error}");
    for (target, length, label) in [
        ("unused.png", MAX_PNG + 1, "individual encoded PNG"),
        ("LICENSE.txt", MAX_LICENSE + 1, "license byte cap"),
        ("recipe.json", MAX_RECIPE + 1, "recipe byte cap"),
    ] {
        let fixture = Fixture::new();
        fs::OpenOptions::new()
            .write(true)
            .open(fixture.source.join(target))
            .unwrap()
            .set_len(length as u64)
            .unwrap();
        fixture.reject(&binary, label);
    }
    let fixture = Fixture::new();
    let mut license = vec![b'x'; MAX_LICENSE];
    license[MAX_LICENSE - 1] = b'\n';
    fs::write(fixture.source.join("LICENSE.txt"), &license).unwrap();
    assert_eq!(fixture.success(&binary)["LICENSE.txt"], license);
    let fixture = Fixture::new();
    let mut recipe_bytes = serde_json::to_vec(&recipe()).unwrap();
    recipe_bytes.resize(MAX_RECIPE, b' ');
    fs::write(&fixture.recipe, recipe_bytes).unwrap();
    fixture.success(&binary);

    // A single full-size source reaches both decoded/atlas caps exactly. Its
    // 4096 animation references must not multiply decoded-memory admission.
    let fixture = Fixture::new();
    let pixels = vec![113u8; 2048 * 2048 * 4];
    fs::write(
        fixture.source.join("unused.png"),
        png(2048, 2048, &pixels, false),
    )
    .unwrap();
    let mut value = recipe();
    value["frames"] = json!([{"png":"unused.png","offset":{"x":0,"y":0}}]);
    value["columns"] = 1.into();
    value["canvas"] = json!({"width":2048,"height":2048});
    value["clips"] = json!([{"id":"reuse-max-source","mode":"once","frames":vec![json!({"frame":0,"duration_ms":1}); 4096]}]);
    fixture.write(&value);
    let files = fixture.success(&binary);
    assert_eq!(document(&files).regions().len(), 1);
    assert_eq!(rgba(&files), pixels);
}

#[test]
fn dimension_and_decoded_budgets_preflight_all_sources_before_decoding() {
    let Some(binary) = binary() else { return };
    for dimension in [
        (2048u32, 2048u32),
        (2049, 1),
        (1, 2049),
        (u32::MAX, u32::MAX),
    ] {
        let fixture = Fixture::new();
        let mut value = recipe();
        value["canvas"] = json!({"width":dimension.0,"height":dimension.1});
        value["columns"] = 1.into();
        fixture.write(&value);
        // Valid signature/IHDR dimensions, intentionally undecodable payload.
        let mut header = png(1, 1, &[1, 2, 3, 4], false)[..24].to_vec();
        header[16..20].copy_from_slice(&dimension.0.to_be_bytes());
        header[20..24].copy_from_slice(&dimension.1.to_be_bytes());
        for name in ["z-first.png", "a-second.png", "m-third.png", "unused.png"] {
            fs::write(fixture.source.join(name), &header).unwrap();
        }
        for frame in value["frames"].as_array_mut().unwrap() {
            frame["offset"] = json!({"x":0,"y":0});
        }
        fixture.write(&value);
        let error = fixture.reject(&binary, "dimensions and aggregate decoded source budget");
        if dimension == (2048, 2048) {
            assert!(error.contains("aggregate decoded RGBA"), "{error}");
        } else {
            assert!(
                error.contains("dimension") || error.contains("overflow"),
                "{error}"
            );
        }
        assert!(!error.contains("decode PNG"), "{error}");
    }
    for (canvas, columns) in [
        (json!({"width":1025,"height":1}), 2),
        (json!({"width":2,"height":1025}), 1),
    ] {
        let fixture = Fixture::new();
        let mut value = recipe();
        value["canvas"] = canvas;
        value["columns"] = columns.into();
        value["frames"] = json!([
            {"png":"unused.png","offset":{"x":0,"y":0}},
            {"png":"unused.png","offset":{"x":0,"y":0}}]);
        value["clips"] =
            json!([{"id":"only","mode":"loop","frames":[{"frame":0,"duration_ms":1}]}]);
        fixture.write(&value);
        let error = fixture.reject(&binary, "atlas axes independently bounded");
        assert!(
            error.contains("dimension") || error.contains("overflow"),
            "{error}"
        );
    }
}

#[test]
fn even_unused_sources_reject_malformed_truncated_apng_and_invalid_license() {
    let Some(binary) = binary() else { return };
    for bad in [
        b"not png".to_vec(),
        png(1, 1, &[1, 2, 3, 4], false)[..24].to_vec(),
        png(1, 1, &[1, 2, 3, 4], true),
    ] {
        let fixture = Fixture::new();
        fs::write(fixture.source.join("unused.png"), bad).unwrap();
        fixture.reject(&binary, "invalid unreferenced PNG");
    }
    for bad in [Vec::new(), b" \n\r\t".to_vec(), vec![0xff, 0xfe]] {
        let fixture = Fixture::new();
        fs::write(fixture.source.join("LICENSE.txt"), bad).unwrap();
        fixture.reject(&binary, "invalid license text");
    }
}

#[test]
fn recipe_relative_paths_reject_traversal_absolute_backslash_and_colon() {
    let Some(binary) = binary() else { return };
    for pointer in ["/license", "/frames/0/png", "/frames/3/png"] {
        for path in [
            "",
            "../outside.png",
            "/tmp/outside.png",
            "sub/../../outside.png",
            "sub\\image.png",
            "C:outside.png",
        ] {
            let fixture = Fixture::new();
            let mut value = recipe();
            *value.pointer_mut(pointer).unwrap() = path.into();
            fixture.write(&value);
            fixture.reject(&binary, &format!("{pointer}: {path}"));
        }
    }
    let fixture = Fixture::new();
    let mut value = recipe();
    fs::create_dir(fixture.source.join("nested")).unwrap();
    fs::rename(
        fixture.source.join("unused.png"),
        fixture.source.join("nested/inside.png"),
    )
    .unwrap();
    value["frames"][3]["png"] = "nested/inside.png".into();
    fixture.write(&value);
    fixture.success(&binary);
}

#[test]
fn symlink_fifo_directory_and_missing_inputs_never_publish_or_change_sources() {
    let Some(binary) = binary() else { return };
    for name in ["recipe.json", "LICENSE.txt", "unused.png"] {
        for kind in ["missing", "directory", "fifo", "symlink"] {
            let fixture = Fixture::new();
            let path = fixture.source.join(name);
            let original = path.with_extension("original");
            fs::rename(&path, &original).unwrap();
            match kind {
                "missing" => {}
                "directory" => fs::create_dir(&path).unwrap(),
                "fifo" => rustix::fs::mknodat(
                    rustix::fs::CWD,
                    &path,
                    rustix::fs::FileType::Fifo,
                    rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
                    0,
                )
                .unwrap(),
                "symlink" => std::os::unix::fs::symlink(&original, &path).unwrap(),
                _ => unreachable!(),
            }
            fixture.reject(&binary, &format!("{kind}: {name}"));
        }
    }
    let mut fixture = Fixture::new();
    let alias = fixture.root.join("aliased inputs");
    std::os::unix::fs::symlink(&fixture.source, &alias).unwrap();
    fixture.recipe = alias.join("recipe.json");
    fixture.reject(&binary, "symlink recipe ancestor");
}

#[test]
fn existing_outputs_and_symlink_or_missing_parents_are_preserved() {
    let Some(binary) = binary() else { return };
    for kind in ["empty directory", "populated directory", "file", "symlink"] {
        let fixture = Fixture::new();
        match kind {
            "empty directory" => fs::create_dir(&fixture.out).unwrap(),
            "populated directory" => {
                fs::create_dir(&fixture.out).unwrap();
                fs::write(fixture.out.join("keep.txt"), b"never overwrite").unwrap();
            }
            "file" => fs::write(&fixture.out, b"never overwrite").unwrap(),
            "symlink" => std::os::unix::fs::symlink(&fixture.cwd, &fixture.out).unwrap(),
            _ => unreachable!(),
        }
        fixture.reject(&binary, kind);
    }
    let fixture = Fixture::new();
    fixture.success(&binary);
    fixture.reject(&binary, "already published package");
    let mut fixture = Fixture::new();
    fixture.out = fixture.root.join("missing-parent/output");
    fixture.reject(&binary, "missing output parent");
    let mut fixture = Fixture::new();
    let alias = fixture.root.join("output alias");
    std::os::unix::fs::symlink(&fixture.cwd, &alias).unwrap();
    fixture.out = alias.join("new package");
    fixture.reject(&binary, "symlink output ancestor");
}

#[test]
fn actual_cli_requires_standalone_help_and_exact_nonempty_options() {
    let Some(binary) = binary() else { return };
    let fixture = Fixture::new();
    let before = snapshot(&fixture.root);
    let result = fixture.command(&binary).arg("--help").output().unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("--recipe PATH --out NEW_DIR"));
    let recipe = fixture.recipe.to_str().unwrap();
    let out = fixture.out.to_str().unwrap();
    for arguments in [
        vec![],
        vec!["--recipe"],
        vec!["--out", out],
        vec!["--recipe", recipe, "--recipe", recipe],
        vec!["--out", out, "--out", out],
        vec!["--recipe", "", "--out", out],
        vec!["--recipe", recipe, "--out", ""],
        vec!["--recipe", recipe, "--unknown", out],
        vec!["--help", "--recipe", recipe, "--out", out],
        vec!["--recipe", recipe, "--out", out, "trailing"],
    ] {
        let result = fixture.command(&binary).args(&arguments).output().unwrap();
        assert!(!result.status.success(), "accepted {arguments:?}");
        assert_eq!(snapshot(&fixture.root), before);
    }
}

#[test]
fn actual_cli_schema1_matches_all_four_frozen_pre_canvas_files() {
    let Some(binary) = binary() else { return };
    let fixture = Fixture::new();
    for (index, pixels) in [
        [255, 0, 0, 255, 0, 255, 0, 128],
        [0, 0, 255, 64, 7, 11, 13, 0],
        [31, 41, 59, 255, 26, 53, 58, 127],
    ]
    .iter()
    .enumerate()
    {
        fs::write(
            fixture.source.join(format!("{index}.png")),
            png(2, 1, pixels, false),
        )
        .unwrap();
    }
    fixture.write(&json!({"schema":1,"name":"original-sequence","version":"1.2.3","license":"LICENSE.txt","clip":"run","mode":"loop","columns":2,
        "frames":[{"png":"2.png","duration_ms":30},{"png":"0.png","duration_ms":70},{"png":"1.png","duration_ms":20}]}));
    let files = fixture.success(&binary);
    for (name, expected, digest) in [
        (
            "LICENSE.txt",
            include_bytes!("../examples/png_sequence_package/fixtures/schema1/LICENSE.txt")
                .as_slice(),
            "947c920c9c134b00e590c40861d1f925ea1fb3d5c444d59d273f37ba53502db8",
        ),
        (
            "atlas.png",
            include_bytes!("../examples/png_sequence_package/fixtures/schema1/atlas.png")
                .as_slice(),
            "291746191733d5d55a4e73987b17dfe0cb764194a919ff749bfd24e5d1c17e3c",
        ),
        (
            "orr.package.json",
            include_bytes!("../examples/png_sequence_package/fixtures/schema1/orr.package.json")
                .as_slice(),
            "ebe1b6b2f12223b1cfe60e7cf7efb251e1bc5da540955f4a8e1178cb1f3966c6",
        ),
        (
            "sprites.json",
            include_bytes!("../examples/png_sequence_package/fixtures/schema1/sprites.json")
                .as_slice(),
            "2097787159dfa8c9fd7d9bdff72bedd5094e8dfc8e4aae0e04b727ee64e41fa8",
        ),
    ] {
        assert_eq!(
            files[name], expected,
            "schema1 frozen bytes changed: {name}"
        );
        assert_eq!(
            Sha256::digest(expected)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            digest,
            "schema1 baseline changed: {name}"
        );
    }
}

#[test]
fn actual_cli_schema2_matches_all_four_frozen_pre_named_clip_files() {
    let Some(binary) = binary() else { return };
    let fixture = Fixture::new();
    // Frozen by the untouched 177ff1ca producer, before schema3 source edits.
    for (name, bytes) in [
        (
            "recipe.json",
            include_bytes!("fixtures/png-sequence-clips-baseline/schema2/input/recipe.json")
                .as_slice(),
        ),
        (
            "LICENSE.txt",
            include_bytes!("fixtures/png-sequence-clips-baseline/schema2/input/LICENSE.txt")
                .as_slice(),
        ),
        (
            "0.png",
            include_bytes!("fixtures/png-sequence-clips-baseline/schema2/input/0.png").as_slice(),
        ),
        (
            "1.png",
            include_bytes!("fixtures/png-sequence-clips-baseline/schema2/input/1.png").as_slice(),
        ),
        (
            "2.png",
            include_bytes!("fixtures/png-sequence-clips-baseline/schema2/input/2.png").as_slice(),
        ),
    ] {
        fs::write(fixture.source.join(name), bytes).unwrap();
    }
    let files = fixture.success(&binary);
    for (name, expected, digest) in [
        (
            "LICENSE.txt",
            include_bytes!("fixtures/png-sequence-clips-baseline/schema2/output/LICENSE.txt")
                .as_slice(),
            "5d7d15d508f6d41340952bfbaf43b5413d210a377e86e90273c48b0b66d586fc",
        ),
        (
            "atlas.png",
            include_bytes!("fixtures/png-sequence-clips-baseline/schema2/output/atlas.png")
                .as_slice(),
            "a91b9b66052b84cdc784e1642763c53a16d6e603e3db739bd8dc8b32b3519895",
        ),
        (
            "orr.package.json",
            include_bytes!("fixtures/png-sequence-clips-baseline/schema2/output/orr.package.json")
                .as_slice(),
            "2c29902eb666b5a7fd638b23f7807ae71027d220a80442c4cc1aa92c27365dc8",
        ),
        (
            "sprites.json",
            include_bytes!("fixtures/png-sequence-clips-baseline/schema2/output/sprites.json")
                .as_slice(),
            "5d0781f6e1847093000b0eceee2df60682cb4955f248864807cbf4ff964a5468",
        ),
    ] {
        assert_eq!(
            files[name], expected,
            "schema2 frozen bytes changed: {name}"
        );
        assert_eq!(
            Sha256::digest(expected)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            digest,
            "schema2 baseline changed: {name}"
        );
    }
}
