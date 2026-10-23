//! Presentation-only sibling alignment. Explicit Apply remains the caller's job.
//! Geometry uses exact thousandths of a logical pixel until the final offset is
//! rounded to nearest, with ties away from zero. Consequently the aligned edge
//! or midpoint can differ from the reference by at most 0.5 logical pixels.
use orr_sample::authored_ui::{Document, Node, Screen};
use std::collections::BTreeSet;
use std::path::PathBuf;

mod distribution;
pub use distribution::DistributionAxis;

#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    pub scene: PathBuf,
    pub epoch: u64,
    pub screen: Screen,
    pub viewport: [u32; 2],
    pub canvas: [f32; 2],
}

impl Context {
    fn validate(&self) -> Result<(), String> {
        if self.viewport.contains(&0) || self.canvas.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err("Alignment requires positive finite viewport and canvas dimensions".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
}

impl Mode {
    fn axis_and_factor(self) -> (usize, i64) {
        match self {
            Self::Left => (0, 0),
            Self::Center => (0, 500),
            Self::Right => (0, 1000),
            Self::Top => (1, 0),
            Self::Middle => (1, 500),
            Self::Bottom => (1, 1000),
        }
    }
}

struct Draft {
    context: Context,
    original: Document,
    selected_ids: Vec<String>,
    reference_id: Option<String>,
    candidate: Document,
}

#[derive(Default)]
pub struct State {
    draft: Option<Draft>,
}

impl State {
    /// Replace the previous draft atomically. Reject duplicate IDs, missing IDs,
    /// cross-screen or cross-parent selections, and a reference outside selection.
    /// A rounded no-op has no candidate and cannot create an undo entry.
    pub fn stage(
        &mut self,
        context: Context,
        accepted: &Document,
        selected_ids: &[String],
        reference_id: &str,
        mode: Mode,
    ) -> Result<(), String> {
        self.cancel();
        context.validate()?;
        accepted.to_bytes()?;
        let unique: BTreeSet<&str> = selected_ids.iter().map(String::as_str).collect();
        if unique.len() < 2 || unique.len() != selected_ids.len() {
            return Err("Select at least two distinct UI siblings without duplicate IDs".into());
        }
        if !unique.contains(reference_id) {
            return Err("The explicit alignment reference must be selected".into());
        }
        let indices: Vec<usize> = selected_ids
            .iter()
            .map(|id| {
                accepted
                    .nodes
                    .iter()
                    .position(|node| node.id == *id)
                    .ok_or_else(|| format!("Selected UI node {id} does not exist"))
            })
            .collect::<Result<_, _>>()?;
        let reference = accepted
            .nodes
            .iter()
            .find(|node| node.id == reference_id)
            .ok_or("The alignment reference does not exist")?;
        if indices.iter().any(|&index| {
            let node = &accepted.nodes[index];
            node.screen != context.screen || node.parent != reference.parent
        }) {
            return Err(
                "Alignment requires siblings on the preview screen with the same parent".into(),
            );
        }
        let (axis, factor) = mode.axis_and_factor();
        let extent = match &reference.parent {
            Some(parent) => i64::from(
                accepted
                    .nodes
                    .iter()
                    .find(|node| node.id == *parent)
                    .ok_or("The alignment parent does not exist")?
                    .size[axis],
            ),
            None => i64::from(context.viewport[axis]),
        };
        // Calculate every offset before staging any document mutation. The
        // shared parent's world origin cancels, including for nested siblings.
        let proposals: Vec<(usize, i16)> = indices
            .into_iter()
            .filter(|&index| accepted.nodes[index].id != reference_id)
            .map(|index| {
                proposed_offset(reference, &accepted.nodes[index], axis, factor, extent)
                    .map(|offset| (index, offset))
            })
            .collect::<Result<_, _>>()?;
        if proposals
            .iter()
            .all(|&(index, offset)| accepted.nodes[index].offset[axis] == offset)
        {
            return Ok(());
        }
        let mut candidate = accepted.clone();
        for (index, offset) in proposals {
            candidate.nodes[index].offset[axis] = offset;
        }
        candidate.to_bytes()?;
        self.draft = Some(Draft {
            context,
            original: accepted.clone(),
            selected_ids: selected_ids.to_vec(),
            reference_id: Some(reference_id.into()),
            candidate,
        });
        Ok(())
    }

    /// Pin the complete accepted snapshot and all preview coordinate identity.
    /// Callers also cancel on selection/reference changes, focus loss and Play.
    pub fn validate_context(
        &mut self,
        current: &Context,
        accepted: &Document,
    ) -> Result<(), String> {
        let Some(draft) = &self.draft else {
            return Ok(());
        };
        let result = check_current(draft, current, accepted);
        if result.is_err() {
            self.cancel();
        }
        result
    }

    pub fn candidate(&self, current: &Context, accepted: &Document) -> Result<Document, String> {
        let draft = self
            .draft
            .as_ref()
            .ok_or("No UI alignment draft is ready")?;
        check_current(draft, current, accepted)?;
        Ok(draft.candidate.clone())
    }

    /// Read-only preview; `candidate` must guard explicit Apply.
    pub fn document(&self) -> Option<&Document> {
        self.draft.as_ref().map(|d| &d.candidate)
    }
    pub fn is_ready(&self) -> bool {
        self.draft.is_some()
    }
    pub fn reference_id(&self) -> Option<&str> {
        self.draft.as_ref().and_then(|d| d.reference_id.as_deref())
    }
    pub fn selected_ids(&self) -> Option<&[String]> {
        self.draft.as_ref().map(|d| d.selected_ids.as_slice())
    }
    pub fn cancel(&mut self) {
        self.draft = None;
    }
}

fn check_current(draft: &Draft, current: &Context, accepted: &Document) -> Result<(), String> {
    current.validate()?;
    if draft.context != *current {
        return Err("UI alignment context changed; stage again".into());
    }
    if draft.original != *accepted {
        return Err("Accepted UI document changed; stage alignment again".into());
    }
    Ok(())
}

fn proposed_offset(
    reference: &Node,
    target: &Node,
    axis: usize,
    factor: i64,
    extent: i64,
) -> Result<i16, String> {
    let overflow = || "UI alignment arithmetic overflow".to_string();
    let anchors = i64::from(reference.anchor[axis])
        .checked_sub(i64::from(target.anchor[axis]))
        .ok_or_else(overflow)?;
    let sizes = i64::from(reference.size[axis])
        .checked_sub(i64::from(target.size[axis]))
        .ok_or_else(overflow)?;
    let numerator = i64::from(reference.offset[axis])
        .checked_mul(1000)
        .and_then(|n| extent.checked_mul(anchors).and_then(|a| n.checked_add(a)))
        .and_then(|n| sizes.checked_mul(factor).and_then(|s| n.checked_add(s)))
        .ok_or_else(overflow)?;
    // Reject the UNROUNDED proposal: even a 0.001-pixel excursion is invalid.
    if !(-4_096_000..=4_096_000).contains(&numerator) {
        return Err("Aligned UI offset must stay within -4096..=4096 logical pixels".into());
    }
    let rounded = numerator
        .checked_add(if numerator < 0 { -500 } else { 500 })
        .ok_or_else(overflow)?
        / 1000;
    i16::try_from(rounded).map_err(|_| overflow())
}

/// Eligible siblings in authored order, including the explicit reference.
pub fn sibling_ids(
    document: &Document,
    screen: Screen,
    reference_id: &str,
) -> Result<Vec<String>, String> {
    document.to_bytes()?;
    let reference = document
        .nodes
        .iter()
        .find(|node| node.id == reference_id)
        .ok_or("The alignment reference does not exist")?;
    if reference.screen != screen {
        return Err("The alignment reference is not on the preview screen".into());
    }
    Ok(document
        .nodes
        .iter()
        .filter(|node| node.screen == screen && node.parent == reference.parent)
        .map(|node| node.id.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_sample::authored_ui::{Kind, SCHEMA};

    const MODES: [Mode; 6] = [
        Mode::Left,
        Mode::Center,
        Mode::Right,
        Mode::Top,
        Mode::Middle,
        Mode::Bottom,
    ];

    fn context() -> Context {
        Context {
            scene: "/virtual/collect/scene.yaml".into(),
            epoch: 7,
            screen: Screen::Title,
            viewport: [641, 359],
            canvas: [320.5, 179.5],
        }
    }
    fn node(
        id: &str,
        parent: Option<&str>,
        anchor: [u16; 2],
        offset: [i16; 2],
        size: [u16; 2],
    ) -> Node {
        Node {
            id: id.into(),
            parent: parent.map(str::to_string),
            kind: Kind::Container,
            screen: Screen::Title,
            anchor,
            offset,
            size,
        }
    }
    fn document(nested: bool) -> Document {
        let mut nodes = vec![];
        if nested {
            nodes.push(node("root", None, [333, 777], [27, -31], [601, 303]));
            nodes.push(node(
                "parent",
                Some("root"),
                [500, 1000],
                [-13, 17],
                [401, 201],
            ));
        }
        let parent = nested.then_some("parent");
        nodes.push(node("reference", parent, [500, 500], [-19, 23], [101, 51]));
        nodes.push(node("target", parent, [0, 1000], [31, -17], [70, 40]));
        nodes.push(node("third", parent, [333, 777], [-7, 9], [63, 33]));
        nodes.push(node("child", Some("target"), [500, 500], [3, -5], [20, 10]));
        nodes.push(node("unrelated", None, [0, 0], [0, 0], [10, 10]));
        Document {
            schema: SCHEMA,
            nodes,
        }
    }
    fn selection() -> Vec<String> {
        ["target", "reference", "third"]
            .map(str::to_string)
            .to_vec()
    }
    fn index(document: &Document, id: &str) -> usize {
        document.nodes.iter().position(|n| n.id == id).unwrap()
    }
    fn ready(current: &Context, original: &Document) -> State {
        let mut state = State::default();
        state
            .stage(
                current.clone(),
                original,
                &selection(),
                "reference",
                Mode::Left,
            )
            .unwrap();
        assert!(state.is_ready());
        state
    }

    #[test]
    fn all_modes_root_and_nested_preserve_every_other_field_and_bound_residual() {
        for nested in [false, true] {
            for reference_anchor in [0, 333, 500, 1000] {
                for target_anchor in [0, 333, 500, 1000] {
                    let mut original = document(nested);
                    let r = index(&original, "reference");
                    let t = index(&original, "target");
                    original.nodes[r].anchor = [reference_anchor; 2];
                    original.nodes[t].anchor = [target_anchor; 2];
                    for mode in MODES {
                        let current = context();
                        let mut state = State::default();
                        state
                            .stage(current.clone(), &original, &selection(), "reference", mode)
                            .unwrap();
                        let candidate = state.candidate(&current, &original).unwrap();
                        let (axis, factor) = mode.axis_and_factor();
                        let extent = if nested {
                            i64::from(original.nodes[index(&original, "parent")].size[axis])
                        } else {
                            i64::from(current.viewport[axis])
                        };
                        let edge = |node: &Node| {
                            i64::from(node.offset[axis]) * 1000
                                + extent * i64::from(node.anchor[axis])
                                + factor * i64::from(node.size[axis])
                        };
                        for id in ["target", "third"] {
                            let n = index(&candidate, id);
                            assert!(
                                (edge(&candidate.nodes[n]) - edge(&candidate.nodes[r])).abs()
                                    <= 500
                            );
                        }
                        let mut restored = candidate.clone();
                        for id in ["target", "third"] {
                            let n = index(&restored, id);
                            restored.nodes[n].offset[axis] = original.nodes[n].offset[axis];
                        }
                        assert_eq!(restored, original);
                        assert_eq!(
                            Document::parse(&candidate.to_bytes().unwrap()).unwrap(),
                            candidate
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn rounds_final_offset_ties_away_from_zero_not_displacement() {
        for (reference_offset, reference_size, target_size, expected) in [
            (1, 1, 2, 1),
            (-1, 2, 1, -1),
            (0, 2, 1, 1),
            (0, 1, 2, -1),
            (1, 2, 1, 2),
            (-1, 1, 2, -2),
        ] {
            let reference = node(
                "reference",
                None,
                [0; 2],
                [reference_offset; 2],
                [reference_size; 2],
            );
            let target = node("target", None, [0; 2], [100; 2], [target_size; 2]);
            for axis in 0..2 {
                assert_eq!(
                    proposed_offset(&reference, &target, axis, 500, 641).unwrap(),
                    expected
                );
            }
        }
    }

    #[test]
    fn exact_bounds_allowed_but_unrounded_fractional_excursions_fail() {
        for sign in [-1, 1] {
            let mut reference = node("reference", None, [500; 2], [sign * 4096; 2], [10; 2]);
            let mut target = node("target", None, [500; 2], [0; 2], [10; 2]);
            assert_eq!(
                proposed_offset(&reference, &target, 0, 0, 1).unwrap(),
                sign * 4096
            );
            if sign > 0 {
                reference.anchor[0] += 1;
            } else {
                target.anchor[0] += 1;
            }
            assert!(proposed_offset(&reference, &target, 0, 0, 1).is_err());
        }
    }

    #[test]
    fn checked_arithmetic_rejects_internal_overflow_and_huge_viewports() {
        let reference = node("reference", None, [1000; 2], [1; 2], [10; 2]);
        let target = node("target", None, [0; 2], [0; 2], [10; 2]);
        assert!(proposed_offset(&reference, &target, 0, 1000, i64::MAX).is_err());
        let original = document(false);
        let mut current = context();
        current.viewport = [u32::MAX; 2];
        assert!(State::default()
            .stage(current, &original, &selection(), "reference", Mode::Left)
            .is_err());
    }

    #[test]
    fn invalid_selection_or_document_clears_whole_candidate() {
        let current = context();
        let original = document(true);
        for ids in [
            vec![],
            vec!["reference"],
            vec!["reference", "reference"],
            vec!["target", "third"],
            vec!["reference", "missing"],
            vec!["reference", "child"],
            vec!["reference", "unrelated"],
        ] {
            let mut state = ready(&current, &original);
            assert!(state
                .stage(
                    current.clone(),
                    &original,
                    &ids.into_iter().map(str::to_string).collect::<Vec<_>>(),
                    "reference",
                    Mode::Left
                )
                .is_err());
            assert!(!state.is_ready());
            assert!(state.document().is_none());
        }
        // A valid document may have root nodes on different screens. Sharing
        // parent=None still does not make those nodes eligible siblings.
        let mut mixed = document(false);
        let third = index(&mixed, "third");
        mixed.nodes[third].screen = Screen::Menu;
        mixed.to_bytes().unwrap();
        let mut state = ready(&current, &original);
        assert!(state
            .stage(
                current.clone(),
                &mixed,
                &selection(),
                "reference",
                Mode::Left
            )
            .is_err());
        assert!(!state.is_ready());
        assert_eq!(
            sibling_ids(&mixed, Screen::Title, "reference").unwrap(),
            vec!["reference", "target", "unrelated"]
        );
        let mut invalids = vec![original.clone(); 4];
        invalids[0].schema += 1;
        invalids[1].nodes[index(&original, "third")].size[0] = 0;
        invalids[2].nodes[index(&original, "third")].screen = Screen::Menu;
        invalids[3].nodes[index(&original, "third")].id = "reference".into();
        for invalid in invalids {
            let mut state = ready(&current, &original);
            assert!(state
                .stage(
                    current.clone(),
                    &invalid,
                    &selection(),
                    "reference",
                    Mode::Left
                )
                .is_err());
            assert!(!state.is_ready());
        }
    }

    #[test]
    fn later_target_failure_never_leaves_a_partial_candidate() {
        let current = context();
        let mut original = document(false);
        let r = index(&original, "reference");
        let t = index(&original, "target");
        let third = index(&original, "third");
        original.nodes[r].anchor = [1000; 2];
        original.nodes[r].offset = [4096; 2];
        original.nodes[t].anchor = [1000; 2];
        original.nodes[third].anchor = [999; 2];
        let snapshot = original.clone();
        let mut state = State::default();
        assert!(state
            .stage(current, &original, &selection(), "reference", Mode::Left)
            .is_err());
        assert!(state.document().is_none());
        assert_eq!(original, snapshot);
    }

    #[test]
    fn all_context_fields_and_entire_document_are_pinned() {
        let current = context();
        let original = document(true);
        let mut contexts = vec![current.clone(); 7];
        contexts[0].scene = "other".into();
        contexts[1].epoch += 1;
        contexts[2].screen = Screen::Menu;
        contexts[3].viewport[0] += 1;
        contexts[4].viewport[1] += 1;
        contexts[5].canvas[0] += 1.0;
        contexts[6].canvas[1] += 1.0;
        for changed in contexts {
            let mut state = ready(&current, &original);
            assert!(state.candidate(&changed, &original).is_err());
            assert!(state.validate_context(&changed, &original).is_err());
            assert!(!state.is_ready());
            assert!(state.candidate(&current, &original).is_err());
        }
        let mut documents = vec![original.clone(); 8];
        documents[0].schema += 1;
        documents[1].nodes[0].offset[0] += 1;
        documents[2].nodes.swap(0, 1);
        documents[3].nodes.pop();
        documents[4].nodes[0].id = "renamed".into();
        documents[5].nodes[0].size[0] += 1;
        documents[6].nodes[0].anchor[0] += 1;
        documents[7].nodes[index(&original, "child")].offset[1] += 1;
        for changed in documents {
            let mut state = ready(&current, &original);
            assert!(state.candidate(&current, &changed).is_err());
            assert!(state.validate_context(&current, &changed).is_err());
            assert!(!state.is_ready());
        }
        let mut state = ready(&current, &original);
        let candidate = state.candidate(&current, &original).unwrap();
        assert!(state.candidate(&current, &candidate).is_err());
        state.validate_context(&current, &original).unwrap();
        assert!(state.is_ready());
    }

    #[test]
    fn invalid_dimensions_fail_closed_at_stage_and_apply() {
        let original = document(false);
        let current = context();
        let mut invalids = vec![];
        for axis in 0..2 {
            let mut changed = current.clone();
            changed.viewport[axis] = 0;
            invalids.push(changed);
            for bad in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut changed = current.clone();
                changed.canvas[axis] = bad;
                invalids.push(changed);
            }
        }
        for invalid in invalids {
            let mut state = ready(&current, &original);
            assert!(state.candidate(&invalid, &original).is_err());
            assert!(state.validate_context(&invalid, &original).is_err());
            assert!(!state.is_ready());
            let mut state = ready(&current, &original);
            assert!(state
                .stage(invalid, &original, &selection(), "reference", Mode::Left)
                .is_err());
            assert!(!state.is_ready());
        }
    }

    #[test]
    fn noop_and_cancel_have_no_candidate_or_identity() {
        let current = context();
        let original = document(false);
        for mode in MODES {
            let mut state = State::default();
            state
                .stage(current.clone(), &original, &selection(), "reference", mode)
                .unwrap();
            let aligned = state.candidate(&current, &original).unwrap();
            state
                .stage(current.clone(), &aligned, &selection(), "reference", mode)
                .unwrap();
            assert!(!state.is_ready());
            assert!(state.document().is_none());
            assert!(state.reference_id().is_none());
            assert!(state.selected_ids().is_none());
            assert!(state.candidate(&current, &aligned).is_err());
        }
        let mut state = ready(&current, &original);
        assert_eq!(state.reference_id(), Some("reference"));
        assert_eq!(state.selected_ids(), Some(selection().as_slice()));
        state.cancel();
        state.cancel();
        assert!(!state.is_ready());
        state.validate_context(&current, &original).unwrap();
    }

    #[test]
    fn sibling_choices_follow_document_order_and_exclude_other_parents_or_screens() {
        let mut original = document(true);
        assert_eq!(
            sibling_ids(&original, Screen::Title, "reference").unwrap(),
            vec!["reference", "target", "third"]
        );
        assert!(sibling_ids(&original, Screen::Menu, "reference").is_err());
        assert!(sibling_ids(&original, Screen::Title, "missing").is_err());
        original.nodes[index(&document(true), "unrelated")].screen = Screen::Menu;
        assert_eq!(
            sibling_ids(&original, Screen::Title, "root").unwrap(),
            vec!["root"]
        );
    }
}
