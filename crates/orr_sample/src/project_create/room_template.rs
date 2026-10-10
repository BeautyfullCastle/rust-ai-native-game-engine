//! Closed RoomEscapeV1 starter. This creates initial authoring data, never saves.
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform, ModelKind};
use orr_reflect::{Guid, Scene};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) const PACKAGE: &str = "sample-imported-scene";
pub(super) const SOURCE: &[(&str, &[u8])] = &[
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

pub(super) fn scene(seed: &str) -> Result<Scene, String> {
    use orr_games::room_escape_game::{RoomConfig, RoomEscapeV1, TICK_RATE};
    let types = crate::room_project::types();
    let simulation =
        orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, TICK_RATE, crate::room_project::SEED);
    let mut scene = Scene::unbake(&types, simulation.frame(), None).map_err(super::error)?;
    scene.header_comments = vec![format!(
        "Orrery {} closed starter; seed namespaces authored GUIDs only.",
        super::ROOM_TEMPLATE
    )];
    scene.header_comments.extend(
        include_str!("../../../../assets/imported_scene_demo/LICENSE.txt")
            .lines()
            .map(str::to_owned),
    );
    let mut digest = Sha256::new();
    digest.update(b"orrery.room.project-create.guid.v1\0");
    for part in [super::ROOM_TEMPLATE, env!("CARGO_PKG_VERSION"), seed] {
        digest.update((part.len() as u32).to_le_bytes());
        digest.update(part.as_bytes());
    }
    let prefix: String = digest.finalize()[..12]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mapping = scene
        .entities
        .keys()
        .enumerate()
        .map(|(index, guid)| {
            let ordinal = u32::try_from(index + 1).map_err(|_| "too many room entities")?;
            Ok((
                guid.clone(),
                Guid::parse(&format!("e_{prefix}{ordinal:08x}"))?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    super::template::remap_scene(&mut scene, &mapping)?;
    crate::room_project::PreparedScene::parse(&scene.to_yaml())?;
    Ok(scene)
}

pub(super) fn models(project: &orr_package::Project, scene: &Scene) -> Result<Document, String> {
    use orr_games::room_escape_game::{RoomActor, KEY, PLAYER};
    let loaded = model_bindings::load_asset_from_project(
        project,
        PACKAGE,
        "foreground.glb",
        ModelKind::Static,
    )?;
    let binding = Binding::from_asset(
        PACKAGE.into(),
        "foreground.glb".into(),
        &loaded,
        LocalTransform::default(),
    )?;
    let prepared = crate::room_project::PreparedScene::parse(&scene.to_yaml())?;
    let bindings = prepared
        .frame()
        .entities()
        .filter(|&entity| {
            prepared
                .frame()
                .get::<RoomActor>(entity)
                .is_some_and(|actor| matches!(actor.kind, PLAYER | KEY))
        })
        .map(|entity| {
            let guid = prepared
                .index()
                .guid(entity)
                .ok_or("room actor lacks authored GUID")?;
            Ok((guid.to_string(), binding.clone()))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    if bindings.len() != 2 {
        return Err("closed room requires player and key model bindings".into());
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

pub(super) fn readme(seed: &str) -> String {
    format!("Orrery RoomEscapeV1 starter\nTemplate: {}\nAuthoring seed: {seed}\n\nMove with arrow keys/WASD; interact with E near the key, then the exit; R restarts the admitted authored initial state.\nOpen with room-project-enabled orr_editor --room-project /absolute/project. Run room-project-enabled room_escape --project /absolute/project. Export with project-export,room-project-enabled orr_export_room and a trusted prebuilt room_escape runtime.\n\nThis Linux generator installs its immutable allowlisted CC0 sample-imported-scene package through Project.install, including both static GLBs, LICENSE.txt and inert generate.py. It never executes that script, downloads assets or compiles code. The meshes contain authored UVs and embedded opaque PNG textures. Models are presentation only; the sphere controller uses the saved collider state.\n\nThe seed changes authored GUIDs only; RoomEscapeV1 build identity, seed 42 and 60 Hz are unchanged. No game/progress identity or persistent save is created. Same template/tool/seed reproduces bytes; sorted entity order preserves initial Frame checksum across seeds. Runtime restart retains the admitted initial Frame. No settings, caches or user data are copied.\n\nThis is a bounded flat-room key/exit starter, not a general character controller or a completed #94 game. The template declares room.camera.json: a bounded portrait-fit orthographic view shared by native play, capture and editor. Manual orbit/pan/zoom is session-only; runtime Restart restores the admitted camera. Editor Apply/Undo reset the preview; Save camera atomically saves only this sidecar, independently from scene/model Save. Old projects without a camera keep their former defaults. Richer HUD remains separate work.\n", super::ROOM_TEMPLATE)
}
