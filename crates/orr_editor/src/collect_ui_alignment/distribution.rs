//! Equal edge gaps between fixed spatial endpoints. Never rounds a gap first.
use super::{Context, Draft, State};
use orr_sample::authored_ui::{Document, Node};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistributionAxis {
    Horizontal,
    Vertical,
}

impl DistributionAxis {
    fn index(self) -> usize {
        match self {
            Self::Horizontal => 0,
            Self::Vertical => 1,
        }
    }
}

impl State {
    /// Stage equal edge gaps for at least three siblings, fixing the spatially
    /// first/last nodes. Ties use stable IDs, never selection or authored order.
    /// Insufficient room or overlap after integer quantization rejects the whole
    /// candidate. Each moved edge has <=0.5px rounding error; gaps have <=1px.
    /// Apply must use `candidate` with the current context and accepted document.
    pub fn stage_distribution(
        &mut self,
        context: Context,
        accepted: &Document,
        selected_ids: &[String],
        axis: DistributionAxis,
    ) -> Result<(), String> {
        self.cancel();
        context.validate()?;
        accepted.to_bytes()?;
        let unique: BTreeSet<&str> = selected_ids.iter().map(String::as_str).collect();
        if unique.len() < 3 || unique.len() != selected_ids.len() {
            return Err("Select at least three distinct UI siblings without duplicate IDs".into());
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
        let first = &accepted.nodes[indices[0]];
        if indices.iter().any(|&i| {
            accepted.nodes[i].screen != context.screen || accepted.nodes[i].parent != first.parent
        }) {
            return Err(
                "Distribution requires siblings on the preview screen with the same parent".into(),
            );
        }
        let axis = axis.index();
        let extent = match &first.parent {
            Some(parent) => i64::from(
                accepted
                    .nodes
                    .iter()
                    .find(|node| node.id == *parent)
                    .ok_or("The distribution parent does not exist")?
                    .size[axis],
            ),
            None => i64::from(context.viewport[axis]),
        };
        let mut ordered = indices
            .into_iter()
            .map(|i| leading(&accepted.nodes[i], axis, extent).map(|left| (left, i)))
            .collect::<Result<Vec<_>, _>>()?;
        ordered.sort_by(|(left_a, a), (left_b, b)| {
            left_a
                .cmp(left_b)
                .then_with(|| accepted.nodes[*a].id.cmp(&accepted.nodes[*b].id))
        });
        let d = i64::try_from(ordered.len() - 1).map_err(|_| overflow())?;
        let widths = ordered[..ordered.len() - 1]
            .iter()
            .try_fold(0i64, |sum, &(_, i)| {
                sum.checked_add(i64::from(accepted.nodes[i].size[axis]) * 1000)
                    .ok_or_else(overflow)
            })?;
        let start = ordered[0].0;
        let space = ordered[ordered.len() - 1]
            .0
            .checked_sub(start)
            .and_then(|span| span.checked_sub(widths))
            .ok_or_else(overflow)?;
        if space < 0 {
            return Err(
                "Not enough room between fixed endpoints for non-overlapping equal gaps".into(),
            );
        }
        let mut proposals = Vec::new();
        let mut prefix = 0i64;
        for (ordinal, &(_, index)) in ordered.iter().enumerate().take(ordered.len() - 1) {
            let node = &accepted.nodes[index];
            if ordinal > 0 {
                let ordinal = i64::try_from(ordinal).map_err(|_| overflow())?;
                let anchor = extent
                    .checked_mul(i64::from(node.anchor[axis]))
                    .ok_or_else(overflow)?;
                let numerator = start
                    .checked_add(prefix)
                    .and_then(|n| n.checked_sub(anchor))
                    .and_then(|n| n.checked_mul(d))
                    .and_then(|n| {
                        ordinal
                            .checked_mul(space)
                            .and_then(|gap| n.checked_add(gap))
                    })
                    .ok_or_else(overflow)?;
                proposals.push((index, round_offset(numerator, d)?));
            }
            prefix = prefix
                .checked_add(i64::from(node.size[axis]) * 1000)
                .ok_or_else(overflow)?;
        }
        // Every proposal is validated before constructing any candidate.
        let mut candidate = accepted.clone();
        for (index, offset) in proposals {
            candidate.nodes[index].offset[axis] = offset;
        }
        for pair in ordered.windows(2) {
            let left = &candidate.nodes[pair[0].1];
            let right = &candidate.nodes[pair[1].1];
            let end = leading(left, axis, extent)?
                .checked_add(i64::from(left.size[axis]) * 1000)
                .ok_or_else(overflow)?;
            if leading(right, axis, extent)? < end {
                return Err(
                    "Integer offset rounding would overlap siblings; increase endpoint spacing"
                        .into(),
                );
            }
        }
        candidate.to_bytes()?;
        if candidate == *accepted {
            return Ok(());
        }
        self.draft = Some(Draft {
            context,
            original: accepted.clone(),
            selected_ids: selected_ids.to_vec(),
            reference_id: None,
            candidate,
        });
        Ok(())
    }
}

fn overflow() -> String {
    "UI distribution arithmetic overflow".into()
}

fn leading(node: &Node, axis: usize, extent: i64) -> Result<i64, String> {
    extent
        .checked_mul(i64::from(node.anchor[axis]))
        .and_then(|n| n.checked_add(i64::from(node.offset[axis]) * 1000))
        .ok_or_else(overflow)
}

fn round_offset(numerator: i64, gaps: i64) -> Result<i16, String> {
    if gaps <= 0 {
        return Err(overflow());
    }
    let limit = 4_096_000i64.checked_mul(gaps).ok_or_else(overflow)?;
    if !(-limit..=limit).contains(&numerator) {
        return Err(
            "Distributed UI offset must stay within -4096..=4096 logical pixels before rounding"
                .into(),
        );
    }
    let denominator = 1000i64.checked_mul(gaps).ok_or_else(overflow)?;
    let rounded = numerator
        .checked_add(if numerator < 0 {
            -denominator / 2
        } else {
            denominator / 2
        })
        .ok_or_else(overflow)?
        / denominator;
    i16::try_from(rounded).map_err(|_| overflow())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect_ui_alignment::Mode;
    use orr_sample::authored_ui::{Kind, Screen, SCHEMA};

    fn context() -> Context {
        Context {
            scene: "/virtual/collect/scene.yaml".into(),
            epoch: 7,
            screen: Screen::Title,
            viewport: [641, 359],
            canvas: [480.75, 269.25],
        }
    }
    fn node(id: &str, offset: i16, size: u16) -> Node {
        Node {
            id: id.into(),
            parent: None,
            kind: Kind::Container,
            screen: Screen::Title,
            anchor: [0, 0],
            offset: [offset, offset],
            size: [size, size],
        }
    }
    fn document() -> Document {
        Document {
            schema: SCHEMA,
            nodes: vec![node("a", 0, 10), node("b", 20, 20), node("z", 100, 30)],
        }
    }
    fn ids() -> Vec<String> {
        ["z", "b", "a"].map(str::to_string).to_vec()
    }
    fn staged(doc: &Document, ctx: &Context, axis: DistributionAxis) -> State {
        let mut state = State::default();
        state
            .stage_distribution(ctx.clone(), doc, &ids(), axis)
            .unwrap();
        assert!(state.is_ready());
        state
    }

    #[test]
    fn both_axes_root_and_nested_preserve_every_unowned_field() {
        for nested in [false, true] {
            for axis in [DistributionAxis::Horizontal, DistributionAxis::Vertical] {
                let mut doc = document();
                if nested {
                    for node in &mut doc.nodes {
                        node.parent = Some("parent".into());
                    }
                    let mut parent = node("parent", -31, 401);
                    parent.anchor = [333, 777];
                    doc.nodes.insert(0, parent);
                }
                let mut child = node("child", -5, 7);
                child.parent = Some("b".into());
                doc.nodes.push(child);
                doc.nodes.push(node("unrelated", -400, 5));
                let state = staged(&doc, &context(), axis);
                let candidate = state.candidate(&context(), &doc).unwrap();
                let mut expected = doc.clone();
                expected
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == "b")
                    .unwrap()
                    .offset[axis.index()] = 45;
                assert_eq!(candidate, expected);
                assert_eq!(state.reference_id(), None);
                assert_eq!(state.selected_ids(), Some(ids().as_slice()));
            }
        }
    }

