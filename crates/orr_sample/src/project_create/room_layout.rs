//! Strict creation recipe applied only inside the creator's unpublished stage.
use super::*;
use crate::room_layout::{decimal, position, Recipe};
use orr_fp::{FPVec3, FP};
use orr_reflect::{Scene, SceneEntity, Value};

const BODY: &str = "orr_physics3d::Body";
const COLLIDER: &str = "orr_physics3d::Collider";
const LIMIT: usize = 65536;

/// Create a plain Room with authored positions and a replacement wall set.
/// The captured recipe is read-only. Existing destinations are never replaced.
pub fn create_room_layout(options: &CreateOptions, recipe: &Path) -> Result<CreateReport, String> {
    if options.template != ROOM_TEMPLATE {
        return Err("--room-layout requires room-escape-3d-v1".into());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(recipe)
        .map_err(error)?;
    let metadata = file.metadata().map_err(error)?;
    if !metadata.is_file() || metadata.len() > LIMIT as u64 {
        return Err("Room layout requires a regular nonsymlink file at most 65536 bytes".into());
    }
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > LIMIT {
        return Err("Room layout exceeds 65536 bytes".into());
    }
    let scene = authored_scene(&options.seed, &bytes)?;
    let guids = scene.entities.keys().cloned().collect();
    let mut report = create_transaction_finalized(
        options,
        None,
        |_, _| Ok(()),
        publish_no_replace,
        |root, expected| {
            let bytes = scene.to_yaml().into_bytes();
            fs::write(root.join("room.scene.yaml"), &bytes).map_err(error)?;
            expected.insert("room.scene.yaml".into(), bytes);
            let mut readme = expected["README.md"].clone();
            readme.extend_from_slice(b"\nCustom Room layout: actor X/Z and replacement walls were authored from a captured schema-1 recipe. Admission does not prove the puzzle is solvable; verify a route with orr_playtest_room.\n");
            fs::write(root.join("README.md"), &readme).map_err(error)?;
            expected.insert("README.md".into(), readme);
            Ok(())
        },
    )?;
    report.entity_guids = guids;
    Ok(report)
}

fn component<'a>(entity: &'a mut SceneEntity, name: &str) -> Result<&'a mut Value, String> {
    entity
        .components
        .iter_mut()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
        .ok_or_else(|| format!("starter missing {name}"))
}
fn field<'a>(value: &'a mut Value, name: &str) -> Result<&'a mut Value, String> {
    match value {
        Value::Struct(fields) | Value::Variant(_, fields) => fields
            .iter_mut()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
            .ok_or_else(|| format!("starter missing {name}")),
        _ => Err("starter value is not structured".into()),
    }
}
fn role(entity: &SceneEntity) -> Option<i128> {
    entity
        .components
        .iter()
        .find(|(n, _)| n == crate::room_project::ACTOR)
        .and_then(|(_, v)| v.field("kind"))
        .and_then(|v| {
            if let Value::Int(n) = v {
                Some(*n)
            } else {
                None
            }
        })
}
fn authored_scene(seed: &str, bytes: &[u8]) -> Result<Scene, String> {
    let recipe = Recipe::parse(bytes)?;
    let positions = [
        position(&recipe.player)?,
        position(&recipe.key)?,
        position(&recipe.exit)?,
    ];
    let mut scene = room_template::scene(seed)?;
    let wall = scene
        .entities
        .values()
        .find(|entity| role(entity) == Some(4))
        .cloned()
        .ok_or("starter wall missing")?;
    scene.entities.retain(|_, entity| role(entity) != Some(4));
    for entity in scene.entities.values_mut() {
        if let Some(kind @ 1..=3) = role(entity) {
            *field(component(entity, BODY)?, "pos")? = Value::Vec3(positions[(kind - 1) as usize]);
        }
    }
    // Reuse the starter namespace and put walls after its seven reserved roles.
    // This keeps entity ordering (and Frame checksums) independent of the seed.
    let first = scene
        .entities
        .keys()
        .next()
        .ok_or("starter actors missing")?
        .to_string();
    let prefix = first
        .get(..first.len().checked_sub(8).ok_or("invalid starter GUID")?)
        .ok_or("invalid starter GUID")?;
    for (i, spec) in recipe.walls.iter().enumerate() {
        let mut entity = wall.clone();
        entity.name = Some(format!("Wall {}", i + 1));
        *field(component(&mut entity, BODY)?, "pos")? = Value::Vec3(position(&spec.center)?);
        *field(
            field(component(&mut entity, COLLIDER)?, "shape")?,
            "half_extents",
        )? = Value::Vec3(FPVec3::new(
            decimal(&spec.half_extents[0])?,
            FP::ONE,
            decimal(&spec.half_extents[1])?,
        ));
        let guid = Guid::parse(&format!("{prefix}{:08x}", 0x100 + i as u32))?;
        if scene.entities.insert(guid, entity).is_some() {
            return Err("layout GUID collision".into());
        }
    }
    crate::room_project::PreparedScene::parse(&scene.to_yaml())?;
    Ok(scene)
}

#[cfg(test)]
mod tests;
