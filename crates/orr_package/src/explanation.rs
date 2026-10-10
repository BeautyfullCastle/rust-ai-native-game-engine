use crate::{fail, Lock, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Read-only activation explanation from a validated lock, not an installed-byte
/// integrity report or evidence that a consuming host supports these capabilities.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PackageExplanation {
    pub name: String,
    pub version: String,
    pub digest: String,
    pub direct: bool,
    pub capabilities: BTreeSet<String>,
    /// Active packages that immediately depend on this package.
    pub required_by: BTreeSet<String>,
    /// One shortest chain (including both endpoints) per selecting root.
    /// Lexicographically ordered neighbors break ties deterministically.
    pub selected_by: BTreeMap<String, Vec<String>>,
}

// Only called after Project::list validates the complete bounded, acyclic graph.
pub(crate) fn explain(lock: &Lock, name: &str) -> Result<PackageExplanation> {
    let Some(package) = lock.packages.get(name) else {
        return fail(format!("missing package: {name}"));
    };
    let mut selected_by = BTreeMap::new();
    for root in lock.direct.keys() {
        // Store one parent per visited node, not every path through a diamond.
        let mut parents = BTreeMap::<&str, Option<&str>>::new();
        let mut queue = VecDeque::from([root.as_str()]);
        parents.insert(root, None);
        while let Some(node) = queue.pop_front() {
            if node == name {
                let mut chain = vec![node.to_owned()];
                let mut current = node;
                while let Some(Some(parent)) = parents.get(current) {
                    chain.push((*parent).to_owned());
                    current = parent;
                }
                chain.reverse();
                selected_by.insert(root.clone(), chain);
                break;
            }
            for dependency in lock.packages[node].manifest.dependencies.keys() {
                if !parents.contains_key(dependency.as_str()) {
                    parents.insert(dependency, Some(node));
                    queue.push_back(dependency);
                }
            }
        }
    }
    Ok(PackageExplanation {
        name: name.to_owned(),
        version: package.manifest.version.clone(),
        digest: package.digest.clone(),
        direct: lock.direct.contains_key(name),
        capabilities: package.manifest.capabilities.clone(),
        required_by: lock
            .packages
            .iter()
            .filter(|(_, p)| p.manifest.dependencies.contains_key(name))
            .map(|(name, _)| name.clone())
            .collect(),
        selected_by,
    })
}
