//! One closed starter, with immutable, explicitly allowlisted package source bytes.
use crate::project_sprites::{Binding, Document, Source};
use orr_reflect::{Guid, Scene, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) const ID: &str = "arena-2d-v1";
pub(super) const PACKAGE: &str = "sample-sprites";
pub(super) const SOURCE: &[(&str, &[u8])] = &[
    (
        "orr.package.json",
        include_bytes!("../../../../assets/sprite_demo/orr.package.json"),
    ),
    (
        "sprites.json",
        include_bytes!("../../../../assets/sprite_demo/sprites.json"),
    ),
    (
        "lantern_keeper.png",
        include_bytes!("../../../../assets/sprite_demo/lantern_keeper.png"),
    ),
    (
        "lantern_keeper.rgba",
        include_bytes!("../../../../assets/sprite_demo/lantern_keeper.rgba"),
    ),
    (
        "LICENSE.txt",
        include_bytes!("../../../../assets/sprite_demo/LICENSE.txt"),
    ),
];
const SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  # Human-controlled player 0.\n  e_00000001:\n    name: hero\n    # Authored spawn position.\n    Position: { pos: [-60,0] }\n    PlayerTag: { slot: 0 }\n  # Human-controlled player 1.\n  e_00000002:\n    name: target\n    Position: { pos: [60,0] }\n    PlayerTag: { slot: 1 }\n";

pub(super) fn documents(seed: &str) -> Result<(Scene, Document), String> {
    let mut scene =
        Scene::parse(SCENE, &crate::project::arena_types()).map_err(|e| e.to_string())?;
    scene.header_comments = vec![
        format!("Orrery {ID} starter scene; generated with an explicit authoring namespace."),
        "Existing Arena game identity and shared player preferences are unchanged.".into(),
        "Starter scene and bundled original sprite content: MIT license below.".into(),
    ];
    // Keep full provenance and license in supported scene header comments, so the
    // existing exporter retains them even though it excludes the root README.
    scene.header_comments.extend(
        include_str!("../../../../assets/sprite_demo/LICENSE.txt")
            .lines()
            .map(str::to_owned),
    );
    let binding = Binding {
        package: PACKAGE.into(),
        document: "sprites.json".into(),
        source: Source::Locomotion {
            idle: "idle".into(),
            walk: "walk".into(),
        },
        units_per_pixel: 2.0,
    };
    let mut sprites = Document {
        version: 2,
        scene: "arena.scene.yaml".into(),
        project: ".".into(),
        bindings: scene
            .entities
            .keys()
            .map(|guid| (guid.to_string(), binding.clone()))
            .collect(),
        camera_follow: Some("e_00000001".into()),
    };
    let mut digest = Sha256::new();
    digest.update(b"orrery.arena.project-create.guid.v1\0");
    for part in [ID, env!("CARGO_PKG_VERSION"), seed] {
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
    remap(&mut scene, &mut sprites, &mapping)?;
    // Validate after the typed remap, rather than rewriting arbitrary text.
    let scene = Scene::parse(&scene.to_yaml(), &crate::project::arena_types())
        .map_err(|e| e.to_string())?;
    sprites.validate()?;
    Ok((scene, sprites))
}

pub(super) fn remap(
    scene: &mut Scene,
    sprites: &mut Document,
    mapping: &BTreeMap<Guid, Guid>,
) -> Result<(), String> {
    remap_scene(scene, mapping)?;
    fn guid(text: &str, mapping: &BTreeMap<Guid, Guid>) -> Result<String, String> {
        mapping
            .get(&Guid::parse(text)?)
            .map(ToString::to_string)
            .ok_or_else(|| format!("template reference has no GUID mapping: {text}"))
    }
    sprites.bindings = std::mem::take(&mut sprites.bindings)
        .into_iter()
        .map(|(old, binding)| Ok((guid(&old, mapping)?, binding)))
        .collect::<Result<_, String>>()?;
    sprites.camera_follow = sprites
        .camera_follow
        .as_deref()
        .map(|old| guid(old, mapping))
        .transpose()?;
    Ok(())
}

pub(super) fn remap_scene(scene: &mut Scene, mapping: &BTreeMap<Guid, Guid>) -> Result<(), String> {
    fn guid(text: &str, mapping: &BTreeMap<Guid, Guid>) -> Result<String, String> {
        mapping
            .get(&Guid::parse(text)?)
            .map(ToString::to_string)
            .ok_or_else(|| format!("template reference has no GUID mapping: {text}"))
    }
    fn value(v: &mut Value, mapping: &BTreeMap<Guid, Guid>) -> Result<(), String> {
        match v {
            Value::EntityGuid(Some(old)) => *old = guid(old, mapping)?,
            Value::Array(values) => {
                for v in values {
                    value(v, mapping)?;
                }
            }
            Value::Struct(fields) | Value::Variant(_, fields) => {
                for (_, v) in fields {
                    value(v, mapping)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    if mapping.len() != scene.entities.len()
        || mapping
            .values()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != mapping.len()
        || !scene.entities.keys().all(|key| mapping.contains_key(key))
    {
        return Err("template GUID mapping must be complete and unique".into());
    }
    for (_, v) in &mut scene.singletons {
        value(v, mapping)?;
    }
    let mut entities = BTreeMap::new();
    for (old, mut entity) in std::mem::take(&mut scene.entities) {
        for (_, v) in &mut entity.components {
            value(v, mapping)?;
        }
        entities.insert(mapping[&old].clone(), entity);
    }
    scene.entities = entities;
    let mut comments = BTreeMap::new();
    for (key, lines) in std::mem::take(&mut scene.comments) {
        let key = if let Some(old) = key.strip_prefix("entity:") {
            format!("entity:{}", guid(old, mapping)?)
        } else if let Some(rest) = key.strip_prefix("component:") {
            let (old, component) = rest
                .split_once(':')
                .ok_or("invalid template component comment")?;
            format!("component:{}:{component}", guid(old, mapping)?)
        } else {
            key
        };
        comments.insert(key, lines);
    }
    scene.comments = comments;
    Ok(())
}
