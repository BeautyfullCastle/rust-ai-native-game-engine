//! Authored collect-game UI. Layout and pointer ownership stay entirely in the view.
use crate::authored_ui::{Action, Binding, Document, Kind, Profile, Screen};

#[derive(Clone, Debug, Default)]
pub struct Hud {
    pub score: u32,
    pub phase: u32,
    pub best: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RoomHud {
    pub key_acquired: bool,
    pub won: bool,
}

enum Values {
    Collect(Hud),
    Room(RoomHud),
}
impl Values {
    fn phase(&self) -> u32 {
        match self {
            Self::Collect(h) => h.phase,
            Self::Room(h) => u32::from(h.won),
        }
    }
    fn value(&self, binding: Binding) -> String {
        match (self, binding) {
            (Self::Collect(h), Binding::Score) => h.score.to_string(),
            (Self::Collect(h), Binding::Phase) => phase_text(h.phase).into(),
            (Self::Collect(h), Binding::Best) => h.best.clone(),
            (Self::Room(h), Binding::KeyAcquired) => if h.key_acquired {
                "ACQUIRED"
            } else {
                "MISSING"
            }
            .into(),
            (Self::Room(h), Binding::ExitState) => {
                if h.key_acquired { "UNLOCKED" } else { "LOCKED" }.into()
            }
            (Self::Room(h), Binding::RoomPhase) => if h.won { "WON" } else { "PLAYING" }.into(),
            _ => "INVALID".into(),
        }
    }
}

#[derive(Clone)]
struct Placed {
    index: usize,
    rect: egui::Rect,
    clip: egui::Rect,
}

pub struct CollectUi {
    pub context: egui::Context,
    document: Document,
    profile: Profile,
    screen: Screen,
    placed: Vec<Placed>,
    pressed: Option<String>,
    pointer_layout_stale: bool,
    viewport: Option<egui::Rect>,
    pixels_per_point: Option<f32>,
    phase: u32,
    awaiting_restart: bool,
}

impl CollectUi {
    pub fn new(document: Document, font: Vec<u8>) -> Result<Self, String> {
        Self::new_for(document, font, Profile::Collect)
    }

    pub fn new_for(document: Document, font: Vec<u8>, profile: Profile) -> Result<Self, String> {
        Self::validate_font_for(&font, &document, profile)?;
        let context = egui::Context::default();
        let mut definitions = egui::FontDefinitions::default();
        definitions.font_data.insert(
            "collect-authored".into(),
            egui::FontData::from_owned(font).into(),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            definitions
                .families
                .entry(family)
                .or_default()
                .insert(0, "collect-authored".into());
        }
        context.set_fonts(definitions);
        Ok(Self {
            context,
            document,
            profile,
            screen: Screen::Title,
            placed: Vec::new(),
            pressed: None,
            pointer_layout_stale: true,
            viewport: None,
            pixels_per_point: None,
            phase: 0,
            awaiting_restart: false,
        })
    }

    /// Validate schema and the complete authored/dynamic text corpus without a GPU context.
    pub fn validate_font(font: &[u8], document: &Document) -> Result<(), String> {
        Self::validate_font_for(font, document, Profile::Collect)
    }

    pub fn validate_font_for(
        font: &[u8],
        document: &Document,
        profile: Profile,
    ) -> Result<(), String> {
        document.validate_for(profile)?;
        crate::game_ui::GameUi::validate_font_with_corpus(font, &document.corpus_for(profile))
    }

    /// The validated document is immutable; construct a new UI to load an edit.
    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn blocks_controls(&self) -> bool {
        self.screen != Screen::Playing
    }

    pub fn apply(&mut self, action: Action) {
        let screen = match action {
            Action::Play | Action::Restart => Screen::Playing,
            Action::Continue if self.phase != 0 && !self.awaiting_restart => Screen::Terminal,
            Action::Continue => Screen::Playing,
            Action::Menu => Screen::Menu,
            Action::Quit => return,
        };
        if matches!(action, Action::Play | Action::Restart) {
            // The next render may still carry the pre-restart terminal snapshot.
            // Keep controls available until the simulation acknowledges the edge.
            self.awaiting_restart = action == Action::Restart || self.phase != 0;
            self.phase = 0;
        }
        self.screen = screen;
        self.invalidate_pointer_layout();
    }

