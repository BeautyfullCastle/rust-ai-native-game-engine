mod common_keep;
use common_keep::*;
use orr_package::{Project,Runtime};
use std::{cell::Cell,fs,path::PathBuf};
#[test]
fn preview_and_commit_preserve_exact_objects_and_keep_after_parent_removal_without_sources() {
    let f=Fixture::new();fs::rename(&f.dep,f.temp.path().join("hidden dep")).unwrap();fs::rename(&f.parent,f.temp.path().join("hidden parent")).unwrap();let before=files(f.temp.path());let p=f.project.preview_keep_dependency("dep").unwrap();assert_eq!(p.previous,f.old);assert_eq!(p.candidate.packages,f.old.packages);let mut direct=f.old.direct.clone();direct.insert("dep".into(),"1.0.0".into());assert_eq!(p.candidate.direct,direct);assert_eq!(p.changes.len(),1);assert!(!p.changes["dep"].before.as_ref().unwrap().direct);assert!(p.changes["dep"].after.as_ref().unwrap().direct);assert_eq!(files(f.temp.path()),before);
    let committed=f.project.keep_dependency_checked("dep",&p.previous,&p.candidate,||Ok(())).unwrap();assert_eq!(committed,p.candidate);let mut after=files(f.temp.path());let lock=PathBuf::from("project with spaces/orr.packages.lock.json");after.insert(lock.clone(),before[&lock].clone());assert_eq!(after,before);let after=f.project.remove("parent").unwrap();assert!(after.direct.contains_key("dep"));assert_eq!(after.packages["dep"],f.old.packages["dep"]);assert_eq!(f.project.read_asset("dep","asset.txt").unwrap(),b"dep");
}
#[test]
fn missing_direct_invalid_and_hidden_cache_names_refuse_without_writes() {
    let f=Fixture::new();let before=files(f.temp.path());for name in ["parent","missing","../dep","",&f.old.packages["dep"].digest] {assert!(f.project.preview_keep_dependency(name).is_err());}assert_eq!(files(f.temp.path()),before);
}
#[test]
fn stale_expected_lock_and_forged_candidate_never_call_consumer() {
    let f=Fixture::new();let p=f.project.preview_keep_dependency("dep").unwrap();let called=Cell::new(false);let mut wrong=p.candidate.clone();wrong.direct.remove("parent");assert!(f.project.keep_dependency_checked("dep",&p.previous,&wrong,|| {called.set(true);Ok(())}).is_err());assert!(!called.get());assert_eq!(f.project.verify().unwrap(),f.old);
    let other=source(f.temp.path(),"other",&[]);let current=f.project.install(&[other]).unwrap();let before=files(f.temp.path());assert!(f.project.keep_dependency_checked("dep",&p.previous,&p.candidate,|| {called.set(true);Ok(())}).is_err());assert!(!called.get());assert_eq!(f.project.verify().unwrap(),current);assert_eq!(files(f.temp.path()),before);
}
#[test]
fn tampered_dependency_or_unrelated_active_object_blocks_preview_and_commit() {
    for name in ["dep","parent"] {let f=Fixture::new();let p=f.project.preview_keep_dependency("dep").unwrap();let object=f.project.root().join(".orr/packages/objects").join(&f.old.packages[name].digest).join("asset.txt");fs::write(&object,"tampered").unwrap();let before=files(f.temp.path());assert!(f.project.preview_keep_dependency("dep").is_err());let called=Cell::new(false);assert!(f.project.keep_dependency_checked("dep",&p.previous,&p.candidate,|| {called.set(true);Ok(())}).is_err());assert!(!called.get());assert_eq!(files(f.temp.path()),before);}
}
#[test]
fn callback_failure_and_competing_writer_preserve_lock_and_guard() {
    let f=Fixture::new();let p=f.project.preview_keep_dependency("dep").unwrap();let before=files(f.temp.path());assert!(f.project.keep_dependency_checked("dep",&p.previous,&p.candidate,||Err("consumer changed".into())).is_err());assert_eq!(files(f.temp.path()),before);
    let guard=f.project.root().join(".orr/packages/writer.lock");fs::write(&guard,"foreign writer").unwrap();let before=files(f.temp.path());assert!(f.project.preview_keep_dependency("dep").is_ok());assert!(f.project.keep_dependency_checked("dep",&p.previous,&p.candidate,||Ok(())).is_err());assert_eq!(files(f.temp.path()),before);assert_eq!(fs::read_to_string(guard).unwrap(),"foreign writer");
}
#[test]
fn unsupported_runtime_and_changed_project_manifest_fail_closed() {
    let f=Fixture::new();let p=f.project.preview_keep_dependency("dep").unwrap();fs::write(f.project.root().join("orr.project.json"),br#"{"schema":1,"engine":"^0.0.1"}"#).unwrap();let before=files(f.temp.path());assert!(f.project.keep_dependency_checked("dep",&p.previous,&p.candidate,||Ok(())).is_err());assert_eq!(files(f.temp.path()),before);
    let f=Fixture::new();let source=source(f.temp.path(),"unsupported",&[]);let path=source.join("orr.package.json");let mut manifest:serde_json::Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();manifest["capabilities"]=serde_json::json!(["unavailable-host"]);fs::write(path,serde_json::to_vec(&manifest).unwrap()).unwrap();Project::open_for_install(f.project.root(),Runtime::content_only().engine_version).unwrap().install(&[source]).unwrap();let before=files(f.temp.path());assert!(f.project.preview_keep_dependency("dep").is_err());assert_eq!(files(f.temp.path()),before);
}
