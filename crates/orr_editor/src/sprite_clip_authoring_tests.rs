//! Prepared core contracts; feature registration and execution are separate gates.
use orr_package::{LockedPackage, Manifest};
use orr_sprite::{Frame, PlaybackMode, SpriteSource};
use sha2::{Digest, Sha256};
use super::{ClipDocument, DestinationPolicy, SaveAsPlan, MAX_DOCUMENT_BYTES, MAX_HISTORY, MAX_PACKAGE_BYTES};
use std::collections::{BTreeMap, BTreeSet};

fn source() -> Vec<u8> {
    br#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":2,"height":1},"regions":[{"id":10,"x":0,"y":0,"width":1,"height":1},{"id":20,"x":1,"y":0,"width":1,"height":1}],"clips":[{"id":"walk","mode":"loop","frames":[{"region":10,"duration_ms":100},{"region":20,"duration_ms":200}]},{"id":"idle","mode":"once","frames":[{"region":10,"duration_ms":500}]}]}"#.to_vec()
}
fn admitted() -> (LockedPackage, BTreeMap<String, Vec<u8>>) {
    // This pure-core fixture is not a claim that these placeholder PNG bytes decode.
    // Actual package admission/decode and GPU integration remain mandatory later.
    let files: BTreeMap<String, Vec<u8>> = [
        ("sprite.json".into(), source()), ("atlas.png".into(), vec![1, 2, 3]),
        ("LICENSE.txt".into(), b"fixture license retained exactly\n".to_vec()),
    ].into();
    let manifest = Manifest { schema: 1, name: "fixture.sprite".into(), version: "1.0.0".into(),
        engine: "*".into(), capabilities: BTreeSet::new(), dependencies: BTreeMap::new(), files: files.keys().cloned().collect() };
    let hashes = files.iter().map(|(p, b)| (p.clone(), Sha256::digest(b).iter().map(|byte| format!("{byte:02x}")).collect::<String>())).collect();
    (LockedPackage { manifest, digest: "admitted-fixture-provenance".into(), files: hashes }, files)
}
fn frames() -> Vec<Frame> { vec![Frame { region: 10, duration_ms: 100 }, Frame { region: 20, duration_ms: 200 }] }

