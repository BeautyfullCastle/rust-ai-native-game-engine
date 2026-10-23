//! Read-only authored Collect UI preview. This owns a separate production UI
//! context and uses admitted bytes only; it has no host, history or file writer.
use orr_sample::{
    authored_ui::{Action, Document, Kind, Node, Screen, SCHEMA},
    collect_project::PreparedCollectUi,
    collect_ui::{CollectUi, Hud},
};
use std::path::{Path, PathBuf};

pub const MIN_VIEWPORT: u32 = 240;
pub const MAX_VIEWPORT: u32 = 2048;
pub const MAX_SCORE: u32 = 999_999;
pub const MAX_BEST_BYTES: usize = 64;

/// Synthetic presentation values. These never read or replace live game state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub screen: Screen,
    pub viewport: [u32; 2],
    pub score: u32,
    pub phase: u32,
    pub best: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            screen: Screen::Title,
            viewport: [640, 360],
            score: 3,
            phase: 1,
            best: "unavailable".into(),
        }
    }
}

impl Settings {
    fn validate(&self, font: &[u8]) -> Result<(), String> {
        if self
            .viewport
            .iter()
            .any(|axis| !(MIN_VIEWPORT..=MAX_VIEWPORT).contains(axis))
        {
            return Err(format!(
                "Preview viewport must be {MIN_VIEWPORT}..={MAX_VIEWPORT} logical pixels per axis"
            ));
        }
        if self.score > MAX_SCORE || self.phase > 3 {
            return Err(format!(
                "Preview score must be 0..={MAX_SCORE} and phase must be 0..=3"
            ));
        }
        if self.best.len() > MAX_BEST_BYTES || self.best.chars().any(char::is_control) {
            return Err(format!("Preview best text must be at most {MAX_BEST_BYTES} UTF-8 bytes without control characters"));
        }
        // The production validator includes dynamic phase/score characters.
        // Validate custom sample text too, without expanding or editing the
        // authored document (which may already contain the maximum node count).
        let sample = Document {
            schema: SCHEMA,
            nodes: vec![Node {
                id: "preview_sample".into(),
                parent: None,
                kind: Kind::Label {
                    text: self.best.clone(),
                    binding: None,
                },
                screen: Screen::Title,
                anchor: [0, 0],
                offset: [0, 0],
                size: [1, 1],
            }],
        };
        CollectUi::validate_font(font, &sample)
    }

    fn hud(&self) -> Hud {
        Hud {
            score: self.score,
            phase: match self.screen {
                Screen::Playing => 0,
                Screen::Terminal => self.phase.max(1),
                Screen::Title | Screen::Menu => self.phase,
            },
            best: self.best.clone(),
        }
    }
}

struct Admission {
    prepared: PreparedCollectUi,
    scene: PathBuf,
    epoch: u64,
}

/// The GPU owner must consume the output's texture delta with this context.
/// A changed generation means a new context and a fresh texture namespace.
pub struct Frame {
    pub context: egui::Context,
    pub output: egui::FullOutput,
    pub action: Option<Action>,
    pub generation: u64,
}

#[derive(Default)]
pub struct State {
    admission: Option<Admission>,
    active: Option<CollectUi>,
    settings: Settings,
    last_action: Option<Action>,
    generation: u64,
    error: Option<String>,
    // A position candidate is presentation-only; admission remains the accepted
    // panel document until its existing atomic Apply path commits it.
    staged: Option<Document>,
}

impl State {
    /// Capture admitted font/document bytes before the authoring panel takes
    /// ownership. No pathname is opened here or during subsequent previews.
    pub fn admit(
        &mut self,
        prepared: &PreparedCollectUi,
        scene: &Path,
        epoch: u64,
    ) -> Result<(), String> {
        let same_identity = self.admission.as_ref().is_some_and(|admission| {
            admission.scene == scene
                && admission.epoch == epoch
                && admission.prepared.path == prepared.path
        });
        let reopen = same_identity && self.is_open();
        if !same_identity {
            self.clear();
        }
        if let Err(error) =
            validate_document(prepared).and_then(|()| self.settings.validate(&prepared.font))
        {
            self.clear();
            self.error = Some(error.clone());
            return Err(error);
        }
        let unchanged = self.admission.as_ref().is_some_and(|admission| {
            admission.prepared.document == prepared.document
                && admission.prepared.font == prepared.font
        });
        if unchanged {
            return Ok(());
        }
        self.active = None;
        self.staged = None;
        self.last_action = None;
        self.error = None;
        self.admission = Some(Admission {
            prepared: prepared.clone(),
            scene: scene.to_owned(),
            epoch,
        });
        if reopen {
            self.open()?;
        }
        Ok(())
    }

