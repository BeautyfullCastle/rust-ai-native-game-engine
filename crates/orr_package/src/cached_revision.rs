//! Identity-pinned selection of one retained project-local package object.
use crate::{fail, hex_digest, Lock, PackagePreview, Project, Result};
use std::path::PathBuf;

impl Project {
    /// Preview selecting one cached package directly by its exact content identity.
    /// Dependencies must already be active at the required exact versions. This
    /// reads and verifies content but does not acquire a writer or reserve a future
    /// apply. It neither enumerates the cache nor repairs corrupt active packages.
    pub fn preview_restore_cached(&self, name: &str, digest: &str) -> Result<PackagePreview> {
        self.prepare_cached_revision(name, digest)
            .map(|(_, preview)| preview)
    }

    /// Select one retained package object without its original authoring source.
    /// The internally prepared previous and candidate locks are checked again
    /// under the existing writer guard. A separately obtained preview is only an
    /// observation; this call prepares its own fresh checked transaction.
    pub fn restore_cached(&self, name: &str, digest: &str) -> Result<Lock> {
        let (source, preview) = self.prepare_cached_revision(name, digest)?;
        self.install_checked(&[source], &preview.previous, &preview.candidate, || Ok(()))
    }

    fn prepare_cached_revision(
        &self,
        name: &str,
        digest: &str,
    ) -> Result<(PathBuf, PackagePreview)> {
        crate::name(name)?;
        if !hex_digest(digest) {
            return fail("cached package digest must be 64 lowercase hexadecimal characters");
        }
        // Project::open captured a canonical root. The existing snapshot path
        // rejects links/nonregular files and enforces all content bounds. Never
        // infer identity from the directory name alone or scan sibling objects.
        let source = self.root().join(".orr/packages/objects").join(digest);
        let preview = self.preview_install(std::slice::from_ref(&source))?;
        if !preview
            .candidate
            .packages
            .get(name)
            .is_some_and(|package| package.manifest.name == name && package.digest == digest)
        {
            return fail("cached package name or content digest does not match the request");
        }
        Ok((source, preview))
    }
}