#[test]
fn ordered_frames_and_exact_loop_once_boundaries() {
    let mut doc = ClipDocument::open(&source(), "walk").unwrap();
    for (time, region) in [(0, 10), (99, 10), (100, 20), (299, 20), (300, 10)] {
        assert_eq!(doc.sample(time).unwrap().region, region);
    }
    assert!(doc.move_frame(0, 1).unwrap());
    assert_eq!(doc.sample(0).unwrap().region, 20);
    assert_eq!(doc.sample(199).unwrap().region, 20);
    assert_eq!(doc.sample(200).unwrap().region, 10);
    assert!(doc.set_mode(PlaybackMode::Once).unwrap());
    assert!(!doc.sample(299).unwrap().finished);
    assert_eq!(doc.sample(300).unwrap().region, 10);
    assert!(doc.sample(u64::MAX).unwrap().finished);
}
#[test]
fn invalid_edits_and_noops_preserve_document_and_redo() {
    let mut doc = ClipDocument::open(&source(), "walk").unwrap();
    doc.set_duration(0, 150).unwrap(); assert!(doc.undo());
    let before = doc.source().clone();
    assert!(doc.can_redo());
    assert!(!doc.replace_frames(frames(), PlaybackMode::Loop).unwrap());
    for bad in [vec![], vec![Frame { region: 99, duration_ms: 1 }],
        vec![Frame { region: 10, duration_ms: 0 }],
        vec![Frame { region: 10, duration_ms: u64::MAX }, Frame { region: 20, duration_ms: 1 }]] {
        assert!(doc.replace_frames(bad, PlaybackMode::Loop).is_err());
        assert_eq!(doc.source(), &before); assert!(doc.can_redo());
    }
    assert!(doc.set_duration(9, 1).is_err()); assert!(doc.move_frame(0, 9).is_err());
    assert!(doc.redo()); assert_eq!(doc.source().clips[0].frames[0].duration_ms, 150);
}
#[test]
fn bounded_history_and_branching_leave_other_content_identical() {
    let mut doc = ClipDocument::open(&source(), "walk").unwrap();
    let original = doc.source().clone();
    for n in 1..=40 { doc.set_duration(0, 100 + n).unwrap(); }
    let mut count = 0; while doc.undo() { count += 1; }
    assert_eq!(count, MAX_HISTORY);
    assert_eq!(doc.source().clips[0].frames[0].duration_ms, 108);
    doc.set_mode(PlaybackMode::Once).unwrap(); assert!(!doc.can_redo());
    assert_eq!(doc.source().atlas, original.atlas); assert_eq!(doc.source().regions, original.regions);
    assert_eq!(doc.source().clips[0].id, original.clips[0].id);
    assert_eq!(doc.source().clips[1], original.clips[1]); assert!(doc.can_undo());
}
#[test]
fn new_version_save_plan_preserves_original_pixels_license_and_other_clips() {
    let (package, files) = admitted(); let before = files.clone();
    let mut doc = ClipDocument::open(&source(), "walk").unwrap();
    doc.set_duration(0, 250).unwrap();
    let plan = SaveAsPlan::prepare(&doc, &package, &files, "sprite.json", "1.0.1").unwrap();
    assert_eq!(plan.destination_policy(), DestinationPolicy::CreateNewDirectory);
    assert_eq!(plan.original_package_digest(), package.digest);
    assert_eq!(plan.files()["atlas.png"], files["atlas.png"]);
    assert_eq!(plan.files()["LICENSE.txt"], files["LICENSE.txt"]);
    let reopened: SpriteSource = serde_json::from_slice(&plan.files()["sprite.json"]).unwrap();
    assert_eq!(&reopened, doc.source());
    assert!(ClipDocument::open(&plan.files()["sprite.json"], "walk").is_ok());
    let mut expected = package.manifest.clone(); expected.version = "1.0.1".into();
    assert_eq!(serde_json::from_slice::<Manifest>(&plan.files()["orr.package.json"]).unwrap(), expected);
    assert_eq!(files, before); assert_eq!(package.manifest.version, "1.0.0");
}
#[test]
fn save_plan_rejects_old_versions_changed_snapshots_and_foreign_documents() {
    let (package, mut files) = admitted(); let mut doc = ClipDocument::open(&source(), "walk").unwrap();
    for version in ["1.0.0", "0.9.9", "1.00.1", "1.0.1-beta", "1.0", "4294967296.0.0"] {
        assert!(SaveAsPlan::prepare(&doc, &package, &files, "sprite.json", version).is_err());
    }
    files.get_mut("LICENSE.txt").unwrap().push(0);
    assert!(SaveAsPlan::prepare(&doc, &package, &files, "sprite.json", "1.0.1").is_err());
    let (package, files) = admitted(); doc.set_duration(0, 101).unwrap();
    // A new editing session opened from altered source cannot borrow old provenance.
    let bytes = serde_json::to_vec(doc.source()).unwrap(); let foreign = ClipDocument::open(&bytes, "walk").unwrap();
    assert!(SaveAsPlan::prepare(&foreign, &package, &files, "sprite.json", "1.0.1").is_err());
}
#[test]
fn admission_limits_and_closed_package_inventory_are_enforced() {
    assert!(ClipDocument::open(&vec![b' '; MAX_DOCUMENT_BYTES + 1], "walk").is_err());
    assert!(ClipDocument::open(&source(), "missing").is_err());
    assert!(ClipDocument::open(b"{}", "walk").is_err());
    let doc = ClipDocument::open(&source(), "walk").unwrap();
    let (mut package, mut files) = admitted();
    package.manifest.dependencies.insert("extra".into(), "1.0.0".into());
    assert!(SaveAsPlan::prepare(&doc, &package, &files, "sprite.json", "1.0.1").is_err());
    package.manifest.dependencies.clear();
    files.insert("unexpected".into(), vec![]);
    assert!(SaveAsPlan::prepare(&doc, &package, &files, "sprite.json", "1.0.1").is_err());
    files.remove("unexpected"); files.insert("LICENSE.txt".into(), vec![0; MAX_PACKAGE_BYTES + 1]);
    assert!(SaveAsPlan::prepare(&doc, &package, &files, "sprite.json", "1.0.1").is_err());
}