    /// Clear the presentation-side wait after authoritative restart delivery or
    /// cancellation. The host should call this when its input queue acknowledges
    /// an advanced tick, including updates that restart and finish before rendering.
    pub fn acknowledge_restart(&mut self) {
        self.awaiting_restart = false;
    }

    pub fn invalidate_pointer_layout(&mut self) {
        self.pointer_layout_stale = true;
        self.pressed = None;
        self.placed.clear();
    }

    pub fn pending_pointer_over_ui(&self, input: &egui::RawInput) -> bool {
        if self.pointer_layout_stale || self.input_layout_changed(input) {
            return true;
        }
        let position = input
            .events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::PointerMoved(point)
                | egui::Event::PointerButton { pos: point, .. } => Some(Some(*point)),
                egui::Event::PointerGone => Some(None),
                _ => None,
            })
            .unwrap_or_else(|| self.context.pointer_latest_pos());
        position.is_some_and(|point| {
            self.placed
                .iter()
                .any(|placed| placed.rect.intersect(placed.clip).contains(point))
        })
    }

    fn input_layout_changed(&self, input: &egui::RawInput) -> bool {
        input
            .screen_rect
            .is_some_and(|rect| Some(rect) != self.viewport)
            || input
                .viewports
                .get(&egui::ViewportId::ROOT)
                .and_then(|info| info.native_pixels_per_point)
                .is_some_and(|scale| Some(scale) != self.pixels_per_point)
    }

    fn layout(&mut self, viewport: egui::Rect) {
        self.placed.clear();
        for (index, node) in self.document.nodes.iter().enumerate() {
            if node.screen != self.screen {
                continue;
            }
            let (bounds, clip) = if let Some(parent) = &node.parent {
                let Some(placed) = self
                    .placed
                    .iter()
                    .find(|placed| self.document.nodes[placed.index].id == *parent)
                else {
                    continue;
                };
                (placed.rect, placed.clip.intersect(placed.rect))
            } else {
                (viewport, viewport)
            };
            let size = egui::vec2(f32::from(node.size[0]), f32::from(node.size[1]));
            let min = bounds.min
                + egui::vec2(
                    bounds.width() * f32::from(node.anchor[0]) / 1000.0 + f32::from(node.offset[0]),
                    bounds.height() * f32::from(node.anchor[1]) / 1000.0
                        + f32::from(node.offset[1]),
                );
            let rect = egui::Rect::from_min_size(min, size);
            self.placed.push(Placed { index, rect, clip });
        }
    }

    fn hit_button(&self, point: egui::Pos2) -> Option<usize> {
        // Filled containers occlude earlier content, even though they do not
        // dispatch actions. Labels are intentionally passive pointer pass-through.
        // A later child button is visited before its containing panel.
        for placed in self.placed.iter().rev() {
            let visible = placed.rect.intersect(placed.clip);
            if !visible.is_positive() || !visible.contains(point) {
                continue;
            }
            match self.document.nodes[placed.index].kind {
                Kind::Button { .. } => return Some(placed.index),
                Kind::Container => return None,
                Kind::Label { .. } => {}
            }
        }
        None
    }

    pub fn show(&mut self, input: egui::RawInput, hud: Hud) -> (egui::FullOutput, Option<Action>) {
        assert_eq!(
            self.profile,
            Profile::Collect,
            "Collect data cannot drive Room UI"
        );
        self.show_values(input, Values::Collect(hud))
    }

    pub fn show_room(
        &mut self,
        input: egui::RawInput,
        hud: RoomHud,
    ) -> (egui::FullOutput, Option<Action>) {
        assert_eq!(
            self.profile,
            Profile::Room,
            "Room data cannot drive Collect UI"
        );
        self.show_values(input, Values::Room(hud))
    }

    fn show_values(
        &mut self,
        input: egui::RawInput,
        values: Values,
    ) -> (egui::FullOutput, Option<Action>) {
        let phase = values.phase();
        self.phase = phase;
        if phase == 0 {
            self.awaiting_restart = false;
        }
        let terminal_change =
            self.screen == Screen::Playing && phase != 0 && !self.awaiting_restart;
        if terminal_change {
            self.screen = Screen::Terminal;
            self.invalidate_pointer_layout();
        }
        let room_layout_transition = self.profile == Profile::Room
            && (self.pointer_layout_stale || self.input_layout_changed(&input));
        if self.input_layout_changed(&input) {
            self.invalidate_pointer_layout();
        }
        let viewport = input.screen_rect.or(self.viewport).unwrap_or_else(|| {
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 900.0))
        });
        self.viewport = Some(viewport);
        if let Some(scale) = input
            .viewports
            .get(&egui::ViewportId::ROOT)
            .and_then(|info| info.native_pixels_per_point)
        {
            self.pixels_per_point = Some(scale);
        }
        self.layout(viewport);
        let mut action = None;
        for event in &input.events {
            match event {
                egui::Event::PointerGone | egui::Event::WindowFocused(false) => self.pressed = None,
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } if !terminal_change && !room_layout_transition => {
                    let hit = self.hit_button(*pos);
                    if *pressed {
                        self.pressed = hit.map(|index| self.document.nodes[index].id.clone());
                    } else if let Some(owner) = self.pressed.take() {
                        if let Some(index) = hit {
                            let node = &self.document.nodes[index];
                            if node.id == owner && action.is_none() {
                                if let Kind::Button {
                                    action: selected, ..
                                } = node.kind
                                {
                                    action = Some(selected);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let keyboard_activation = input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Enter | egui::Key::Space,
                    pressed: true,
                    ..
                }
            )
        });
        let context = self.context.clone();
        let output = context.run_ui(input, |root| {
            for placed in &self.placed {
                let node = &self.document.nodes[placed.index];
                let clip = placed.clip.intersect(placed.rect);
                if !clip.is_positive() {
                    continue;
                }
                let mut ui = root.new_child(
                    egui::UiBuilder::new()
                        .id_salt(("collect", &node.id))
                        .max_rect(placed.rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                ui.set_clip_rect(clip);
                match &node.kind {
                    Kind::Container => {
                        ui.painter().rect_filled(
                            placed.rect,
                            6.0,
                            egui::Color32::from_rgba_unmultiplied(16, 22, 32, 220),
                        );
                    }
                    Kind::Label { text, binding } => {
                        let value = binding.as_ref().map(|binding| values.value(*binding));
                        let text =
                            value.map_or_else(|| text.clone(), |value| format!("{text} {value}"));
                        // v1 labels are top-left aligned and wrap at their authored
                        // logical width. egui measures the actual installed font,
                        // including Korean glyphs; overflow is clipped to the node.
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(text)
                                    .size(18.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .wrap()
                            .halign(egui::Align::Min)
                            .selectable(false),
                        );
                    }
                    Kind::Button {
                        text,
                        action: selected,
                    } => {
                        // The native widget paints and exposes stable accessibility identity.
                        // Dispatch above owns pointer hit testing explicitly so overlapped siblings
                        // and clipped-away areas can never activate multiple actions.
                        let response = ui.put(placed.rect, egui::Button::new(text));
                        if keyboard_activation
                            && !terminal_change
                            && !room_layout_transition
                            && response.clicked()
                            && response.has_focus()
                            // Conservative v1 keyboard policy: the visible rectangle's
                            // center must belong to this button. A partially covered
                            // center disables keyboard activation; exposed pointer
                            // regions still work through exact point hit testing.
                            && self.hit_button(clip.center()) == Some(placed.index)
                            && action.is_none()
                        {
                            action = Some(*selected);
                        }
                    }
                }
            }
        });
        self.pointer_layout_stale = false;
        (output, action)
    }
}

fn phase_text(phase: u32) -> &'static str {
    match phase {
        0 => "PLAYING",
        1 => "WON",
        2 => "LOST: hazard",
        3 => "LOST: time",
        _ => "INVALID",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authored_ui::Node;

    fn font() -> Vec<u8> {
        include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec()
    }
    fn ui() -> CollectUi {
        CollectUi::new(Document::default_collect(), font()).unwrap()
    }
    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 900.0),
            )),
            events,
            ..Default::default()
        }
    }
    fn button(point: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }
    fn frame(ui: &mut CollectUi, input: egui::RawInput, phase: u32) -> Option<Action> {
        let (output, action) = ui.show(
            input,
            Hud {
                phase,
                ..Default::default()
            },
        );
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        action
    }
    fn click_at(ui: &mut CollectUi, point: egui::Pos2) -> Option<Action> {
        frame(
            ui,
            input(vec![egui::Event::PointerMoved(point), button(point, true)]),
            0,
        );
        frame(ui, input(vec![button(point, false)]), 0)
    }
    fn action_point(ui: &mut CollectUi, action: Action) -> egui::Pos2 {
        frame(ui, input(vec![]), 0);
        let placed = ui.placed.iter().find(|placed| matches!(ui.document.nodes[placed.index].kind, Kind::Button { action: a, .. } if a == action)).unwrap();
        placed.rect.intersect(placed.clip).center()
    }
    fn node(id: &str, action: Action, offset: [i16; 2]) -> Node {
        Node {
            id: id.into(),
            parent: None,
            kind: Kind::Button {
                text: id.into(),
                action,
            },
            screen: Screen::Title,
            anchor: [0, 0],
            offset,
            size: [100, 40],
        }
    }

    #[test]
    fn real_pointer_events_dispatch_all_actions_and_screen_capture() {
        let mut ui = ui();
        assert!(ui.blocks_controls());
        for action in [
            Action::Play,
            Action::Menu,
            Action::Continue,
            Action::Menu,
            Action::Restart,
            Action::Menu,
            Action::Quit,
        ] {
            let point = action_point(&mut ui, action);
            assert_eq!(click_at(&mut ui, point), Some(action));
            ui.apply(action);
            assert!(
                ui.pending_pointer_over_ui(&input(vec![])),
                "screen transitions invalidate bounds"
            );
            assert_eq!(ui.blocks_controls(), ui.screen() != Screen::Playing);
        }
    }

    #[test]
    fn overlap_only_last_button_receives_release_and_drag_cannot_retarget() {
        let doc = Document {
            schema: 1,
            nodes: vec![
                node("lower", Action::Play, [20, 20]),
                node("upper", Action::Quit, [40, 20]),
            ],
        };
        let mut ui = CollectUi::new(doc, font()).unwrap();
        frame(&mut ui, input(vec![]), 0);
        assert_eq!(
            click_at(&mut ui, egui::pos2(60.0, 30.0)),
            Some(Action::Quit)
        );
        frame(
            &mut ui,
            input(vec![button(egui::pos2(30.0, 30.0), true)]),
            0,
        );
        assert_eq!(
            frame(
                &mut ui,
                input(vec![button(egui::pos2(60.0, 30.0), false)]),
                0
            ),
            None
        );
        assert_eq!(
            click_at(&mut ui, egui::pos2(30.0, 30.0)),
            Some(Action::Play)
        );
    }

    #[test]
    fn later_filled_container_blocks_covered_button_but_its_child_wins() {
        let lower = node("covered", Action::Play, [20, 20]);
        let mut panel = node("cover", Action::Quit, [40, 20]);
        panel.kind = Kind::Container;
        panel.size = [80, 80];
        let mut child = node("child", Action::Restart, [10, 30]);
        child.parent = Some("cover".into());
        child.size = [50, 30];
        let mut label = node("passive", Action::Quit, [20, 20]);
        label.kind = Kind::Label {
            text: "Overlay label".into(),
            binding: None,
        };
        let mut ui = CollectUi::new(
            Document {
                schema: 1,
                nodes: vec![lower, panel, child, label],
            },
            font(),
        )
        .unwrap();
        frame(&mut ui, input(vec![]), 0);
        assert_eq!(
            click_at(&mut ui, egui::pos2(60.0, 30.0)),
            None,
            "panel occludes lower button"
        );
        assert_eq!(
            click_at(&mut ui, egui::pos2(60.0, 60.0)),
            Some(Action::Restart),
            "child appears above its parent"
        );
        assert_eq!(
            click_at(&mut ui, egui::pos2(30.0, 30.0)),
            Some(Action::Play),
            "passive label does not occlude uncovered button"
        );
    }

    #[test]
    fn clipped_container_only_occludes_its_visible_intersection() {
        let lower = node("under", Action::Play, [20, 20]);
        let mut parent = node("parent", Action::Quit, [20, 20]);
        parent.kind = Kind::Container;
        parent.size = [40, 40];
        let mut clipped = node("clipped-cover", Action::Quit, [20, 0]);
        clipped.kind = Kind::Container;
        clipped.parent = Some("parent".into());
        let mut ui = CollectUi::new(
            Document {
                schema: 1,
                nodes: vec![lower, parent, clipped],
            },
            font(),
        )
        .unwrap();
        frame(&mut ui, input(vec![]), 0);
        assert_eq!(click_at(&mut ui, egui::pos2(50.0, 30.0)), None);
        assert_eq!(
            click_at(&mut ui, egui::pos2(80.0, 30.0)),
            Some(Action::Play),
            "clipped-away cover cannot hide visible earlier button"
        );
    }

    #[test]
    fn keyboard_activation_requires_unoccluded_button_center() {
        let key = |key, pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        for (cover_offset, expected) in [
            ([20, 20], None),
            ([200, 20], Some(Action::Play)),
            ([60, 20], None),
        ] {
            let lower = node("keyboard-button", Action::Play, [20, 20]);
            let mut cover = node("keyboard-cover", Action::Quit, cover_offset);
            cover.kind = Kind::Container;
            let mut ui = CollectUi::new(
                Document {
                    schema: 1,
                    nodes: vec![lower, cover],
                },
                font(),
            )
            .unwrap();
            for _ in 0..2 {
                frame(&mut ui, input(vec![]), 0);
            }
            frame(&mut ui, input(vec![key(egui::Key::Tab, true)]), 0);
            assert!(
                ui.context.memory(|memory| memory.focused().is_some()),
                "actual Tab gives native button focus"
            );
            assert_eq!(
                frame(
                    &mut ui,
                    input(vec![
                        key(egui::Key::Tab, false),
                        key(egui::Key::Enter, true)
                    ]),
                    0
                ),
                expected
            );
            if cover_offset == [60, 20] {
                assert_eq!(
                    click_at(&mut ui, egui::pos2(30.0, 30.0)),
                    Some(Action::Play),
                    "partial overlap still permits exposed pointer region"
                );
            }
        }
    }

    #[test]
    fn parent_relative_layout_and_clipping_apply_to_paint_and_hit_testing() {
        let mut container = node("panel", Action::Play, [10, 10]);
        container.kind = Kind::Container;
        container.size = [100, 100];
        let mut child = node("child", Action::Quit, [0, 0]);
        child.parent = Some("panel".into());
        child.anchor = [500, 500];
        let mut ui = CollectUi::new(
            Document {
                schema: 1,
                nodes: vec![container, child],
            },
            font(),
        )
        .unwrap();
        frame(&mut ui, input(vec![]), 0);
        assert_eq!(ui.placed[1].rect.min, egui::pos2(60.0, 60.0));
        assert_eq!(ui.placed[1].clip.max, egui::pos2(110.0, 110.0));
        assert_eq!(click_at(&mut ui, egui::pos2(120.0, 70.0)), None);
        assert!(
            !ui.pending_pointer_over_ui(&input(vec![egui::Event::PointerMoved(egui::pos2(
                120.0, 70.0
            ))]))
        );
        assert_eq!(
            click_at(&mut ui, egui::pos2(70.0, 70.0)),
            Some(Action::Quit)
        );
    }

    #[test]
    fn viewport_clip_excludes_offscreen_button_area() {
        let mut ui = CollectUi::new(
            Document {
                schema: 1,
                nodes: vec![node("edge", Action::Quit, [870, 20])],
            },
            font(),
        )
        .unwrap();
        frame(&mut ui, input(vec![]), 0);
        assert_eq!(click_at(&mut ui, egui::pos2(920.0, 30.0)), None);
        assert_eq!(
            click_at(&mut ui, egui::pos2(880.0, 30.0)),
            Some(Action::Quit)
        );
    }

    #[test]
    fn resize_and_dpi_cancel_press_and_use_fresh_anchored_bounds() {
        let mut n = node("anchored", Action::Play, [-100, -40]);
        n.anchor = [1000, 1000];
        let mut ui = CollectUi::new(
            Document {
                schema: 1,
                nodes: vec![n],
            },
            font(),
        )
        .unwrap();
        frame(
            &mut ui,
            input(vec![button(egui::pos2(850.0, 880.0), true)]),
            0,
        );
        let mut resized = input(vec![]);
        resized.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(450.0, 450.0),
        ));
        resized
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(2.0);
        assert!(ui.pending_pointer_over_ui(&resized));
        resized.events = vec![button(egui::pos2(400.0, 430.0), false)];
        assert_eq!(frame(&mut ui, resized.clone(), 0), None);
        assert_eq!(ui.placed[0].rect.min, egui::pos2(350.0, 410.0));
        resized.events = vec![button(egui::pos2(400.0, 430.0), true)];
        frame(&mut ui, resized.clone(), 0);
        resized.events = vec![button(egui::pos2(400.0, 430.0), false)];
        assert_eq!(frame(&mut ui, resized, 0), Some(Action::Play));
    }

    #[test]
    fn terminal_transition_discards_old_click_and_menu_remains_open() {
        let mut ui = ui();
        ui.apply(Action::Play);
        let point = action_point(&mut ui, Action::Menu);
        frame(&mut ui, input(vec![button(point, true)]), 0);
        assert_eq!(frame(&mut ui, input(vec![button(point, false)]), 1), None);
        assert_eq!(ui.screen(), Screen::Terminal);
        assert!(ui.blocks_controls());
        ui.apply(Action::Menu);
        frame(&mut ui, input(vec![]), 2);
        assert_eq!(ui.screen(), Screen::Menu);
        ui.apply(Action::Continue);
        assert_eq!(ui.screen(), Screen::Terminal);
        ui.apply(Action::Restart);
        frame(&mut ui, input(vec![]), 0);
        assert_eq!(ui.screen(), Screen::Playing);
    }

    #[test]
    fn restart_waits_through_zero_tick_terminal_snapshots_until_acknowledged() {
        let mut ui = ui();
        ui.apply(Action::Play);
        frame(&mut ui, input(vec![]), 1);
        assert_eq!(ui.screen(), Screen::Terminal);
        ui.apply(Action::Restart);
        for _ in 0..8 {
            frame(&mut ui, input(vec![]), 1);
            assert_eq!(ui.screen(), Screen::Playing);
            assert!(!ui.blocks_controls());
        }
        ui.apply(Action::Menu);
        frame(&mut ui, input(vec![]), 1);
        ui.apply(Action::Continue);
        assert_eq!(ui.screen(), Screen::Playing);
        frame(&mut ui, input(vec![]), 0);
        frame(&mut ui, input(vec![]), 2);
        assert_eq!(ui.screen(), Screen::Terminal);

        ui.apply(Action::Restart);
        frame(&mut ui, input(vec![]), 2);
        assert_eq!(ui.screen(), Screen::Playing);
        ui.acknowledge_restart();
        frame(&mut ui, input(vec![]), 3);
        assert_eq!(
            ui.screen(),
            Screen::Terminal,
            "catch-up terminal after acknowledged restart"
        );
    }

    #[test]
    fn korean_label_uses_measured_wrapped_galley_and_authored_clip() {
        let text = "한글 글꼴로 측정하는 긴 문장은 작성한 너비에서 자동으로 줄을 바꿉니다";
        let mut label = node("korean-label", Action::Play, [20, 30]);
        label.kind = Kind::Label {
            text: text.into(),
            binding: None,
        };
        label.size = [90, 40];
        let document = Document {
            schema: 1,
            nodes: vec![label],
        };
        let mut ui = CollectUi::new(document.clone(), font()).unwrap();
        assert_eq!(ui.document(), &document);
        let (output, action) = ui.show(input(vec![]), Hud::default());
        assert_eq!(action, None);
        let rendered = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(rendered) if rendered.galley.job.text == text => {
                    Some((shape.clip_rect, rendered))
                }
                _ => None,
            })
            .expect("actual egui Label produces the Korean text galley");
        assert!(
            rendered.1.galley.rows.len() > 1,
            "text must wrap, not draw one clipped line"
        );
        assert!(rendered.1.galley.rect.width() <= 90.5);
        assert_eq!(rendered.1.pos, egui::pos2(20.0, 30.0));
        assert_eq!(
            rendered.0,
            egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(90.0, 40.0))
        );
        assert!(
            rendered.1.galley.rect.height() > 40.0,
            "clip constrains overflow without changing measurement"
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn font_validation_uses_actual_document_not_only_default_corpus() {
        assert!(CollectUi::new(Document::default_collect(), vec![0; 20]).is_err());
        let mut document = Document::default_collect();
        document.nodes.push(Node {
            kind: Kind::Label {
                text: "🦄".into(),
                binding: None,
            },
            ..node("missing-glyph", Action::Play, [0, 0])
        });
        let error = CollectUi::new(document, font()).err().unwrap();
        assert!(error.contains("lacks required glyph"));
        let mut document = Document::default_collect();
        document.schema = 99;
        assert!(CollectUi::new(document, font()).is_err());
    }
}

