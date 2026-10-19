//! Presentation-only Collect UI resize drafts. Pointer motion updates a ghost;
//! release prepares one validated document for an explicit authoring-panel Apply.
//! This module has no renderer, history, host or filesystem mutation authority.
use orr_sample::authored_ui::{Document, Screen};
use std::path::PathBuf;

/// Every coordinate-space and scene identity used by a resize gesture.
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    pub scene: PathBuf,
    pub epoch: u64,
    pub screen: Screen,
    /// The preview's logical-pixel viewport.
    pub viewport: [u32; 2],
    /// The displayed canvas dimensions in the pointer's coordinate space.
    pub canvas: [f32; 2],
}

impl Context {
    fn validate(&self) -> Result<(), String> {
        if self.viewport.contains(&0)
            || self
                .canvas
                .iter()
                .any(|axis| !axis.is_finite() || *axis <= 0.0)
        {
            return Err(
                "Resize draft requires positive finite viewport and canvas dimensions".into(),
            );
        }
        Ok(())
    }
}

struct Draft {
    context: Context,
    original: Document,
    selected_id: String,
    index: usize,
    start: [f32; 2],
    delta: [f32; 2],
    size: [u16; 2],
    candidate: Option<Document>,
}

#[derive(Default)]
pub struct State {
    draft: Option<Draft>,
}

impl State {
    /// Start against the complete accepted document. A failed or new start
    /// discards any older gesture, including its unapplied candidate.
    pub fn begin(
        &mut self,
        context: Context,
        accepted: &Document,
        node_id: &str,
        start_pointer: [f32; 2],
    ) -> Result<(), String> {
        self.cancel();
        context.validate()?;
        validate_pointer(start_pointer)?;
        accepted.to_bytes()?;
        let index = accepted
            .nodes
            .iter()
            .position(|node| node.id == node_id)
            .ok_or("Select an existing UI node to resize")?;
        let node = &accepted.nodes[index];
        if node.screen != context.screen {
            return Err("The selected UI node is not on the preview screen".into());
        }
        self.draft = Some(Draft {
            context,
            original: accepted.clone(),
            selected_id: node.id.clone(),
            index,
            start: start_pointer,
            delta: [0.0, 0.0],
            size: node.size,
            candidate: None,
        });
        Ok(())
    }

    /// Update only the pointer ghost and proposed size. No document clone,
    /// serialization, font validation or UI-context reconstruction happens here.
    /// Invalid input cancels the gesture rather than retaining a stale target.
    pub fn update(&mut self, pointer: [f32; 2]) -> Result<(), String> {
        let draft = self.draft.as_mut().ok_or("No UI resize drag is active")?;
        if draft.candidate.is_some() {
            return Err("The UI resize drag has already finished".into());
        }
        let result = proposed_resize(draft, pointer);
        match result {
            Ok((delta, size)) => {
                draft.delta = delta;
                draft.size = size;
                Ok(())
            }
            Err(error) => {
                self.cancel();
                Err(error)
            }
        }
    }

    /// Include the release position, then build and validate exactly one
    /// candidate. A rounded no-op clears the gesture and creates no candidate.
    /// The accepted document is unchanged until the caller explicitly applies it.
    pub fn finish(&mut self, release: [f32; 2]) -> Result<(), String> {
        self.update(release)?;
        let draft = self.draft.as_mut().expect("successful active drag update");
        if draft.size == draft.original.nodes[draft.index].size {
            self.cancel();
            return Ok(());
        }
        let mut candidate = draft.original.clone();
        candidate.nodes[draft.index].size = draft.size;
        if let Err(error) = candidate.to_bytes() {
            self.cancel();
            return Err(error);
        }
        draft.candidate = Some(candidate);
        Ok(())
    }

    /// Check both moving and released drafts before exposing them. Any accepted
    /// edit, scene/epoch change or coordinate-space change cancels the draft.
    /// The caller also cancels on selection changes, focus loss, Play or close.
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

    /// Return the released candidate only if its complete source snapshot still
    /// matches. Apply it through the existing panel once, then cancel this draft.
    /// This deliberately does not silently rebase onto intervening accepted edits.
    pub fn candidate(&self, current: &Context, accepted: &Document) -> Result<Document, String> {
        let draft = self.draft.as_ref().ok_or("No UI resize draft is ready")?;
        check_current(draft, current, accepted)?;
        draft
            .candidate
            .clone()
            .ok_or_else(|| "Release the UI resize drag before applying it".into())
    }

