//! Offline package install/reopen/read_asset boundary. The consuming host's real
//! compiled inventory supplies terrain_v1/navigation_v1; package JSON never does.
use crate::Result;
use orr_fp::FP;
use orr_navigation::{AgentProfile, TerrainGraph};
use orr_package::{Manifest, Project, Runtime};
use orr_terrain::Terrain;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub const PACKAGE_NAME: &str = "navigation-lab";
pub const TERRAIN_PATH: &str = "lab.orrt";
pub const NAVIGATION_PATH: &str = "lab.orrnav";
pub const SCENARIO_PATH: &str = "scenario.json";
pub const CAPABILITY: &str = "navigation_v1";
pub const NAVIGATION_ASSET_ID: &str = "navigation/navigation-lab.orrnav";
pub const SCENARIO_ASSET_ID: &str = "scenarios/navigation-lab.json";
const MAX_SCENARIO_BYTES: usize = 8192;

/// Raw Q48.16 coordinates preserve exact values across the package boundary.
/// This bounded prototype accepts only a zero-radius/headroom/step point agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub format: String,
    pub version: u32,
    pub asset_id: String,
    pub terrain_asset_id: String,
    pub terrain_revision: [u8; 32],
    pub navigation_asset_id: String,
    pub navigation_revision: [u8; 32],
    pub start_raw: [i64; 2],
    pub goal_raw: [i64; 2],
    pub max_slope_raw: i64,
    pub radius_raw: i64,
    pub headroom_raw: i64,
    pub max_step_raw: i64,
    pub distance_per_tick_raw: i64,
}
impl Scenario {
    pub fn new(
        terrain: &Terrain,
        graph: &TerrainGraph,
        start: [FP; 2],
        goal: [FP; 2],
        distance_per_tick: FP,
    ) -> Self {
        let profile = graph.profile();
        Self {
            format: "orr_navigation_lab".into(),
            version: 1,
            asset_id: SCENARIO_ASSET_ID.into(),
            terrain_asset_id: terrain.asset_id().into(),
            terrain_revision: terrain.revision(),
            navigation_asset_id: NAVIGATION_ASSET_ID.into(),
            navigation_revision: graph.revision(),
            start_raw: start.map(FP::raw),
            goal_raw: goal.map(FP::raw),
            max_slope_raw: profile.max_slope.raw(),
            radius_raw: profile.radius.raw(),
            headroom_raw: profile.headroom.raw(),
            max_step_raw: profile.max_step.raw(),
            distance_per_tick_raw: distance_per_tick.raw(),
        }
    }
    pub fn profile(&self) -> AgentProfile {
        AgentProfile {
            max_slope: FP::from_raw(self.max_slope_raw),
            radius: FP::from_raw(self.radius_raw),
            headroom: FP::from_raw(self.headroom_raw),
            max_step: FP::from_raw(self.max_step_raw),
        }
    }
    pub fn start(&self) -> [FP; 2] {
        self.start_raw.map(FP::from_raw)
    }
    pub fn goal(&self) -> [FP; 2] {
        self.goal_raw.map(FP::from_raw)
    }
    pub fn distance_per_tick(&self) -> FP {
        FP::from_raw(self.distance_per_tick_raw)
    }
    fn validate(&self, terrain: &Terrain) -> Result<()> {
        if self.format != "orr_navigation_lab"
            || self.version != 1
            || self.asset_id != SCENARIO_ASSET_ID
            || self.navigation_asset_id != NAVIGATION_ASSET_ID
        {
            return Err("unsupported navigation lab scenario format/identity".into());
        }
        if self.terrain_asset_id != terrain.asset_id()
            || self.terrain_revision != terrain.revision()
        {
            return Err("scenario terrain dependency identity/revision mismatch".into());
        }
        if self.distance_per_tick_raw <= 0 {
            return Err("scenario distance per tick must be positive".into());
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct InstalledNavigation {
    pub terrain: Terrain,
    pub graph: TerrainGraph,
    pub scenario: Scenario,
    pub package_digest: String,
}
pub fn runtime() -> Runtime {
    let mut runtime = Runtime::content_only();
    runtime
        .capabilities
        .extend(["terrain_v1".into(), CAPABILITY.into()]);
    runtime
}
fn write_source(
    terrain: &Terrain,
    graph: &TerrainGraph,
    scenario: &Scenario,
    new_source_root: &Path,
    version: &str,
) -> Result<()> {
    fs::create_dir(new_source_root)?;
    fs::write(new_source_root.join(TERRAIN_PATH), terrain.cook())?;
    fs::write(new_source_root.join(NAVIGATION_PATH), graph.cook())?;
    fs::write(
        new_source_root.join(SCENARIO_PATH),
        serde_json::to_vec_pretty(scenario)?,
    )?;
    let manifest = Manifest {
        schema: 1,
        name: PACKAGE_NAME.into(),
        version: version.into(),
        engine: format!("={}", env!("CARGO_PKG_VERSION")),
        capabilities: BTreeSet::from(["terrain_v1".into(), CAPABILITY.into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from([
            TERRAIN_PATH.into(),
            NAVIGATION_PATH.into(),
            SCENARIO_PATH.into(),
        ]),
    };
    fs::write(
        new_source_root.join("orr.package.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}

/// Build/cook to a NEW source directory, actually install, then reopen and load
/// each asset exclusively from verified immutable installed package bytes.
pub fn install_and_reload(
    terrain: &Terrain,
    project_root: &Path,
    new_source_root: &Path,
    version: &str,
) -> Result<InstalledNavigation> {
    let graph = TerrainGraph::build(terrain, AgentProfile::default())?;
    let scenario = Scenario::new(
        terrain,
        &graph,
        crate::start(),
        crate::goal(),
        FP::from_raw(32768),
    );
    fs::create_dir_all(project_root)?;
    write_source(terrain, &graph, &scenario, new_source_root, version)?;
    Project::open(project_root, runtime())?.install(&[new_source_root.to_path_buf()])?;
    let loaded = load_installed(project_root)?;
    if loaded.terrain.cook() != terrain.cook()
        || loaded.graph.cook() != graph.cook()
        || loaded.scenario != scenario
    {
        return Err("installed navigation assets differ from source cooked bytes".into());
    }
    Ok(loaded)
}
/// Reopen as the consuming host; never trust an install-tool-only runtime.
pub fn load_installed(project_root: &Path) -> Result<InstalledNavigation> {
    let project = Project::open(project_root, runtime())?;
    let terrain = Terrain::load(&project.read_asset(PACKAGE_NAME, TERRAIN_PATH)?)?;
    let bytes = project.read_asset(PACKAGE_NAME, SCENARIO_PATH)?;
    if bytes.len() > MAX_SCENARIO_BYTES {
        return Err("navigation scenario exceeds byte budget".into());
    }
    let scenario: Scenario = serde_json::from_slice(&bytes)?;
    scenario.validate(&terrain)?;
    let graph = TerrainGraph::load(
        &project.read_asset(PACKAGE_NAME, NAVIGATION_PATH)?,
        &terrain,
        scenario.profile(),
    )?;
    if graph.revision() != scenario.navigation_revision {
        return Err("scenario cooked navigation revision mismatch".into());
    }
    graph.project(&terrain, scenario.start())?;
    graph.project(&terrain, scenario.goal())?;
    let lock = project.list()?;
    Ok(InstalledNavigation {
        terrain,
        graph,
        scenario,
        package_digest: lock.packages[PACKAGE_NAME].digest.clone(),
    })
}

/// Runnable lab evidence using disposable, real installed package projects.
/// Deliberate corruption is confined to this newly created evidence directory.
pub fn exercise_failure_cases(new_root: &Path) -> Result<serde_json::Value> {
    fs::create_dir(new_root)?;
    let terrain = crate::fixture();
    let tamper_root = new_root.join("tamper-project");
    let installed = install_and_reload(
        &terrain,
        &tamper_root,
        &new_root.join("tamper-source"),
        "1.0.0",
    )?;
    let nav_object = tamper_root
        .join(".orr/packages/objects")
        .join(&installed.package_digest)
        .join(NAVIGATION_PATH);
    let mut permissions = fs::metadata(&nav_object)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o200);
    }
    #[cfg(windows)]
    permissions.set_readonly(false);
    fs::set_permissions(&nav_object, permissions)?;
    fs::write(
        nav_object,
        b"deliberately tampered fixture cooked navigation",
    )?;
    let tamper_error = load_installed(&tamper_root)
        .err()
        .ok_or("tampered navigation unexpectedly loaded")?
        .to_string();
    let mut document = orr_terrain::TerrainDocument::new(terrain.clone());
    document.apply(&[orr_terrain::Edit::SetHole {
        x: 3,
        z: 6,
        hole: true,
    }])?;
    let new_graph = TerrainGraph::build(document.terrain(), AgentProfile::default())?;
    let wrong_root = new_root.join("wrong-dependency-project");
    fs::create_dir(&wrong_root)?;
    let wrong_source = new_root.join("wrong-dependency-source");
    let scenario = Scenario::new(
        document.terrain(),
        &new_graph,
        crate::start(),
        crate::goal(),
        FP::ONE,
    );
    write_source(
        document.terrain(),
        &installed.graph,
        &scenario,
        &wrong_source,
        "1.0.0",
    )?;
    Project::open(&wrong_root, runtime())?.install(&[wrong_source])?;
    let dependency_error = load_installed(&wrong_root)
        .err()
        .ok_or("wrong terrain dependency unexpectedly loaded")?
        .to_string();
    let missing_root = new_root.join("missing-project");
    install_and_reload(
        &terrain,
        &missing_root,
        &new_root.join("missing-source"),
        "1.0.0",
    )?;
    let missing = Project::open(&missing_root, runtime())?;
    let undeclared_error = missing
        .read_asset(PACKAGE_NAME, "missing.orrnav")
        .unwrap_err()
        .to_string();
    let capability_error = Project::open(&missing_root, Runtime::content_only())?
        .read_asset(PACKAGE_NAME, NAVIGATION_PATH)
        .unwrap_err()
        .to_string();
    missing.remove(PACKAGE_NAME)?;
    let missing_error = load_installed(&missing_root)
        .err()
        .ok_or("removed package unexpectedly loaded")?
        .to_string();
    Ok(
        serde_json::json!({"tamper_rejected":tamper_error,"wrong_cooked_dependency_rejected":dependency_error,"undeclared_asset_rejected":undeclared_error,"missing_host_capability_rejected":capability_error,"removed_package_rejected":missing_error}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_terrain::{Edit, TerrainDocument};
    fn fixture_tempdir() -> tempfile::TempDir {
        // Resolve only the trusted system-temp root before creating fixture content.
        // macOS may use /var -> /private/var; package inputs must still reject links.
        let temp_root = std::env::temp_dir();
        #[cfg(unix)]
        let temp_root = std::fs::canonicalize(temp_root).unwrap();
        tempfile::tempdir_in(temp_root).unwrap()
    }
    #[test]
    fn actual_install_reload_identity_source_isolation_missing_and_capability_checks() {
        let tmp = fixture_tempdir();
        let root = tmp.path().join("project");
        let source = tmp.path().join("source");
        let first = install_and_reload(&crate::fixture(), &root, &source, "1.0.0").unwrap();
        for path in [TERRAIN_PATH, NAVIGATION_PATH, SCENARIO_PATH] {
            fs::write(source.join(path), b"source changed after install").unwrap();
        }
        let loaded = load_installed(&root).unwrap();
        assert_eq!(loaded.terrain, first.terrain);
        assert_eq!(loaded.graph.cook(), first.graph.cook());
        assert_eq!(loaded.scenario, first.scenario);
        assert_eq!(loaded.scenario.asset_id, SCENARIO_ASSET_ID);
        assert_eq!(loaded.scenario.navigation_asset_id, NAVIGATION_ASSET_ID);
        assert!(Project::open(&root, Runtime::content_only())
            .unwrap()
            .read_asset(PACKAGE_NAME, NAVIGATION_PATH)
            .is_err());
        let mut only_terrain = Runtime::content_only();
        only_terrain.capabilities.insert("terrain_v1".into());
        assert!(Project::open(&root, only_terrain)
            .unwrap()
            .read_asset(PACKAGE_NAME, NAVIGATION_PATH)
            .is_err());
        let mut only_navigation = Runtime::content_only();
        only_navigation.capabilities.insert(CAPABILITY.into());
        assert!(Project::open(&root, only_navigation)
            .unwrap()
            .read_asset(PACKAGE_NAME, TERRAIN_PATH)
            .is_err());
        let project = Project::open(&root, runtime()).unwrap();
        assert!(project
            .read_asset(PACKAGE_NAME, "undeclared.orrnav")
            .is_err());
        project.remove(PACKAGE_NAME).unwrap();
        assert!(load_installed(&root)
            .unwrap_err()
            .to_string()
            .contains("missing package"));
    }
    #[test]
    fn missing_declared_installed_cooked_navigation_file_is_rejected() {
        let tmp = fixture_tempdir();
        let root = tmp.path().join("project");
        let loaded = install_and_reload(
            &crate::fixture(),
            &root,
            &tmp.path().join("source"),
            "1.0.0",
        )
        .unwrap();
        let object = root
            .join(".orr/packages/objects")
            .join(loaded.package_digest);
        let mut permissions = fs::metadata(&object).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() | 0o200);
        }
        #[cfg(windows)]
        permissions.set_readonly(false);
        fs::set_permissions(&object, permissions).unwrap();
        fs::remove_file(object.join(NAVIGATION_PATH)).unwrap();
        assert!(load_installed(&root).is_err());
    }
    #[test]
    fn installed_cooked_navigation_tamper_is_rejected() {
        let tmp = fixture_tempdir();
        let root = tmp.path().join("project");
        let loaded = install_and_reload(
            &crate::fixture(),
            &root,
            &tmp.path().join("source"),
            "1.0.0",
        )
        .unwrap();
        let installed = root
            .join(".orr/packages/objects")
            .join(loaded.package_digest)
            .join(NAVIGATION_PATH);
        let mut permissions = fs::metadata(&installed).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() | 0o200);
        }
        #[cfg(windows)]
        permissions.set_readonly(false);
        fs::set_permissions(&installed, permissions).unwrap();
        fs::write(installed, b"tampered cooked graph").unwrap();
        assert!(load_installed(&root)
            .unwrap_err()
            .to_string()
            .contains("installed content changed"));
    }
    #[test]
    fn valid_package_with_wrong_cooked_terrain_dependency_is_rejected() {
        let tmp = fixture_tempdir();
        let root = tmp.path().join("project");
        fs::create_dir(&root).unwrap();
        let original = crate::fixture();
        let old_graph = TerrainGraph::build(&original, AgentProfile::default()).unwrap();
        let mut doc = TerrainDocument::new(original);
        doc.apply(&[Edit::SetHole {
            x: 3,
            z: 6,
            hole: true,
        }])
        .unwrap();
        let new_graph = TerrainGraph::build(doc.terrain(), AgentProfile::default()).unwrap();
        let scenario = Scenario::new(
            doc.terrain(),
            &new_graph,
            crate::start(),
            crate::goal(),
            FP::ONE,
        );
        let source = tmp.path().join("wrong-dependency");
        // All package hashes are valid, but this old graph binds another terrain.
        write_source(doc.terrain(), &old_graph, &scenario, &source, "1.0.0").unwrap();
        Project::open(&root, runtime())
            .unwrap()
            .install(&[source])
            .unwrap();
        assert!(load_installed(&root).is_err());
    }
    #[test]
    fn edited_package_keeps_logical_ids_changes_content_and_rejects_scenario_revision_mismatch() {
        let tmp = fixture_tempdir();
        let root = tmp.path().join("project");
        let first = install_and_reload(
            &crate::fixture(),
            &root,
            &tmp.path().join("source-v1"),
            "1.0.0",
        )
        .unwrap();
        let mut doc = TerrainDocument::new(first.terrain.clone());
        doc.apply(&[Edit::SetHole {
            x: 3,
            z: 6,
            hole: true,
        }])
        .unwrap();
        let edited =
            install_and_reload(doc.terrain(), &root, &tmp.path().join("source-v2"), "1.0.1")
                .unwrap();
        assert_eq!(first.terrain.asset_id(), edited.terrain.asset_id());
        assert_eq!(first.scenario.asset_id, edited.scenario.asset_id);
        assert_eq!(
            first.scenario.navigation_asset_id,
            edited.scenario.navigation_asset_id
        );
        assert_ne!(first.package_digest, edited.package_digest);
        assert_ne!(first.graph.revision(), edited.graph.revision());
        let source = tmp.path().join("wrong-scenario");
        write_source(
            &edited.terrain,
            &edited.graph,
            &first.scenario,
            &source,
            "1.0.2",
        )
        .unwrap();
        Project::open(&root, runtime())
            .unwrap()
            .install(&[source])
            .unwrap();
        assert!(load_installed(&root)
            .unwrap_err()
            .to_string()
            .contains("scenario terrain dependency"));
    }
}