#[cfg(all(test, feature = "room-ui"))]
mod room_tests {
    use super::*;
    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        }
    }
    fn frame(ui: &mut CollectUi, events: Vec<egui::Event>, hud: RoomHud) -> Option<Action> {
        let (output, action) = ui.show_room(input(events), hud);
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
        action
    }
    fn click(ui: &mut CollectUi, action: Action, hud: RoomHud) -> Option<Action> {
        frame(ui, vec![], hud);
        let placed = ui.placed.iter().find(|p| matches!(ui.document.nodes[p.index].kind, Kind::Button { action: a, .. } if a == action)).unwrap();
        let point = placed.rect.intersect(placed.clip).center();
        let event = |pressed| egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        assert_eq!(
            frame(ui, vec![egui::Event::PointerMoved(point), event(true)], hud),
            None
        );
        frame(ui, vec![event(false)], hud)
    }
    #[test]
    fn room_widgets_title_menu_win_restart_and_profile_values() {
        let font = include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec();
        let mut ui = CollectUi::new_for(Document::default_room(), font, Profile::Room).unwrap();
        let playing = RoomHud::default();
        assert_eq!(ui.screen(), Screen::Title);
        for action in [Action::Play, Action::Menu, Action::Continue] {
            assert_eq!(click(&mut ui, action, playing), Some(action));
            ui.apply(action);
            ui.acknowledge_restart();
        }
        let won = RoomHud {
            key_acquired: true,
            won: true,
        };
        frame(&mut ui, vec![], won);
        assert_eq!(ui.screen(), Screen::Terminal);
        assert_eq!(click(&mut ui, Action::Restart, won), Some(Action::Restart));
        ui.apply(Action::Restart);
        frame(&mut ui, vec![], won);
        assert_eq!(ui.screen(), Screen::Playing); // old terminal frame cannot undo pending restart
        ui.acknowledge_restart();
        frame(&mut ui, vec![], playing);
        assert_eq!(ui.screen(), Screen::Playing);
        assert_eq!(Values::Room(won).value(Binding::KeyAcquired), "ACQUIRED");
        assert_eq!(Values::Room(won).value(Binding::ExitState), "UNLOCKED");
        assert_eq!(Values::Room(won).value(Binding::RoomPhase), "WON");
    }
    #[test]
    fn room_focus_dpi_and_release_without_press_do_not_activate() {
        let font = include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec();
        let mut ui = CollectUi::new_for(Document::default_room(), font, Profile::Room).unwrap();
        let hud = RoomHud::default();
        frame(&mut ui, vec![], hud);
        let point = ui
            .placed
            .iter()
            .find(|p| {
                matches!(
                    ui.document.nodes[p.index].kind,
                    Kind::Button {
                        action: Action::Play,
                        ..
                    }
                )
            })
            .unwrap()
            .rect
            .center();
        let event = |pressed| egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut ui, vec![event(true)], hud);
        ui.invalidate_pointer_layout();
        assert_eq!(frame(&mut ui, vec![event(false)], hud), None);
        assert_eq!(frame(&mut ui, vec![event(false)], hud), None);
        let mut changed = input(vec![event(true), event(false)]);
        changed
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(2.0);
        let (output, action) = ui.show_room(changed, hud);
        output.drop_without_applying_deltas();
        assert_eq!(action, None);
        assert_eq!(ui.screen(), Screen::Title);
    }
}
