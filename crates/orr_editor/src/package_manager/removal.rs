//! Checked deactivation of a direct selection, never deletion of content files.
use super::{checked_directory, ensure_regular, room_runtime};
use orr_model_bindings::model_bindings::Document;
use orr_package::{Lock, PackagePreview, Project, ProjectManifest};
use orr_sample::room_project::{self, PreparedProject};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// An observed candidate and the exact saved Room content admitted with it.
/// Applying still verifies the active lock and re-admits Room under its writer.
/// The editor must separately guard its current project and unsaved host edits.
pub struct RemovalPlan {
    pub root: PathBuf,
    pub name: String,
    pub preview: PackagePreview,
    snapshot: Snapshot,
}

/// Same-session evidence of one successfully completed, re-admitted removal.
/// This opaque receipt cannot choose a different revision or enumerate the cache.
/// It is deliberately not Clone; restoring consumes it even when the attempt fails.
pub struct RemovalReceipt {
    root: PathBuf,
    name: String,
    previous: Lock,
    removed: Lock,
    snapshot: Snapshot,
}

impl RemovalReceipt {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    root_identity: RootIdentity,
    files: BTreeMap<PathBuf, Vec<u8>>,
    frame: Vec<u8>,
    checksum: u64,
}

pub fn plan_remove(root: &Path, name: &str) -> Result<RemovalPlan, String> {
    let root = checked_directory(root, "Room project")?;
    let project = Project::open(&root, room_runtime()).map_err(|error| error.to_string())?;
    let preview = project
        .preview_remove(name)
        .map_err(|error| error.to_string())?;
    validate_removal(name, &preview.previous, &preview.candidate)?;
    let snapshot = admit_candidate(&root, &preview.previous, &preview.candidate)?;
    Ok(RemovalPlan {
        root,
        name: name.to_owned(),
        preview,
        snapshot,
    })
}

pub fn apply_remove(plan: &RemovalPlan) -> Result<Lock, String> {
    apply_remove_with_receipt(plan).map(|(lock, _)| lock)
}

