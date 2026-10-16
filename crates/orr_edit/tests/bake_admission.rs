//! Admission is shared by loads, structural and field edits, history and staging.
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_edit::{BakeAdmission, EditError, EditorDoc, Op, Origin};
use orr_fp::FrameRng;
use orr_reflect::{Guid, Scene, SceneIndex, TypeRegistry};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

struct Admission {
    reject: AtomicBool,
    calls: AtomicUsize,
}
impl BakeAdmission for Admission {
    fn admit(&self, _: &Scene, frame: &mut Frame, _: &SceneIndex) -> Result<(), EditError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // Deliberately modify the candidate before failing. Nothing may leak.
        frame.set_singleton(FrameRng::new(123));
        if self.reject.load(Ordering::SeqCst) {
            Err(EditError::Invalid("asset unavailable".into()))
        } else {
            Ok(())
        }
    }
    fn allow_play_edits(&self) -> bool {
        false
    }
}
fn fixture() -> (EditorDoc, Arc<Admission>) {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_singleton::<FrameRng>("FrameRng");
    let admission = Arc::new(Admission {
        reject: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });
    let doc = EditorDoc::from_scene_with_admission(
        Scene::default(),
        TypeRegistry::new(),
        registry.build(),
        7,
        Some(admission.clone()),
    )
    .unwrap();
    (doc, admission)
}
fn spawn(name: &str) -> Op {
    Op::SpawnEntity {
        guid: None,
        name: Some(name.into()),
        components: vec![],
    }
}
fn rename(guid: &Guid, name: &str) -> Op {
    Op::Rename {
        guid: guid.clone(),
        name: Some(name.into()),
    }
}
fn state(doc: &EditorDoc) -> (String, Vec<u8>, u64, bool, Vec<orr_edit::HistoryEntry>) {
    (
        doc.to_yaml(),
        doc.frame().to_bytes(),
        doc.revision(),
        doc.is_dirty(),
        doc.history(),
    )
}
#[test]
fn every_admitted_path_uses_common_bake_and_failure_is_atomic() {
    let (mut doc, admission) = fixture();
    assert_eq!(admission.calls.load(Ordering::SeqCst), 1);
    let id = doc
        .apply(spawn("first"), Origin::User)
        .unwrap()
        .guid
        .unwrap();
    doc.apply(rename(&id, "second"), Origin::User).unwrap();
    doc.undo().unwrap();
    let before = state(&doc);
    admission.reject.store(true, Ordering::SeqCst);
    assert!(doc.redo().is_err());
    assert_eq!(state(&doc), before);
    assert!(doc.undo().is_err());
    assert_eq!(state(&doc), before);
    assert!(doc.apply(rename(&id, "rejected"), Origin::User).is_err());
    assert_eq!(state(&doc), before);
    assert!(doc.apply(spawn("rejected"), Origin::User).is_err());
    assert_eq!(state(&doc), before);
    assert!(doc
        .load_yaml("schema: orr.scene/1\nsingletons: {}\nentities: {}\n")
        .is_err());
    assert_eq!(state(&doc), before);
    assert!(doc.rebake().is_err());
    assert_eq!(state(&doc), before);
    admission.reject.store(false, Ordering::SeqCst);
    doc.redo().unwrap();
    let created = doc
        .apply(spawn("after failure"), Origin::User)
        .unwrap()
        .guid
        .unwrap();
    assert_eq!(created, Guid::from_u32(2));
    assert!(admission.calls.load(Ordering::SeqCst) >= 12);
}
#[test]
fn staged_batch_retains_admission_and_preserves_metadata_on_rejection() {
    let (mut doc, admission) = fixture();
    doc.apply(spawn("first"), Origin::User).unwrap();
    let before = state(&doc);
    admission.reject.store(true, Ordering::SeqCst);
    assert!(doc
        .apply_batch(
            "rejected",
            vec![spawn("prefix"), spawn("suffix")],
            Origin::User
        )
        .is_err());
    assert_eq!(state(&doc), before);
}

#[test]
fn failed_proposal_admission_does_not_reserve_document_guids() {
    let (mut doc, admission) = fixture();
    let proposal = doc.propose("assets", Origin::User).unwrap();
    let initial = state(&doc);
    let initial_proposal = doc.proposal_state(proposal).unwrap();
    admission.reject.store(true, Ordering::SeqCst);
    assert!(doc.proposal_apply(proposal, spawn("rejected")).is_err());
    assert_eq!(state(&doc), initial);
    assert_eq!(doc.proposal_state(proposal).unwrap(), initial_proposal);
    assert!(doc
        .proposal_apply_all(proposal, vec![spawn("prefix"), spawn("suffix")])
        .is_err());
    assert_eq!(state(&doc), initial);
    assert_eq!(doc.proposal_state(proposal).unwrap(), initial_proposal);
    assert!(doc.proposal_ops(proposal).unwrap().is_empty());
    admission.reject.store(false, Ordering::SeqCst);
    let created = doc
        .proposal_apply(proposal, spawn("first accepted"))
        .unwrap()
        .guid
        .unwrap();
    assert_eq!(created, Guid::from_u32(1));
    let created = doc
        .apply(spawn("second accepted"), Origin::User)
        .unwrap()
        .guid
        .unwrap();
    assert_eq!(created, Guid::from_u32(2));
}

/// Emulates an asset that becomes unavailable between two accepted operations
/// and remains unavailable during any attempted rollback.
struct DisappearingAdmission {
    calls: AtomicUsize,
    fail_at: AtomicUsize,
}
impl BakeAdmission for DisappearingAdmission {
    fn admit(&self, _: &Scene, frame: &mut Frame, _: &SceneIndex) -> Result<(), EditError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        frame.set_singleton(FrameRng::new(call as u64));
        if call >= self.fail_at.load(Ordering::SeqCst) {
            Err(EditError::Invalid("asset disappeared".into()))
        } else {
            Ok(())
        }
    }
}

