#[path = "platform.rs"]
mod platform;

use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::Project;
use orr_reflect::{Guid, Scene};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, KEY, PLAYER, TICK_RATE},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::Simulation;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub const USAGE: &str = "room_create_windows --output ABSOLUTE_NEW_DIRECTORY --seed ASCII_KEY";
const TEMPLATE: &str = "room-escape-3d-v1";
const PACKAGE: &str = "sample-imported-scene";
const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILES: usize = 16;
const SOURCE: &[(&str, &[u8])] = &[
    (
        "orr.package.json",
        include_bytes!("../../../../assets/imported_scene_demo/orr.package.json"),
    ),
    (
        "background.glb",
        include_bytes!("../../../../assets/imported_scene_demo/background.glb"),
    ),
    (
        "foreground.glb",
        include_bytes!("../../../../assets/imported_scene_demo/foreground.glb"),
    ),
    (
        "LICENSE.txt",
        include_bytes!("../../../../assets/imported_scene_demo/LICENSE.txt"),
    ),
    (
        "generate.py",
        include_bytes!("../../../../assets/imported_scene_demo/generate.py"),
    ),
];

#[derive(Clone, Debug)]
pub struct Options {
    pub output: PathBuf,
    pub seed: String,
}

impl Options {
    pub fn parse(args: Vec<OsString>) -> Result<Self, String> {
        if args.len() != 4 {
            return Err(USAGE.into());
        }
        let (mut output, mut seed) = (None, None);
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            match flag.to_str() {
                Some("--output") if output.is_none() => {
                    let value = args.next().ok_or(USAGE)?;
                    if value.is_empty() || value.as_encoded_bytes().len() > 4096 {
                        return Err("output path must contain 1..=4096 native bytes".into());
                    }
                    output = Some(PathBuf::from(value));
                }
                Some("--seed") if seed.is_none() => {
                    let value = args.next().ok_or(USAGE)?;
                    if value.as_encoded_bytes().len() > 128 {
                        return Err("seed exceeds 128 bytes".into());
                    }
                    seed = Some(value.into_string().map_err(|_| "seed must be ASCII")?);
                }
                _ => return Err(format!("unknown or duplicate option; {USAGE}")),
            }
        }
        let options = Self {
            output: output.ok_or(USAGE)?,
            seed: seed.ok_or(USAGE)?,
        };
        validate_seed(&options.seed)?;
        Ok(options)
    }
}

pub struct Report {
    pub output: PathBuf,
    pub initial_checksum: u64,
    pub entity_guids: Vec<Guid>,
}

fn validate_seed(seed: &str) -> Result<(), String> {
    if seed.is_empty()
        || seed.len() > 128
        || !seed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err(
            "seed must contain 1..=128 ASCII letters, digits, dots, underscores or hyphens".into(),
        );
    }
    Ok(())
}