    pub fn cancel(&mut self) {
        self.draft = None;
    }

    pub fn is_dragging(&self) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|draft| draft.candidate.is_none())
    }

    pub fn is_ready(&self) -> bool {
        self.document().is_some()
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.draft.as_ref().map(|draft| draft.selected_id.as_str())
    }

    /// Raw pointer displacement in display-canvas coordinates, for a ghost.
    pub fn delta(&self) -> Option<[f32; 2]> {
        self.draft.as_ref().map(|draft| draft.delta)
    }

    /// Accepted size plus logical-pixel delta rounded with ties away from zero.
    /// Resizing preserves the anchored top-left, offset and all child records.
    pub fn size(&self) -> Option<[u16; 2]> {
        self.draft.as_ref().map(|draft| draft.size)
    }

    /// Read-only access for rendering a released candidate. This is not admission
    /// to the accepted document: use `candidate` to guard the explicit Apply.
    pub fn document(&self) -> Option<&Document> {
        self.draft.as_ref()?.candidate.as_ref()
    }
}

fn validate_pointer(pointer: [f32; 2]) -> Result<(), String> {
    if pointer.iter().any(|axis| !axis.is_finite()) {
        return Err("UI resize pointer must be finite".into());
    }
    Ok(())
}

fn check_current(draft: &Draft, current: &Context, accepted: &Document) -> Result<(), String> {
    current.validate()?;
    if draft.context != *current {
        return Err("UI resize context changed; start a new drag".into());
    }
    if draft.original != *accepted {
        return Err("Accepted UI document changed; start a new drag".into());
    }
    Ok(())
}

