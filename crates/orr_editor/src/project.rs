//! Saved Arena project admission. This is a bounded, non-adversarial file-open
//! boundary, not a sandbox against concurrent hostile filesystem replacement.
//! All scene bytes and sprite assets are checked before any host or texture is
//! created. Package installation and lock authority stay in `orr_package`.
use crate::{
    app::EditorApp,
    backend::HostSpec,
    editor::Editor,
    game::EditorGame,
    sprite_bindings::{self, Asset, Bindings},
    sprite_panel::SpritePanel,
};
use orr_reflect::{Guid, Scene, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

// Match the ordinary ERP message bound and the editor's complete-row ceiling.
const MAX_SCENE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SCENE_ENTITIES: usize = 20_000;

/// Opaque, already admitted Arena scene. Only project preflight constructs it.
#[derive(Clone, Debug)]
pub struct PreparedArenaScene {
    pub(crate) path: PathBuf,
    pub(crate) text: Arc<str>,
}

type PreparedSprites = (Bindings, BTreeMap<(String, String), Asset>);

/// An entirely validated startup candidate, with no live host or GPU textures.
/// Connect an `Editor` using [`Self::host_spec`], then consume this candidate
/// with [`Self::into_app`] to restore its optional sprite presentation.
pub struct PreparedProject {
    root: PathBuf,
    scene: PreparedArenaScene,
    sprites: Option<PreparedSprites>,
}
impl PreparedProject {
    /// Open a schema-2 Arena project and verify the whole active package lock,
    /// including installed packages not referenced by its optional sidecar.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let project =
            orr_package::Project::open(root.as_ref(), sprite_bindings::compiled_runtime())
                .map_err(|e| format!("project: {e}"))?;
        let lock = project
            .verify()
            .map_err(|e| format!("package verification: {e}"))?;
        let manifest = project
            .manifest()
            .ok_or("saved project requires orr.project.json schema 2 with an entry")?;
        let entry = manifest
            .entry
            .as_ref()
            .ok_or("saved project requires schema 2 with an entry; schema 1 is metadata-only")?;
        if manifest.schema != 2 || entry.game != orr_package::ProjectGame::Arena {
            return Err("saved project entry must be schema 2 and game arena".into());
        }
        let root = project.root().to_path_buf();
        let scene_path = entry_file(&root, &entry.scene)?;
        let bytes = read_regular(&scene_path, MAX_SCENE_BYTES)?;
        let text = String::from_utf8(bytes).map_err(|e| format!("scene must be UTF-8: {e}"))?;
        let scene = Scene::parse(&text, &EditorGame::Arena.types())
            .map_err(|e| format!("Arena scene {}: {e}", scene_path.display()))?;
        if scene.entities.len() > MAX_SCENE_ENTITIES {
            return Err(format!("Arena scene exceeds {MAX_SCENE_ENTITIES} entities"));
        }
        let sprites = if let Some(relative) = &entry.sprites {
            let path = entry_file(&root, relative)?;
            let bytes = read_regular(&path, sprite_bindings::MAX_BYTES)?;
            let bindings =
                Bindings::from_bytes(path, &bytes).map_err(|e| format!("sprite sidecar: {e}"))?;
            let document = bindings.document();
            let sidecar_project = resolve_inside(&root, bindings.base(), &document.project, true)?;
            if sidecar_project != root {
                return Err("sprite sidecar project must resolve to the same project root".into());
            }
            let sidecar_scene = resolve_inside(&root, bindings.base(), &document.scene, false)?;
            if sidecar_scene != scene_path {
                return Err("sprite sidecar scene must resolve to the project entry scene".into());
            }
            for (guid, binding) in &document.bindings {
                validate_target(&scene, guid, "sprite binding")?;
                let package = lock.packages.get(&binding.package).ok_or_else(|| {
                    format!("sprite binding {guid}: missing package {}", binding.package)
                })?;
                if !package.manifest.capabilities.contains("sprite") {
                    return Err(format!(
                        "sprite package {} must declare the sprite capability",
                        binding.package
                    ));
                }
            }
            if let Some(guid) = &document.camera_follow {
                validate_target(&scene, guid, "camera follow")?;
            }
            let assets = SpritePanel::load_project_assets(&bindings, &project)?;
            Some((bindings, assets))
        } else {
            None
        };
        Ok(Self {
            root,
            scene: PreparedArenaScene {
                path: scene_path,
                text: text.into(),
            },
            sprites,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn scene_path(&self) -> &Path {
        &self.scene.path
    }

    /// Uses the owned admitted scene bytes, never an unchecked startup re-read.
    pub fn host_spec(&self) -> HostSpec {
        HostSpec::PreparedArena {
            scene: self.scene.clone(),
            listen: None,
            debug_hooks: false,
        }
    }

    /// Restore decoded assets only after the caller connected the admitted host.
    /// There are no fallible reads or partial texture-cache changes in this step.
    pub fn into_app(
        self,
        editor: Editor,
        render_state: Option<egui_wgpu::RenderState>,
        ctx: &egui::Context,
    ) -> EditorApp {
        let mut app = EditorApp::new(editor, render_state);
        if let Some((bindings, assets)) = self.sprites {
            app.sprites.install_prepared(ctx, bindings, assets);
        }
        app
    }
}

fn validate_target(scene: &Scene, guid: &str, role: &str) -> Result<(), String> {
    let parsed = Guid::parse(guid).map_err(|e| format!("{role} GUID {guid}: {e}"))?;
    let entity = scene
        .entities
        .get(&parsed)
        .ok_or_else(|| format!("{role} GUID {guid} is missing from the Arena scene"))?;
    let position = entity
        .components
        .iter()
        .find(|(name, _)| name == "Position");
    let has_position =
        position.is_some_and(|(_, value)| matches!(value.field("pos"), Some(Value::Vec2(_))));
    // The compiled Arena extractor emits bodies only for players and bullets.
    // Position alone would silently disappear from both sprite and follow views.
    let drawable = entity
        .components
        .iter()
        .any(|(name, _)| name == "PlayerTag" || name == "Bullet");
    if !has_position || !drawable {
        return Err(format!(
            "{role} GUID {guid} needs a usable Arena Position and PlayerTag or Bullet"
        ));
    }
    Ok(())
}

fn entry_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.split('/').any(|part| part == "." || part == "..") {
        return Err("project entry paths must be root-relative without traversal".into());
    }
    resolve_inside(root, root, relative, false)
}

/// Sidecar paths retain their relative-to-sidecar semantics, including `..`
/// within the project. Check each directory before parent removal, and reject
/// even temporary escapes, symlinks, special files, and different final roots.
fn resolve_inside(
    root: &Path,
    base: &Path,
    relative: &str,
    directory: bool,
) -> Result<PathBuf, String> {
    if relative.is_empty()
        || relative.contains('\\')
        || relative.contains(':')
        || Path::new(relative).is_absolute()
        || !base.starts_with(root)
    {
        return Err("project path must be an explicit relative path inside the project".into());
    }
    let parts: Vec<_> = relative.split('/').collect();
    let mut path = base.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        match *part {
            "" => return Err("empty project path component".into()),
            "." => {}
            ".." => {
                if path == root || !path.pop() {
                    return Err("sidecar path escapes the project root".into());
                }
            }
            part => path.push(part),
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|e| format!("project path {}: {e}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("symlink rejected: {}", path.display()));
        }
        let needs_directory = directory || i + 1 < parts.len();
        if (needs_directory && !metadata.is_dir()) || (!needs_directory && !metadata.is_file()) {
            return Err(format!(
                "project path requires a regular {}: {}",
                if needs_directory { "directory" } else { "file" },
                path.display()
            ));
        }
    }
    let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) {
        return Err("project path escapes the canonical project root".into());
    }
    Ok(canonical)
}

fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    // Do not open a FIFO/device merely to inspect its descriptor afterward.
    let before = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if before.file_type().is_symlink() || !before.is_file() || before.len() > limit {
        return Err(format!(
            "{} must be a regular file within {limit} bytes",
            path.display()
        ));
    }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(format!(
            "{} must be a regular file within {limit} bytes",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds {limit} bytes", path.display()));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value as J, json};

    const SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    Position: { pos: [0,0] }\n    PlayerTag: { slot: 0 }\n";

    fn write_json(path: impl AsRef<Path>, value: &J) {
        fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    }
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("arena.scene.yaml"), SCENE).unwrap();
        write_json(
            root.join("orr.project.json"),
            &json!({
                "schema":2,"engine":"*",
                "entry":{"game":"arena","scene":"arena.scene.yaml"}
            }),
        );
        dir
    }
    fn change_json(path: impl AsRef<Path>, edit: impl FnOnce(&mut J)) {
        let mut value = serde_json::from_slice(&fs::read(path.as_ref()).unwrap()).unwrap();
        edit(&mut value);
        write_json(path, &value);
    }
    fn add_sidecar(root: &Path, binding: bool) {
        change_json(root.join("orr.project.json"), |m| {
            m["entry"]["sprites"] = json!("view.json")
        });
        let mut document =
            json!({"version":2,"scene":"arena.scene.yaml","project":".","bindings":{}});
        if binding {
            document["bindings"]["e_00000001"] = json!({
                "package":"art","document":"sprite.json",
                "source":{"Locomotion":{"idle":"idle","walk":"walk"}},"units_per_pixel":0.1
            });
            document["camera_follow"] = json!("e_00000001");
        }
        write_json(root.join("view.json"), &document);
    }
    fn sprite() -> J {
        json!({"format":"orr_sprite","version":1,
        "atlas":{"image":"atlas.png","width":1,"height":1},
        "regions":[{"id":1,"x":0,"y":0,"width":1,"height":1}],
        "clips":[
            {"id":"idle","mode":"loop","frames":[{"region":1,"duration_ms":100}]},
            {"id":"walk","mode":"loop","frames":[{"region":1,"duration_ms":50}]}
        ]})
    }
    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 255])
                .unwrap();
        }
        bytes
    }
    fn install(root: &Path, capabilities: &[&str], document: J, image: &[u8]) -> PathBuf {
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        write_json(
            source.join("orr.package.json"),
            &json!({
                "schema":1,"name":"art","version":"1.0.0","engine":"*",
                "capabilities":capabilities,"files":["sprite.json","atlas.png"]
            }),
        );
        write_json(source.join("sprite.json"), &document);
        fs::write(source.join("atlas.png"), image).unwrap();
        let project = orr_package::Project::open_for_install(
            root,
            orr_package::Runtime::content_only().engine_version,
        )
        .unwrap();
        let lock = project.install(&[source]).unwrap();
        root.join(".orr/packages/objects")
            .join(&lock.packages["art"].digest)
    }
    fn error(root: &Path) -> String {
        match PreparedProject::open(root) {
            Ok(_) => panic!("invalid project was admitted"),
            Err(error) => error,
        }
    }

    #[test]
    fn metadata_only_or_missing_manifest_is_not_an_entry() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        write_json(
            root.join("orr.project.json"),
            &json!({"schema":1,"engine":"*"}),
        );
        assert!(error(&root).contains("metadata-only"));
        fs::remove_file(root.join("orr.project.json")).unwrap();
        assert!(error(&root).contains("requires orr.project.json"));
    }

    #[test]
    fn scene_only_and_empty_sidecar_are_valid_clean_candidates() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        let prepared = PreparedProject::open(&root).unwrap();
        assert_eq!(prepared.root(), root);
        assert_eq!(prepared.scene_path(), root.join("arena.scene.yaml"));
        assert!(prepared.sprites.is_none());
        add_sidecar(&root, false);
        let prepared = PreparedProject::open(&root).unwrap();
        let (bindings, assets) = prepared.sprites.unwrap();
        assert!(!bindings.dirty());
        assert!(assets.is_empty());
    }

    #[test]
    fn complete_candidate_owns_checked_scene_binding_and_decoded_pixels() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        add_sidecar(&root, true);
        let object = install(&root, &["sprite"], sprite(), &png());
        let prepared = PreparedProject::open(&root).unwrap();
        fs::write(root.join("arena.scene.yaml"), b"unchecked replacement").unwrap();
        fs::write(root.join("view.json"), b"unchecked replacement").unwrap();
        fs::remove_dir_all(object).unwrap();
        assert_eq!(&*prepared.scene.text, SCENE);
        let (bindings, assets) = prepared.sprites.unwrap();
        assert_eq!(
            bindings.document().camera_follow.as_deref(),
            Some("e_00000001")
        );
        assert_eq!(
            assets[&("art".into(), "sprite.json".into())].rgba,
            [255, 0, 0, 255]
        );
    }

    #[test]
    fn whole_active_lock_is_verified_without_sprite_references() {
        for sidecar in [false, true] {
            let dir = fixture();
            let root = dir.path().canonicalize().unwrap();
            if sidecar {
                add_sidecar(&root, false);
            }
            let object = install(&root, &["sprite"], sprite(), &png());
            fs::write(object.join("atlas.png"), b"tampered unreferenced bytes").unwrap();
            assert!(error(&root).contains("package verification"));
        }
    }

    #[test]
    fn unknown_runtime_capability_rejects_even_without_a_sidecar() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        install(&root, &["not-compiled"], sprite(), &png());
        assert!(error(&root).contains("compiled capability"));
    }

    #[test]
    fn inventory_includes_exactly_the_compiled_content_features() {
        let runtime = sprite_bindings::compiled_runtime();
        let mut expected = std::collections::BTreeSet::from(["sprite".to_owned()]);
        if cfg!(feature = "models") {
            expected.insert("models".into());
        }
        if cfg!(feature = "animated-models") {
            expected.insert("animation".into());
        }
        if cfg!(feature = "irradiance-probes") {
            expected.insert("irradiance-probes".into());
        }
        if cfg!(feature = "terrain") {
            expected.insert("terrain_v1".into());
        }
        assert_eq!(runtime.capabilities, expected);
    }

    #[test]
    fn referenced_package_must_declare_sprite_capability() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        add_sidecar(&root, true);
        install(&root, &[], sprite(), &png());
        assert!(error(&root).contains("must declare the sprite capability"));
    }

    #[test]
    fn scene_parse_and_size_and_entity_bounds_precede_host_admission() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        fs::write(
            root.join("arena.scene.yaml"),
            SCENE.replace("Position", "orr_physics::Body"),
        )
        .unwrap();
        assert!(error(&root).contains("Arena scene"));
        let path = root.join("arena.scene.yaml");
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_SCENE_BYTES + 1)
            .unwrap();
        assert!(error(&root).contains("bytes"));
        let mut text = String::from("schema: orr.scene/1\nsingletons: {}\nentities:\n");
        for i in 1..=MAX_SCENE_ENTITIES + 1 {
            text.push_str(&format!("  e_{i:08x}: {{}}\n"));
        }
        fs::write(path, text).unwrap();
        assert!(error(&root).contains("20000 entities"));
    }

    #[test]
    fn missing_or_unusable_bound_and_follow_targets_are_rejected() {
        for role in ["bindings", "camera_follow"] {
            for target in ["e_00000002", "invalid-guid"] {
                let dir = fixture();
                let root = dir.path().canonicalize().unwrap();
                add_sidecar(&root, false);
                change_json(root.join("view.json"), |d| {
                    if role == "bindings" {
                        d[role][target] = json!({"package":"art","document":"sprite.json","source":{"Region":1},"units_per_pixel":0.1});
                    } else {
                        d[role] = json!(target);
                    }
                });
                assert!(error(&root).contains("GUID"));
            }
        }
        for remove in [
            "    Position: { pos: [0,0] }\n",
            "    PlayerTag: { slot: 0 }\n",
        ] {
            let dir = fixture();
            let root = dir.path().canonicalize().unwrap();
            add_sidecar(&root, false);
            change_json(root.join("view.json"), |d| {
                d["camera_follow"] = json!("e_00000001")
            });
            fs::write(root.join("arena.scene.yaml"), SCENE.replace(remove, "")).unwrap();
            assert!(error(&root).contains("usable Arena Position"));
        }
    }

    #[test]
    fn nested_sidecar_resolves_only_the_same_root_and_scene() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        add_sidecar(&root, false);
        fs::create_dir(root.join("views")).unwrap();
        fs::rename(root.join("view.json"), root.join("views/view.json")).unwrap();
        change_json(root.join("orr.project.json"), |m| {
            m["entry"]["sprites"] = json!("views/view.json")
        });
        let path = root.join("views/view.json");
        change_json(&path, |d| {
            d["project"] = json!("..");
            d["scene"] = json!("../arena.scene.yaml");
        });
        PreparedProject::open(&root).unwrap();
        change_json(&path, |d| d["project"] = json!("."));
        assert!(error(&root).contains("same project root"));
        fs::write(root.join("other.scene.yaml"), SCENE).unwrap();
        change_json(&path, |d| {
            d["project"] = json!("..");
            d["scene"] = json!("../other.scene.yaml");
        });
        assert!(error(&root).contains("entry scene"));
        change_json(&path, |d| d["scene"] = json!("../../escape"));
        assert!(error(&root).contains("escapes"));
    }

    #[test]
    fn invalid_regions_clips_documents_and_atlases_are_rejected() {
        for kind in ["region", "walk", "atlas", "document", "png"] {
            let dir = fixture();
            let root = dir.path().canonicalize().unwrap();
            add_sidecar(&root, true);
            let mut document = sprite();
            match kind {
                "region" => change_json(root.join("view.json"), |d| {
                    d["bindings"]["e_00000001"]["source"] = json!({"Region":999})
                }),
                "walk" => change_json(root.join("view.json"), |d| {
                    d["bindings"]["e_00000001"]["source"]["Locomotion"]["walk"] = json!("missing")
                }),
                "atlas" => document["atlas"]["width"] = json!(2),
                "document" => document["regions"][0]["width"] = json!(99),
                _ => {}
            }
            let bytes = if kind == "png" {
                b"invalid PNG".to_vec()
            } else {
                png()
            };
            install(&root, &["sprite"], document, &bytes);
            let _ = error(&root);
        }
    }

    #[test]
    fn missing_files_special_files_and_sidecar_size_are_rejected() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        add_sidecar(&root, false);
        fs::remove_file(root.join("view.json")).unwrap();
        assert!(error(&root).contains("view.json"));
        fs::create_dir(root.join("view.json")).unwrap();
        assert!(error(&root).contains("regular file"));
        fs::remove_dir(root.join("view.json")).unwrap();
        fs::File::create(root.join("view.json"))
            .unwrap()
            .set_len(sprite_bindings::MAX_BYTES + 1)
            .unwrap();
        assert!(error(&root).contains("bytes"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_ancestors_and_parent_normalization_cannot_hide_aliases() {
        use std::os::unix::fs::symlink;
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        add_sidecar(&root, false);
        fs::create_dir(root.join("real")).unwrap();
        symlink(root.join("real"), root.join("alias")).unwrap();
        change_json(root.join("view.json"), |d| d["project"] = json!("alias/.."));
        assert!(error(&root).contains("symlink"));
        change_json(root.join("view.json"), |d| d["project"] = json!("."));
        fs::rename(root.join("view.json"), root.join("real/view.json")).unwrap();
        change_json(root.join("orr.project.json"), |m| {
            m["entry"]["sprites"] = json!("alias/view.json")
        });
        assert!(error(&root).contains("symlink"));
        change_json(root.join("orr.project.json"), |m| {
            m["entry"].as_object_mut().unwrap().remove("sprites");
        });
        fs::rename(
            root.join("arena.scene.yaml"),
            root.join("real/arena.scene.yaml"),
        )
        .unwrap();
        symlink(
            root.join("real/arena.scene.yaml"),
            root.join("arena.scene.yaml"),
        )
        .unwrap();
        assert!(error(&root).contains("symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn fifo_inputs_reject_before_open_with_timeout_guard() {
        use std::{
            process::Command,
            time::{Duration, Instant},
        };
        const ROOT: &str = "ORR_EDITOR_PROJECT_FIFO_ROOT";
        if let Some(root) = std::env::var_os(ROOT) {
            let root = PathBuf::from(root);
            assert!(error(&root).contains("regular file"));
            return;
        }
        for file in ["arena.scene.yaml", "view.json"] {
            let dir = fixture();
            let root = dir.path().canonicalize().unwrap();
            add_sidecar(&root, false);
            fs::remove_file(root.join(file)).unwrap();
            assert!(
                Command::new("mkfifo")
                    .arg(root.join(file))
                    .status()
                    .unwrap()
                    .success()
            );
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "project::tests::fifo_inputs_reject_before_open_with_timeout_guard",
                    "--nocapture",
                ])
                .env(ROOT, &root)
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("FIFO admission blocked");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    #[test]
    fn first_host_uses_admitted_bytes_then_restart_uses_saved_local_scene() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        let prepared = PreparedProject::open(&root).unwrap();
        fs::write(
            root.join("arena.scene.yaml"),
            SCENE.replace("[0,0]", "[9,0]"),
        )
        .unwrap();
        let mut backend = crate::backend::Backend::connect(&prepared.host_spec()).unwrap();
        let position = json!({"entity":"e_00000001","component":"Position","path":"pos"});
        assert_eq!(
            backend.erp.call("world.get", position.clone()).unwrap()["value"],
            json!([0, 0])
        );
        assert!(
            matches!(&backend.spec, HostSpec::LocalGame { game: EditorGame::Arena, scene, .. } if scene == prepared.scene_path())
        );
        // Save through ERP, which does not use the editor UI's save hook.
        backend
            .erp
            .call(
                "world.patch",
                json!({"entity":"e_00000001","component":"Position","path":"pos","value":[7,0]}),
            )
            .unwrap();
        backend
            .erp
            .call("scene.save", json!({"write":true}))
            .unwrap();
        let restart = backend.spec.clone();
        drop(backend);
        let mut backend = crate::backend::Backend::connect(&restart).unwrap();
        assert_eq!(
            backend.erp.call("world.get", position).unwrap()["value"],
            json!([7, 0])
        );
    }

    #[test]
    fn legacy_v1_sprite_sidecar_is_admitted_without_migration() {
        let dir = fixture();
        let root = dir.path().canonicalize().unwrap();
        add_sidecar(&root, true);
        install(&root, &["sprite"], sprite(), &png());
        for source in [json!({"Region":1}), json!({"Clip":"idle"})] {
            change_json(root.join("view.json"), |d| {
                d["version"] = json!(1);
                d.as_object_mut().unwrap().remove("camera_follow");
                d["bindings"]["e_00000001"]["source"] = source;
            });
            let prepared = PreparedProject::open(&root).unwrap();
            let (bindings, assets) = prepared.sprites.unwrap();
            assert_eq!(bindings.document().version, 1);
            assert!(!bindings.dirty());
            assert_eq!(assets.len(), 1);
        }
    }

    #[test]
    fn missing_and_malformed_locks_and_missing_atlas_fail_admission() {
        for missing in ["lock", "malformed-lock", "atlas"] {
            let dir = fixture();
            let root = dir.path().canonicalize().unwrap();
            add_sidecar(&root, true);
            let object = install(&root, &["sprite"], sprite(), &png());
            match missing {
                "lock" => fs::remove_file(root.join("orr.packages.lock.json")).unwrap(),
                "malformed-lock" => {
                    fs::write(root.join("orr.packages.lock.json"), b"{invalid lock}").unwrap()
                }
                _ => fs::remove_file(object.join("atlas.png")).unwrap(),
            }
            let message = error(&root);
            if missing == "lock" {
                assert!(message.contains("missing package"), "{message}");
            } else {
                assert!(message.contains("package verification"), "{message}");
            }
        }
    }
}