pub fn create(options: &Options) -> Result<Report, String> {
    validate_seed(&options.seed)?;
    let output = platform::destination(&options.output)?;
    let parent = output
        .parent()
        .ok_or("output requires an existing parent")?;
    let transaction = tempfile::Builder::new()
        .prefix(".orr-win-room-")
        .tempdir_in(parent)
        .map_err(|error| error.to_string())?;
    let root = transaction.path().join("project");
    let source = transaction.path().join("source");
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    fs::create_dir(&source).map_err(|error| error.to_string())?;

    let scene = scene(&options.seed)?;
    let entity_guids = scene.entities.keys().cloned().collect();
    let mut expected = BTreeMap::from([
        (
            "orr.project.json".to_owned(),
            json_line(&serde_json::json!({
                "schema": 2, "engine": "^0.0.1",
                "entry": {"game": "room-escape-v1", "scene": "room.scene.yaml", "models": "room.models.json", "camera": "room.camera.json"}
            }))?,
        ),
        ("room.scene.yaml".to_owned(), scene.to_yaml().into_bytes()),
        (
            "room.camera.json".to_owned(),
            orr_sample::room_camera::Document::readable_default().to_bytes()?,
        ),
        ("README.md".to_owned(), readme(&options.seed).into_bytes()),
    ]);
    for (name, bytes) in &expected {
        write_new(&root.join(name), bytes)?;
    }
    for (name, bytes) in SOURCE {
        write_new(&source.join(name), bytes)?;
    }
    let project = Project::open(&root, room_project::compiled_runtime())
        .map_err(|error| error.to_string())?;
    let lock = project
        .install(&[source])
        .map_err(|error| format!("Room package installation: {error}"))?;
    if project.verify().map_err(|error| error.to_string())? != lock
        || lock.packages.len() != 1
        || lock.direct.len() != 1
    {
        return Err("Room starter package lock differs from its closed bundle".into());
    }
    let package = lock
        .packages
        .get(PACKAGE)
        .ok_or("Room starter package is missing")?;
    let manifest: orr_package::Manifest =
        serde_json::from_slice(SOURCE[0].1).map_err(|error| error.to_string())?;
    let declared = SOURCE
        .iter()
        .skip(1)
        .map(|(name, _)| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    if package.manifest != manifest
        || package.manifest.files != declared
        || lock.direct.get(PACKAGE) != Some(&manifest.version)
    {
        return Err("Room starter package differs from its complete bundled manifest".into());
    }
    let object = format!(".orr/packages/objects/{}", package.digest);
    expected.insert(
        format!("{object}/orr.package.json"),
        serde_json::to_vec_pretty(&package.manifest).map_err(|error| error.to_string())?,
    );
    for (name, bytes) in SOURCE.iter().skip(1) {
        expected.insert(format!("{object}/{name}"), bytes.to_vec());
    }
    expected.insert("orr.packages.lock.json".into(), json_line(&lock)?);
    let models = models(&project, &scene)?;
    let model_bytes = json_line(&models)?;
    write_new(&root.join("room.models.json"), &model_bytes)?;
    expected.insert("room.models.json".into(), model_bytes);
    verify_stage(&root, &expected)?;
    let prepared = PreparedProject::open(&root)?;
    let initial_checksum = prepared.scene().frame().checksum();
    verify_stage(&root, &expected)?;
    if platform::destination(&options.output)? != output {
        return Err("output parent changed during creation".into());
    }
    platform::publish(&root, &output)?;
    drop(transaction);
    Ok(Report {
        output,
        initial_checksum,
        entity_guids,
    })
}

fn scene(seed: &str) -> Result<Scene, String> {
    let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, TICK_RATE, SEED);
    let mut scene = Scene::unbake(&room_project::types(), simulation.frame(), None)
        .map_err(|error| error.to_string())?;
    if scene.entities.len() != 7 {
        return Err("closed Room starter requires seven canonical actors".into());
    }
    scene.header_comments = vec![format!(
        "Orrery {TEMPLATE} closed starter; seed namespaces authored GUIDs only."
    )];
    scene.header_comments.extend(
        include_str!("../../../../assets/imported_scene_demo/LICENSE.txt")
            .lines()
            .map(str::to_owned),
    );
    let mut digest = Sha256::new();
    digest.update(b"orrery.room.project-create.guid.v1\0");
    for part in [TEMPLATE, env!("CARGO_PKG_VERSION"), seed] {
        digest.update((part.len() as u32).to_le_bytes());
        digest.update(part.as_bytes());
    }
    let prefix = digest.finalize()[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    scene.entities = scene
        .entities
        .into_values()
        .enumerate()
        .map(|(index, entity)| {
            let ordinal = u32::try_from(index + 1).map_err(|_| "too many Room actors")?;
            let guid = Guid::parse(&format!("e_{prefix}{ordinal:08x}"))?;
            Ok((guid, entity))
        })
        .collect::<Result<_, String>>()?;
    PreparedScene::parse(&scene.to_yaml())?;
    Ok(scene)
}

fn models(project: &Project, scene: &Scene) -> Result<Document, String> {
    let asset = model_bindings::load_asset_from_project(
        project,
        PACKAGE,
        "foreground.glb",
        model_bindings::ModelKind::Static,
    )?;
    let binding = Binding::from_asset(
        PACKAGE.into(),
        "foreground.glb".into(),
        &asset,
        LocalTransform::default(),
    )?;
    let prepared = PreparedScene::parse(&scene.to_yaml())?;
    let mut bindings = BTreeMap::new();
    for entity in prepared.frame().entities() {
        let actor = prepared
            .frame()
            .get::<RoomActor>(entity)
            .ok_or("missing Room actor")?;
        if matches!(actor.kind, PLAYER | KEY) {
            let guid = prepared
                .index()
                .guid(entity)
                .ok_or("Room actor lacks authored GUID")?;
            bindings.insert(guid.to_string(), binding.clone());
        }
    }
    if bindings.len() != 2 {
        return Err("Room starter requires player and key model bindings".into());
    }
    let document = Document {
        version: 2,
        scene: "room.scene.yaml".into(),
        project: ".".into(),
        bindings,
    };
    document.validate()?;
    Ok(document)
}

fn json_line(value: &impl serde::Serialize) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("Room starter file exceeds byte limit".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())
}

