//! Editor-only, bounded box recipes. Baking produces existing immutable models;
//! neither the recipe nor its history enters simulation, collision or runtime.
#![allow(clippy::float_arithmetic)]

use orr_model::{Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY};
use orr_package::{Lock, Manifest, Project, Runtime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Write}, path::{Component, Path, PathBuf}};

pub const MAX_RECIPE_BYTES: usize = 16 * 1024;
pub const MAX_HISTORY: usize = 64;
pub const MODEL_ASSET: &str = "box.model.json";
pub const RECIPE_ASSET: &str = "box.blockout.json";
const LICENSE: &str = "SPDX-License-Identifier: CC0-1.0\nThe procedural checker in this package is dedicated to the public domain under CC0 1.0 Universal.\nhttps://creativecommons.org/publicdomain/zero/1.0/\n";
const NOTICE: &str = "Orrery blockout-v1 procedural asymmetric 4x4 checker. Original generated pixels; no third-party image inputs.\nRecipe is authoring-only; box.model.json is the immutable runtime asset. No collision is generated.\n";

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face { PosX, NegX, PosY, NegY, PosZ, NegZ }
impl Face {
    pub const ALL: [Self; 6] = [Self::PosX, Self::NegX, Self::PosY, Self::NegY, Self::PosZ, Self::NegZ];
    pub fn index(self) -> usize { Self::ALL.iter().position(|face| *face == self).expect("six faces") }
    pub fn name(self) -> &'static str { ["+X", "-X", "+Y", "-Y", "+Z", "-Z"][self.index()] }
    fn id(self) -> &'static str { ["positive_x", "negative_x", "positive_y", "negative_y", "positive_z", "negative_z"][self.index()] }
    pub fn normal(self) -> [f32; 3] { let mut normal = [0.0; 3]; normal[self.index() / 2] = if self.index().is_multiple_of(2) { 1.0 } else { -1.0 }; normal }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceUv { pub offset: [f32; 2], pub scale: [f32; 2], pub quarter_turn: u8 }
impl Default for FaceUv { fn default() -> Self { Self { offset: [0.0; 2], scale: [1.0; 2], quarter_turn: 0 } } }
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceSettings { pub uv: FaceUv, pub material: u8 }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds { pub min: [f32; 3], pub max: [f32; 3] }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub format: String,
    pub version: u32,
    pub bounds: Bounds,
    /// Stable order: +X, -X, +Y, -Y, +Z, -Z. Topology cannot change.
    pub faces: [FaceSettings; 6],
}
impl Default for Recipe {
    fn default() -> Self { Self { format: "orr_box_blockout".into(), version: 1, bounds: Bounds { min: [-0.5; 3], max: [0.5; 3] }, faces: [FaceSettings { uv: FaceUv::default(), material: 0 }; 6] } }
}
impl Recipe {
    pub fn validate(&self) -> Result<()> {
        if self.format != "orr_box_blockout" || self.version != 1 { return Err("Unsupported box recipe format/version".into()); }
        for axis in 0..3 {
            let (min, max) = (self.bounds.min[axis], self.bounds.max[axis]);
            if !min.is_finite() || !max.is_finite() || min.abs() > 100.0 || max.abs() > 100.0 || max - min < 0.001 {
                return Err("Box bounds must be finite, within +/-100, and at least 0.001 apart".into());
            }
        }
        for face in &self.faces {
            if face.material > 1 || face.uv.quarter_turn > 3
                || !face.uv.offset.iter().all(|v| v.is_finite() && v.abs() <= 16.0)
                || !face.uv.scale.iter().all(|v| v.is_finite() && (0.001..=16.0).contains(v)) {
                return Err("Face UV requires finite offset +/-16, positive scale 0.001..16, quarter-turn 0..3 and material 0..1".into());
            }
        }
        Ok(())
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_RECIPE_BYTES { return Err("Box recipe exceeds 16 KiB".into()); }
        // Deserialize the typed structure first: duplicate and unknown fields are
        // rejected before any Value conversion could erase duplicate keys.
        let recipe: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let shape: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if !shape.is_object() || !shape["bounds"].is_object()
            || shape["faces"].as_array().is_none_or(|faces| faces.iter().any(|f| !f.is_object() || !f["uv"].is_object())) {
            return Err("Recipe, bounds, face and UV records must be JSON objects".into());
        }
        recipe.validate()?;
        Ok(recipe)
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>> { self.validate()?; serde_json::to_vec_pretty(self).map_err(|e| e.to_string()) }
    /// Unit planar coordinates are scaled, then quarter-turned counterclockwise
    /// around (0,0), then offset. Texture repeat is explicit in both slots.
    pub fn face_uvs(&self, face: Face) -> [[f32; 2]; 4] {
        let uv = self.faces[face.index()].uv;
        [[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0]].map(|p| {
            let [u,v] = [p[0] * uv.scale[0], p[1] * uv.scale[1]];
            let [u,v] = match uv.quarter_turn { 0 => [u,v], 1 => [-v,u], 2 => [-u,-v], _ => [v,-u] };
            [u + uv.offset[0], v + uv.offset[1]]
        })
    }
    pub fn bake_model(&self, exact_recipe_bytes: &[u8]) -> Result<StaticModel> {
        self.validate()?;
        if Self::from_bytes(exact_recipe_bytes)? != *self { return Err("Recipe bytes do not describe this box".into()); }
        let [x,y,z] = self.bounds.min;
        let [a,b,c] = self.bounds.max;
        let points = [
            [[a,y,z],[a,b,z],[a,b,c],[a,y,c]],
            [[x,y,c],[x,b,c],[x,b,z],[x,y,z]],
            [[x,b,c],[a,b,c],[a,b,z],[x,b,z]],
            [[x,y,z],[a,y,z],[a,y,c],[x,y,c]],
            [[x,y,c],[a,y,c],[a,b,c],[x,b,c]],
            [[a,y,z],[x,y,z],[x,b,z],[a,b,z]],
        ];
        let primitives = Face::ALL.into_iter().map(|face| {
            let uv = self.face_uvs(face);
            Primitive { id: format!("{RECIPE_ASSET}#node={}", face.id()), vertices: points[face.index()].iter().enumerate().map(|(i, position)| Vertex { position: *position, normal: face.normal(), uv: uv[i] }).collect(), indices: vec![0,1,2,0,2,3], material: u32::from(self.faces[face.index()].material), transform: IDENTITY }
        }).collect();
        StaticModel::new(ModelSource {
            format: "orr_static_model".into(), version: 1, asset_id: RECIPE_ASSET.into(),
            dependencies: vec![Dependency { uri: "$source".into(), sha256: digest(exact_recipe_bytes) }],
            primitives,
            materials: [[1.0,1.0,1.0,1.0],[0.3,0.65,1.0,1.0]].into_iter().map(|base_color| Material { base_color, image: 0, linear_filter: false, wrap_s: Wrap::Repeat, wrap_t: Wrap::Repeat }).collect(),
            images: vec![checker()],
        }).map_err(|e| e.to_string())
    }
}
/// Deliberately asymmetric: all sixteen cells differ, so flips/turns cannot
/// accidentally pass a symmetric checker oracle. Rows are top-to-bottom.
pub fn checker() -> Image {
    Image { width: 4, height: 4, rgba8: vec![
        255,32,16,255, 224,208,32,255, 24,176,64,255, 16,96,240,255,
        144,24,192,255, 240,120,24,255, 32,216,200,255, 192,64,96,255,
        48,64,112,255, 176,240,80,255, 104,32,40,255, 80,144,224,255,
        248,192,160,255, 64,112,32,255, 208,96,232,255, 24,40,56,255,
    ] }
}
fn digest(bytes: &[u8]) -> String { Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect() }

fn no_links(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        current.push(component.as_os_str());
        // A Windows prefix (C: or \\?\C:) is not the rooted volume yet.
        // Inspect the complete root on the next iteration, never skip an ancestor.
        if matches!(component, Component::Prefix(_))
            && matches!(components.peek(), Some(Component::RootDir)) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() { return Err("Recipe paths may not contain symlinks".into()); }
    }
    Ok(())
}
fn recipe_path(root: &Path, relative: &str) -> Result<(PathBuf, PathBuf)> {
    if relative.is_empty() || relative.len() > 128 || relative.contains('\\') || relative.contains(':')
        || Path::new(relative).components().any(|c| !matches!(c, Component::Normal(_)))
        || relative.split('/').any(|part| part.is_empty() || part == "." || part == ".." || part.starts_with('.'))
        || !relative.ends_with(".blockout.json") {
        return Err("Recipe path must be a contained portable *.blockout.json path".into());
    }
    no_links(root)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !root.is_dir() { return Err("Recipe root must be a directory".into()); }
    let path = root.join(relative);
    no_links(path.parent().ok_or("Recipe parent missing")?)?;
    if !path.parent().ok_or("Recipe parent missing")?.is_dir() { return Err("Recipe parent must be a directory".into()); }
    Ok((root, path))
}
fn read_recipe(path: &Path) -> Result<Vec<u8>> {
    no_links(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_RECIPE_BYTES as u64 { return Err("Recipe must be a bounded regular file".into()); }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() { return Err("Recipe must be a regular file".into()); }
    let mut bytes = Vec::new();
    file.take((MAX_RECIPE_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_RECIPE_BYTES { return Err("Recipe exceeds byte limit".into()); }
    Ok(bytes)
}

pub struct Blockout {
    root: PathBuf,
    path: PathBuf,
    recipe: Recipe,
    saved: Option<(Recipe, Vec<u8>)>,
    undo: Vec<Recipe>,
    redo: Vec<Recipe>,
}
impl Blockout {
    pub fn create(root: &Path, relative: &str) -> Result<Self> {
        let (root, path) = recipe_path(root, relative)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => return Err("Recipe already exists; use Open recipe".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.to_string()),
        }
        Ok(Self { root, path, recipe: Recipe::default(), saved: None, undo: Vec::new(), redo: Vec::new() })
    }
    pub fn open(root: &Path, relative: &str) -> Result<Self> {
        let (root, path) = recipe_path(root, relative)?;
        let bytes = read_recipe(&path)?;
        let recipe = Recipe::from_bytes(&bytes)?;
        Ok(Self { root, path, saved: Some((recipe.clone(), bytes)), recipe, undo: Vec::new(), redo: Vec::new() })
    }
    pub fn recipe(&self) -> &Recipe { &self.recipe }
    pub fn root(&self) -> &Path { &self.root }
    pub fn path(&self) -> &Path { &self.path }
    pub fn dirty(&self) -> bool { self.saved.as_ref().is_none_or(|(saved,_)| saved != &self.recipe) }
    pub fn history_len(&self) -> (usize, usize) { (self.undo.len(), self.redo.len()) }
    fn edit(&mut self, edit: impl FnOnce(&mut Recipe)) -> Result<()> {
        let mut candidate = self.recipe.clone();
        edit(&mut candidate);
        candidate.validate()?;
        if candidate != self.recipe {
            if self.undo.len() == MAX_HISTORY { self.undo.remove(0); }
            self.undo.push(std::mem::replace(&mut self.recipe, candidate));
            self.redo.clear();
        }
        Ok(())
    }
    /// Signed distance along the selected face's outward normal.
    pub fn move_face(&mut self, face: Face, delta: f32) -> Result<()> {
        self.edit(|r| { let i = face.index() / 2; if face.index().is_multiple_of(2) { r.bounds.max[i] += delta; } else { r.bounds.min[i] -= delta; } })
    }
    pub fn set_uv(&mut self, face: Face, uv: FaceUv) -> Result<()> { self.edit(|r| r.faces[face.index()].uv = uv) }
    pub fn set_material(&mut self, face: Face, material: u8) -> Result<()> { self.edit(|r| r.faces[face.index()].material = material) }
    pub fn undo(&mut self) { if let Some(old) = self.undo.pop() { self.redo.push(std::mem::replace(&mut self.recipe, old)); } }
    pub fn redo(&mut self) { if let Some(next) = self.redo.pop() { self.undo.push(std::mem::replace(&mut self.recipe, next)); } }
    fn unchanged_file(&self) -> Result<()> {
        match &self.saved {
            Some((_, bytes)) if read_recipe(&self.path)? == *bytes => Ok(()),
            Some(_) => Err("Recipe changed on disk; reopen before saving or baking".into()),
            None => match fs::symlink_metadata(&self.path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                _ => Err("Recipe path appeared since creation".into()),
            },
        }
    }
    pub fn save(&mut self) -> Result<()> {
        self.recipe.validate()?;
        no_links(self.path.parent().ok_or("Recipe parent missing")?)?;
        self.unchanged_file()?;
        // Unedited Open/Save retains the exact input bytes, including whitespace.
        let bytes = if !self.dirty() { self.saved.as_ref().expect("saved").1.clone() } else { self.recipe.to_bytes()? };
        let mut temp = tempfile::NamedTempFile::new_in(self.path.parent().ok_or("Recipe parent missing")?).map_err(|e| e.to_string())?;
        temp.write_all(&bytes).and_then(|_| temp.as_file().sync_all()).map_err(|e| e.to_string())?;
        self.unchanged_file()?;
        if self.saved.is_some() { temp.persist(&self.path).map_err(|e| e.to_string())?; }
        else { temp.persist_noclobber(&self.path).map_err(|e| e.to_string())?; }
        self.saved = Some((self.recipe.clone(), bytes));
        Ok(())
    }
    pub fn prepare_bake(&self, project: &Project) -> Result<PreparedBake> {
        if self.root != project.root() { return Err("Recipe and package must share the admitted project root".into()); }
        if self.dirty() { return Err("Save the recipe before baking".into()); }
        self.unchanged_file()?;
        let bytes = self.saved.as_ref().ok_or("Save recipe first")?.1.clone();
        let model = self.recipe.bake_model(&bytes)?;
        let model_bytes = model.to_bytes().map_err(|e| e.to_string())?;
        StaticModel::from_bytes(&model_bytes).map_err(|e| e.to_string())?;
        let recipe_hash = digest(&bytes);
        let package = format!("blockout-{}", &recipe_hash[..48]);
        let expected = project.verify().map_err(|e| e.to_string())?;
        let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
        for (name, contents) in [(RECIPE_ASSET, bytes.as_slice()), (MODEL_ASSET, model_bytes.as_slice()), ("CHECKER-LICENSE.txt", LICENSE.as_bytes()), ("NOTICE.txt", NOTICE.as_bytes())] {
            fs::write(stage.path().join(name), contents).map_err(|e| e.to_string())?;
        }
        let manifest = Manifest { schema: 1, name: package.clone(), version: "1.0.0".into(), engine: format!("={}", env!("CARGO_PKG_VERSION")), capabilities: ["models".into()].into(), dependencies: Default::default(), files: [RECIPE_ASSET.into(), MODEL_ASSET.into(), "CHECKER-LICENSE.txt".into(), "NOTICE.txt".into()].into() };
        fs::write(stage.path().join("orr.package.json"), serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let mut runtime = Runtime::content_only(); runtime.capabilities.insert("models".into());
        let locked = orr_package::inspect(stage.path(), &runtime).map_err(|e| e.to_string())?;
        if expected.packages.get(&package).is_some_and(|old| old != &locked) { return Err("Immutable blockout identity collision; existing package retained".into()); }
        let mut approved = expected.clone();
        approved.direct.insert(package.clone(), manifest.version);
        approved.packages.insert(package.clone(), locked);
        if approved.packages.len() > 128 { return Err("128-package limit reached; old revisions are retained for model undo".into()); }
        Ok(PreparedBake { stage, root: self.root.clone(), path: self.path.clone(), bytes, expected, approved, result: BakeResult { package, asset: MODEL_ASSET.into(), recipe_hash } })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeResult { pub package: String, pub asset: String, pub recipe_hash: String }
pub struct PreparedBake {
    stage: tempfile::TempDir,
    root: PathBuf,
    path: PathBuf,
    bytes: Vec<u8>,
    expected: Lock,
    approved: Lock,
    result: BakeResult,
}
impl PreparedBake {
    pub fn commit(self, project: &Project, before_publish: impl FnOnce() -> Result<()>) -> Result<BakeResult> {
        if project.root() != self.root { return Err("Bake project changed since preparation".into()); }
        project.install_checked(&[self.stage.path().to_path_buf()], &self.expected, &self.approved, || {
            before_publish()?;
            if read_recipe(&self.path)? != self.bytes { return Err("Recipe changed while baking; active lock retained".into()); }
            Ok(())
        }).map_err(|e| e.to_string())?;
        Ok(self.result)
    }
}

#[cfg(all(test, windows))]
mod windows_path_tests {
    use super::*;

    #[test]
    fn drive_and_verbatim_roots_preserve_recipe_path_checks() {
        let temporary = tempfile::tempdir().unwrap();
        let canonical = temporary.path().canonicalize().unwrap();
        for (index, directory) in [temporary.path(), canonical.as_path()].into_iter().enumerate() {
            let root: PathBuf = directory.components().take(2).collect();
            no_links(&root).unwrap();
            no_links(directory).unwrap();
            let relative = format!("root-{index}.blockout.json");
            let mut document = Blockout::create(directory, &relative).unwrap();
            document.save().unwrap();
            let bytes = fs::read(document.path()).unwrap();
            let mut reopened = Blockout::open(directory, &relative).unwrap();
            reopened.save().unwrap();
            assert_eq!(fs::read(reopened.path()).unwrap(), bytes);
            assert!(Blockout::create(directory, &relative).is_err());
            assert!(Blockout::create(directory, "../escape.blockout.json").is_err());
            assert!(Blockout::create(directory, "missing/child.blockout.json").is_err());
            assert!(Blockout::create(document.path(), "child.blockout.json").is_err());
            assert!(Blockout::create(directory, &format!("{relative}/child.blockout.json")).is_err());
        }
    }
}
