//! Optional presentation-only authoring document. Never sent to the host.
//! Sidecar saves are atomic for this file, not a joint scene/sidecar transaction.
use orr_sprite::SpriteDocument;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_BINDINGS: usize = 4096;
const MAX_HISTORY: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Source {
    Region(u32),
    Clip(String),
    /// Presentation-only clip selection from caller-observed motion.
    Locomotion {
        idle: String,
        walk: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub package: String,
    pub document: String,
    pub source: Source,
    /// World units per sprite pixel.
    pub units_per_pixel: f32,
}
impl Binding {
    /// Existing callers preview the idle clip of a locomotion binding.
    pub fn region(&self, document: &SpriteDocument, elapsed_ms: u64) -> Result<u32, String> {
        self.region_for_motion(document, elapsed_ms, false)
    }
    pub fn region_for_motion(
        &self,
        document: &SpriteDocument,
        elapsed_ms: u64,
        moving: bool,
    ) -> Result<u32, String> {
        if !self.units_per_pixel.is_finite() || !(0.0001..=100.0).contains(&self.units_per_pixel) {
            return Err("units per pixel must be finite and between 0.0001 and 100".into());
        }
        match &self.source {
            Source::Region(id) => document
                .region(*id)
                .map(|_| *id)
                .ok_or_else(|| format!("missing region {id}")),
            Source::Clip(id) => document
                .clip(id)
                .map(|clip| clip.sample(elapsed_ms).region)
                .ok_or_else(|| format!("missing clip {id}")),
            Source::Locomotion { idle, walk } => {
                // Resolve both clips so a missing inactive clip cannot hide
                // until the entity starts or stops moving.
                let idle = document
                    .clip(idle)
                    .ok_or_else(|| format!("missing idle clip {idle}"))?;
                let walk = document
                    .clip(walk)
                    .ok_or_else(|| format!("missing walk clip {walk}"))?;
                let clip = if moving { walk } else { idle };
                Ok(clip.sample(elapsed_ms).region)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u32,
    /// Scene filename, relative to this sidecar's parent directory.
    pub scene: String,
    /// Package project directory, relative to this sidecar's parent directory.
    pub project: String,
    /// Persistent scene GUIDs, never recyclable frame handles.
    pub bindings: BTreeMap<String, Binding>,
    /// Optional persistent scene GUID. View-only, never sent to the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_follow: Option<String>,
}
fn relative(path: &str) -> bool {
    !path.is_empty()
        && !Path::new(path).is_absolute()
        && !path.contains('\\')
        && !path.contains(':')
}
impl Document {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.version, 1 | 2) {
            return Err("unsupported sprite binding version".into());
        }
        if self.version == 1 && self.has_v2_features() {
            return Err(
                "version-1 sprite bindings cannot contain locomotion or camera follow".into(),
            );
        }
        if let Some(guid) = &self.camera_follow {
            orr_reflect::Guid::parse(guid)
                .map_err(|_| format!("invalid camera follow scene GUID: {guid}"))?;
        }
        if !relative(&self.scene) || !relative(&self.project) {
            return Err("scene and project must be explicit relative paths".into());
        }
        if self.bindings.len() > MAX_BINDINGS {
            return Err("too many sprite bindings".into());
        }
        for (guid, binding) in &self.bindings {
            orr_reflect::Guid::parse(guid).map_err(|_| format!("invalid scene GUID: {guid}"))?;
            if binding.package.is_empty() || !relative(&binding.document) {
                return Err(format!("invalid package/document binding for {guid}"));
            }
            if !binding.units_per_pixel.is_finite()
                || !(0.0001..=100.0).contains(&binding.units_per_pixel)
            {
                return Err(format!("invalid sprite scale for {guid}"));
            }
            if let Source::Locomotion { idle, walk } = &binding.source {
                for (role, clip) in [("idle", idle), ("walk", walk)] {
                    if clip.trim().is_empty() || clip.len() > orr_sprite::MAX_CLIP_ID_BYTES {
                        return Err(format!("invalid {role} clip name for {guid}"));
                    }
                }
            }
        }
        Ok(())
    }
    fn has_v2_features(&self) -> bool {
        self.camera_follow.is_some()
            || self
                .bindings
                .values()
                .any(|binding| matches!(binding.source, Source::Locomotion { .. }))
    }
}

pub struct Bindings {
    pub path: PathBuf,
    document: Document,
    saved: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
}
impl Bindings {
    pub fn create(path: PathBuf, scene: String, project: String) -> Result<Self, String> {
        if path.exists() {
            return Err("sidecar exists; use Open bindings".into());
        }
        let document = Document {
            version: 2,
            scene,
            project,
            bindings: BTreeMap::new(),
            camera_follow: None,
        };
        document.validate()?;
        Ok(Self {
            path,
            saved: document.clone(),
            document,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("binding file exceeds byte limit".into());
        }
        let document: Document = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        document.validate()?;
        Ok(Self {
            path,
            saved: document.clone(),
            document,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn dirty(&self) -> bool {
        self.document != self.saved || !self.path.exists()
    }
    pub fn base(&self) -> &Path {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }
    pub fn matches_scene(&self, scene: &str) -> bool {
        match (
            std::fs::canonicalize(self.base().join(&self.document.scene)),
            std::fs::canonicalize(scene),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
    /// Assignment/removal of the entire selection is one transaction.
    pub fn assign(
        &mut self,
        guids: &[orr_reflect::Guid],
        binding: Option<Binding>,
    ) -> Result<(), String> {
        if guids.is_empty() {
            return Err("select at least one scene entity".into());
        }
        let mut next = self.document.clone();
        for guid in guids {
            if let Some(binding) = &binding {
                next.bindings.insert(guid.to_string(), binding.clone());
            } else {
                next.bindings.remove(&guid.to_string());
            }
        }
        self.commit(next)
    }
    /// Follow assignment/removal shares the sprite binding undo history.
    pub fn set_camera_follow(&mut self, guid: Option<orr_reflect::Guid>) -> Result<(), String> {
        let mut next = self.document.clone();
        next.camera_follow = guid.map(|guid| guid.to_string());
        self.commit(next)
    }
    fn commit(&mut self, mut next: Document) -> Result<(), String> {
        // Preserve legacy documents until a v2 feature is authored. Migration
        // is part of the same transaction, so undo restores the prior version.
        if next.version == 1 && next.has_v2_features() {
            next.version = 2;
        }
        next.validate()?;
        if next != self.document {
            if self.undo.len() == MAX_HISTORY {
                self.undo.remove(0);
            }
            self.undo.push(std::mem::replace(&mut self.document, next));
            self.redo.clear();
        }
        Ok(())
    }
    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.document, previous));
        }
    }
    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.document, next));
        }
    }
    pub fn save(&mut self) -> Result<(), String> {
        self.document.validate()?;
        let bytes = serde_json::to_vec_pretty(&self.document).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("binding file exceeds byte limit".into());
        }
        let mut temp = tempfile::NamedTempFile::new_in(self.base()).map_err(|e| e.to_string())?;
        temp.write_all(&bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        temp.persist(&self.path).map_err(|e| e.to_string())?;
        self.saved = self.document.clone();
        Ok(())
    }
}