fn proposed_resize(draft: &Draft, pointer: [f32; 2]) -> Result<([f32; 2], [u16; 2]), String> {
    validate_pointer(pointer)?;
    let mut delta = [0.0; 2];
    let mut size = [0; 2];
    for axis in 0..2 {
        // Widen before subtracting/scaling so finite f32 inputs cannot silently
        // overflow intermediate f32 arithmetic. Check all bounds before casts.
        let display = f64::from(pointer[axis]) - f64::from(draft.start[axis]);
        let logical = display * f64::from(draft.context.viewport[axis])
            / f64::from(draft.context.canvas[axis]);
        let proposed = f64::from(draft.original.nodes[draft.index].size[axis]) + logical.round();
        if !display.is_finite()
            || display.abs() > f64::from(f32::MAX)
            || !logical.is_finite()
            || !proposed.is_finite()
            || !(1.0..=4096.0).contains(&proposed)
        {
            return Err("UI size must stay within 1..=4096 logical pixels".into());
        }
        delta[axis] = display as f32;
        size[axis] = proposed as u16;
    }
    Ok((delta, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_sample::authored_ui::{Action, Kind, Node, SCHEMA};

    fn context() -> Context {
        Context {
            scene: PathBuf::from("/virtual/collect/scene.yaml"),
            epoch: 7,
            screen: Screen::Title,
            viewport: [640, 360],
            canvas: [640.0, 360.0],
        }
    }

    fn document() -> Document {
        Document::default_collect()
    }

    fn started(context: &Context, original: &Document) -> State {
        let mut state = State::default();
        state
            .begin(context.clone(), original, "play", [20.0, 30.0])
            .unwrap();
        state
    }

    fn ready(context: &Context, original: &Document) -> State {
        let mut state = started(context, original);
        state.finish([40.0, 45.0]).unwrap();
        state
    }

    fn node(document: &Document) -> &Node {
        document
            .nodes
            .iter()
            .find(|node| node.id == "play")
            .unwrap()
    }

    #[test]
    fn finite_extreme_coordinates_cannot_overflow_intermediate_arithmetic() {
        let current = context();
        let original = document();
        let mut state = State::default();
        state
            .begin(current.clone(), &original, "play", [-f32::MAX, f32::MAX])
            .unwrap();
        assert!(state.finish([f32::MAX, -f32::MAX]).is_err());
        assert!(state.document().is_none());
        let current = Context {
            canvas: [f32::MIN_POSITIVE, f32::MIN_POSITIVE],
            ..current
        };
        let mut state = started(&current, &original);
        assert!(state.finish([21.0, 31.0]).is_err());
        assert!(state.document().is_none());
    }

    #[test]
    fn invalid_pointer_at_start_motion_or_release_fails_closed() {
        let current = context();
        let original = document();
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for axis in 0..2 {
                let mut pointer = [20.0, 30.0];
                pointer[axis] = bad;
                let mut state = ready(&current, &original);
                assert!(state
                    .begin(current.clone(), &original, "play", pointer)
                    .is_err());
                assert!(state.document().is_none());
                let mut state = started(&current, &original);
                assert!(state.update(pointer).is_err());
                assert!(!state.is_dragging());
                let mut state = started(&current, &original);
                assert!(state.finish(pointer).is_err());
                assert!(!state.is_ready());
            }
        }
    }

    #[test]
    fn dimensions_must_be_positive_and_finite_on_each_axis() {
        let original = document();
        for axis in 0..2 {
            for bad in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut current = context();
                current.canvas[axis] = bad;
                let mut state = State::default();
                assert!(state.begin(current, &original, "play", [0.0, 0.0]).is_err());
                assert!(state.document().is_none());
            }
            let mut current = context();
            current.viewport[axis] = 0;
            assert!(State::default()
                .begin(current, &original, "play", [0.0, 0.0])
                .is_err());
        }
    }

    #[test]
    fn every_pinned_context_change_rejects_drag_and_released_candidate() {
        let current = context();
        let original = document();
        let mut changed = vec![current.clone(); 7];
        changed[0].scene = PathBuf::from("/virtual/other/scene.yaml");
        changed[1].epoch += 1;
        changed[2].screen = Screen::Playing;
        changed[3].viewport[0] += 1;
        changed[4].viewport[1] += 1;
        changed[5].canvas[0] += 1.0;
        changed[6].canvas[1] += 1.0;
        for changed in changed {
            let mut moving = started(&current, &original);
            assert!(moving.validate_context(&changed, &original).is_err());
            assert!(!moving.is_dragging());
            let mut state = ready(&current, &original);
            assert!(state.candidate(&changed, &original).is_err());
            assert!(state.validate_context(&changed, &original).is_err());
            assert!(!state.is_ready());
            // Returning to the old context never resurrects a cancelled draft.
            state.validate_context(&current, &original).unwrap();
            assert!(state.candidate(&current, &original).is_err());
        }
    }

    #[test]
    fn invalid_current_dimensions_cancel_an_existing_draft() {
        let current = context();
        let original = document();
        let mut state = ready(&current, &original);
        let invalid = Context {
            canvas: [f32::NAN, 360.0],
            ..current.clone()
        };
        assert!(state.candidate(&invalid, &original).is_err());
        assert!(state.validate_context(&invalid, &original).is_err());
        assert!(!state.is_ready());
    }

    #[test]
    fn entire_accepted_document_is_pinned_not_just_selected_geometry() {
        let current = context();
        let original = document();
        let index = original
            .nodes
            .iter()
            .position(|node| node.id == "play")
            .unwrap();
        let mut changed = vec![original.clone(); 10];
        changed[0].schema += 1;
        changed[1].nodes[index].id = "renamed".into();
        changed[2].nodes[index].offset[0] += 1;
        changed[3].nodes[index].size[1] += 1;
        changed[4].nodes[index].anchor[0] += 1;
        changed[5].nodes[index].screen = Screen::Menu;
        changed[6].nodes[index].parent = Some("new_parent".into());
        changed[7].nodes[index].kind = Kind::Button {
            text: "Changed".into(),
            action: Action::Quit,
        };
        changed[8].nodes[0].offset[0] += 1;
        changed[9].nodes.swap(0, 1);
        for changed in changed {
            let mut moving = started(&current, &original);
            assert!(moving.validate_context(&current, &changed).is_err());
            assert!(!moving.is_dragging());
            let mut state = ready(&current, &original);
            assert!(state.candidate(&current, &changed).is_err());
            assert!(state.validate_context(&current, &changed).is_err());
            assert!(!state.is_ready());
        }
    }

    #[test]
    fn missing_node_wrong_screen_and_invalid_source_refuse_start() {
        let current = context();
        let original = document();
        for id in ["missing", "score"] {
            let mut state = ready(&current, &original);
            assert!(state
                .begin(current.clone(), &original, id, [0.0, 0.0])
                .is_err());
            assert!(!state.is_ready());
        }
        let mut invalid = original.clone();
        invalid.nodes[0].size = [0, 0];
        assert!(State::default()
            .begin(current, &invalid, "play", [0.0, 0.0])
            .is_err());
    }

    #[test]
    fn removed_selected_or_unrelated_node_rejects_apply() {
        let current = context();
        let original = document();
        let state = ready(&current, &original);
        for id in ["play", "score"] {
            let mut changed = original.clone();
            changed.nodes.retain(|node| node.id != id);
            assert!(state.candidate(&current, &changed).is_err());
        }
    }

    #[test]
    fn unchanged_context_and_document_keep_motion_and_ready_candidate() {
        let current = context();
        let original = document();
        let mut state = started(&current, &original);
        state.update([25.0, 35.0]).unwrap();
        state
            .validate_context(&current.clone(), &original.clone())
            .unwrap();
        assert_eq!(state.delta(), Some([5.0, 5.0]));
        state.finish([30.0, 40.0]).unwrap();
        let candidate = state.document().unwrap().clone();
        state
            .validate_context(&current.clone(), &original.clone())
            .unwrap();
        assert_eq!(state.document(), Some(&candidate));
    }

    #[test]
    fn zero_subpixel_and_return_to_origin_create_no_candidate() {
        let current = context();
        let original = document();
        for release in [[20.0, 30.0], [20.25, 29.75]] {
            let mut state = started(&current, &original);
            state.update([50.0, 60.0]).unwrap();
            state.finish(release).unwrap();
            assert!(!state.is_dragging());
            assert!(!state.is_ready());
            assert!(state.candidate(&current, &original).is_err());
            assert!(state.selected_id().is_none());
        }
    }

    #[test]
    fn cancel_clears_moving_or_released_state_without_touching_source() {
        let current = context();
        let original = document();
        for mut state in [started(&current, &original), ready(&current, &original)] {
            state.cancel();
            state.cancel();
            assert!(state.document().is_none());
            assert!(state.selected_id().is_none());
            assert!(state.delta().is_none());
            assert!(state.size().is_none());
            assert!(!state.is_ready());
            assert!(!state.is_dragging());
            assert!(state.update([30.0, 40.0]).is_err());
            assert!(state.finish([30.0, 40.0]).is_err());
            assert!(state.candidate(&current, &original).is_err());
            state.validate_context(&current, &original).unwrap();
        }
        assert_eq!(original, document());
    }

    #[test]
    fn new_begin_replaces_old_selection_context_and_candidate() {
        let current = context();
        let original = document();
        let mut state = ready(&current, &original);
        let changed = Context {
            screen: Screen::Playing,
            epoch: 9,
            ..current.clone()
        };
        state
            .begin(changed.clone(), &original, "score", [10.0, 10.0])
            .unwrap();
        assert!(state.document().is_none());
        assert_eq!(state.selected_id(), Some("score"));
        assert_eq!(state.size(), Some([320, 36]));
        state.finish([12.0, 13.0]).unwrap();
        assert!(state.candidate(&current, &original).is_err());
        assert_eq!(
            state.candidate(&changed, &original).unwrap().nodes[4].size,
            [322, 39]
        );
    }

    #[test]
    fn resizes_with_landscape_portrait_and_independent_axis_scales() {
        for (viewport, canvas, expected) in [
            ([640, 360], [640.0, 360.0], [329, 25]),
            ([640, 360], [320.0, 180.0], [338, 14]),
            ([360, 640], [180.0, 320.0], [338, 14]),
            ([360, 640], [180.0, 640.0], [338, 25]),
            ([640, 360], [1280.0, 720.0], [325, 30]),
        ] {
            let current = Context {
                viewport,
                canvas,
                ..context()
            };
            let original = document();
            let mut state = started(&current, &original);
            state.finish([29.0, 19.0]).unwrap();
            let candidate = state.candidate(&current, &original).unwrap();
            assert_eq!(node(&candidate).size, expected);
            assert_eq!(
                Document::parse(&candidate.to_bytes().unwrap()).unwrap(),
                candidate
            );
        }
    }

    #[test]
    fn rounds_delta_ties_away_before_adding_accepted_size() {
        let current = context();
        let original = document();
        for (release, expected) in [
            ([20.5, 29.5], [321, 35]),
            ([19.5, 30.5], [319, 37]),
            ([21.5, 28.5], [322, 34]),
            ([18.5, 31.5], [318, 38]),
        ] {
            let mut state = started(&current, &original);
            state.finish(release).unwrap();
            assert_eq!(state.size(), Some(expected));
        }
    }

    #[test]
    fn motion_only_tracks_numeric_ghost_and_release_includes_final_pointer() {
        let current = context();
        let original = document();
        let mut state = started(&current, &original);
        for step in 1..50 {
            state.update([20.0 + step as f32, 30.0]).unwrap();
            assert!(state.is_dragging());
            assert!(!state.is_ready());
            assert!(state.document().is_none());
            assert!(state.candidate(&current, &original).is_err());
        }
        assert_eq!(state.delta(), Some([49.0, 0.0]));
        assert_eq!(state.size(), Some([369, 36]));
        state.finish([80.0, 35.0]).unwrap();
        assert!(!state.is_dragging());
        assert!(state.is_ready());
        assert_eq!(state.size(), Some([380, 41]));
        assert_eq!(state.delta(), Some([60.0, 5.0]));
        assert_eq!(original, document());
    }

    #[test]
    fn repeated_release_or_motion_cannot_change_candidate_and_second_apply_is_stale() {
        let current = context();
        let original = document();
        let mut state = ready(&current, &original);
        let candidate = state.candidate(&current, &original).unwrap();
        assert!(state.finish([60.0, 80.0]).is_err());
        assert!(state.update([60.0, 80.0]).is_err());
        assert_eq!(state.document(), Some(&candidate));
        assert!(state.candidate(&current, &candidate).is_err());
    }

    #[test]
    fn exact_limits_are_accepted_and_overflow_is_rejected_without_clamping() {
        let current = context();
        let mut original = document();
        original
            .nodes
            .iter_mut()
            .find(|node| node.id == "play")
            .unwrap()
            .size = [4095, 2];
        let mut state = started(&current, &original);
        state.finish([21.0, 29.0]).unwrap();
        assert_eq!(state.size(), Some([4096, 1]));
        for release in [
            [21.5, 29.0],
            [21.0, 28.5],
            [f32::MAX, 30.0],
            [20.0, -f32::MAX],
        ] {
            let mut state = started(&current, &original);
            assert!(state.finish(release).is_err());
            assert!(!state.is_ready());
            assert!(!state.is_dragging());
            assert!(state.candidate(&current, &original).is_err());
        }
        assert_eq!(node(&original).size, [4095, 2]);
    }

    #[test]
    fn subpixel_movement_at_either_limit_can_remain_noop() {
        let current = context();
        let mut original = document();
        original
            .nodes
            .iter_mut()
            .find(|node| node.id == "play")
            .unwrap()
            .size = [4096, 1];
        let mut state = started(&current, &original);
        state.finish([20.25, 29.75]).unwrap();
        assert!(!state.is_ready());
        assert!(!state.is_dragging());
    }

    #[test]
    fn all_anchors_and_nested_nodes_preserve_every_field_except_selected_size() {
        for anchor in [0, 500, 1000] {
            let original = Document {
                schema: SCHEMA,
                nodes: vec![
                    Node {
                        id: "root".into(),
                        parent: None,
                        kind: Kind::Container,
                        screen: Screen::Title,
                        anchor: [anchor, anchor],
                        offset: [25, -15],
                        size: [500, 300],
                    },
                    Node {
                        id: "inner".into(),
                        parent: Some("root".into()),
                        kind: Kind::Container,
                        screen: Screen::Title,
                        anchor: [anchor, anchor],
                        offset: [-7, 8],
                        size: [400, 200],
                    },
                    Node {
                        id: "play".into(),
                        parent: Some("inner".into()),
                        kind: Kind::Button {
                            text: "플레이".into(),
                            action: Action::Play,
                        },
                        screen: Screen::Title,
                        anchor: [anchor, anchor],
                        offset: [-12, 14],
                        size: [120, 30],
                    },
                    Node {
                        id: "sibling".into(),
                        parent: Some("inner".into()),
                        kind: Kind::Container,
                        screen: Screen::Title,
                        anchor: [anchor, anchor],
                        offset: [10, 10],
                        size: [100, 20],
                    },
                ],
            };
            let current = context();
            for id in ["root", "inner", "play", "sibling"] {
                let mut state = State::default();
                state
                    .begin(current.clone(), &original, id, [0.0, 0.0])
                    .unwrap();
                state.finish([5.0, -6.0]).unwrap();
                let mut expected = original.clone();
                let selected = expected
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == id)
                    .unwrap();
                selected.size[0] += 5;
                selected.size[1] -= 6;
                assert_eq!(state.candidate(&current, &original).unwrap(), expected);
                assert_eq!(state.selected_id(), Some(id));
            }
        }
    }
}