    /// Call before exposing the preview each editor frame. The accepted panel
    /// document is authoritative, including unsaved edits and panel Undo.
    pub fn synchronize(
        &mut self,
        scene: Option<&Path>,
        epoch: u64,
        document: Option<&Document>,
    ) -> Result<(), String> {
        let matches = self.admission.as_ref().is_some_and(|admission| {
            scene == Some(admission.scene.as_path()) && epoch == admission.epoch
        });
        if !matches || document.is_none() {
            self.clear();
            return Ok(());
        }
        let document = document.expect("checked panel presence");
        let admission = self.admission.as_mut().expect("matched admission");
        if admission.prepared.document == *document {
            return self.error.clone().map_or(Ok(()), Err);
        }
        let reopen = self.active.take().is_some();
        self.staged = None;
        self.last_action = None;
        admission.prepared.document = document.clone();
        if let Err(error) = validate_document(&admission.prepared) {
            // Never retain a rendered stale document after current data fails.
            // The font stays admitted, but open must validate this candidate.
            self.error = Some(error.clone());
            return Err(error);
        }
        self.error = None;
        if reopen {
            self.open()?;
        }
        Ok(())
    }

    pub fn open(&mut self) -> Result<(), String> {
        if self.active.is_some() {
            return Ok(());
        }
        let result = self.build_active();
        match result {
            Ok(active) => {
                self.active = Some(active);
                self.generation = self.generation.wrapping_add(1);
                self.last_action = None;
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.error = Some(error.clone());
                Err(error)
            }
        }
    }

    fn build_active(&self) -> Result<CollectUi, String> {
        let admission = self
            .admission
            .as_ref()
            .ok_or("Open a Collect project with an admitted UI document first")?;
        validate_document(&admission.prepared)?;
        self.settings.validate(&admission.prepared.font)?;
        self.build_document(&admission.prepared.document)
    }

    fn build_document(&self, document: &Document) -> Result<CollectUi, String> {
        let admission = self
            .admission
            .as_ref()
            .ok_or("No admitted Collect UI is available")?;
        document.to_bytes()?;
        let mut active = CollectUi::new(document.clone(), admission.prepared.font.clone())?;
        // Use only public production transitions. Terminal is entered by the
        // synthetic nonzero phase on its first show, before processing input.
        match self.settings.screen {
            Screen::Title => {}
            Screen::Playing | Screen::Terminal => active.apply(Action::Play),
            Screen::Menu => active.apply(Action::Menu),
        }
        Ok(active)
    }

