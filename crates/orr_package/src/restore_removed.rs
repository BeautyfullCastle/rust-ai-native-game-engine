//! Exact inverse of one direct-selection removal, using retained local objects.
use crate::{fail, Lock, PackagePreview, Project, Result};
use std::path::PathBuf;

impl Project {
    /// Observe restoring exactly one removed direct selection and its former
    /// dependencies. Both lock identities are supplied by the consumer's removal
    /// receipt; this does not discover cache entries or reserve a future commit.
    pub fn preview_restore_removed(
        &self,
        name: &str,
        expected: &Lock,
        restore: &Lock,
    ) -> Result<PackagePreview> {
        let (source, candidates) = self.removed_restore_sources(name, expected, restore)?;
        let current = self.verify()?;
        if &current != expected {
            return fail("active package lock changed since removal");
        }
        let prepared = self.prepare_install(current.clone(), &[source], &candidates)?;
        if &prepared.lock != restore {
            return fail("cached content differs from the removed package identities");
        }
        Ok(PackagePreview::between(current, prepared.lock))
    }

    /// Restore the exact inverse of a removal under the existing writer guard.
    /// The complete active lock must equal `expected`, and freshly read cached
    /// objects must reconstruct `restore` exactly, including direct selections.
    ///
    /// The callback runs immediately before publication with the old lock still
    /// active. Consumer document identity/reference validation belongs there;
    /// external document writers are not isolated. No source directory is read,
    /// and neither sources nor cached objects are deleted. This does not repair
    /// corrupt active content or perform arbitrary whole-lock rollback.
    pub fn restore_removed_checked(
        &self,
        name: &str,
        expected: &Lock,
        restore: &Lock,
        before_publish: impl FnOnce() -> std::result::Result<(), String>,
    ) -> Result<Lock> {
        let (source, candidates) = self.removed_restore_sources(name, expected, restore)?;
        self.install_transaction(
            &[source],
            &candidates,
            Some(expected),
            Some(restore),
            before_publish,
        )
    }

    fn removed_restore_sources(
        &self,
        name: &str,
        expected: &Lock,
        restore: &Lock,
    ) -> Result<(PathBuf, Vec<PathBuf>)> {
        // Validate structure, capabilities, identities, path components and size
        // before allowing any caller-supplied digest to contribute to a path.
        self.encode_lock(expected)?;
        self.encode_lock(restore)?;
        if self.prepare_remove(restore.clone(), name)? != *expected {
            return fail("restoration must exactly invert one direct package removal");
        }
        let source = self.object(&restore.packages[name]);
        let candidates = restore
            .packages
            .iter()
            .filter(|(package_name, _)| {
                package_name.as_str() != name && !expected.packages.contains_key(*package_name)
            })
            .map(|(_, package)| self.object(package))
            .collect();
        Ok((source, candidates))
    }
}