    #[test]
    fn fractional_anchors_round_final_offset_and_bound_gap_residual() {
        let mut doc = document();
        doc.nodes[1].anchor[0] = 333;
        doc.nodes[1].offset[0] = -200;
        let candidate = staged(&doc, &context(), DistributionAxis::Horizontal)
            .candidate(&context(), &doc)
            .unwrap();
        assert_eq!(candidate.nodes[1].offset[0], -168);
        let left_gap = leading(&candidate.nodes[1], 0, 641).unwrap() - 10_000;
        let right_gap = 100_000 - leading(&candidate.nodes[1], 0, 641).unwrap() - 20_000;
        assert!((left_gap - 35_000).abs() <= 500);
        assert!((right_gap - 35_000).abs() <= 500);
        assert_eq!(candidate.nodes[0], doc.nodes[0]);
        assert_eq!(candidate.nodes[2], doc.nodes[2]);
    }

    #[test]
    fn selection_and_authored_order_do_not_change_stable_id_ties() {
        let mut doc = document();
        doc.nodes[1].offset[0] = 0;
        let expected = staged(&doc, &context(), DistributionAxis::Horizontal)
            .candidate(&context(), &doc)
            .unwrap();
        for selection in [["a", "z", "b"], ["b", "a", "z"], ["z", "b", "a"]] {
            let mut reordered = doc.clone();
            reordered.nodes.swap(0, 1);
            let mut state = State::default();
            state
                .stage_distribution(
                    context(),
                    &reordered,
                    &selection.map(str::to_string),
                    DistributionAxis::Horizontal,
                )
                .unwrap();
            let actual = state.candidate(&context(), &reordered).unwrap();
            for node in &expected.nodes {
                assert_eq!(actual.nodes.iter().find(|n| n.id == node.id), Some(node));
            }
            assert_eq!(actual.nodes[0].id, "b");
        }
    }