fn relative_name(path: &Path) -> Result<String, String> {
    path.components()
        .map(|component| match component {
            Component::Normal(name) => name
                .to_str()
                .map(str::to_owned)
                .ok_or("Room stage path must be Unicode".to_owned()),
            _ => Err("Room stage path must be relative and normalized".into()),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}

fn verify_stage(root: &Path, expected: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    if expected.len() > MAX_FILES
        || expected.values().map(Vec::len).sum::<usize>() > MAX_TOTAL_BYTES
        || !platform::is_directory(&fs::symlink_metadata(root).map_err(|error| error.to_string())?)
    {
        return Err("Room starter stage exceeds its closed limits".into());
    }
    let mut directories = BTreeSet::new();
    for name in expected.keys() {
        let mut parent = Path::new(name).parent();
        while let Some(path) = parent.filter(|path| !path.as_os_str().is_empty()) {
            directories.insert(path.to_path_buf());
            parent = path.parent();
        }
    }
    let mut pending = vec![root.to_path_buf()];
    let mut seen_files = BTreeSet::new();
    let mut seen_dirs = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
            let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if platform::is_directory(&metadata) {
                if !directories.contains(relative) || !seen_dirs.insert(relative.to_path_buf()) {
                    return Err("unexpected Room starter directory".into());
                }
                pending.push(path);
            } else if platform::is_regular(&metadata) {
                let name = relative_name(relative)?;
                let bytes = expected.get(&name).ok_or("unexpected Room starter file")?;
                if metadata.len() != bytes.len() as u64 || bytes.len() > MAX_FILE_BYTES {
                    return Err("Room starter stage file size changed".into());
                }
                let mut actual = Vec::new();
                platform::open_read(&path)?
                    .take(MAX_FILE_BYTES as u64 + 1)
                    .read_to_end(&mut actual)
                    .map_err(|error| error.to_string())?;
                if actual != *bytes || !seen_files.insert(name) {
                    return Err("Room starter stage bytes changed".into());
                }
            } else {
                return Err("Room starter stage contains a reparse point or special file".into());
            }
        }
    }
    if seen_files != expected.keys().cloned().collect() || seen_dirs != directories {
        return Err("Room starter stage closure changed".into());
    }
    Ok(())
}

fn readme(seed: &str) -> String {
    format!("# Windows Room starter\n\nTemplate: {TEMPLATE}\nAuthoring seed: {seed}\n\nOpen: orr_editor --room-project PATH\nRun: room_escape --project PATH\nMove with WASD/arrows; press E near the key, then at the exit; R restarts.\n\nThe seed namespaces authored GUIDs only; game identity, simulation seed and tick rate stay unchanged. The complete CC0 sample-imported-scene package and its license are installed through the existing package manager. Its generate.py is inert content and is never executed. No downloads, builds, settings or user data are copied.\n\nThis optional Windows example creates a bounded seven-actor flat-room project with two static model bindings and the existing authored camera. It does not add HUD, checkpoints, skinned characters or a Windows exporter. The production folder exporter currently supports Linux x86-64 hosts only.\n\nPublication uses a native no-replace same-volume directory move. Checks address ordinary filesystem errors; hostile concurrent path mutation, crash atomicity and power-loss durability are not claimed. Windows files inherit the parent ACL.\n")
}