/// Resolve a portable sidecar project path without hiding symlinks behind `..`.
/// Every traversed directory is checked before lexical parent removal; Project
/// repeats its own no-symlink check on the resulting absolute path.
pub fn resolve_project(base: &Path, relative_project: &str) -> Result<PathBuf, String> {
    use std::path::Component;
    if !relative(relative_project) {
        return Err("project path must be relative".into());
    }
    let current = if base.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?
    };
    let mut resolved = PathBuf::new();
    // Do not join onto a Windows verbatim path: push/join normalizes ParentDir
    // before we can reject root escapes or symlinks along the original path.
    // Parse portable relative separators before appending individual components.
    let mut components = current
        .components()
        .chain(base.components())
        .chain(Path::new(relative_project).components())
        .peekable();
    while let Some(component) = components.next() {
        match component {
            Component::CurDir => continue,
            Component::ParentDir => {
                if !resolved.pop() {
                    return Err("project path escapes filesystem root".into());
                }
            }
            _ => resolved.push(component),
        }
        // Canonical Windows paths include a verbatim drive prefix. Its bare
        // prefix is a device, not a directory; check it together with RootDir.
        // A prefix-only UNC root still needs its own metadata check.
        if matches!(component, Component::Prefix(_))
            && components.peek() == Some(&Component::RootDir)
        {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&resolved)
            .map_err(|e| format!("project directory {}: {e}", resolved.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "project path must contain only real directories: {}",
                resolved.display()
            ));
        }
    }
    Ok(resolved)
}

