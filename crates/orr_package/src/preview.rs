//! Read-only package activation differences. A preview does not reserve a writer.
use crate::Lock;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Exact content identity and selection status at one observed point in time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackageSelection {
    pub version: String,
    pub digest: String,
    pub direct: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackageChange {
    pub before: Option<PackageSelection>,
    pub after: Option<PackageSelection>,
}

/// Observed, validated before/after locks and deterministic changed identities.
/// This report is not an apply token, permission check, scene-reference audit or
/// disk-reclamation promise. Mutations must revalidate under their writer guard.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackagePreview {
    pub previous: Lock,
    pub candidate: Lock,
    pub changes: BTreeMap<String, PackageChange>,
}

impl PackagePreview {
    pub(crate) fn between(previous: Lock, candidate: Lock) -> Self {
        fn selection(lock: &Lock, name: &str) -> Option<PackageSelection> {
            lock.packages.get(name).map(|package| PackageSelection {
                version: package.manifest.version.clone(),
                digest: package.digest.clone(),
                direct: lock.direct.contains_key(name),
            })
        }
        let names: BTreeSet<_> = previous.packages.keys().chain(candidate.packages.keys()).collect();
        let changes = names.into_iter().filter_map(|name| {
            let before = selection(&previous, name);
            let after = selection(&candidate, name);
            (before != after).then(|| (name.clone(), PackageChange { before, after }))
        }).collect();
        Self { previous, candidate, changes }
    }
}

#[cfg(test)]
mod tests {
    use crate::{identity, Lock, LockedPackage, Manifest, Project, Runtime};
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn planners_reject_oversized_candidate_before_any_publication() {
        let temp_root = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let root = tempfile::tempdir_in(temp_root).unwrap();
        let project = Project::open(root.path(), Runtime::content_only()).unwrap();
        let mut old = Lock::default();
        for index in 0..8 {
            let name = format!("package-{index}");
            let files: BTreeMap<_, _> = (0..4096)
                .map(|file| (format!("asset-{file:04}.bin"), "0".repeat(64)))
                .collect();
            let mut package = LockedPackage {
                manifest: Manifest {
                    schema: 1, name: name.clone(), version: "1.0.0".into(),
                    engine: "*".into(), capabilities: BTreeSet::new(),
                    dependencies: BTreeMap::new(), files: files.keys().cloned().collect(),
                },
                digest: String::new(), files,
            };
            package.digest = identity(&package).unwrap();
            old.direct.insert(name.clone(), "1.0.0".into());
            old.packages.insert(name, package);
        }
        project.validate_lock(&old).unwrap();
        assert!(project.prepare_remove(old.clone(), "package-0").unwrap_err()
            .to_string().contains("lock byte limit exceeded"));
        let source = tempfile::tempdir_in(root.path()).unwrap();
        let manifest = Manifest {
            schema: 1, name: "new-package".into(), version: "1.0.0".into(),
            engine: "*".into(), capabilities: BTreeSet::new(),
            dependencies: BTreeMap::new(), files: BTreeSet::new(),
        };
        std::fs::write(source.path().join("orr.package.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = project.prepare_install(old, &[source.path().to_path_buf()], &[])
            .err().expect("oversized candidate must fail");
        assert!(error.to_string().contains("lock byte limit exceeded"));
        assert!(!root.path().join(".orr").exists());
        assert!(!root.path().join("orr.packages.lock.json").exists());
    }
}