    #[test]
    fn rational_gaps_do_not_accumulate_rounding_and_restage_is_noop() {
        for count in [3, 4, 32] {
            let mut doc = Document {
                schema: SCHEMA,
                nodes: (0..count)
                    .map(|i| node(&format!("n{i:02}"), (i * 2) as i16, 1))
                    .collect(),
            };
            doc.nodes[count - 1].offset[0] = 101;
            let selected: Vec<_> = doc.nodes.iter().map(|n| n.id.clone()).collect();
            let mut state = State::default();
            state
                .stage_distribution(context(), &doc, &selected, DistributionAxis::Horizontal)
                .unwrap();
            let candidate = state.candidate(&context(), &doc).unwrap();
            let d = (count - 1) as i64;
            for (i, node) in candidate.nodes.iter().enumerate() {
                assert!(
                    (i64::from(node.offset[0]) * 1000 * d - i as i64 * 101_000).abs() <= 500 * d
                );
            }
            assert_eq!(candidate.nodes[0], doc.nodes[0]);
            assert_eq!(candidate.nodes[count - 1], doc.nodes[count - 1]);
            state
                .stage_distribution(
                    context(),
                    &candidate,
                    &selected,
                    DistributionAxis::Horizontal,
                )
                .unwrap();
            assert!(!state.is_ready());
        }
    }

    #[test]
    fn signed_ties_and_unrounded_bounds_are_exact() {
        for d in [2, 3, 31] {
            assert_eq!(round_offset(1500 * d, d).unwrap(), 2);
            assert_eq!(round_offset(-1500 * d, d).unwrap(), -2);
            assert_eq!(round_offset(499 * d, d).unwrap(), 0);
            assert_eq!(round_offset(-499 * d, d).unwrap(), 0);
            assert_eq!(round_offset(4_096_000 * d, d).unwrap(), 4096);
            assert_eq!(round_offset(-4_096_000 * d, d).unwrap(), -4096);
            assert!(round_offset(4_096_000 * d + 1, d).is_err());
            assert!(round_offset(-4_096_000 * d - 1, d).is_err());
        }
    }

    #[test]
    fn checked_helpers_reject_overflow_and_bad_divisors() {
        assert!(round_offset(0, 0).is_err());
        assert!(round_offset(0, -1).is_err());
        assert!(round_offset(0, i64::MAX).is_err());
        let mut n = node("a", 0, 1);
        n.anchor[0] = 1000;
        assert!(leading(&n, 0, i64::MAX).is_err());
        let mut ctx = context();
        ctx.viewport = [u32::MAX, u32::MAX];
        let mut doc = document();
        doc.nodes[1].anchor[0] = 1000;
        let mut state = staged(&document(), &context(), DistributionAxis::Horizontal);
        assert!(state
            .stage_distribution(ctx, &doc, &ids(), DistributionAxis::Horizontal)
            .is_err());
        assert!(!state.is_ready());
    }

    #[test]
    fn negative_space_and_quantized_overlap_reject_whole_draft() {
        let mut state = staged(&document(), &context(), DistributionAxis::Horizontal);
        let mut doc = document();
        doc.nodes[2].offset[0] = 25;
        assert!(state
            .stage_distribution(context(), &doc, &ids(), DistributionAxis::Horizontal)
            .unwrap_err()
            .contains("Not enough room"));
        assert!(!state.is_ready());
        state = staged(&document(), &context(), DistributionAxis::Horizontal);
        let mut ctx = context();
        ctx.viewport[0] = 1;
        let mut doc = Document {
            schema: SCHEMA,
            nodes: vec![node("a", 0, 1), node("b", 0, 1), node("z", 2, 1)],
        };
        doc.nodes[1].anchor[0] = 500;
        assert!(state
            .stage_distribution(ctx, &doc, &ids(), DistributionAxis::Horizontal)
            .unwrap_err()
            .contains("rounding would overlap"));
        assert!(!state.is_ready());
        assert!(state.document().is_none());
    }

