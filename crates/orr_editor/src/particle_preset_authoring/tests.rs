use super::*;
use std::fs;

fn snapshot(
    panel: &Panel,
) -> (
    ParticlePreset,
    Draft,
    Vec<ParticlePreset>,
    Vec<ParticlePreset>,
) {
    (
        panel.document.clone(),
        panel.draft.clone(),
        panel.undo.clone(),
        panel.redo.clone(),
    )
}
#[test]
fn invalid_apply_is_atomic_and_discard_restores_accepted() {
    let mut p = Panel::default();
    for invalid in ["garbage", "NaN", "inf", "-1", "11"] {
        p.draft.lifetime = invalid.into();
        let before = snapshot(&p);
        assert!(p.apply_draft().is_err());
        assert_eq!(snapshot(&p), before);
        p.discard_draft();
        assert!(!p.has_pending_draft());
    }
}
#[test]
fn pending_drafts_block_destructive_operations_but_save_accepted_only() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("accepted.json");
    let mut p = Panel::default();
    p.draft.burst = "17".into();
    p.apply_draft().unwrap();
    p.undo().unwrap();
    p.draft.burst = "unfinished".into();
    let before = snapshot(&p);
    assert!(p.undo().is_err());
    assert!(p.redo().is_err());
    assert!(p.use_defaults().is_err());
    assert!(p.open(&path).is_err());
    p.save_as(&path).unwrap();
    assert_eq!(snapshot(&p), before);
    assert_eq!(ParticlePreset::open(&path).unwrap(), p.document);
    let bytes = fs::read(&path).unwrap();
    assert!(p.save_as(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    p.discard_draft();
    assert!(p.redo().unwrap());
    assert_eq!(p.document.burst, 17);
}
#[test]
fn history_is_bounded_and_divergence_clears_redo() {
    let mut p = Panel::default();
    for n in 1..=80 {
        p.draft.burst = n.to_string();
        p.apply_draft().unwrap();
    }
    assert_eq!(p.undo.len(), 32);
    for _ in 0..32 {
        assert!(p.undo().unwrap());
    }
    assert!(!p.undo().unwrap());
    assert_eq!(p.document.burst, 48);
    for _ in 0..32 {
        assert!(p.redo().unwrap());
    }
    assert!(!p.redo().unwrap());
    assert_eq!(p.document.burst, 80);
    p.undo().unwrap();
    p.draft.burst = "81".into();
    p.apply_draft().unwrap();
    assert!(!p.can_redo());
    let before = snapshot(&p);
    p.apply_draft().unwrap();
    assert_eq!(snapshot(&p), before);
}
#[test]
fn invalid_open_preserves_state_and_valid_open_is_undoable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preset.json");
    let mut p = Panel::default();
    p.draft.burst = "17".into();
    p.apply_draft().unwrap();
    p.save_as(&path).unwrap();
    p.undo().unwrap();
    let before = snapshot(&p);
    let bad = directory.path().join("bad.json");
    for bytes in [
        b"{".to_vec(),
        vec![b' '; orr_sample::particle_preset::MAX_PRESET_BYTES + 1],
    ] {
        fs::write(&bad, bytes).unwrap();
        assert!(p.open(&bad).is_err());
        assert_eq!(snapshot(&p), before);
    }
    assert!(p.open(directory.path()).is_err());
    assert_eq!(snapshot(&p), before);
    p.open(&path).unwrap();
    assert_eq!(p.document.burst, 17);
    p.undo().unwrap();
    assert_eq!(p.document, ParticlePreset::default());
    p.redo().unwrap();
    assert_eq!(p.document.burst, 17);
}

#[test]
fn timed_draft_version_history_and_downgrade_are_explicit() {
    let mut p = Panel::default();
    p.draft.schema = orr_sample::particle_preset::TIMED_PRESET_SCHEMA;
    p.draft.burst = "0".into();
    p.draft.rate = "4".into();
    p.draft.duration = "1".into();
    p.draft.lifetime = "0.125".into();
    assert_eq!(p.document.schema, 1);
    p.apply_draft().unwrap();
    let timed = p.document.clone();
    p.draft.schema = 1;
    p.draft.burst = "1".into();
    let before = snapshot(&p);
    assert!(p.apply_draft().is_err());
    assert_eq!(snapshot(&p), before);
    p.discard_draft();
    p.undo().unwrap();
    assert_eq!(p.document, ParticlePreset::default());
    p.redo().unwrap();
    assert_eq!(p.document, timed);
    for n in 1..=40 {
        p.draft.rate = n.to_string();
        p.apply_draft().unwrap();
    }
    assert_eq!(p.undo.len(), 32);
    p.use_defaults().unwrap();
    assert_eq!(p.document.schema, 1);
    p.undo().unwrap();
    assert_eq!(p.document.schema, 2);
}