pub struct Asset {
    pub document: SpriteDocument,
    pub rgba: Vec<u8>,
}
/// Verified package reads include manifest/capability/hash checks. PNG dimensions
/// are checked before pixel decoder allocation against the validated document.
pub fn load_asset(project_root: &Path, package: &str, document: &str) -> Result<Asset, String> {
    let project = open_project(project_root)?;
    load_project_asset(&project, package, document)
}
pub fn open_project(project_root: &Path) -> Result<orr_package::Project, String> {
    let mut runtime = orr_package::Runtime::content_only();
    runtime.capabilities.insert("sprite".into());
    let project =
        orr_package::Project::open(project_root, runtime).map_err(|e| format!("project: {e}"))?;
    project
        .verify()
        .map_err(|e| format!("package verification: {e}"))?;
    Ok(project)
}
pub fn load_project_asset(
    project: &orr_package::Project,
    package: &str,
    document: &str,
) -> Result<Asset, String> {
    let bytes = project
        .read_asset(package, document)
        .map_err(|e| format!("sprite document: {e}"))?;
    let sprite = SpriteDocument::from_json(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
        .map_err(|e| format!("sprite document: {e}"))?;
    let image = project
        .read_asset(package, &sprite.atlas().image)
        .map_err(|e| format!("atlas: {e}"))?;
    let rgba = decode_atlas(&image, sprite.atlas().width, sprite.atlas().height)?;
    Ok(Asset {
        document: sprite,
        rgba,
    })
}
fn decode_atlas(
    image: &[u8],
    expected_width: u32,
    expected_height: u32,
) -> Result<Vec<u8>, String> {
    if expected_width == 0
        || expected_height == 0
        || expected_width > orr_sprite::MAX_ATLAS_DIMENSION
        || expected_height > orr_sprite::MAX_ATLAS_DIMENSION
    {
        return Err("invalid atlas dimensions".into());
    }
    if image.len() < 24 || &image[..8] != b"\x89PNG\r\n\x1a\n" || &image[12..16] != b"IHDR" {
        return Err("atlas must be PNG".into());
    }
    let width = u32::from_be_bytes(image[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(image[20..24].try_into().unwrap());
    if width != expected_width || height != expected_height {
        return Err("atlas PNG dimensions differ from sprite document".into());
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(image));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    if reader.info().animation_control.is_some() {
        return Err("animated PNG atlases are unsupported; use sprite clips".into());
    }
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).map_err(|e| e.to_string())?;
    if info.width != width || info.height != height {
        return Err("decoded PNG frame dimensions differ from atlas".into());
    }
    let expected_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("atlas byte size overflow")?;
    let mut rgba = Vec::with_capacity(expected_bytes);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(&pixels[..info.buffer_size()]),
        png::ColorType::Rgb => {
            for p in pixels[..info.buffer_size()].chunks_exact(3) {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for p in &pixels[..info.buffer_size()] {
                rgba.extend_from_slice(&[*p, *p, *p, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for p in pixels[..info.buffer_size()].chunks_exact(2) {
                rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        _ => return Err("unsupported PNG color type".into()),
    }
    if rgba.len() != expected_bytes {
        return Err("decoded PNG pixel count differs from atlas".into());
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(source: Source) -> Binding {
        Binding {
            package: "demo".into(),
            document: "hero.json".into(),
            source,
            units_per_pixel: 0.125,
        }
    }

    fn sprite_document() -> SpriteDocument {
        SpriteDocument::from_json(
            r#"{
                "format":"orr_sprite","version":1,
                "atlas":{"image":"hero.png","width":3,"height":1},
                "regions":[
                    {"id":1,"x":0,"y":0,"width":1,"height":1},
                    {"id":2,"x":1,"y":0,"width":1,"height":1},
                    {"id":3,"x":2,"y":0,"width":1,"height":1}
                ],
                "clips":[
                    {"id":"idle","mode":"loop","frames":[{"region":1,"duration_ms":100}]},
                    {"id":"walk","mode":"loop","frames":[
                        {"region":2,"duration_ms":100},{"region":3,"duration_ms":100}
                    ]}
                ]
            }"#,
        )
        .unwrap()
    }

    fn legacy_json() -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "scene": "arena.scene.yaml",
            "project": ".",
            "bindings": {
                "e_00000001": binding(Source::Region(1)),
                "e_00000002": binding(Source::Clip("walk".into()))
            }
        })
    }

    fn open_json(path: &Path, json: &serde_json::Value) -> Result<Bindings, String> {
        std::fs::write(path, serde_json::to_vec(json).unwrap()).unwrap();
        Bindings::open(path.to_path_buf())
    }

    #[test]
    fn locomotion_samples_motion_and_legacy_sources_keep_their_behavior() {
        let document = sprite_document();
        let locomotion = binding(Source::Locomotion {
            idle: "idle".into(),
            walk: "walk".into(),
        });
        assert_eq!(locomotion.region(&document, 100).unwrap(), 1);
        assert_eq!(
            locomotion.region_for_motion(&document, 100, false).unwrap(),
            1
        );
        assert_eq!(locomotion.region_for_motion(&document, 0, true).unwrap(), 2);
        assert_eq!(
            locomotion.region_for_motion(&document, 100, true).unwrap(),
            3
        );
        assert_eq!(
            locomotion.region_for_motion(&document, 200, true).unwrap(),
            2
        );
        for moving in [false, true] {
            assert_eq!(
                binding(Source::Region(2))
                    .region_for_motion(&document, 100, moving)
                    .unwrap(),
                2
            );
            assert_eq!(
                binding(Source::Clip("walk".into()))
                    .region_for_motion(&document, 100, moving)
                    .unwrap(),
                3
            );
        }
    }

    #[test]
    fn locomotion_validates_both_clips_even_when_the_missing_clip_is_inactive() {
        let document = sprite_document();
        for (idle, walk, missing) in [
            ("absent", "walk", "missing idle clip absent"),
            ("idle", "absent", "missing walk clip absent"),
        ] {
            let binding = binding(Source::Locomotion {
                idle: idle.into(),
                walk: walk.into(),
            });
            for moving in [false, true] {
                assert_eq!(
                    binding.region_for_motion(&document, 0, moving).unwrap_err(),
                    missing
                );
            }
            assert_eq!(binding.region(&document, 0).unwrap_err(), missing);
        }
    }

    #[test]
    fn legacy_read_save_and_ordinary_edits_retain_version_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.sprites.json");
        let mut bindings = open_json(&path, &legacy_json()).unwrap();
        assert_eq!(bindings.document().version, 1);
        assert_eq!(bindings.document().camera_follow, None);
        assert!(!bindings.dirty());
        assert_eq!(
            bindings.document().bindings["e_00000002"]
                .region(&sprite_document(), 100)
                .unwrap(),
            3
        );
        bindings.set_camera_follow(None).unwrap();
        assert!(bindings.undo.is_empty());
        bindings
            .assign(
                &[orr_reflect::Guid::from_u32(3)],
                Some(binding(Source::Clip("idle".into()))),
            )
            .unwrap();
        assert_eq!(bindings.document().version, 1);
        bindings.undo();
        assert!(!bindings.dirty());
        bindings.save().unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved, legacy_json());
        assert_eq!(
            Bindings::open(path).unwrap().document(),
            bindings.document()
        );
    }

    #[test]
    fn legacy_migration_is_part_of_each_new_feature_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.sprites.json");
        for follow in [false, true] {
            let mut bindings = open_json(&path, &legacy_json()).unwrap();
            let legacy = bindings.document().clone();
            if follow {
                bindings
                    .set_camera_follow(Some(orr_reflect::Guid::from_u32(1)))
                    .unwrap();
            } else {
                bindings
                    .assign(
                        &[orr_reflect::Guid::from_u32(1)],
                        Some(binding(Source::Locomotion {
                            idle: "idle".into(),
                            walk: "walk".into(),
                        })),
                    )
                    .unwrap();
            }
            assert_eq!(bindings.document().version, 2);
            assert!(bindings.dirty());
            let migrated = bindings.document().clone();
            bindings.undo();
            assert_eq!(bindings.document(), &legacy);
            assert!(!bindings.dirty());
            bindings.redo();
            assert_eq!(bindings.document(), &migrated);
            bindings.save().unwrap();
            assert_eq!(Bindings::open(path.clone()).unwrap().document(), &migrated);
        }
    }

    #[test]
    fn legacy_documents_cannot_smuggle_new_features_or_unknown_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.sprites.json");
        let mut follow = legacy_json();
        follow["camera_follow"] = serde_json::json!("e_00000001");
        assert!(open_json(&path, &follow)
            .err()
            .unwrap()
            .contains("version-1"));
        let mut locomotion = legacy_json();
        locomotion["bindings"]["e_00000001"]["source"] =
            serde_json::json!({"Locomotion": {"idle": "idle", "walk": "walk"}});
        assert!(open_json(&path, &locomotion)
            .err()
            .unwrap()
            .contains("version-1"));
        for version in [0, 3, u32::MAX] {
            let mut unknown = legacy_json();
            unknown["version"] = serde_json::json!(version);
            assert!(open_json(&path, &unknown)
                .err()
                .unwrap()
                .contains("unsupported"));
        }
    }

    #[test]
    fn camera_follow_accepts_only_persistent_scene_guids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("follow.sprites.json");
        let mut document = legacy_json();
        document["version"] = serde_json::json!(2);
        for invalid in [
            "",
            "hero",
            "1v0",
            "e_1",
            "e_DEADBEEF",
            "e_0000000g",
            "e_000000000000000000000000000000000",
        ] {
            document["camera_follow"] = serde_json::json!(invalid);
            assert!(open_json(&path, &document)
                .err()
                .unwrap()
                .contains("camera follow scene GUID"));
        }
        for valid in ["e_deadbeef", "e_0123456789abcdef0123456789abcdef"] {
            document["camera_follow"] = serde_json::json!(valid);
            let mut bindings = open_json(&path, &document).unwrap();
            assert_eq!(bindings.document().camera_follow.as_deref(), Some(valid));
            bindings.save().unwrap();
            assert_eq!(
                Bindings::open(path.clone())
                    .unwrap()
                    .document()
                    .camera_follow
                    .as_deref(),
                Some(valid)
            );
        }
        document["camera_follow"] = serde_json::Value::Null;
        assert_eq!(
            open_json(&path, &document)
                .unwrap()
                .document()
                .camera_follow,
            None
        );
    }

    #[test]
    fn follow_and_sprite_edits_share_history_and_noops_preserve_redo() {
        let dir = tempfile::tempdir().unwrap();
        let mut bindings = Bindings::create(
            dir.path().join("view.json"),
            "scene.yaml".into(),
            ".".into(),
        )
        .unwrap();
        assert_eq!(bindings.document().version, 2);
        let empty = bindings.document().clone();
        let guid = orr_reflect::Guid::from_u32(1);
        bindings.set_camera_follow(Some(guid.clone())).unwrap();
        let follow_only = bindings.document().clone();
        bindings
            .assign(std::slice::from_ref(&guid), Some(binding(Source::Region(1))))
            .unwrap();
        let both = bindings.document().clone();
        bindings.set_camera_follow(None).unwrap();
        assert_eq!(bindings.document().camera_follow, None);
        assert_eq!(bindings.document().bindings.len(), 1);
        bindings.undo();
        assert_eq!(bindings.document(), &both);
        bindings.undo();
        assert_eq!(bindings.document(), &follow_only);
        let redo = bindings.redo.clone();
        bindings.set_camera_follow(Some(guid.clone())).unwrap();
        bindings.assign(&[guid], None).unwrap();
        assert_eq!(bindings.redo, redo);
        bindings.undo();
        assert_eq!(bindings.document(), &empty);
        bindings.redo();
        assert_eq!(bindings.document(), &follow_only);
        bindings.redo();
        assert_eq!(bindings.document(), &both);
        bindings
            .set_camera_follow(Some(orr_reflect::Guid::from_u32(2)))
            .unwrap();
        assert!(bindings.redo.is_empty());
    }

    #[test]
    fn shared_history_is_bounded_across_both_edit_types() {
        let dir = tempfile::tempdir().unwrap();
        let mut bindings = Bindings::create(
            dir.path().join("view.json"),
            "scene.yaml".into(),
            ".".into(),
        )
        .unwrap();
        let guid = orr_reflect::Guid::from_u32(1);
        let mut retained_start = bindings.document().clone();
        for i in 0..MAX_HISTORY + 2 {
            if i % 2 == 0 {
                bindings
                    .set_camera_follow(Some(orr_reflect::Guid::from_u32(i as u32)))
                    .unwrap();
            } else {
                bindings
                    .assign(
                        std::slice::from_ref(&guid),
                        Some(binding(Source::Region(i as u32))),
                    )
                    .unwrap();
            }
            if i == 1 {
                retained_start = bindings.document().clone();
            }
        }
        assert_eq!(bindings.undo.len(), MAX_HISTORY);
        let latest = bindings.document().clone();
        for _ in 0..MAX_HISTORY + 2 {
            bindings.undo();
        }
        assert_eq!(bindings.document(), &retained_start);
        assert_eq!(bindings.redo.len(), MAX_HISTORY);
        for _ in 0..MAX_HISTORY + 2 {
            bindings.redo();
        }
        assert_eq!(bindings.document(), &latest);
        assert_eq!(bindings.undo.len(), MAX_HISTORY);
    }

    #[test]
    fn invalid_locomotion_names_preserve_document_version_and_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.sprites.json");
        let mut bindings = open_json(&path, &legacy_json()).unwrap();
        bindings
            .set_camera_follow(Some(orr_reflect::Guid::from_u32(1)))
            .unwrap();
        bindings.undo();
        let before = bindings.document().clone();
        let redo = bindings.redo.clone();
        for invalid in [
            String::new(),
            " \t".into(),
            "x".repeat(orr_sprite::MAX_CLIP_ID_BYTES + 1),
        ] {
            for source in [
                Source::Locomotion {
                    idle: invalid.clone(),
                    walk: "walk".into(),
                },
                Source::Locomotion {
                    idle: "idle".into(),
                    walk: invalid.clone(),
                },
            ] {
                assert!(bindings
                    .assign(&[orr_reflect::Guid::from_u32(1)], Some(binding(source)))
                    .is_err());
                assert_eq!(bindings.document(), &before);
                assert!(bindings.undo.is_empty());
                assert_eq!(bindings.redo, redo);
                assert!(!bindings.dirty());
            }
        }
    }

    #[test]
    fn failed_follow_save_preserves_file_saved_document_and_shared_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("view.json");
        let mut bindings = Bindings::create(path.clone(), "scene.yaml".into(), ".".into()).unwrap();
        bindings.save().unwrap();
        let original = std::fs::read(&path).unwrap();
        bindings
            .set_camera_follow(Some(orr_reflect::Guid::from_u32(1)))
            .unwrap();
        let mut oversized = binding(Source::Locomotion {
            idle: "idle".into(),
            walk: "walk".into(),
        });
        oversized.package = "x".repeat(MAX_BYTES as usize);
        bindings
            .assign(&[orr_reflect::Guid::from_u32(1)], Some(oversized))
            .unwrap();
        bindings.set_camera_follow(None).unwrap();
        bindings.undo();
        let before = bindings.document().clone();
        let saved = bindings.saved.clone();
        let undo = bindings.undo.clone();
        let redo = bindings.redo.clone();
        assert!(bindings.save().unwrap_err().contains("byte limit"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(bindings.document(), &before);
        assert_eq!(bindings.saved, saved);
        assert_eq!(bindings.undo, undo);
        assert_eq!(bindings.redo, redo);
        assert!(bindings.dirty());
        bindings.undo();
        assert!(bindings.document().bindings.is_empty());
        assert_eq!(
            bindings.document().camera_follow.as_deref(),
            Some("e_00000001")
        );
        bindings.undo();
        assert!(!bindings.dirty());
        bindings.redo();
        bindings.redo();
        assert_eq!(bindings.document(), &before);
    }

    #[test]
    fn transaction_undo_save_reopen_and_scene_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let scene = dir.path().join("arena.scene.yaml");
        std::fs::write(&scene, b"original scene bytes").unwrap();
        let path = dir.path().join("arena.sprites.json");
        let mut bindings =
            Bindings::create(path.clone(), "arena.scene.yaml".into(), ".".into()).unwrap();
        let ids = [orr_reflect::Guid::parse("e_00000001").unwrap()];
        let binding = Binding {
            package: "demo".into(),
            document: "hero.json".into(),
            source: Source::Region(1),
            units_per_pixel: 0.1,
        };
        bindings.assign(&ids, Some(binding.clone())).unwrap();
        bindings.undo();
        assert!(bindings.document().bindings.is_empty());
        bindings.redo();
        assert_eq!(bindings.document().bindings[&ids[0].to_string()], binding);
        bindings.save().unwrap();
        assert!(!bindings.dirty());
        let reopened = Bindings::open(path).unwrap();
        assert_eq!(reopened.document(), bindings.document());
        assert!(reopened.matches_scene(scene.to_str().unwrap()));
        assert_eq!(std::fs::read(scene).unwrap(), b"original scene bytes");
    }
    #[test]
    fn invalid_transaction_is_atomic_and_absolute_paths_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Bindings::create(dir.path().join("x"), "/absolute".into(), ".".into()).is_err());
        let mut bindings =
            Bindings::create(dir.path().join("x"), "scene".into(), ".".into()).unwrap();
        let before = bindings.document().clone();
        let ids = [orr_reflect::Guid::parse("e_00000001").unwrap()];
        let invalid = Binding {
            package: "x".into(),
            document: "x.json".into(),
            source: Source::Region(1),
            units_per_pixel: f32::NAN,
        };
        assert!(bindings.assign(&ids, Some(invalid)).is_err());
        assert_eq!(bindings.document(), &before);
    }
    #[test]
    fn project_parent_path_works_without_bypassing_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let scenes = dir.path().join("scenes");
        std::fs::create_dir(&scenes).unwrap();
        assert_eq!(resolve_project(&scenes, "..").unwrap(), dir.path());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&scenes, dir.path().join("alias")).unwrap();
            assert!(resolve_project(dir.path(), "alias/..").is_err());
        }
    }
    #[test]
    fn project_resolution_accepts_canonical_bases_and_portable_parent_paths() {
        let dir = tempfile::tempdir().unwrap();
        // Windows canonicalize returns a verbatim path; joining a portable
        // relative string onto it before parsing changes its traversal semantics.
        let root = dir.path().canonicalize().unwrap();
        let sidecars = root.join("sidecars");
        let project = root.join("project");
        std::fs::create_dir(&sidecars).unwrap();
        std::fs::create_dir_all(project.join("nested")).unwrap();
        assert_eq!(resolve_project(&root, ".").unwrap(), root);
        assert_eq!(resolve_project(&sidecars, "../project").unwrap(), project);
        assert_eq!(
            resolve_project(&sidecars, "../project/nested").unwrap(),
            project.join("nested")
        );
    }
    #[test]
    fn project_resolution_checks_ancestors_before_parent_removal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("file"), b"not a directory").unwrap();
        for relative in ["missing/..", "file/..", "missing/../.", "file/../."] {
            assert!(resolve_project(&root, relative).is_err(), "{relative}");
        }
        let volume_root = root.ancestors().last().unwrap();
        assert!(resolve_project(volume_root, "..").is_err());
        assert!(resolve_project(volume_root, "../.").is_err());
        // Keep unchecked ancestors in the base too, without PathBuf::push
        // normalizing them away on Windows verbatim paths.
        for ancestor in ["missing", "file"] {
            let mut base = root.join(ancestor).into_os_string();
            base.push(format!("{0}..", std::path::MAIN_SEPARATOR));
            assert!(resolve_project(Path::new(&base), ".").is_err());
        }
    }
    #[cfg(windows)]
    #[test]
    fn project_resolution_checks_prefix_only_unc_roots() {
        use std::path::Component;
        // The NUL makes metadata fail locally, without contacting a UNC server.
        // Unlike a drive prefix followed by RootDir, this prefix is the whole
        // root and must not be skipped merely because it is a Prefix component.
        let root = Path::new("\\\\?\\UNC\\invalid\\share\0");
        let mut components = root.components();
        assert!(matches!(components.next(), Some(Component::Prefix(_))));
        assert_eq!(components.next(), None);
        assert!(resolve_project(root, ".").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn project_resolution_rejects_symlink_ancestors_even_when_removed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let project = root.join("project");
        std::fs::create_dir(&project).unwrap();
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        for relative in ["alias", "alias/..", "alias/../project"] {
            assert!(resolve_project(&root, relative).is_err(), "{relative}");
        }
        assert!(resolve_project(&alias, "../project").is_err());
        assert!(resolve_project(&alias.join(".."), ".").is_err());
    }
    #[test]
    fn decoder_rejects_apng_small_subframe_and_bad_png() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(1, 0).unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer.set_frame_dimension(1, 1).unwrap();
            writer.write_image_data(&[255, 0, 0, 255]).unwrap();
        }
        assert!(decode_atlas(&bytes, 2, 2)
            .unwrap_err()
            .contains("animated PNG"));
        assert!(decode_atlas(&bytes[..20], 2, 2).is_err());
        assert!(decode_atlas(b"not PNG", 2, 2).is_err());
    }
    #[test]
    fn scene_switch_and_save_as_retain_dirty_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.scene.yaml");
        let new = dir.path().join("new.scene.yaml");
        std::fs::write(&old, "old scene").unwrap();
        std::fs::write(&new, "new scene").unwrap();
        let mut bindings = Bindings::create(
            dir.path().join("view.json"),
            "old.scene.yaml".into(),
            ".".into(),
        )
        .unwrap();
        bindings.save().unwrap();
        bindings
            .assign(
                &[orr_reflect::Guid::from_u32(1)],
                Some(Binding {
                    package: "demo".into(),
                    document: "sprite.json".into(),
                    source: Source::Region(1),
                    units_per_pixel: 0.1,
                }),
            )
            .unwrap();
        let before = bindings.document().clone();
        assert!(!bindings.matches_scene(new.to_str().unwrap()));
        assert!(bindings.dirty());
        assert_eq!(bindings.document(), &before);
        assert!(bindings.matches_scene(old.to_str().unwrap()));
        bindings.undo();
        assert!(!bindings.dirty());
        bindings.redo();
        assert!(bindings.dirty());
    }
    #[test]
    fn failed_save_preserves_destination_and_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("view.json");
        let mut bindings = Bindings::create(path.clone(), "scene.yaml".into(), ".".into()).unwrap();
        bindings.save().unwrap();
        let original = std::fs::read(&path).unwrap();
        bindings
            .assign(
                &[orr_reflect::Guid::from_u32(1)],
                Some(Binding {
                    package: "demo".into(),
                    document: "sprite.json".into(),
                    source: Source::Region(1),
                    units_per_pixel: 0.1,
                }),
            )
            .unwrap();
        // A non-replaceable destination gives a deterministic rename failure,
        // including root-run CI where chmod cannot simulate write denial.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("existing-content"), &original).unwrap();
        let before = bindings.document().clone();
        assert!(bindings.save().is_err());
        assert!(bindings.dirty());
        assert_eq!(bindings.document(), &before);
        assert_eq!(
            std::fs::read(path.join("existing-content")).unwrap(),
            original
        );
        bindings.undo();
        assert!(bindings.document().bindings.is_empty());
        bindings.redo();
        assert_eq!(bindings.document(), &before);
    }
}
