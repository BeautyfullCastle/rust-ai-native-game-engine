//! Exact installed dependency promotion with a pinned saved Room observation.
use super::{checked_directory,room_runtime};
use super::removal::{admit_candidate,Snapshot};
use orr_package::{Lock,PackagePreview,Project};
use std::path::{Path,PathBuf};
pub struct KeepPlan {root:PathBuf,name:String,preview:PackagePreview,snapshot:Snapshot}
impl KeepPlan {pub fn root(&self)->&Path {&self.root}pub fn name(&self)->&str {&self.name}pub fn preview(&self)->&PackagePreview {&self.preview}}
pub fn plan_keep(root:&Path,name:&str)->Result<KeepPlan,String> {
    let root=checked_directory(root,"Room project")?;let project=Project::open(&root,room_runtime()).map_err(|e|e.to_string())?;
    let preview=project.preview_keep_dependency(name).map_err(|e|e.to_string())?;
    let snapshot=admit_candidate(&root,&preview.previous,&preview.candidate)?;
    Ok(KeepPlan {root,name:name.into(),preview,snapshot})
}
pub fn apply_keep(plan:&KeepPlan)->Result<Lock,String> {
    let root=checked_directory(&plan.root,"Room project")?;if root!=plan.root {return Err("Room project changed; preview dependency keep again".into());}
    let project=Project::open(&root,room_runtime()).map_err(|e|e.to_string())?;
    let installed=project.keep_dependency_checked(&plan.name,&plan.preview.previous,&plan.preview.candidate,|| {
        if admit_candidate(&root,&plan.preview.previous,&plan.preview.candidate)?!=plan.snapshot {return Err("Saved Room files or project identity changed; preview dependency keep again".into());}Ok(())
    }).map_err(|e|e.to_string())?;
    let after=admit_candidate(&root,&installed,&installed).map_err(|e|format!("Dependency selection was published but Room re-admission failed; retained lock needs inspection: {e}"))?;
    if after!=plan.snapshot {return Err("Dependency selection was published but saved Room content changed afterward; retained lock needs inspection".into());}Ok(installed)
}
