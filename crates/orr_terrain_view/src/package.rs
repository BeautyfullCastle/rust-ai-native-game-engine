//! Actual offline package boundary for the lab. Never load the source directory
//! after installation: render/query the verified bytes returned by `read_asset`.
use orr_package::{Manifest, Project, Runtime};
use orr_terrain::Terrain;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub const PACKAGE_NAME: &str = "heightfield-lab";
pub const ASSET_PATH: &str = "lab.orrt";
pub const CAPABILITY: &str = "terrain_v1";
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct InstalledTerrain {
    pub terrain: Terrain,
    pub package_digest: String,
}

/// The lab's compiled terrain capability inventory, not a source-file claim.
pub fn runtime() -> Runtime {
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert(CAPABILITY.into());
    runtime
}

/// Cook to a *new* package-source directory, install into an existing/new project,
/// then load only verified installed bytes. Different content needs a new version.
/// Existing source directories are rejected rather than overwritten.
pub fn install_and_reload(
    terrain: &Terrain,
    project_root: &Path,
    new_source_root: &Path,
    version: &str,
) -> Result<InstalledTerrain> {
    fs::create_dir_all(project_root)?;
    fs::create_dir(new_source_root)?;
    let bytes = terrain.cook();
    fs::write(new_source_root.join(ASSET_PATH), &bytes)?;
    let manifest = Manifest {
        schema: 1,
        name: PACKAGE_NAME.into(),
        version: version.into(),
        engine: format!("={}", env!("CARGO_PKG_VERSION")),
        capabilities: BTreeSet::from([CAPABILITY.into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from([ASSET_PATH.into()]),
    };
    fs::write(
        new_source_root.join("orr.package.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    let project = Project::open(project_root, runtime())?;
    let lock = project.install(&[new_source_root.to_path_buf()])?;
    // Reopen as a consuming host, rather than relying on install-time state.
    let project = Project::open(project_root, runtime())?;
    let loaded = project.read_asset(PACKAGE_NAME, ASSET_PATH)?;
    let terrain = Terrain::load(&loaded)?;
    if loaded != bytes {
        return Err("installed terrain bytes differ from cooked bytes".into());
    }
    Ok(InstalledTerrain {
        terrain,
        package_digest: lock.packages[PACKAGE_NAME].digest.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_fp::FP;
    use orr_terrain::{Edit, TerrainDocument};
    #[test]
    fn edited_package_is_reloaded_isolated_capability_checked_and_tamper_checked() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        let source = tmp.path().join("source");
        let mut doc = TerrainDocument::new(crate::fixture());
        let first = install_and_reload(doc.terrain(), &root, &source, "1.0.0").unwrap();
        let project = Project::open(&root, runtime()).unwrap();
        fs::write(
            source.join(ASSET_PATH),
            b"source changed after installation",
        )
        .unwrap();
        assert_eq!(
            Terrain::load(&project.read_asset(PACKAGE_NAME, ASSET_PATH).unwrap()).unwrap(),
            first.terrain
        );
        assert!(Project::open(&root, Runtime::content_only())
            .unwrap()
            .read_asset(PACKAGE_NAME, ASSET_PATH)
            .is_err());
        let first_lock = project.list().unwrap();
        project.remove(PACKAGE_NAME).unwrap();
        assert!(project
            .read_asset(PACKAGE_NAME, ASSET_PATH)
            .unwrap_err()
            .to_string()
            .contains("missing package"));
        fs::write(source.join(ASSET_PATH), first.terrain.cook()).unwrap();
        assert_eq!(
            project.install(std::slice::from_ref(&source)).unwrap(),
            first_lock
        );
        assert_eq!(
            Terrain::load(&project.read_asset(PACKAGE_NAME, ASSET_PATH).unwrap()).unwrap(),
            first.terrain
        );
        doc.apply(&[
            Edit::SetHeight {
                x: 4,
                z: 4,
                height: FP::from_int(4),
            },
            Edit::SetHole {
                x: 1,
                z: 1,
                hole: true,
            },
        ])
        .unwrap();
        let next =
            install_and_reload(doc.terrain(), &root, &tmp.path().join("edited"), "1.0.1").unwrap();
        assert_ne!(first.package_digest, next.package_digest);
        assert_eq!(next.terrain, *doc.terrain());
        let installed = root
            .join(".orr/packages/objects")
            .join(&next.package_digest)
            .join(ASSET_PATH);
        // Fixture-local corruption is intentional; package objects are read-only.
        let mut permissions = fs::metadata(&installed).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() | 0o200);
        }
        #[cfg(windows)]
        permissions.set_readonly(false);
        fs::set_permissions(&installed, permissions).unwrap();
        fs::write(installed, b"tampered installed content").unwrap();
        assert!(project.read_asset(PACKAGE_NAME, ASSET_PATH).is_err());
    }
}