    /// Validate and render one released candidate without changing admission.
    /// Call only at release, never for each pointer-move event.
    pub fn stage_document(&mut self, document: &Document) -> Result<(), String> {
        if !self.is_open() {
            return Err("Collect UI preview is closed".into());
        }
        let active = self.build_document(document)?;
        self.active = Some(active);
        self.staged = Some(document.clone());
        self.last_action = None;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    /// Restore the accepted renderer after Cancel, stale input, or interruption.
    pub fn cancel_staged(&mut self) -> Result<(), String> {
        self.cancel_pointer();
        if self.staged.take().is_some() && self.is_open() {
            self.active = None;
            self.open()?;
        }
        Ok(())
    }

    pub fn rendered_document(&self) -> Option<&Document> {
        self.active.as_ref().map(CollectUi::document)
    }

    /// Closing releases all active UI state, retaining only admission/settings.
    pub fn close(&mut self) {
        self.active = None;
        self.staged = None;
        self.last_action = None;
    }

    /// Project/scene lifecycle invalidation also releases retained font bytes.
    pub fn clear(&mut self) {
        *self = Self {
            generation: self.generation,
            ..Self::default()
        };
    }

    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }
    pub fn is_available(&self) -> bool {
        self.admission.is_some() && self.error.is_none()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    pub fn last_action(&self) -> Option<Action> {
        self.last_action
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn admitted_path(&self) -> Option<&Path> {
        self.admission
            .as_ref()
            .map(|admission| admission.prepared.path.as_path())
    }
    pub fn document(&self) -> Option<&Document> {
        self.admission
            .as_ref()
            .map(|admission| &admission.prepared.document)
    }

    pub fn cancel_pointer(&mut self) {
        if let Some(active) = &mut self.active {
            active.invalidate_pointer_layout();
        }
    }

    /// Invalid settings leave the prior valid preview untouched.
    pub fn set_settings(&mut self, settings: Settings) -> Result<(), String> {
        let admission = self
            .admission
            .as_ref()
            .ok_or("No admitted Collect UI is available")?;
        settings.validate(&admission.prepared.font)?;
        if settings == self.settings {
            return Ok(());
        }
        self.cancel_staged()?;
        let rebuild = settings.screen != self.settings.screen && self.is_open();
        self.settings = settings;
        self.last_action = None;
        self.cancel_pointer();
        if rebuild {
            self.active = None;
            self.open()?;
        }
        Ok(())
    }

    /// Layout, clipping, focus and hit testing come from the standalone
    /// production renderer. Returned actions are observations, never dispatches.
    pub fn render(&mut self, mut input: egui::RawInput) -> Result<Frame, String> {
        let hud = self.settings.hud();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(
                self.settings.viewport[0] as f32,
                self.settings.viewport[1] as f32,
            ),
        ));
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(1.0);
        let active = self.active.as_mut().ok_or("Collect UI preview is closed")?;
        let (output, action) = active.show(input, hud);
        if action.is_some() {
            self.last_action = action;
        }
        Ok(Frame {
            context: active.context.clone(),
            output,
            action,
            generation: self.generation,
        })
    }
}