#[test]
fn oversized_frame_and_version_inputs_reject_without_history_change() {
    let mut doc = ClipDocument::open(&source(), "walk").unwrap();
    doc.set_duration(0, 150).unwrap(); doc.undo();
    let before = doc.source().clone();
    let frames = vec![Frame { region: 10, duration_ms: 1 }; orr_sprite::MAX_FRAMES_PER_CLIP + 1];
    assert!(doc.replace_frames(frames, PlaybackMode::Loop).is_err());
    assert_eq!(doc.source(), &before); assert!(doc.can_redo());
    let (package, files) = admitted();
    for version in ["1.".repeat(1000), "9".repeat(1000)] {
        assert!(SaveAsPlan::prepare(&doc, &package, &files, "sprite.json", &version).is_err());
    }
    assert_eq!(doc.source(), &before); assert!(doc.can_redo());
}

#[cfg(target_os = "linux")]
fn save_plan() -> SaveAsPlan {
    let (package, files) = admitted();
    let mut document = ClipDocument::open(&source(), "walk").unwrap();
    document.set_duration(0, 125).unwrap();
    SaveAsPlan::prepare(&document, &package, &files, "sprite.json", "1.0.1").unwrap()
}
#[cfg(target_os = "linux")]
#[test]
fn filesystem_save_publishes_exact_new_bytes_and_never_replaces_existing_paths() {
    let root = tempfile::tempdir().unwrap(); let root_path = root.path().canonicalize().unwrap();
    let output = root_path.join("clip-v2"); let plan = save_plan();
    plan.publish_new(&output).unwrap();
    for (path, bytes) in plan.files() { assert_eq!(std::fs::read(output.join(path)).unwrap(), *bytes); }
    assert!(!plan.publish_new(&output).unwrap_err().published);
    let file = root_path.join("existing-file"); std::fs::write(&file, b"keep").unwrap();
    assert!(!plan.publish_new(&file).unwrap_err().published);
    assert_eq!(std::fs::read(file).unwrap(), b"keep");
    let empty = root_path.join("existing-empty-dir"); std::fs::create_dir(&empty).unwrap();
    assert!(plan.publish_new(&empty).is_err()); assert_eq!(std::fs::read_dir(empty).unwrap().count(), 0);
}
#[cfg(target_os = "linux")]
#[test]
fn filesystem_save_rejects_symlink_ancestors_destinations_and_traversal() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap(); let base = root.path().canonicalize().unwrap(); let plan = save_plan();
    std::fs::create_dir(base.join("real")).unwrap(); symlink(base.join("real"), base.join("alias")).unwrap();
    assert!(plan.publish_new(&base.join("alias/output")).is_err());
    symlink(base.join("missing"), base.join("dangling")).unwrap();
    assert!(plan.publish_new(&base.join("dangling")).is_err());
    assert!(plan.publish_new(&base.join("real/../output")).is_err());
    assert!(plan.publish_new(std::path::Path::new("relative/output")).is_err());
    assert!(!base.join("output").exists()); assert_eq!(std::fs::read_dir(base.join("real")).unwrap().count(), 0);
}
#[cfg(target_os = "linux")]
#[test]
fn injected_partial_write_retains_only_private_stage_and_never_deletes_old_files() {
    let root = tempfile::tempdir().unwrap(); let base = root.path().canonicalize().unwrap(); let output = base.join("new");
    let sentinel = base.join("keep"); std::fs::write(&sentinel, b"original").unwrap();
    let error = save_plan().publish_test_hook(&output, |phase, _| {
        if phase == "file-written" { Err("injected write boundary failure".into()) } else { Ok(()) }
    }).unwrap_err();
    assert!(!error.published); assert!(error.message.contains("injected")); assert!(!output.exists());
    let stage = error.leftover_stage.unwrap(); assert!(stage.starts_with(&base)); assert!(stage.is_dir());
    assert_eq!(std::fs::read(sentinel).unwrap(), b"original");
}
#[cfg(target_os = "linux")]
#[test]
fn concurrent_destination_and_postpublish_failure_have_honest_outcomes() {
    let root = tempfile::tempdir().unwrap(); let base = root.path().canonicalize().unwrap(); let output = base.join("raced");
    let error = save_plan().publish_test_hook(&output, |phase, _| {
        if phase == "before-publish" { std::fs::create_dir(&output).map_err(|e| e.to_string())?; }
        Ok(())
    }).unwrap_err();
    assert!(!error.published); assert!(error.leftover_stage.unwrap().is_dir());
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
    let published = base.join("published"); let plan = save_plan();
    let error = plan.publish_test_hook(&published, |phase, _| {
        if phase == "after-publish" { Err("injected durability boundary".into()) } else { Ok(()) }
    }).unwrap_err();
    assert!(error.published); assert!(error.leftover_stage.is_none());
    for (path, bytes) in plan.files() { assert_eq!(std::fs::read(published.join(path)).unwrap(), *bytes); }
}
#[cfg(target_os = "linux")]
#[test]
fn changed_stage_bytes_are_rejected_before_publication() {
    let root = tempfile::tempdir().unwrap(); let base = root.path().canonicalize().unwrap(); let output = base.join("new");
    let error = save_plan().publish_test_hook(&output, |phase, stage| {
        if phase == "before-publish" { std::fs::write(stage.join("LICENSE.txt"), b"tampered").map_err(|e| e.to_string())?; }
        Ok(())
    }).unwrap_err();
    assert!(!error.published); assert!(!output.exists()); assert!(error.leftover_stage.unwrap().is_dir());
}
#[cfg(not(target_os = "linux"))]
#[test]
fn filesystem_save_fails_closed_on_unsupported_platforms() {
    let (package, files) = admitted(); let doc = ClipDocument::open(&source(), "walk").unwrap();
    let plan = SaveAsPlan::prepare(&doc, &package, &files, "sprite.json", "1.0.1").unwrap();
    let error = plan.publish_new(std::path::Path::new("unsupported-output")).unwrap_err();
    assert!(!error.published); assert!(error.leftover_stage.is_none()); assert!(error.message.contains("requires Linux"));
}

