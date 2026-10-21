//! Read-only admission of one visual background from installed sprite assets.
//! No package mutation or removal authorization is implied by usage information.
use crate::{
    project::{entry_file, read_regular, PreparedProject, TilemapSupport, UiSupport},
    project_compositor::{SpriteDraw, MAX_PROJECT_SPRITES},
    project_sprites::{load_project_asset, Asset, AssetKey},
};
use orr_tilemap::{Document, SpriteRef, ValidatedDocument};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const MAX_ATLASES: usize = 8;

#[derive(Clone, Debug)]
pub struct PreparedTilemap {
    pub path: PathBuf,
    pub document: ValidatedDocument,
    original_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TilemapUsage {
    pub path: PathBuf,
    pub sprite: SpriteRef,
    pub occupied_cells: usize,
}

impl PreparedTilemap {
    /// Exact bounded bytes read during admission, for authoring CAS checks.
    pub fn original_bytes(&self) -> &[u8] {
        &self.original_bytes
    }

    /// Make a presentation candidate without changing its saved-byte identity.
    pub fn with_document(&self, document: ValidatedDocument) -> Self {
        Self {
            path: self.path.clone(),
            document,
            original_bytes: self.original_bytes.clone(),
        }
    }

    /// A reference summary, never permission to remove or replace its package.
    pub fn usage(&self) -> TilemapUsage {
        TilemapUsage {
            path: self.path.clone(),
            sprite: self.document.document().sprite.clone(),
            occupied_cells: self.document.occupied_count(),
        }
    }
}

fn check_count(actors: usize, occupied: usize) -> Result<(), String> {
    match actors.checked_add(occupied) {
        Some(total) if total <= MAX_PROJECT_SPRITES => Ok(()),
        _ => Err(format!(
            "actor bindings plus occupied tiles exceed {MAX_PROJECT_SPRITES}"
        )),
    }
}

/// Caller-private admission seam: `existing_assets` MUST be the fresh actor map
/// loaded in this same admission from this Project and verified lock. It is not
/// a persistent cache or a public arbitrary-Asset input API. Asset deliberately
/// has no revision identity, so exposing this function publicly would lose that
/// invariant. The supplied map is never mutated; a missing atlas is returned
/// only after the complete candidate succeeds.
pub(crate) fn prepare(
    project: &orr_package::Project,
    lock: &orr_package::Lock,
    root: &Path,
    relative: &str,
    existing_assets: &BTreeMap<AssetKey, Asset>,
    actor_count: usize,
) -> Result<(PreparedTilemap, Option<(AssetKey, Asset)>), String> {
    if root != project.root() {
        return Err("tilemap root differs from the admitted project root".into());
    }
    if relative.starts_with('.') || relative.contains('/') {
        return Err("tilemap must be a non-hidden root-level filename".into());
    }
    let path = entry_file(root, relative)?;
    let bytes = read_regular(&path, orr_tilemap::MAX_BYTES as u64)?;
    let document = Document::from_bytes(&bytes).map_err(|e| format!("tilemap: {e}"))?;
    let occupied = document.cells.iter().filter(|cell| cell.is_some()).count();
    check_count(actor_count, occupied)?;
    let key = (
        document.sprite.package.clone(),
        document.sprite.document.clone(),
    );
    let missing = !existing_assets.contains_key(&key);
    if existing_assets
        .len()
        .checked_add(usize::from(missing))
        .is_none_or(|n| n > MAX_ATLASES)
    {
        return Err("actor and tilemap references exceed 8 sprite documents".into());
    }
    let package = lock
        .packages
        .get(&key.0)
        .ok_or_else(|| format!("tilemap: missing package {}", key.0))?;
    if !package.manifest.capabilities.contains("sprite") {
        return Err("tilemap package must declare the sprite capability".into());
    }
    // Check full installed bytes even on a cache hit, and bind this admission to
    // exactly the supplied revision rather than accepting a refreshed lock.
    let current = project
        .verify()
        .map_err(|e| format!("tilemap package verification: {e}"))?;
    if &current != lock {
        return Err("tilemap active lock changed during admission".into());
    }
    // read_asset also rejects installer-only handles lacking host authority.
    // On cache hits this reads verified bytes but never decodes another atlas.
    let admitted_bytes = project
        .read_asset(&key.0, &key.1)
        .map_err(|e| format!("tilemap sprite document: {e}"))?;
    if admitted_bytes.len() > orr_sprite::MAX_JSON_BYTES {
        return Err("tilemap sprite document exceeds byte limit".into());
    }
    let new_asset = if missing {
        Some(load_project_asset(project, &key.0, &key.1)?)
    } else {
        None
    };
    let asset = new_asset
        .as_ref()
        .or_else(|| existing_assets.get(&key))
        .expect("missing asset was loaded");
    if !package.files.contains_key(&asset.document.atlas().image) {
        return Err("tilemap atlas is not declared by its locked package".into());
    }
    let document = document
        .validate(&asset.document)
        .map_err(|e| format!("tilemap: {e}"))?;
    // A lock-changing operation between individual reads must not publish a
    // mixed-revision candidate. This is ordinary filesystem consistency, not a
    // hostile concurrent-filesystem sandbox guarantee.
    if project.list().map_err(|e| e.to_string())? != *lock {
        return Err("tilemap active lock changed during admission".into());
    }
    Ok((
        PreparedTilemap {
            path,
            document,
            original_bytes: bytes,
        },
        new_asset.map(|asset| (key, asset)),
    ))
}

/// Build the immutable background sequence after combined-budget admission.
pub fn draws(prepared: &PreparedTilemap, actor_count: usize) -> Result<Vec<SpriteDraw>, String> {
    check_count(actor_count, prepared.document.occupied_count())?;
    let reference = &prepared.document.document().sprite;
    let key = (reference.package.clone(), reference.document.clone());
    Ok(prepared
        .document
        .placements()
        .enumerate()
        .map(|(order, tile)| SpriteDraw {
            asset: key.clone(),
            instance: orr_render::SpriteInstance {
                region: tile.region,
                position: tile.position,
                size: tile.size,
                order: order as i32,
                ..Default::default()
            },
        })
        .collect())
}

/// Full project admission, not JSON grep. A usage result is not a safe-removal
/// authorization; v1 tilemap projects intentionally refuse package mutation.
pub fn usage(root: &Path, runtime: orr_package::Runtime) -> Result<Option<TilemapUsage>, String> {
    let ui = if cfg!(feature = "game-ui") {
        UiSupport::Supported
    } else {
        UiSupport::Unsupported
    };
    let prepared =
        PreparedProject::open_with_capabilities(root, runtime, ui, TilemapSupport::Supported)?;
    Ok(prepared.tilemap().map(PreparedTilemap::usage))
}

#[cfg(test)]
use crate::project_fixture as fixture;

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (
        fixture::ProjectFixture,
        orr_package::Project,
        orr_package::Lock,
    ) {
        let fixture = fixture::ProjectFixture::new();
        let project =
            orr_package::Project::open(&fixture.root, crate::project_runtime::compiled_runtime())
                .unwrap();
        let lock = project.verify().unwrap();
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version":1,"sprite":{"package":"sample-sprites","document":"sprites.json"},
            "width":2,"height":1,"tile_size":[16,16],"origin":[0,0],"units_per_pixel":1,
            "cells":[10,null]
        }))
        .unwrap();
        std::fs::write(fixture.root.join("map.json"), bytes).unwrap();
        (fixture, project, lock)
    }

    #[test]
    fn cache_reuse_budget_and_failure_are_transactional() {
        let (fixture, project, lock) = setup();
        let key: AssetKey = ("sample-sprites".into(), "sprites.json".into());
        let asset = load_project_asset(&project, &key.0, &key.1).unwrap();
        let mut assets = BTreeMap::from([(key.clone(), asset)]);
        let pointer = assets[&key].rgba.as_ptr();
        let (_, extra) =
            prepare(&project, &lock, &fixture.root, "map.json", &assets, 4095).unwrap();
        assert!(extra.is_none());
        assert_eq!(assets[&key].rgba.as_ptr(), pointer);
        for count in [4096, usize::MAX] {
            assert!(prepare(&project, &lock, &fixture.root, "map.json", &assets, count).is_err());
            assert_eq!(assets[&key].rgba.as_ptr(), pointer);
        }
        // Synthetic keys exercise admission arithmetic only; production maps
        // are caller-private fresh verified actor maps, never arbitrary caches.
        for n in 0..7 {
            assets.insert(
                (format!("other-{n}"), "sprites.json".into()),
                assets[&key].clone(),
            );
        }
        assert!(prepare(&project, &lock, &fixture.root, "map.json", &assets, 0).is_ok());
        assets.insert(
            ("ninth".into(), "sprites.json".into()),
            assets[&key].clone(),
        );
        assert!(prepare(&project, &lock, &fixture.root, "map.json", &assets, 0).is_err());
        assert_eq!(assets.len(), 9);
        assets.remove(&key);
        assert!(prepare(&project, &lock, &fixture.root, "map.json", &assets, 0).is_err());
        assets.remove(&("ninth".into(), "sprites.json".into()));
        let (_, extra) = prepare(&project, &lock, &fixture.root, "map.json", &assets, 0).unwrap();
        assert_eq!(extra.unwrap().0, key);
        assert_eq!(assets.len(), 7);
    }

    #[test]
    fn cache_hit_still_checks_lock_tamper_and_installer_authority() {
        let (fixture, project, lock) = setup();
        let key: AssetKey = ("sample-sprites".into(), "sprites.json".into());
        let assets = BTreeMap::from([(
            key.clone(),
            load_project_asset(&project, &key.0, &key.1).unwrap(),
        )]);
        assert!(prepare(
            &project,
            &lock,
            &fixture.root.join("other"),
            "map.json",
            &assets,
            0
        )
        .is_err());
        for relative in [
            "../map.json",
            "./map.json",
            "nested/map.json",
            "",
            "map\\bad.json",
        ] {
            assert!(prepare(&project, &lock, &fixture.root, relative, &assets, 0).is_err());
        }
        let mut wrong = lock.clone();
        wrong.direct.clear();
        assert!(prepare(&project, &wrong, &fixture.root, "map.json", &assets, 0).is_err());
        let installer = orr_package::Project::open_for_install(
            &fixture.root,
            crate::project_runtime::compiled_runtime().engine_version,
        )
        .unwrap();
        assert!(prepare(&installer, &lock, &fixture.root, "map.json", &assets, 0).is_err());
        let object = fixture
            .root
            .join(".orr/packages/objects")
            .join(&lock.packages["sample-sprites"].digest);
        std::fs::write(object.join("lantern_keeper.png"), b"tampered").unwrap();
        assert!(prepare(&project, &lock, &fixture.root, "map.json", &assets, 0).is_err());
        assert_eq!(assets.len(), 1);
    }
}
