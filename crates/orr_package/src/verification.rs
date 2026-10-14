//! Explicitly scoped installed-byte verification; never an activation authority.
use crate::{fail, Error, PackageSelection, Project, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// One observation of a package and its transitive dependency payloads.
///
/// The entire active lock passed structural, identity, graph and runtime admission.
/// Only `verified_packages` had their stored manifests and declared bytes checked.
/// Entries in `unverified_packages` contain admitted lock metadata, not byte-integrity
/// evidence. This report is neither a whole-project validation nor a future-use token.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PackageVerification {
    pub requested_package: String,
    pub verified_packages: BTreeMap<String, PackageSelection>,
    pub unverified_packages: BTreeMap<String, PackageSelection>,
}

impl Project {
    /// Verify one active package and every dependency it transitively requires.
    ///
    /// Validates the complete lock before traversing its bounded, acyclic graph.
    /// Unrelated active payloads are deliberately not read. Does not create a writer,
    /// change activation, repair content, or replace `verify`/consumer admission.
    /// The same trusted-directory and concurrent-mutation limits as `verify` apply.
    pub fn verify_package(&self, name: &str) -> Result<PackageVerification> {
        crate::name(name)?;
        let lock = self.list()?;
        if !lock.packages.contains_key(name) {
            return fail(format!("missing package: {name}"));
        }
        let mut selected = BTreeSet::new();
        let mut pending = vec![name];
        while let Some(package) = pending.pop() {
            if selected.insert(package) {
                pending.extend(
                    lock.packages[package]
                        .manifest
                        .dependencies
                        .keys()
                        .map(String::as_str),
                );
            }
        }
        let mut verified_packages = BTreeMap::new();
        let mut unverified_packages = BTreeMap::new();
        for (package_name, package) in &lock.packages {
            let selection = PackageSelection {
                version: package.manifest.version.clone(),
                digest: package.digest.clone(),
                direct: lock.direct.contains_key(package_name),
            };
            if selected.contains(package_name.as_str()) {
                self.verify_object(package)
                    .map_err(|error| Error(format!("package {package_name}: {error}")))?;
                verified_packages.insert(package_name.clone(), selection);
            } else {
                unverified_packages.insert(package_name.clone(), selection);
            }
        }
        Ok(PackageVerification {
            requested_package: name.to_owned(),
            verified_packages,
            unverified_packages,
        })
    }
}
