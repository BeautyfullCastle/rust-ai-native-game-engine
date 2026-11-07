//! Keep one exact installed dependency as a direct selection, without source discovery.
use crate::{fail,Error,Lock,PackagePreview,Project,Result};
impl Project {
    /// Verify the entire installed lock and preview one indirect-to-direct change.
    /// Every package identity and existing direct selection stays unchanged. This
    /// reads no original source, creates no writer and does not reserve a commit.
    pub fn preview_keep_dependency(&self,name:&str)->Result<PackagePreview> {
        let previous=self.verify()?;
        let candidate=self.prepare_keep_dependency(previous.clone(),name)?;
        Ok(PackagePreview::between(previous,candidate))
    }
    /// Publish exactly the previewed selection change under the existing writer.
    /// All installed bytes and both locks are checked before the callback. It
    /// may reject stale consumer documents but must not mutate package authority.
    /// No object is written, removed, discovered or repaired. Existing lock
    /// publication/durability and trusted-directory concurrency limits apply.
    pub fn keep_dependency_checked(&self,name:&str,expected:&Lock,approved:&Lock,before_publish:impl FnOnce()->std::result::Result<(),String>)->Result<Lock> {
        self.require_mutable_presentation()?;
        let _guard=self.writer()?;
        let previous=self.verify()?;
        if &previous!=expected {return fail("active package lock changed since preparation");}
        let candidate=self.prepare_keep_dependency(previous,name)?;
        if &candidate!=approved {return fail("candidate package lock differs from preflighted dependency selection");}
        before_publish().map_err(Error)?;
        self.require_mutable_presentation()?;
        self.publish(&candidate)?;
        Ok(candidate)
    }
    fn prepare_keep_dependency(&self,mut previous:Lock,name:&str)->Result<Lock> {
        crate::name(name)?;
        if previous.direct.contains_key(name) {return fail(format!("package is already directly selected: {name}"));}
        let package=previous.packages.get(name).ok_or_else(||Error(format!("not an installed dependency: {name}")))?;
        previous.direct.insert(name.to_owned(),package.manifest.version.clone());
        self.encode_lock(&previous)?;
        Ok(previous)
    }
}
