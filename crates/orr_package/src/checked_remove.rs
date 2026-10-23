//! Checked direct-selection removal, retaining all cached and source bytes.
use crate::{fail, Error, Lock, Project, Result};

impl Project {
    /// Remove precisely the previewed direct selection under the existing writer
    /// guard. Both the complete current lock and newly prepared candidate must
    /// match the caller's observations before the callback can run.
    ///
    /// The callback runs immediately before lock publication and can reject stale
    /// consumer documents. It must not mutate package authority. Consumer reference
    /// validation is its responsibility: package resolution alone does not prove
    /// that a scene remains usable. External document writers are not isolated.
    ///
    /// A package still required transitively remains active. No cached object or
    /// source file is removed, and failures before publication retain the old lock.
    pub fn remove_checked(
        &self,
        name: &str,
        expected: &Lock,
        approved: &Lock,
        before_publish: impl FnOnce() -> std::result::Result<(), String>,
    ) -> Result<Lock> {
        self.require_mutable_presentation()?;
        let _guard = self.writer()?;
        let previous = self.verify()?;
        if &previous != expected {
            return fail("active package lock changed since preparation");
        }
        let candidate = self.prepare_remove(previous, name)?;
        if &candidate != approved {
            return fail("candidate package lock differs from preflighted removal");
        }
        before_publish().map_err(Error)?;
        self.require_mutable_presentation()?;
        self.publish(&candidate)?;
        Ok(candidate)
    }
}