    #[test]
    fn original_overlap_can_be_resolved_and_zero_gaps_are_allowed() {
        let mut doc = document();
        doc.nodes[1].offset[0] = 5;
        let candidate = staged(&doc, &context(), DistributionAxis::Horizontal)
            .candidate(&context(), &doc)
            .unwrap();
        assert_eq!(candidate.nodes[1].offset[0], 45);
        doc.nodes[2].offset[0] = 30;
        let candidate = staged(&doc, &context(), DistributionAxis::Horizontal)
            .candidate(&context(), &doc)
            .unwrap();
        assert_eq!(candidate.nodes[1].offset[0], 10);
    }

    #[test]
    fn invalid_selection_document_or_context_clears_previous_draft() {
        let original = document();
        for selected in [
            vec!["a", "b"],
            vec!["a", "b", "b"],
            vec!["a", "b", "missing"],
        ] {
            let mut state = staged(&original, &context(), DistributionAxis::Horizontal);
            assert!(state
                .stage_distribution(
                    context(),
                    &original,
                    &selected.into_iter().map(str::to_string).collect::<Vec<_>>(),
                    DistributionAxis::Horizontal
                )
                .is_err());
            assert!(!state.is_ready());
        }
        for variant in 0..5 {
            let mut doc = original.clone();
            let mut ctx = context();
            match variant {
                0 => doc.nodes[1].screen = Screen::Menu,
                1 => doc.nodes[1].parent = Some("a".into()),
                2 => doc.schema = 999,
                3 => ctx.viewport[0] = 0,
                _ => ctx.canvas[0] = f32::NAN,
            }
            let mut state = staged(&original, &context(), DistributionAxis::Horizontal);
            assert!(state
                .stage_distribution(ctx, &doc, &ids(), DistributionAxis::Horizontal)
                .is_err());
            assert!(!state.is_ready());
        }
    }

    #[test]
    fn entire_document_and_every_context_field_are_pinned() {
        let doc = document();
        for variant in 0..7 {
            let mut current = context();
            let mut accepted = doc.clone();
            match variant {
                0 => current.scene = "/other".into(),
                1 => current.epoch += 1,
                2 => current.screen = Screen::Menu,
                3 => current.viewport[0] += 1,
                4 => current.canvas[1] += 1.0,
                5 => accepted.nodes[0].size[1] += 1,
                _ => accepted.nodes.swap(0, 1),
            }
            let mut state = staged(&doc, &context(), DistributionAxis::Horizontal);
            assert!(state.candidate(&current, &accepted).is_err());
            assert!(state.validate_context(&current, &accepted).is_err());
            assert!(!state.is_ready());
        }
    }

    #[test]
    fn later_target_out_of_bounds_never_stages_partial_result() {
        let doc = Document {
            schema: SCHEMA,
            nodes: vec![
                node("a", 0, 10),
                node("b", 100, 10),
                node("c", 200, 10),
                Node {
                    anchor: [1000, 0],
                    ..node("z", 2904, 10)
                },
            ],
        };
        let mut ctx = context();
        ctx.viewport[0] = 4096;
        let mut state = State::default();
        assert!(state
            .stage_distribution(
                ctx,
                &doc,
                &["a", "b", "c", "z"].map(str::to_string),
                DistributionAxis::Horizontal
            )
            .is_err());
        assert!(!state.is_ready());
        assert!(state.document().is_none());
    }

    #[test]
    fn cancellation_and_switching_alignment_preserve_compatible_api() {
        let doc = document();
        let ctx = context();
        let mut state = staged(&doc, &ctx, DistributionAxis::Horizontal);
        state.cancel();
        assert!(!state.is_ready());
        assert!(state.selected_ids().is_none());
        state
            .stage(ctx.clone(), &doc, &ids(), "b", Mode::Left)
            .unwrap();
        assert_eq!(state.reference_id(), Some("b"));
        state
            .stage_distribution(ctx.clone(), &doc, &ids(), DistributionAxis::Horizontal)
            .unwrap();
        assert_eq!(state.reference_id(), None);
        let candidate = state.candidate(&ctx, &doc).unwrap();
        state
            .stage_distribution(ctx, &candidate, &ids(), DistributionAxis::Horizontal)
            .unwrap();
        assert!(!state.is_ready());
        assert!(state.selected_ids().is_none());
    }
}
