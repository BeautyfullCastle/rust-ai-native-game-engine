use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
pub const PACKAGE: &str = "delivery-tiles";
pub const LICENSE: &[u8] = b"Original asymmetric acceptance atlas. CC0-1.0\n";
pub fn texel(region: u32, x: usize, y: usize) -> [u8; 4] {
    match region {
        31 => [
            31 + x as u8 * 11,
            47 + y as u8 * 13,
            197 - x as u8 * 3 - y as u8 * 2,
            255,
        ],
        47 => [
            231 - y as u8 * 9,
            211 - x as u8 * 9,
            17 + x as u8 * 5 + y as u8 * 7,
            255,
        ],
        _ => panic!("unknown literal region"),
    }
}
pub fn package(root: &Path) {
    fs::create_dir(root).unwrap();
    let pixels: Vec<u8> = (0..12)
        .flat_map(|y| (0..32).flat_map(move |x| texel(if x < 16 { 31 } else { 47 }, x % 16, y)))
        .collect();
    let mut bytes = Vec::new();
    {
        let mut encoder = png17::Encoder::new(&mut bytes, 32, 12);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
    fs::write(root.join("atlas.png"), bytes).unwrap();
    fs::write(root.join("LICENSE.txt"), LICENSE).unwrap();
    write_json(
        &root.join("sprites.json"),
        &json!({"format":"orr_sprite","version":1,
        "atlas":{"image":"atlas.png","width":32,"height":12},
        "regions":[{"id":31,"x":0,"y":0,"width":16,"height":12},
                   {"id":47,"x":16,"y":0,"width":16,"height":12}],"clips":[]}),
    );
    write_json(
        &root.join("orr.package.json"),
        &json!({"schema":1,"name":PACKAGE,"version":"1.0.0","engine":"^0.0.1",
        "capabilities":["sprite"],"dependencies":{},"files":["sprites.json","atlas.png","LICENSE.txt"]}),
    );
}
pub fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
pub fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}
pub fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
pub fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, at: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_symlink() {
                result.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read_link(path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else if kind.is_dir() {
                visit(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}
pub fn copy(source: &Path, dest: &Path) {
    fs::create_dir(dest).unwrap();
    for (relative, bytes) in files(source) {
        let path = dest.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}
pub struct Fixture {
    pub temp: tempfile::TempDir,
    pub source: PathBuf,
    pub project: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("external package");
        let project = temp.path().join("created project");
        package(&source);
        Self {
            temp,
            source,
            project,
        }
    }
    pub fn command(&self, output: &Path) -> Command {
        self.options(output, &self.source, "16", "12")
    }
    pub fn options(&self, output: &Path, package: &Path, width: &str, height: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_orr_new_arena"));
        command
            .args([
                "--template",
                "arena-custom-tileset-2d-v1",
                "--seed",
                "custom-delivery",
                "--output",
            ])
            .arg(output)
            .arg("--tileset-package")
            .arg(package)
            .args(["--tile-width", width, "--tile-height", height])
            .current_dir(self.temp.path())
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        command
    }
    pub fn create(&self) {
        success(self.command(&self.project).output().unwrap());
    }
    pub fn reject(&self) {
        let before = files(self.temp.path());
        let result = self.command(&self.project).output().unwrap();
        assert!(!result.status.success(), "invalid package accepted");
        assert!(!self.project.exists());
        assert_eq!(files(self.temp.path()), before);
        assert!(!fs::read_dir(self.temp.path()).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".orr-create-")
        }));
    }
}
