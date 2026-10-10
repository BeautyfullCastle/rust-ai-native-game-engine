//! Installed asymmetric non-square fixture and independent texel oracle.
#![allow(dead_code)]
use orr_sample::project_sprites::Orientation;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const PACKAGE: &str = "orientation-fixture";
pub const REGION: u32 = 7;
pub const WIDTH: u32 = 5;
pub const HEIGHT: u32 = 3;
pub struct Fixture {
    pub temp: tempfile::TempDir,
    pub project: PathBuf,
    pub source: PathBuf,
    pub scale: f32,
}
impl Fixture {
    pub fn new(collect: bool) -> Self {
        let root = fs::canonicalize(std::env::temp_dir()).unwrap();
        let temp = tempfile::tempdir_in(root).unwrap();
        let project = temp.path().join("authored project");
        let source = temp.path().join("package source");
        fs::create_dir(&project).unwrap();
        fs::create_dir(&source).unwrap();
        write_json(
            source.join("orr.package.json"),
            &json!({
                "schema":1,"name":PACKAGE,"version":"1.0.0","engine":"^0.0.1",
                "capabilities":["sprite"],"dependencies":{},"files":["sprites.json","atlas.png","LICENSE.txt"]
            }),
        );
        fs::write(
            source.join("LICENSE.txt"),
            "Original procedural orientation test atlas and sprite descriptor.\n\
             Copyright (c) 2026 Orrery contributors\n\
             SPDX-License-Identifier: MIT\n",
        )
        .unwrap();
        // A non-square, asymmetric region inset in a differently colored
        // border catches both atlas-axis and rotated-extent mistakes.
        write_json(
            source.join("sprites.json"),
            &json!({
                "format":"orr_sprite","version":1,
                "atlas":{"image":"atlas.png","width":7,"height":5},
                "regions":[{"id":REGION,"x":1,"y":1,"width":WIDTH,"height":HEIGHT}],
                "clips":[{"id":"idle","mode":"loop","frames":[{"region":REGION,"duration_ms":100}]}]
            }),
        );
        write_png(&source.join("atlas.png"), &pixels(false));
        orr_package::Project::open_for_install(
            &project,
            orr_package::Runtime::content_only().engine_version,
        )
        .unwrap()
        .install(std::slice::from_ref(&source))
        .unwrap();
        let scene = if collect {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../scenes/collect_dodge_v1.scene.yaml"
            ))
        } else {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/saved_arena_project/arena.scene.yaml"
            ))
        };
        fs::write(project.join("scene.yaml"), scene).unwrap();
        write_json(
            project.join("orr.project.json"),
            &json!({"schema":2,"engine":"*","entry":{
                "game":if collect {"collect-dodge-v1"} else {"arena"},"scene":"scene.yaml","sprites":"view.json"
            }}),
        );
        let scale = if collect { 1.5 } else { 4.0 };
        write_json(
            project.join("view.json"),
            &json!({"version":2,"scene":"scene.yaml","project":".","bindings":{
                "e_00000001":{"package":PACKAGE,"document":"sprites.json","source":{"Region":REGION},"units_per_pixel":scale},
                "e_00000002":{"package":PACKAGE,"document":"sprites.json","source":{"Region":REGION},"units_per_pixel":scale}
            }}),
        );
        Self {
            temp,
            project,
            source,
            scale,
        }
    }
    pub fn orientation(&self, value: Option<Orientation>) {
        change(self.project.join("view.json"), |document| {
            if let Some(value) = value {
                document["version"] = json!(3);
                document["bindings"]["e_00000001"]["orientation"] =
                    serde_json::to_value(value).unwrap();
            } else {
                document["bindings"]["e_00000001"]
                    .as_object_mut()
                    .unwrap()
                    .remove("orientation");
            }
        });
    }
}
pub fn write_json(path: impl AsRef<Path>, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
pub fn change(path: impl AsRef<Path>, edit: impl FnOnce(&mut Value)) {
    let path = path.as_ref();
    let mut value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    edit(&mut value);
    write_json(path, &value);
}
pub fn color(x: u32, y: u32, replacement: bool) -> [u8; 4] {
    let rgb = [
        31 + x as u8 * 39,
        43 + y as u8 * 73,
        220 - x as u8 * 21 - y as u8 * 23,
    ];
    if replacement {
        [rgb[2], rgb[0], rgb[1], 255]
    } else {
        [rgb[0], rgb[1], rgb[2], 255]
    }
}
pub fn pixels(replacement: bool) -> Vec<u8> {
    let mut rgba = vec![0; 7 * 5 * 4];
    for y in 0..5 {
        for x in 0..7 {
            let color = if (1..=WIDTH).contains(&x) && (1..=HEIGHT).contains(&y) {
                color(x - 1, y - 1, replacement)
            } else {
                [255, 0, 255, 255]
            };
            let at = ((y * 7 + x) * 4) as usize;
            rgba[at..at + 4].copy_from_slice(&color);
        }
    }
    rgba
}
pub fn write_png(path: &Path, rgba: &[u8]) {
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), 7, 5);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
}
pub fn combinations() -> impl Iterator<Item = Orientation> {
    (0..4).flat_map(|quarter_turns| {
        [false, true].into_iter().flat_map(move |flip_x| {
            [false, true].into_iter().map(move |flip_y| Orientation {
                quarter_turns,
                flip_x,
                flip_y,
            })
        })
    })
}
/// Independent integer quarter-turn oracle, never production trigonometry
/// or UV helpers. Starts at source-texel centers in world-up coordinates.
pub fn texel_offset(x: u32, y: u32, orientation: Orientation) -> [f32; 2] {
    let mut u = x as f32 + 0.5 - WIDTH as f32 / 2.0;
    let mut v = HEIGHT as f32 / 2.0 - y as f32 - 0.5;
    if orientation.flip_x {
        u = -u;
    }
    if orientation.flip_y {
        v = -v;
    }
    match orientation.quarter_turns {
        0 => [u, v],
        1 => [-v, u],
        2 => [-u, -v],
        3 => [v, -u],
        _ => panic!("invalid test orientation"),
    }
}
pub fn assert_texels(
    orientation: Orientation,
    scale: f32,
    center: [f32; 2],
    replacement: bool,
    mut sample: impl FnMut([f32; 2]) -> [u8; 4],
) {
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let [u, v] = texel_offset(x, y, orientation);
            let actual = sample([center[0] + u * scale, center[1] + v * scale]);
            let expected = color(x, y, replacement);
            for channel in 0..4 {
                assert!(actual[channel].abs_diff(expected[channel]) <= 4,
                    "{orientation:?}, texel({x},{y}), channel{channel}: actual {actual:?}, expected {expected:?}");
            }
        }
    }
}
pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                visit(root, &entry.path(), out);
            } else {
                assert!(kind.is_file());
                out.insert(
                    entry.path().strip_prefix(root).unwrap().into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