/// Issue a restoration receipt only after removal and post-publication Room
/// re-admission both succeed. An uncertain/failed completion gives no receipt.
pub fn apply_remove_with_receipt(plan: &RemovalPlan) -> Result<(Lock, RemovalReceipt), String> {
    let root = checked_directory(&plan.root, "Room project")?;
    if root != plan.root {
        return Err("Room project path identity changed since removal planning".into());
    }
    validate_removal(&plan.name, &plan.preview.previous, &plan.preview.candidate)?;
    let project = Project::open(&root, room_runtime()).map_err(|error| error.to_string())?;
    let installed = project
        .remove_checked(
            &plan.name,
            &plan.preview.previous,
            &plan.preview.candidate,
            || {
                // This runs after core's complete active-object verification and
                // candidate comparison, while its existing writer guard is held.
                let current = admit_candidate(
                    &root,
                    &plan.preview.previous,
                    &plan.preview.candidate,
                )?;
                if current != plan.snapshot {
                    return Err(
                        "saved Room project, scene, model bindings or camera changed since removal planning; preview again"
                            .into(),
                    );
                }
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;

    let after = admit_candidate(&root, &installed, &installed).map_err(|error| {
        format!(
            "package selection was removed at {} but Room re-admission failed; the published lock is retained: {error}",
            root.display()
        )
    })?;
    if after != plan.snapshot {
        return Err(format!(
            "package selection was removed at {} but saved Room content changed after publication; the published lock is retained",
            root.display()
        ));
    }
    let receipt = RemovalReceipt {
        root,
        name: plan.name.clone(),
        previous: plan.preview.previous.clone(),
        removed: installed.clone(),
        snapshot: after,
    };
    Ok((installed, receipt))
}

/// Preview the exact inverse of this receipt, with no generic revision selection.
/// The UI must discard the receipt on project changes or other package mutations.
pub fn plan_restore(receipt: &RemovalReceipt) -> Result<PackagePreview, String> {
    let project = receipt_project(receipt)?;
    let preview = project
        .preview_restore_removed(&receipt.name, &receipt.removed, &receipt.previous)
        .map_err(|error| error.to_string())?;
    if preview.previous != receipt.removed || preview.candidate != receipt.previous {
        return Err("restoration preview differs from the successful removal receipt".into());
    }
    unchanged_receipt_room(receipt)?;
    Ok(preview)
}

/// Restore one exact successful removal. Consuming the receipt prevents a
/// failed/stale request from being silently retained or reused as a rollback.
pub fn apply_restore(receipt: RemovalReceipt) -> Result<Lock, String> {
    let project = receipt_project(&receipt)?;
    let restored = project
        .restore_removed_checked(&receipt.name, &receipt.removed, &receipt.previous, || {
            unchanged_receipt_room(&receipt)
        })
        .map_err(|error| error.to_string())?;
    let after = admit_candidate(&receipt.root, &restored, &restored).map_err(|error| {
        format!(
            "package selection was restored at {} but Room re-admission failed; the published lock is retained: {error}",
            receipt.root.display()
        )
    })?;
    if after != receipt.snapshot {
        return Err(format!(
            "package selection was restored at {} but saved Room content changed after publication; the published lock is retained",
            receipt.root.display()
        ));
    }
    Ok(restored)
}

fn receipt_project(receipt: &RemovalReceipt) -> Result<Project, String> {
    let root = checked_directory(&receipt.root, "Room project")?;
    if root != receipt.root || root_identity(&root)? != receipt.snapshot.root_identity {
        return Err(
            "Room project identity changed since package removal; receipt discarded".into(),
        );
    }
    Project::open(&root, room_runtime()).map_err(|error| error.to_string())
}

fn unchanged_receipt_room(receipt: &RemovalReceipt) -> Result<(), String> {
    let current = admit_candidate(&receipt.root, &receipt.removed, &receipt.previous)?;
    if current != receipt.snapshot {
        return Err(
            "saved Room project, scene, model bindings or camera changed since package removal; receipt discarded"
                .into(),
        );
    }
    Ok(())
}

fn validate_removal(name: &str, previous: &Lock, candidate: &Lock) -> Result<(), String> {
    if !previous.direct.contains_key(name) {
        return Err(format!("not a direct package: {name}"));
    }
    let mut direct = previous.direct.clone();
    direct.remove(name);
    if candidate.schema != previous.schema || candidate.direct != direct {
        return Err("removal preview must remove exactly the selected direct package".into());
    }
    for (name, package) in &candidate.packages {
        if previous.packages.get(name) != Some(package) {
            return Err(format!(
                "removal preview changes retained package identity: {name}"
            ));
        }
    }
    Ok(())
}

pub(super) fn admit_candidate(
    root: &Path,
    expected: &Lock,
    candidate: &Lock,
) -> Result<Snapshot, String> {
    let checked = checked_directory(root, "Room project")?;
    if checked != root {
        return Err("Room project path identity changed before package publication".into());
    }
    let root_identity = root_identity(root)?;
    let project = Project::open(root, room_runtime()).map_err(|error| error.to_string())?;
    if project.verify().map_err(|error| error.to_string())? != *expected {
        return Err("active Room package lock changed since removal planning".into());
    }
    let files = read_room_files(&project)?;
    // Closed Room admission rejects unknown/opaque scene or sidecar fields,
    // unresolved bindings, unsupported kinds and stale package/file identities.
    let prepared = PreparedProject::open(root)?;
    if prepared.root() != root
        || files.get(prepared.path()).map(Vec::as_slice) != Some(prepared.scene().text().as_bytes())
    {
        return Err("Room project or scene changed during removal admission".into());
    }
    let model_bytes = files
        .get(&prepared.models().path)
        .ok_or("Room model sidecar changed during removal admission")?;
    let document: Document = serde_json::from_slice(model_bytes)
        .map_err(|error| format!("Room model bindings: {error}"))?;
    if document != prepared.models().document {
        return Err("Room model bindings changed during removal admission".into());
    }
    if let Some(camera) = prepared.camera() {
        if files.get(&camera.path) != Some(&camera.bytes)
            || files.get(&camera.manifest_path) != Some(&camera.manifest_bytes)
        {
            return Err("Room camera changed during removal admission".into());
        }
    }
    for (guid, binding) in &prepared.models().document.bindings {
        let retained = candidate.packages.get(&binding.package).ok_or_else(|| {
            format!(
                "cannot remove package selection: model binding {guid} requires deactivated package {}",
                binding.package
            )
        })?;
        if expected.packages.get(&binding.package) != Some(retained) {
            return Err(format!(
                "removal changes the identity of model binding package {}",
                binding.package
            ));
        }
        let asset = prepared
            .models()
            .assets
            .get(&(binding.package.clone(), binding.asset.clone()))
            .ok_or("Room model binding is unresolved during removal admission")?;
        // Retained packages are exactly the verified immutable objects already
        // decoded by Room, including all same-package GLB/glTF resources. This
        // checks both static and animated bindings without publishing a trial lock.
        binding.validate(asset)?;
    }
    if root_identity != self::root_identity(root)?
        || files != read_room_files(&project)?
        || project.verify().map_err(|error| error.to_string())? != *expected
    {
        return Err("Room files or active package lock changed during removal admission".into());
    }
    Ok(Snapshot {
        root_identity,
        files,
        frame: prepared.scene().frame().to_bytes(),
        checksum: prepared.scene().frame().checksum(),
    })
}

fn read_room_files(project: &Project) -> Result<BTreeMap<PathBuf, Vec<u8>>, String> {
    let manifest_path = project.root().join("orr.project.json");
    let manifest_bytes = read_regular(&manifest_path, 1024 * 1024)?;
    let manifest: ProjectManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("Room project manifest: {error}"))?;
    if Some(&manifest) != project.manifest() {
        return Err("Room project manifest changed during removal admission".into());
    }
    let entry = manifest
        .entry
        .as_ref()
        .ok_or("Room project entry missing")?;
    let models = entry.models.as_ref().ok_or("Room model sidecar missing")?;
    let mut files = BTreeMap::from([(manifest_path, manifest_bytes)]);
    for (relative, limit) in [
        (entry.scene.as_str(), room_project::MAX_SCENE_BYTES),
        (models.as_str(), 1024 * 1024),
    ] {
        let path = project.root().join(relative);
        files.insert(path.clone(), read_regular(&path, limit)?);
    }
    if let Some(relative) = &entry.camera {
        let path = project.root().join(relative);
        files.insert(
            path.clone(),
            read_regular(&path, orr_sample::room_camera::MAX_BYTES as u64)?,
        );
    }
    Ok(files)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RootIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(not(unix))]
    created: std::time::SystemTime,
}