fn validate_document(prepared: &PreparedCollectUi) -> Result<(), String> {
    CollectUi::validate_font(&prepared.font, &prepared.document)?;
    prepared.document.to_bytes()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PreparedCollectUi {
        PreparedCollectUi {
            path: PathBuf::from("/virtual/collect/ui.json"),
            document: Document::default_collect(),
            font: include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec(),
        }
    }
    fn scene() -> &'static Path {
        Path::new("/virtual/collect/scene.yaml")
    }
    fn opened() -> State {
        let mut state = State::default();
        state.admit(&fixture(), scene(), 9).unwrap();
        state.open().unwrap();
        state
    }
    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
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
    fn render(state: &mut State, events: Vec<egui::Event>) -> Option<Action> {
        let frame = state.render(input(events)).unwrap();
        let action = frame.action;
        frame.output.drop_without_applying_deltas();
        action
    }
    fn click(state: &mut State, point: egui::Pos2) -> Option<Action> {
        render(
            state,
            vec![egui::Event::PointerMoved(point), button(point, true)],
        );
        render(state, vec![button(point, false)])
    }
    fn text(frame: &Frame) -> Vec<String> {
        frame
            .output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect()
    }
    fn select(state: &mut State, screen: Screen) {
        state
            .set_settings(Settings {
                screen,
                ..state.settings().clone()
            })
            .unwrap();
    }

    #[test]
    fn admission_uses_cloned_bytes_without_reading_paths() {
        let mut prepared = fixture();
        let expected = prepared.document.clone();
        let mut state = State::default();
        state.admit(&prepared, scene(), 9).unwrap();
        prepared.font.clear();
        prepared.document.nodes.clear();
        state.open().unwrap();
        assert_eq!(state.document(), Some(&expected));
        assert_eq!(
            state.admitted_path(),
            Some(Path::new("/virtual/collect/ui.json"))
        );
        let frame = state.render(input(vec![])).unwrap();
        assert!(text(&frame).iter().any(|text| text.contains("플레이")));
        frame.output.drop_without_applying_deltas();
    }

    #[test]
    fn close_releases_active_context_and_reopens_retained_admission() {
        let mut state = opened();
        let generation = state.generation();
        state.close();
        assert!(!state.is_open());
        assert!(state.is_available());
        assert!(state.render(input(vec![])).is_err());
        state.open().unwrap();
        assert!(state.generation() > generation);
        assert_eq!(render(&mut state, vec![]), None);
    }

    #[test]
    fn same_identity_and_document_keep_active_context() {
        let mut state = opened();
        let generation = state.generation();
        state
            .synchronize(Some(scene()), 9, Some(&fixture().document))
            .unwrap();
        state.admit(&fixture(), scene(), 9).unwrap();
        assert_eq!(state.generation(), generation);
        assert!(state.is_open());
    }

    #[test]
    fn scene_epoch_and_missing_panel_clear_all_admission() {
        for (path, epoch, present) in [
            (Some(Path::new("/other/scene.yaml")), 9, true),
            (Some(scene()), 10, true),
            (None, 9, true),
            (Some(scene()), 9, false),
        ] {
            let mut state = opened();
            state
                .synchronize(path, epoch, present.then_some(&fixture().document))
                .unwrap();
            assert!(!state.is_open());
            assert!(!state.is_available());
            assert!(state.document().is_none());
            assert!(state.admitted_path().is_none());
            assert!(state.open().is_err());
        }
    }

    #[test]
    fn readmission_at_new_path_or_epoch_resets_settings_and_closes() {
        for change_epoch in [false, true] {
            let mut state = opened();
            select(&mut state, Screen::Menu);
            let mut prepared = fixture();
            if !change_epoch {
                prepared.path = PathBuf::from("/other/ui.json");
            }
            state
                .admit(&prepared, scene(), if change_epoch { 10 } else { 9 })
                .unwrap();
            assert!(!state.is_open());
            assert!(state.is_available());
            assert_eq!(state.settings(), &Settings::default());
        }
    }

    #[test]
    fn accepted_unsaved_document_and_undo_rebuild_context() {
        let mut state = opened();
        let original = fixture().document;
        let mut edited = original.clone();
        edited.nodes[0].kind = Kind::Label {
            text: "Edited 한글".into(),
            binding: None,
        };
        let first = state.generation();
        state.synchronize(Some(scene()), 9, Some(&edited)).unwrap();
        assert!(state.generation() > first);
        assert_eq!(state.document(), Some(&edited));
        let frame = state.render(input(vec![])).unwrap();
        assert!(text(&frame).iter().any(|text| text == "Edited 한글"));
        frame.output.drop_without_applying_deltas();
        let second = state.generation();
        state
            .synchronize(Some(scene()), 9, Some(&original))
            .unwrap();
        assert!(state.generation() > second);
        assert_eq!(state.document(), Some(&original));
    }

    #[test]
    fn released_position_preview_keeps_admission_until_apply_and_cancel_restores() {
        let mut state = opened();
        let original = state.document().unwrap().clone();
        let mut candidate = original.clone();
        candidate.nodes[1].offset[0] += 40;
        let generation = state.generation();
        state.stage_document(&candidate).unwrap();
        assert_eq!(state.generation(), generation + 1);
        assert_eq!(state.document(), Some(&original));
        assert_eq!(state.rendered_document(), Some(&candidate));
        state
            .synchronize(Some(scene()), 9, Some(&original))
            .unwrap();
        assert_eq!(state.generation(), generation + 1);
        assert_eq!(state.rendered_document(), Some(&candidate));
        state.cancel_staged().unwrap();
        assert_eq!(state.rendered_document(), Some(&original));
        let restored = state.generation();
        state.cancel_staged().unwrap();
        assert_eq!(state.generation(), restored);
    }

    #[test]
    fn invalid_position_preview_is_atomic_and_accepted_changes_replace_candidate() {
        let mut state = opened();
        let original = state.document().unwrap().clone();
        let mut invalid = original.clone();
        invalid.nodes[0].offset = [5000, 0];
        let generation = state.generation();
        assert!(state.stage_document(&invalid).is_err());
        assert_eq!(state.generation(), generation);
        assert_eq!(state.rendered_document(), Some(&original));
        let mut candidate = original.clone();
        candidate.nodes[1].offset[0] += 40;
        state.stage_document(&candidate).unwrap();
        let mut accepted = original;
        accepted.nodes[0].offset[1] += 10;
        state
            .synchronize(Some(scene()), 9, Some(&accepted))
            .unwrap();
        assert_eq!(state.document(), Some(&accepted));
        assert_eq!(state.rendered_document(), Some(&accepted));
        state.cancel_staged().unwrap();
        assert_eq!(state.rendered_document(), Some(&accepted));
    }

    #[test]
    fn invalid_current_document_fails_closed_until_corrected() {
        let mut state = opened();
        let mut invalid = fixture().document;
        invalid.schema = 99;
        assert!(state.synchronize(Some(scene()), 9, Some(&invalid)).is_err());
        assert!(!state.is_open());
        assert!(!state.is_available());
        assert!(state.open().is_err());
        assert!(state.render(input(vec![])).is_err());
        assert!(state.synchronize(Some(scene()), 9, Some(&invalid)).is_err());
        state
            .synchronize(Some(scene()), 9, Some(&fixture().document))
            .unwrap();
        state.open().unwrap();
        assert!(state.is_available());
    }

    #[test]
    fn invalid_font_admission_drops_previous_preview_and_cannot_reopen() {
        let mut state = opened();
        let mut prepared = fixture();
        prepared.font = vec![0; 16];
        assert!(state.admit(&prepared, scene(), 9).is_err());
        assert!(!state.is_open());
        assert!(!state.is_available());
        assert!(state.admitted_path().is_none());
        assert!(state.open().is_err());
    }

    #[test]
    fn changed_valid_font_rebuilds_context_even_for_identical_document() {
        let mut state = opened();
        let generation = state.generation();
        let mut prepared = fixture();
        prepared.font.push(0); // Font table contents stay valid; admitted bytes differ.
        state.admit(&prepared, scene(), 9).unwrap();
        assert!(state.is_open());
        assert!(state.generation() > generation);
        assert_eq!(
            state.admission.as_ref().unwrap().prepared.font,
            prepared.font
        );
    }

    #[test]
    fn all_four_screens_render_authored_labels_and_explicit_selection() {
        let mut state = opened();
        for (screen, label) in [
            (Screen::Title, "Collect and Dodge"),
            (Screen::Playing, "PLAYING"),
            (Screen::Menu, "로컬 조작"),
            (Screen::Terminal, "WON"),
        ] {
            select(&mut state, screen);
            let frame = state.render(input(vec![])).unwrap();
            assert!(
                text(&frame).iter().any(|text| text.contains(label)),
                "missing label for {screen:?}"
            );
            assert_eq!(state.active.as_ref().unwrap().screen(), screen);
            frame.output.drop_without_applying_deltas();
        }
        select(&mut state, Screen::Title);
        assert_eq!(state.active.as_ref().unwrap().screen(), Screen::Title);
    }

    #[test]
    fn terminal_variants_and_playing_phase_use_bounded_synthetic_hud() {
        let mut state = opened();
        for (phase, label) in [
            (0, "WON"),
            (1, "WON"),
            (2, "LOST: hazard"),
            (3, "LOST: time"),
        ] {
            state
                .set_settings(Settings {
                    screen: Screen::Terminal,
                    phase,
                    score: MAX_SCORE,
                    best: "123 / 456".into(),
                    ..Settings::default()
                })
                .unwrap();
            let frame = state.render(input(vec![])).unwrap();
            let labels = text(&frame);
            assert!(labels.iter().any(|text| text.contains(label)));
            assert!(labels.iter().any(|text| text.contains("999999")));
            assert!(labels.iter().any(|text| text.contains("123 / 456")));
            frame.output.drop_without_applying_deltas();
        }
        state
            .set_settings(Settings {
                screen: Screen::Playing,
                phase: 3,
                ..Settings::default()
            })
            .unwrap();
        let frame = state.render(input(vec![])).unwrap();
        assert!(text(&frame).iter().any(|text| text.contains("PLAYING")));
        assert_eq!(state.active.as_ref().unwrap().screen(), Screen::Playing);
        frame.output.drop_without_applying_deltas();
    }

    #[test]
    fn viewport_and_sample_bounds_are_atomic_and_font_checked() {
        let mut state = opened();
        let previous = state.settings().clone();
        let generation = state.generation();
        let invalid = [
            Settings {
                viewport: [0, 360],
                ..previous.clone()
            },
            Settings {
                viewport: [640, MAX_VIEWPORT + 1],
                ..previous.clone()
            },
            Settings {
                score: MAX_SCORE + 1,
                ..previous.clone()
            },
            Settings {
                phase: 4,
                ..previous.clone()
            },
            Settings {
                best: "x".repeat(MAX_BEST_BYTES + 1),
                ..previous.clone()
            },
            Settings {
                best: "line\nfeed".into(),
                ..previous.clone()
            },
            Settings {
                best: "🦄".into(),
                ..previous.clone()
            },
        ];
        for settings in invalid {
            assert!(state.set_settings(settings).is_err());
            assert_eq!(state.settings(), &previous);
            assert_eq!(state.generation(), generation);
        }
        state
            .set_settings(Settings {
                viewport: [MIN_VIEWPORT, MAX_VIEWPORT],
                best: "한글 123".into(),
                ..previous
            })
            .unwrap();
    }

    #[test]
    fn bounded_viewport_overrides_external_size_and_scale() {
        let mut state = opened();
        for viewport in [[640, 360], [360, 640], [MIN_VIEWPORT, MAX_VIEWPORT]] {
            state
                .set_settings(Settings {
                    viewport,
                    ..state.settings().clone()
                })
                .unwrap();
            let mut raw = input(vec![]);
            raw.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(f32::INFINITY, f32::NAN),
            ));
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(8.0);
            let frame = state.render(raw).unwrap();
            assert_eq!(frame.output.pixels_per_point, 1.0);
            assert_eq!(
                frame.context.input(|input| input.content_rect().size()),
                egui::vec2(viewport[0] as f32, viewport[1] as f32)
            );
            frame.output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn all_actions_are_observed_without_changing_selected_screen_or_document() {
        let mut state = opened();
        let document = state.document().unwrap().clone();
        for (screen, y, action) in [
            (Screen::Title, 180.0, Action::Play),
            (Screen::Title, 220.0, Action::Quit),
            (Screen::Playing, 115.0, Action::Menu),
            (Screen::Menu, 125.0, Action::Continue),
            (Screen::Menu, 170.0, Action::Restart),
            (Screen::Terminal, 210.0, Action::Restart),
        ] {
            select(&mut state, screen);
            render(&mut state, vec![]);
            assert_eq!(click(&mut state, egui::pos2(320.0, y)), Some(action));
            assert_eq!(state.last_action(), Some(action));
            assert_eq!(state.active.as_ref().unwrap().screen(), screen);
            assert_eq!(state.settings().screen, screen);
            assert_eq!(state.document(), Some(&document));
        }
    }

    #[test]
    fn cancellation_resize_screen_and_document_change_drop_pointer_capture() {
        for reset in 0..4 {
            let mut state = opened();
            render(&mut state, vec![button(egui::pos2(320.0, 180.0), true)]);
            match reset {
                0 => state.cancel_pointer(),
                1 => state
                    .set_settings(Settings {
                        viewport: [640, 480],
                        ..state.settings().clone()
                    })
                    .unwrap(),
                2 => {
                    select(&mut state, Screen::Menu);
                    select(&mut state, Screen::Title);
                }
                _ => {
                    let mut edited = fixture().document;
                    edited.nodes[0].offset[0] += 1;
                    state.synchronize(Some(scene()), 9, Some(&edited)).unwrap();
                }
            }
            assert_eq!(
                render(&mut state, vec![button(egui::pos2(320.0, 180.0), false)]),
                None
            );
        }
    }

    #[test]
    fn authored_parent_clipping_and_overlap_use_production_hit_testing() {
        let mut prepared = fixture();
        let node = |id: &str, parent: Option<&str>, kind, offset, size| Node {
            id: id.into(),
            parent: parent.map(str::to_owned),
            kind,
            screen: Screen::Title,
            anchor: [0, 0],
            offset,
            size,
        };
        prepared.document.nodes = vec![
            node(
                "lower",
                None,
                Kind::Button {
                    text: "Play".into(),
                    action: Action::Play,
                },
                [20, 20],
                [140, 60],
            ),
            node("parent", None, Kind::Container, [40, 20], [60, 60]),
            node(
                "child",
                Some("parent"),
                Kind::Button {
                    text: "Quit".into(),
                    action: Action::Quit,
                },
                [20, 0],
                [100, 40],
            ),
        ];
        let mut state = State::default();
        state.admit(&prepared, scene(), 9).unwrap();
        state.open().unwrap();
        assert_eq!(
            click(&mut state, egui::pos2(70.0, 30.0)),
            Some(Action::Quit)
        );
        assert_eq!(
            click(&mut state, egui::pos2(120.0, 30.0)),
            Some(Action::Play)
        );
        assert_eq!(click(&mut state, egui::pos2(50.0, 30.0)), None);
        assert_eq!(
            click(&mut state, egui::pos2(30.0, 30.0)),
            Some(Action::Play)
        );
    }
}
