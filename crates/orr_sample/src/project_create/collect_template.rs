//! Closed, initial-only CollectDodge starter; no project identity is inferred here.
use crate::project_sprites::{Binding, Document, Source};
use orr_reflect::{Guid, Scene};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) fn documents(seed: &str) -> Result<(Scene, Document), String> {
    let mut scene = Scene::parse(
        include_str!("../../../../scenes/collect_dodge_v1.scene.yaml"),
        &crate::collect_project::types(),
    )
    .map_err(|e| e.to_string())?;
    scene.header_comments.extend(
        include_str!("../../../../assets/sprite_demo/LICENSE.txt")
            .lines()
            .map(str::to_owned),
    );
    let mut sprites = Document {
        version: 2,
        scene: "level.scene.yaml".into(),
        project: ".".into(),
        bindings: scene
            .entities
            .keys()
            .enumerate()
            .map(|(index, guid)| {
                (
                    guid.to_string(),
                    Binding {
                        package: super::template::PACKAGE.into(),
                        document: "sprites.json".into(),
                        source: if index == 0 {
                            Source::Clip("idle".into())
                        } else {
                            Source::Region(if index == 3 { 21 } else { 20 })
                        },
                        units_per_pixel: 0.5,
                    },
                )
            })
            .collect(),
        camera_follow: Some("e_00000001".into()),
    };
    let mut digest = Sha256::new();
    digest.update(b"orrery.collect.project-create.guid.v1\0");
    for part in [super::COLLECT_TEMPLATE, env!("CARGO_PKG_VERSION"), seed] {
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
            let ordinal = u32::try_from(index + 1).map_err(|_| "too many template entities")?;
            Ok((
                guid.clone(),
                Guid::parse(&format!("e_{prefix}{ordinal:08x}"))?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    super::template::remap(&mut scene, &mut sprites, &mapping)?;
    let scene = Scene::parse(&scene.to_yaml(), &crate::collect_project::types())
        .map_err(|e| e.to_string())?;
    sprites.validate()?;
    Ok((scene, sprites))
}
pub(super) fn readme(seed: &str, game_id: &str) -> String {
    format!("Orrery CollectDodge starter\nTemplate: {}\nAuthoring seed: {seed}\nProgress game_id: {game_id}\n\nMove with arrow keys/WASD; collect both items before 600 ticks, avoid the hazard. Space restarts.\nEdit level.scene.yaml and level.sprites.json through the CollectDodge editor.\nThe explicit UUIDv4 is the progress namespace. Choose a fresh UUID for a new game; copying or exporting this project intentionally retains identity. The seed only namespaces authored entities.\nBuild the editor with --features collect-dodge,sprites; open with orr_editor --collect-project /absolute/project. Build the standalone collect_dodge binary with --features collect-progress,collect-sprites; run collect_dodge --project /absolute/project. Schema 3 requires a CollectDodge host with declared progress and sprite support. Linux standalone collect-progress stores completed scores in user data, never this project; editor/headless/capture do not submit scores.\nThis closed generator installs bundled MIT assets offline; no downloads, scripts or compilation.\n", super::COLLECT_TEMPLATE)
}