pub(super) fn root_identity(root: &Path) -> Result<RootIdentity, String> {
    let metadata = fs::symlink_metadata(root).map_err(|error| error.to_string())?;
    super::ensure_directory(&metadata, "Room project")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(RootIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Ok(RootIdentity {
            created: metadata
                .created()
                .map_err(|error| format!("Room directory identity unavailable: {error}"))?,
        })
    }
}

pub(super) fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let parent = path.parent().ok_or("Room file parent missing")?;
    if checked_directory(parent, "Room file parent")? != parent {
        return Err("Room file parent identity changed".into());
    }
    let before = fs::symlink_metadata(path)
        .map_err(|error| format!("Room file {}: {error}", path.display()))?;
    ensure_regular(&before, "Room file")?;
    if before.len() > limit {
        return Err("Room file exceeds its admission byte limit".into());
    }
    read_observed(path, limit, &before)
}

fn read_observed(path: &Path, limit: u64, before: &fs::Metadata) -> Result<Vec<u8>, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000 | 0x800); // O_NOFOLLOW | O_NONBLOCK
    }
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x100 | 0x4); // O_NOFOLLOW | O_NONBLOCK
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("Room file: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("Room file: {error}"))?;
    ensure_regular(&opened, "opened Room file")?;
    if opened.len() > limit || !same_file(before, &opened) {
        return Err("Room file changed before reading".into());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Room file: {error}"))?;
    let after = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let handle_after = file.metadata().map_err(|error| error.to_string())?;
    ensure_regular(&after, "Room file after reading")?;
    ensure_regular(&handle_after, "opened Room file after reading")?;
    if bytes.len() as u64 > limit
        || bytes.len() as u64 != opened.len()
        || !same_file(before, &after)
        || !same_file(&opened, &handle_after)
    {
        return Err("Room file changed while being read".into());
    }
    let parent = path.parent().ok_or("Room file parent missing")?;
    if checked_directory(parent, "Room file parent")? != parent {
        return Err("Room file parent changed while being read".into());
    }
    Ok(bytes)
}

fn same_file(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if first.dev() != second.dev()
            || first.ino() != second.ino()
            || first.ctime() != second.ctime()
            || first.ctime_nsec() != second.ctime_nsec()
        {
            return false;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if first.creation_time() != second.creation_time()
            || first.file_attributes() != second.file_attributes()
        {
            return false;
        }
    }
    first.len() == second.len()
        && matches!((first.modified(), second.modified()), (Ok(first), Ok(second)) if first == second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_model_bindings::model_bindings::{self, Binding, LocalTransform};
    use orr_package::{Manifest, Runtime};
    use orr_reflect::Scene;
    use orr_sample::room_game::{RoomActor, RoomConfig, RoomEscapeV1, PLAYER};
    use orr_sim::Simulation;
    use std::collections::BTreeSet;

    const USED: &str = "sample-imported-scene";

    struct Fixture {
        temp: tempfile::TempDir,
        root: PathBuf,
        source: PathBuf,
    }

    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                copy_tree(&entry.path(), &to.join(entry.file_name()));
            } else {
                assert!(kind.is_file());
                fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
    }

    fn inventory(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    visit(root, &entry.path(), files);
                } else {
                    files.insert(
                        entry.path().strip_prefix(root).unwrap().to_path_buf(),
                        fs::read(entry.path()).unwrap(),
                    );
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    impl Fixture {
        fn new() -> Self {
            let temp =
                tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
            let root = temp.path().join("room");
            let source = temp.path().join("source");
            fs::create_dir(&root).unwrap();
            copy_tree(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo"),
                &source,
            );
            let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, room_project::SEED);
            let scene = Scene::unbake(&room_project::types(), simulation.frame(), None)
                .unwrap()
                .to_yaml();
            fs::write(root.join("room.scene.yaml"), &scene).unwrap();
            fs::write(
                root.join("orr.project.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "schema":2,"engine":"*","entry":{
                        "game":"room-escape-v1","scene":"room.scene.yaml",
                        "models":"room.models.json","camera":"room.camera.json"
                    }
                }))
                .unwrap(),
            )
            .unwrap();
            fs::write(
                root.join("room.camera.json"),
                orr_sample::room_camera::Document::readable_default()
                    .to_bytes()
                    .unwrap(),
            )
            .unwrap();
            Project::open_for_install(&root, Runtime::content_only().engine_version)
                .unwrap()
                .install(std::slice::from_ref(&source))
                .unwrap();
            let loaded = model_bindings::load_asset(&root, USED, "foreground.glb").unwrap();
            let binding = Binding::from_asset(
                USED.into(),
                "foreground.glb".into(),
                &loaded,
                LocalTransform::default(),
            )
            .unwrap();
            let prepared = room_project::PreparedScene::parse(&scene).unwrap();
            let player = prepared
                .frame()
                .entities()
                .find(|entity| {
                    prepared
                        .frame()
                        .get::<RoomActor>(*entity)
                        .is_some_and(|actor| actor.kind == PLAYER)
                })
                .unwrap();
            let document = Document {
                version: 2,
                scene: "room.scene.yaml".into(),
                project: ".".into(),
                bindings: BTreeMap::from([(
                    prepared.index().guid(player).unwrap().to_string(),
                    binding,
                )]),
            };
            fs::write(
                root.join("room.models.json"),
                serde_json::to_vec_pretty(&document).unwrap(),
            )
            .unwrap();
            PreparedProject::open(&root).unwrap();
            Self { temp, root, source }
        }

        fn project(&self) -> Project {
            Project::open(&self.root, room_runtime()).unwrap()
        }

        fn source(&self, name: &str, dependencies: &[&str]) -> PathBuf {
            let path = self.temp.path().join(name);
            fs::create_dir(&path).unwrap();
            fs::write(path.join("asset.dat"), b"retained content\n").unwrap();
            fs::write(path.join("LICENSE.txt"), b"fixture license\n").unwrap();
            let manifest = Manifest {
                schema: 1,
                name: name.into(),
                version: "1.0.0".into(),
                engine: "*".into(),
                capabilities: BTreeSet::new(),
                dependencies: dependencies
                    .iter()
                    .map(|name| ((*name).into(), "1.0.0".into()))
                    .collect(),
                files: BTreeSet::from(["asset.dat".into(), "LICENSE.txt".into()]),
            };
            fs::write(
                path.join("orr.package.json"),
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();
            path
        }

        fn unused_plan(&self) -> RemovalPlan {
            let source = self.source("unused", &[]);
            self.project().install(&[source]).unwrap();
            plan_remove(&self.root, "unused").unwrap()
        }

        fn reject_without_changes(&self, plan: &RemovalPlan) -> String {
            let before = inventory(self.temp.path());
            let error = apply_remove(plan).unwrap_err();
            assert_eq!(inventory(self.temp.path()), before);
            error
        }
    }

    #[test]
    fn unused_removal_preserves_sources_cache_scene_camera_checksum_and_history() {
        let fixture = Fixture::new();
        let plan = fixture.unused_plan();
        let before = inventory(fixture.temp.path());
        let prepared = PreparedProject::open(&fixture.root).unwrap();
        let checksum = prepared.scene().frame().checksum();
        let mut editor = crate::Editor::start(&crate::HostSpec::PreparedRoom {
            scene: prepared.path().to_path_buf(),
            text: prepared.scene().text().into(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        editor.sync();
        let history = editor.history().entries.len();
        let host_checksum = editor.checksum();
        let result = apply_remove(&plan).unwrap();
        assert_eq!(result, plan.preview.candidate);
        assert!(!result.packages.contains_key("unused"));
        let after = inventory(fixture.temp.path());
        assert_eq!(
            before.keys().collect::<Vec<_>>(),
            after.keys().collect::<Vec<_>>()
        );
        for (path, bytes) in &before {
            if path != Path::new("room/orr.packages.lock.json") {
                assert_eq!(
                    after.get(path),
                    Some(bytes),
                    "unexpected mutation: {}",
                    path.display()
                );
            }
        }
        assert_eq!(
            PreparedProject::open(&fixture.root)
                .unwrap()
                .scene()
                .frame()
                .checksum(),
            checksum
        );
        editor.sync();
        assert_eq!(editor.checksum(), host_checksum);
        assert_eq!(editor.history().entries.len(), history);
        assert!(fixture.source.join("foreground.glb").exists());
        fixture.reject_without_changes(&plan);
    }

    #[test]
    fn rejects_used_static_package_without_writing() {
        let fixture = Fixture::new();
        let before = inventory(fixture.temp.path());
        let error = plan_remove(&fixture.root, USED).err().unwrap();
        assert!(error.contains("requires deactivated package"), "{error}");
        assert_eq!(inventory(fixture.temp.path()), before);
    }

    #[test]
    fn used_direct_package_can_be_retained_as_required_transitive_content() {
        let fixture = Fixture::new();
        let wrapper = fixture.source("wrapper", &[USED]);
        fixture.project().install(&[wrapper]).unwrap();
        let plan = plan_remove(&fixture.root, USED).unwrap();
        assert!(!plan.preview.candidate.direct.contains_key(USED));
        assert_eq!(
            plan.preview.candidate.packages.get(USED),
            plan.preview.previous.packages.get(USED)
        );
        let result = apply_remove(&plan).unwrap();
        assert!(result.packages.contains_key(USED));
        assert!(!result.direct.contains_key(USED));
        PreparedProject::open(&fixture.root).unwrap();
    }

    #[test]
    fn rejects_deactivating_transitive_package_used_by_a_binding() {
        let fixture = Fixture::new();
        let wrapper = fixture.source("wrapper", &[USED]);
        fixture.project().install(&[wrapper]).unwrap();
        fixture.project().remove(USED).unwrap();
        let before = inventory(fixture.temp.path());
        assert!(plan_remove(&fixture.root, "wrapper")
            .err()
            .unwrap()
            .contains("requires deactivated package"));
        assert!(plan_remove(&fixture.root, USED)
            .err()
            .unwrap()
            .contains("not a direct package"));
        assert_eq!(inventory(fixture.temp.path()), before);
    }

    #[test]
    fn removes_unreferenced_dependency_closure_but_retains_all_bytes() {
        let fixture = Fixture::new();
        let leaf = fixture.source("leaf", &[]);
        let wrapper = fixture.source("wrapper", &["leaf"]);
        fixture
            .project()
            .install_with_dependencies(&[wrapper], &[leaf])
            .unwrap();
        let plan = plan_remove(&fixture.root, "wrapper").unwrap();
        assert!(!plan.preview.candidate.packages.contains_key("leaf"));
        let cached = inventory(&fixture.root.join(".orr/packages/objects"));
        apply_remove(&plan).unwrap();
        assert_eq!(
            inventory(&fixture.root.join(".orr/packages/objects")),
            cached
        );
    }

    #[test]
    fn stale_manifest_scene_bindings_and_camera_bytes_each_reject_without_writing() {
        for name in [
            "orr.project.json",
            "room.scene.yaml",
            "room.models.json",
            "room.camera.json",
        ] {
            let fixture = Fixture::new();
            let plan = fixture.unused_plan();
            let path = fixture.root.join(name);
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(b'\n');
            fs::write(path, bytes).unwrap();
            PreparedProject::open(&fixture.root).unwrap();
            assert!(
                fixture
                    .reject_without_changes(&plan)
                    .contains("changed since removal planning"),
                "{name}"
            );
        }
    }

    #[test]
    fn stale_lock_and_changed_package_bytes_reject_without_writing() {
        let fixture = Fixture::new();
        let plan = fixture.unused_plan();
        let other = fixture.source("other", &[]);
        fixture.project().install(&[other]).unwrap();
        fixture.reject_without_changes(&plan);

        let fixture = Fixture::new();
        let plan = fixture.unused_plan();
        let digest = &plan.preview.previous.packages["unused"].digest;
        fs::write(
            fixture
                .root
                .join(".orr/packages/objects")
                .join(digest)
                .join("asset.dat"),
            b"tampered immutable bytes",
        )
        .unwrap();
        fixture.reject_without_changes(&plan);
    }

    #[test]
    fn unresolved_or_opaque_model_references_fail_closed() {
        for change in 0..3 {
            let fixture = Fixture::new();
            let plan = fixture.unused_plan();
            let path = fixture.root.join("room.models.json");
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            match change {
                0 => {
                    value["opaque_package_references"] = serde_json::json!(["unused"]);
                }
                1 => {
                    value["bindings"]
                        .as_object_mut()
                        .unwrap()
                        .values_mut()
                        .next()
                        .unwrap()["package"] = "missing-package".into();
                }
                _ => {
                    value["bindings"]
                        .as_object_mut()
                        .unwrap()
                        .values_mut()
                        .next()
                        .unwrap()["source_hash"] = "0".repeat(64).into();
                }
            }
            fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
            fixture.reject_without_changes(&plan);
            assert!(plan_remove(&fixture.root, "unused").is_err());
        }
    }

    #[test]
    fn changed_project_target_and_approved_candidate_reject_without_writing() {
        let fixture = Fixture::new();
        let mut plan = fixture.unused_plan();
        let other = fixture.temp.path().join("other-room");
        copy_tree(&fixture.root, &other);
        plan.root = other;
        fixture.reject_without_changes(&plan);
        plan.root = fixture.root.clone();
        plan.preview.candidate = plan.preview.previous.clone();
        fixture.reject_without_changes(&plan);
    }

    #[test]
    fn competing_writer_is_not_stolen() {
        let fixture = Fixture::new();
        let plan = fixture.unused_plan();
        fs::write(
            fixture.root.join(".orr/packages/writer.lock"),
            b"another fixture writer",
        )
        .unwrap();
        fixture.reject_without_changes(&plan);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_and_nonregular_sidecars_are_rejected() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let plan = fixture.unused_plan();
        let path = fixture.root.join("room.models.json");
        let saved = fixture.temp.path().join("saved-models.json");
        fs::rename(&path, &saved).unwrap();
        symlink(&saved, &path).unwrap();
        let lock = fs::read(fixture.root.join("orr.packages.lock.json")).unwrap();
        assert!(apply_remove(&plan).is_err());
        assert_eq!(
            fs::read(fixture.root.join("orr.packages.lock.json")).unwrap(),
            lock
        );
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(apply_remove(&plan).is_err());
        assert_eq!(
            fs::read(fixture.root.join("orr.packages.lock.json")).unwrap(),
            lock
        );
    }

    #[test]
    fn replaced_project_directory_rejects_even_with_identical_saved_bytes() {
        let fixture = Fixture::new();
        let plan = fixture.unused_plan();
        let moved = fixture.temp.path().join("original-room");
        fs::rename(&fixture.root, &moved).unwrap();
        copy_tree(&moved, &fixture.root);
        fixture.reject_without_changes(&plan);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn leaf_replacement_by_fifo_or_symlink_fails_without_blocking_writer() {
        use std::os::unix::fs::symlink;
        for fifo in [false, true] {
            let fixture = Fixture::new();
            let plan = fixture.unused_plan();
            let path = fixture.root.join("room.models.json");
            let before = fs::symlink_metadata(&path).unwrap();
            let lock = fs::read(fixture.root.join("orr.packages.lock.json")).unwrap();
            let error = fixture.project().remove_checked(
                &plan.name,
                &plan.preview.previous,
                &plan.preview.candidate,
                || {
                    let saved = fixture.temp.path().join("original-models.json");
                    fs::rename(&path, &saved).unwrap();
                    if fifo {
                        assert!(std::process::Command::new("mkfifo")
                            .arg(&path)
                            .status()
                            .unwrap()
                            .success());
                    } else {
                        symlink(&saved, &path).unwrap();
                    }
                    read_observed(&path, 1024 * 1024, &before).map(|_| ())
                },
            );
            assert!(error.is_err());
            assert_eq!(
                fs::read(fixture.root.join("orr.packages.lock.json")).unwrap(),
                lock
            );
            assert!(!fixture.root.join(".orr/packages/writer.lock").exists());
        }
    }

    #[test]
    fn receipt_restores_exact_lock_without_source_cache_scene_or_history_mutation() {
        let fixture = Fixture::new();
        let plan = fixture.unused_plan();
        let before = inventory(fixture.temp.path());
        let prepared = PreparedProject::open(&fixture.root).unwrap();
        let mut editor = crate::Editor::start(&crate::HostSpec::PreparedRoom {
            scene: prepared.path().to_path_buf(),
            text: prepared.scene().text().into(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        editor.sync();
        let checksum = editor.checksum();
        let history = editor.history().entries.len();
        let (removed, receipt) = apply_remove_with_receipt(&plan).unwrap();
        assert_eq!(receipt.root(), fixture.root);
        assert_eq!(receipt.name(), "unused");
        let preview = plan_restore(&receipt).unwrap();
        assert_eq!(preview.previous, removed);
        assert_eq!(preview.candidate, plan.preview.previous);
        assert_eq!(apply_restore(receipt).unwrap(), plan.preview.previous);
        assert_eq!(inventory(fixture.temp.path()), before);
        editor.sync();
        assert_eq!(editor.checksum(), checksum);
        assert_eq!(editor.history().entries.len(), history);
        assert_eq!(
            PreparedProject::open(&fixture.root)
                .unwrap()
                .scene()
                .frame()
                .to_bytes(),
            prepared.scene().frame().to_bytes()
        );
    }

    #[test]
    fn receipt_restores_orphaned_dependencies_with_original_direct_flags_without_sources() {
        let fixture = Fixture::new();
        let leaf = fixture.source("leaf", &[]);
        let wrapper = fixture.source("wrapper", &["leaf"]);
        fixture
            .project()
            .install_with_dependencies(std::slice::from_ref(&wrapper), std::slice::from_ref(&leaf))
            .unwrap();
        let plan = plan_remove(&fixture.root, "wrapper").unwrap();
        let (_, receipt) = apply_remove_with_receipt(&plan).unwrap();
        fs::remove_dir_all(wrapper).unwrap();
        fs::remove_dir_all(leaf).unwrap();
        let objects = inventory(&fixture.root.join(".orr/packages/objects"));
        let preview = plan_restore(&receipt).unwrap();
        assert!(preview.candidate.direct.contains_key("wrapper"));
        assert!(!preview.candidate.direct.contains_key("leaf"));
        assert!(preview.candidate.packages.contains_key("leaf"));
        assert_eq!(apply_restore(receipt).unwrap(), plan.preview.previous);
        assert_eq!(
            inventory(&fixture.root.join(".orr/packages/objects")),
            objects
        );
    }

    #[test]
    fn receipt_restores_a_used_shared_dependency_to_its_original_direct_selection() {
        let fixture = Fixture::new();
        let wrapper = fixture.source("wrapper", &[USED]);
        fixture.project().install(&[wrapper]).unwrap();
        let plan = plan_remove(&fixture.root, USED).unwrap();
        let before = inventory(fixture.temp.path());
        let (_, receipt) = apply_remove_with_receipt(&plan).unwrap();
        let preview = plan_restore(&receipt).unwrap();
        assert!(preview.candidate.direct.contains_key(USED));
        assert_eq!(preview.candidate.packages, preview.previous.packages);
        assert_eq!(apply_restore(receipt).unwrap(), plan.preview.previous);
        assert_eq!(inventory(fixture.temp.path()), before);
    }

    #[test]
    fn restore_rejects_each_changed_saved_room_file_without_publishing() {
        for name in [
            "orr.project.json",
            "room.scene.yaml",
            "room.models.json",
            "room.camera.json",
        ] {
            let fixture = Fixture::new();
            let plan = fixture.unused_plan();
            let (_, receipt) = apply_remove_with_receipt(&plan).unwrap();
            let path = fixture.root.join(name);
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(b'\n');
            fs::write(path, bytes).unwrap();
            let before = inventory(fixture.temp.path());
            assert!(plan_restore(&receipt).is_err(), "{name}");
            assert!(apply_restore(receipt).is_err(), "{name}");
            assert_eq!(inventory(fixture.temp.path()), before);
        }
    }

    #[test]
    fn restore_rejects_a_different_current_lock_without_rolling_back_other_packages() {
        let fixture = Fixture::new();
        let plan = fixture.unused_plan();
        let (_, receipt) = apply_remove_with_receipt(&plan).unwrap();
        let other = fixture.source("other", &[]);
        fixture.project().install(&[other]).unwrap();
        let before = inventory(fixture.temp.path());
        assert!(plan_restore(&receipt).is_err());
        assert!(apply_restore(receipt).is_err());
        assert_eq!(inventory(fixture.temp.path()), before);
        assert!(fixture
            .project()
            .verify()
            .unwrap()
            .direct
            .contains_key("other"));
    }

    #[test]
    fn restore_rejects_missing_or_tampered_removed_cache_objects() {
        for missing in [false, true] {
            let fixture = Fixture::new();
            let plan = fixture.unused_plan();
            let (_, receipt) = apply_remove_with_receipt(&plan).unwrap();
            let object = fixture
                .root
                .join(".orr/packages/objects")
                .join(&plan.preview.previous.packages["unused"].digest);
            if missing {
                fs::remove_dir_all(object).unwrap();
            } else {
                fs::write(object.join("asset.dat"), b"tampered cached package").unwrap();
            }
            let before = inventory(fixture.temp.path());
            assert!(plan_restore(&receipt).is_err());
            assert!(apply_restore(receipt).is_err());
            assert_eq!(inventory(fixture.temp.path()), before);
        }
    }

    #[test]
    fn restore_rejects_replaced_root_and_competing_writer_without_publishing() {
        for replace_root in [false, true] {
            let fixture = Fixture::new();
            let plan = fixture.unused_plan();
            let (_, receipt) = apply_remove_with_receipt(&plan).unwrap();
            if replace_root {
                let moved = fixture.temp.path().join("original-room");
                fs::rename(&fixture.root, &moved).unwrap();
                copy_tree(&moved, &fixture.root);
                assert!(plan_restore(&receipt).is_err());
            } else {
                fs::write(
                    fixture.root.join(".orr/packages/writer.lock"),
                    b"another fixture writer",
                )
                .unwrap();
            }
            let before = inventory(fixture.temp.path());
            assert!(apply_restore(receipt).is_err());
            assert_eq!(inventory(fixture.temp.path()), before);
        }
    }

    #[test]
    fn failed_removal_does_not_return_a_restoration_receipt() {
        let fixture = Fixture::new();
        let plan = fixture.unused_plan();
        let other = fixture.source("other", &[]);
        fixture.project().install(&[other]).unwrap();
        let before = inventory(fixture.temp.path());
        assert!(apply_remove_with_receipt(&plan).is_err());
        assert_eq!(inventory(fixture.temp.path()), before);
    }

    #[cfg(feature = "room-animated-models")]
    #[test]
    fn animated_binding_is_protected_and_unused_removal_preserves_playback() {
        use model_bindings::{AnimationDescriptor, ModelKind, PlaybackMode};
        let fixture = Fixture::new();
        let source = fixture.temp.path().join("animated-source");
        copy_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/animation_demo"),
            &source,
        );
        fixture.project().install(&[source]).unwrap();
        let loaded = model_bindings::load_asset_for_kind(
            &fixture.root,
            "sample-animation",
            "animated.glb",
            ModelKind::Animated,
        )
        .unwrap();
        let binding = Binding::from_animated_asset(
            "sample-animation".into(),
            "animated.glb".into(),
            &loaded,
            AnimationDescriptor {
                clip_index: 0,
                playback: PlaybackMode::Loop,
            },
            LocalTransform::default(),
        )
        .unwrap();
        let path = fixture.root.join("room.models.json");
        let mut document: Document = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        *document.bindings.values_mut().next().unwrap() = binding;
        fs::write(path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
        let before = PreparedProject::open(&fixture.root).unwrap();
        assert!(plan_remove(&fixture.root, "sample-animation")
            .err()
            .unwrap()
            .contains("requires deactivated package"));
        let plan = fixture.unused_plan();
        let (_, receipt) = apply_remove_with_receipt(&plan).unwrap();
        let after = PreparedProject::open(&fixture.root).unwrap();
        assert_eq!(before.models().document, after.models().document);
        assert_eq!(
            before.scene().frame().to_bytes(),
            after.scene().frame().to_bytes()
        );
        plan_restore(&receipt).unwrap();
        assert_eq!(apply_restore(receipt).unwrap(), plan.preview.previous);
        let restored = PreparedProject::open(&fixture.root).unwrap();
        assert_eq!(before.models().document, restored.models().document);
        assert_eq!(
            before.scene().frame().to_bytes(),
            restored.scene().frame().to_bytes()
        );
    }
}
