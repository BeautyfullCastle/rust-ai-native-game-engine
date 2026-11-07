use super::{Panel,Completion,Lock,package_manager};
use std::{path::PathBuf,sync::mpsc};
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) struct Identity {root:PathBuf,project_generation:u64,generation:u64,name:String,digest:String}
#[derive(Default)]
pub(super) struct State {name:String,generation:u64,ready:Option<(Identity,package_manager::KeepPlan)>}
impl State {pub(super) fn invalidate(&mut self) {self.generation=self.generation.saturating_add(1);self.ready=None;}fn select(&mut self,name:String) {if self.name!=name {self.invalidate();self.name=name;}}}
impl Panel {
    fn keep_identity(&self)->Option<Identity> {let lock=self.verified.as_ref()?;let package=lock.packages.get(&self.keep.name)?;if lock.direct.contains_key(&self.keep.name) {return None;}Some(Identity {root:self.root.clone()?,project_generation:self.project_generation,generation:self.keep.generation,name:self.keep.name.clone(),digest:package.digest.clone()})}
    fn keep_ready(&self)->bool {self.keep.ready.as_ref().is_some_and(|(id,p)|self.keep_identity().as_ref()==Some(id) && self.verified.as_ref()==Some(&p.preview().previous))}
    pub(super) fn show_keep(&mut self,ui:&mut egui::Ui,can_operate:bool) {
        if self.keep.ready.is_some() && !self.keep_ready() {self.keep.invalidate();}
        ui.label("Keep an installed dependency as a direct selection (same version and bytes):");
        let names:Vec<_>=self.verified.as_ref().map(|lock|lock.packages.keys().filter(|name|!lock.direct.contains_key(*name)).cloned().collect()).unwrap_or_default();
        for name in names {if ui.add_enabled(can_operate,egui::Button::new(format!("Select keep {name}"))).clicked() {self.keep.select(name);}}
        ui.label(format!("Keep selection: {}",self.keep.name));
        if ui.add_enabled(can_operate && self.keep_identity().is_some(),egui::Button::new("Preview dependency keep")).clicked() {self.start_keep_preview();}
        if ui.add_enabled(can_operate && self.keep_ready(),egui::Button::new("Keep previewed dependency")).clicked() {self.start_keep();}
        if let Some((_,plan))=&self.keep.ready {ui.label(format!("Keep preview only: {}. No selection changed; parent removal is separate.",plan.name()));super::lock_table(ui,"keep-dependency-preview",&plan.preview().candidate);}
    }
    fn start_keep_preview(&mut self) {
        if self.busy() {return;}self.keep.invalidate();let Some(identity)=self.keep_identity() else {self.status="Verify installed packages and select an indirect package first.".into();return;};self.usage.invalidate();self.update_impact.invalidate();
        let (sender,receiver)=mpsc::channel();match std::thread::Builder::new().name("orr-package-keep-preview".into()).spawn(move|| {let result=package_manager::plan_keep(&identity.root,&identity.name);let _=sender.send(Completion::KeepPreviewed {identity,result});}) {Ok(_)=>{self.pending=Some(receiver);self.status="Checking saved Room files and previewing exact dependency keep…".into();},Err(e)=>self.status=format!("Cannot start dependency keep preview: {e}")}
    }
    pub(super) fn finish_keep_preview(&mut self,identity:Identity,result:Result<package_manager::KeepPlan,String>) {
        self.keep.ready=None;if self.keep_identity().as_ref()!=Some(&identity) {self.status="Dependency keep preview completed for a prior project or selection; discarded.".into();return;}
        match result {Ok(plan) if plan.root()==identity.root && plan.name()==identity.name && self.verified.as_ref()==Some(&plan.preview().previous)=>{self.keep.ready=Some((identity,plan));self.status="Dependency keep preview ready; review before explicit keep.".into();},Ok(_)=>self.status="Verified lock or keep identity changed; verify and preview again.".into(),Err(e)=>self.status=format!("Dependency keep preview failed: {e}")}
    }
    fn start_keep(&mut self) {
        if self.busy() {return;}
        if !self.keep_ready() {self.keep.invalidate();self.status="Project, selection or verified lock changed; preview keep again.".into();return;}
        let Some((identity,plan))=self.keep.ready.take() else {return;};self.usage.invalidate();self.update_impact.invalidate();self.model_request=None;self.preview_request=None;
        let (sender,receiver)=mpsc::channel();match std::thread::Builder::new().name("orr-package-keep".into()).spawn(move|| {let result=package_manager::apply_keep(&plan);let _=sender.send(Completion::Kept {identity,result});}) {Ok(_)=>{self.pending=Some(receiver);self.keeping=true;self.status="Keeping exact installed dependency with saved Room revalidation…".into();},Err(e)=>self.status=format!("Cannot start dependency keep: {e}")}
    }
    pub(super) fn finish_keep(&mut self,identity:Identity,result:Result<Lock,String>) {
        let current=self.root.as_ref()==Some(&identity.root) && self.project_generation==identity.project_generation;self.invalidate_package_cache();let target=if current {"current project".into()}else{format!("prior project {}",identity.root.display())};self.status=match result {Ok(_)=>format!("Dependency {} kept for {target}. Verify installed packages before further operations; no parent was removed.",identity.name),Err(e)=>format!("Dependency keep for {target} failed or could not be confirmed: {e}. Verify installed packages; no automatic retry or rollback was performed.")};
    }
}
#[cfg(all(test,feature="project-create",target_os="linux"))]
mod tests;
