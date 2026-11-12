use orr_package::{Project, Runtime};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
fn source(root: &Path, name: &str, deps: &[(&str, &str)]) -> PathBuf {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    let deps: BTreeMap<_, _> = deps.iter().copied().collect();
    fs::write(path.join("asset.txt"), name).unwrap();
    fs::write(path.join("orr.package.json"), serde_json::to_vec(&json!({"schema":1,"name":name,"version":"1.0.0","engine":"*","capabilities":[],"dependencies":deps,"files":["asset.txt"]})).unwrap()).unwrap();
    path
}
#[test]
fn checked_dependency_closure_and_callback_failure_preserve_active_lock() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let p = Project::open(&root, Runtime::content_only()).unwrap();
    let a = source(temp.path(), "a", &[("b", "1.0.0")]);
    let b = source(temp.path(), "b", &[("c", "1.0.0")]);
    let c = source(temp.path(), "c", &[]);
    let d = source(temp.path(), "d", &[]);
    let deps = [b, c, d];
    let direct = [a];
    let preview = p.preview_install_with_dependencies(&direct, &deps).unwrap();
    assert_eq!(preview.candidate.direct.len(), 1);
    assert_eq!(preview.candidate.packages.len(), 3);
    let error = p
        .install_checked_with_dependencies(
            &direct,
            &deps,
            &preview.previous,
            &preview.candidate,
            || Err("injected before publication".into()),
        )
        .unwrap_err();
    assert!(error.to_string().contains("injected before publication"));
    assert_eq!(p.verify().unwrap(), preview.previous);
    assert_eq!(
        p.install_checked_with_dependencies(
            &direct,
            &deps,
            &preview.previous,
            &preview.candidate,
            || Ok(())
        )
        .unwrap(),
        preview.candidate
    );
    assert!(!p.verify().unwrap().packages.contains_key("d"));
    assert!(p
        .install_checked_with_dependencies(
            &direct,
            &deps,
            &preview.previous,
            &preview.candidate,
            || Ok(())
        )
        .unwrap_err()
        .to_string()
        .contains("active package lock changed"));
}
#[test]
fn changed_reachable_content_cannot_replace_approved_candidate() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let p = Project::open(&root, Runtime::content_only()).unwrap();
    let a = source(temp.path(), "a", &[("b", "1.0.0")]);
    let b = source(temp.path(), "b", &[]);
    let preview = p
        .preview_install_with_dependencies(std::slice::from_ref(&a), std::slice::from_ref(&b))
        .unwrap();
    fs::write(b.join("asset.txt"), b"changed").unwrap();
    assert!(p
        .install_checked_with_dependencies(
            &[a],
            &[b],
            &preview.previous,
            &preview.candidate,
            || panic!("changed candidate must fail before callback")
        )
        .unwrap_err()
        .to_string()
        .contains("candidate package lock differs"));
    assert_eq!(p.verify().unwrap(), preview.previous);
}