#[cfg(target_os = "linux")]
#[test]
fn unsafe_destination_ancestry_rejects_without_writes() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap(); let base = root.path().canonicalize().unwrap();
    let unsafe_parent = base.join("unsafe"); std::fs::create_dir(&unsafe_parent).unwrap();
    std::fs::set_permissions(&unsafe_parent, std::fs::Permissions::from_mode(0o777)).unwrap();
    let error = save_plan().publish_new(&unsafe_parent.join("new")).unwrap_err();
    assert!(!error.published); assert!(error.leftover_stage.is_none());
    assert!(error.message.contains("sticky")); assert_eq!(std::fs::read_dir(&unsafe_parent).unwrap().count(), 0);
    std::fs::create_dir(unsafe_parent.join("safe-child")).unwrap();
    let error = save_plan().publish_new(&unsafe_parent.join("safe-child/new")).unwrap_err();
    assert!(error.leftover_stage.is_none()); assert_eq!(std::fs::read_dir(unsafe_parent.join("safe-child")).unwrap().count(), 0);
}
#[cfg(target_os = "linux")]
#[test]
fn unexpected_stage_entry_rejects_closed_inventory() {
    let root = tempfile::tempdir().unwrap(); let base = root.path().canonicalize().unwrap();
    let output = base.join("new");
    let error = save_plan().publish_test_hook(&output, |phase, stage| {
        if phase == "before-publish" { std::fs::write(stage.join("unexpected"), b"injected").map_err(|e| e.to_string())?; }
        Ok(())
    }).unwrap_err();
    assert!(!error.published); assert!(!output.exists());
    assert!(error.message.contains("inventory")); assert!(error.leftover_stage.unwrap().is_dir());
}

#[test]
fn sha256_digest_has_exact_lowercase_64_character_known_vectors() {
    for (input, expected) in [
        (b"".as_slice(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        (b"abc".as_slice(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
    ] {
        assert_eq!(Sha256::digest(input).len(), 32);
        let actual = super::digest(input);
        assert_eq!(actual.len(), 64);
        assert!(actual.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        assert_eq!(actual, expected);
    }
}