#[test]
fn proposal_acceptance_discards_successful_prefix_when_asset_disappears() {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_singleton::<FrameRng>("FrameRng");
    let admission = Arc::new(DisappearingAdmission {
        calls: AtomicUsize::new(0),
        fail_at: AtomicUsize::new(usize::MAX),
    });
    let mut doc = EditorDoc::from_scene_with_admission(
        Scene::default(),
        TypeRegistry::new(),
        registry.build(),
        7,
        Some(admission.clone()),
    )
    .unwrap();
    let original = doc
        .apply(spawn("original"), Origin::User)
        .unwrap()
        .guid
        .unwrap();
    doc.apply(rename(&original, "renamed"), Origin::User)
        .unwrap();
    doc.undo().unwrap();
    assert!(doc.can_redo());
    let proposal = doc
        .propose("two asset-dependent ops", Origin::User)
        .unwrap();
    doc.proposal_apply_all(proposal, vec![spawn("prefix"), spawn("suffix")])
        .unwrap();
    let before = state(&doc);
    let before_proposals = doc.list_proposals();
    let guard = doc.proposal_state(proposal).unwrap();
    let preview = doc.proposal_preview(proposal).unwrap().frame().to_bytes();
    admission
        .fail_at
        .store(admission.calls.load(Ordering::SeqCst) + 2, Ordering::SeqCst);
    let error = doc.accept_if_unchanged(proposal, guard).unwrap_err();
    assert!(matches!(
        error,
        EditError::ProposalConflict { op_index: 1, .. }
    ));
    assert_eq!(state(&doc), before);
    assert_eq!(doc.list_proposals(), before_proposals);
    assert_eq!(doc.proposal_state(proposal).unwrap(), guard);
    assert_eq!(
        doc.proposal_preview(proposal).unwrap().frame().to_bytes(),
        preview
    );
    assert!(doc.can_redo());
    assert!(!doc.in_tx());
    admission.fail_at.store(usize::MAX, Ordering::SeqCst);
    let accepted = doc.accept_if_unchanged(proposal, guard).unwrap();
    assert!(accepted.history_id.is_some());
    assert_eq!(accepted.applied.len(), 2);
    assert!(doc.list_proposals().is_empty());
    assert_eq!(doc.scene().entities.len(), 3);
    doc.undo().unwrap();
    assert_eq!(doc.scene().entities.len(), 1);
    doc.redo().unwrap();
    assert_eq!(doc.scene().entities.len(), 3);
}

#[test]
fn transaction_rollback_restores_snapshot_even_after_asset_disappears() {
    let (mut doc, admission) = fixture();
    let original = doc
        .apply(spawn("original"), Origin::User)
        .unwrap()
        .guid
        .unwrap();
    doc.apply(rename(&original, "redo me"), Origin::User)
        .unwrap();
    doc.undo().unwrap();
    let before = state(&doc);
    doc.begin_tx("temporary", Origin::User).unwrap();
    doc.apply(spawn("temporary entity"), Origin::User).unwrap();
    doc.apply(rename(&original, "temporary name"), Origin::User)
        .unwrap();
    assert_ne!(state(&doc), before);
    let calls = admission.calls.load(Ordering::SeqCst);
    admission.reject.store(true, Ordering::SeqCst);
    doc.rollback_tx().unwrap();
    assert_eq!(state(&doc), before);
    assert_eq!(admission.calls.load(Ordering::SeqCst), calls);
    assert!(doc.can_redo());
    assert!(!doc.in_tx());
    admission.reject.store(false, Ordering::SeqCst);
    let created = doc
        .apply(spawn("after rollback"), Origin::User)
        .unwrap()
        .guid
        .unwrap();
    assert_eq!(created, Guid::from_u32(2));
}

struct SourceBound;
impl BakeAdmission for SourceBound {
    fn admit_source(&self, text: &str) -> Result<(), EditError> {
        if text.len() > 128 {
            Err(EditError::Invalid("source byte bound".into()))
        } else {
            Ok(())
        }
    }
    fn admit(&self, _: &Scene, _: &mut Frame, _: &SceneIndex) -> Result<(), EditError> {
        Ok(())
    }
}
#[test]
fn optional_source_policy_runs_before_parse_and_atomic_reload() {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_singleton::<FrameRng>("FrameRng");
    let registry = registry.build();
    let text = "schema: orr.scene/1\nsingletons: {}\nentities: {}\n";
    let huge = format!("{text}{}", " ".repeat(256));
    assert!(EditorDoc::from_yaml_with_admission(
        &huge,
        TypeRegistry::new(),
        registry.clone(),
        7,
        Some(Arc::new(SourceBound))
    )
    .err()
    .unwrap()
    .to_string()
    .contains("source byte bound"));
    let mut doc = EditorDoc::from_yaml_with_admission(
        text,
        TypeRegistry::new(),
        registry.clone(),
        7,
        Some(Arc::new(SourceBound)),
    )
    .unwrap();
    let before = state(&doc);
    assert!(doc
        .load_yaml(&huge)
        .unwrap_err()
        .to_string()
        .contains("source byte bound"));
    assert_eq!(state(&doc), before);
    // Existing/no-policy hosts retain their previous accepted input behavior.
    let mut legacy = EditorDoc::from_yaml(&huge, TypeRegistry::new(), registry, 7).unwrap();
    legacy.load_yaml(&huge).unwrap();
    assert_eq!(legacy.to_yaml(), huge);
}
