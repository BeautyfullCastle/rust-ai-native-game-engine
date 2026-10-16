//! Optional Korean, view-only game UI. The simulation keeps running in menus.
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Title,
    Playing,
    Menu,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Play,
    Menu,
    Continue,
    Restart,
    Quit,
    #[cfg(feature = "player-settings")]
    FireSpace,
    #[cfg(feature = "player-settings")]
    FireLeftMouse,
    #[cfg(feature = "player-settings")]
    SettingsApply,
    #[cfg(feature = "player-settings")]
    SettingsCancel,
    #[cfg(feature = "player-settings")]
    SettingsReset,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Hud {
    pub tick: u64,
    pub verified_tick: u64,
    pub rollbacks: u64,
}

pub struct GameUi {
    pub context: egui::Context,
    screen: Screen,
    restart_allowed: bool,
    /// Last real widget bounds, also useful for accessibility/input regression tests.
    pub buttons: Vec<(Action, egui::Rect)>,
    hud_rect: Option<egui::Rect>,
    pointer_layout_stale: bool,
    #[cfg(feature = "player-settings")]
    player_settings: Option<crate::player_controls::PlayerSettingsSession>,
}
impl GameUi {
    pub fn open(root: &Path, restart_allowed: bool) -> Result<Self, String> {
        let project = orr_package::Project::open(root, orr_package::Runtime::content_only())
            .map_err(|e| format!("game UI project: {e}"))?;
        let font = project
            .read_asset("korean-game-ui", "OrreryKoreanUI.otf")
            .map_err(|e| format!("game UI font package: {e}; install assets/game_ui_font first"))?;
        Self::from_font(font, restart_allowed)
    }
    pub fn from_font(font: Vec<u8>, restart_allowed: bool) -> Result<Self, String> {
        Self::validate_font(&font)?;
        let context = egui::Context::default();
        let mut definitions = egui::FontDefinitions::default();
        definitions
            .font_data
            .insert("korean-ui".into(), egui::FontData::from_owned(font).into());
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            definitions
                .families
                .entry(family)
                .or_default()
                .insert(0, "korean-ui".into());
        }
        context.set_fonts(definitions);
        Ok(Self {
            context,
            screen: Screen::Title,
            restart_allowed,
            buttons: Vec::new(),
            hud_rect: None,
            pointer_layout_stale: true,
            #[cfg(feature = "player-settings")]
            player_settings: None,
        })
    }
    /// Validate a bounded font against the parser and UI text corpus without creating UI state.
    pub fn validate_font(font: &[u8]) -> Result<(), String> {
        Self::validate_font_with_corpus(
            font,
            include_str!("../../../assets/game_ui_font/corpus.txt"),
        )?;
        #[cfg(feature = "player-settings")]
        Self::validate_font_with_corpus(font, SETTINGS_TEXT_CORPUS)?;
        Ok(())
    }
    pub(crate) fn validate_font_with_corpus(font: &[u8], corpus: &str) -> Result<(), String> {
        if font.is_empty() || font.len() > 16 * 1024 * 1024 {
            return Err("game UI font must be 1 byte to 16 MiB".into());
        }
        let face =
            ttf_parser::Face::parse(font, 0).map_err(|e| format!("invalid game UI font: {e:?}"))?;
        for character in corpus.chars() {
            if !character.is_whitespace() && face.glyph_index(character).is_none() {
                return Err(format!(
                    "game UI font lacks required glyph U+{:04X}",
                    character as u32
                ));
            }
        }
        Ok(())
    }
    #[cfg(feature = "player-settings")]
    pub fn set_player_settings(&mut self, session: crate::player_controls::PlayerSettingsSession) {
        self.player_settings = Some(session);
    }
    #[cfg(feature = "player-settings")]
    pub fn player_settings(&self) -> Option<&crate::player_controls::PlayerSettingsSession> {
        self.player_settings.as_ref()
    }
    #[cfg(feature = "player-settings")]
    pub fn apply_player_settings(
        &mut self,
        controls: &mut crate::arena_input::ArenaControls,
    ) -> bool {
        self.player_settings
            .as_mut()
            .is_some_and(|settings| settings.apply(controls))
    }
    pub fn screen(&self) -> Screen {
        self.screen
    }
    pub fn blocks_controls(&self) -> bool {
        self.screen != Screen::Playing
    }
    pub fn apply(&mut self, action: Action) -> bool {
        match action {
            Action::Play | Action::Continue => self.screen = Screen::Playing,
            Action::Menu => self.screen = Screen::Menu,
            Action::Restart if self.restart_allowed => self.screen = Screen::Playing,
            Action::Restart => return false,
            Action::Quit => {}
            #[cfg(feature = "player-settings")]
            Action::FireSpace
            | Action::FireLeftMouse
            | Action::SettingsCancel
            | Action::SettingsReset
            | Action::SettingsApply => {
                let Some(settings) = &mut self.player_settings else {
                    return false;
                };
                match action {
                    Action::FireSpace => {
                        settings.select(crate::player_settings::FireBinding::Space)
                    }
                    Action::FireLeftMouse => {
                        settings.select(crate::player_settings::FireBinding::LeftMouse)
                    }
                    Action::SettingsCancel => settings.cancel(),
                    Action::SettingsReset => settings.reset(),
                    _ => {}
                }
            }
        }
        true
    }
    /// Keep local controls neutral after resize/DPI/resume until a new UI layout.
    pub fn invalidate_pointer_layout(&mut self) {
        self.pointer_layout_stale = true;
    }

    /// Hit-test the latest queued pointer point against the completed HUD layout
    /// before egui's next pass refreshes its previous-frame capture state.
    pub fn pending_pointer_over_ui(&self, input: &egui::RawInput) -> bool {
        // Resized/scaled surfaces must not send a click through old HUD bounds.
        // Only local controls wait for the next completed UI layout.
        if self.pointer_layout_stale {
            return true;
        }
        let position = input
            .events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::PointerMoved(position)
                | egui::Event::PointerButton { pos: position, .. } => Some(Some(*position)),
                egui::Event::PointerGone => Some(None),
                _ => None,
            })
            .unwrap_or_else(|| self.context.pointer_latest_pos());
        position.is_some_and(|point| {
            self.hud_rect.is_some_and(|rect| rect.contains(point))
                || self.buttons.iter().any(|(_, rect)| rect.contains(point))
        })
    }

    pub fn show(&mut self, input: egui::RawInput, hud: Hud) -> (egui::FullOutput, Option<Action>) {
        let mut action = None;
        self.buttons.clear();
        let context = self.context.clone();
        let output = context.run_ui(input, |root| {
            let hud = egui::Panel::top("arena-hud").show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "오러리 투기장 | 틱 {} | 검증 {} | 롤백 {}",
                        hud.tick, hud.verified_tick, hud.rollbacks
                    ));
                    if self.screen == Screen::Playing {
                        let response = ui.button("메뉴");
                        self.buttons.push((Action::Menu, response.rect));
                        if response.clicked() {
                            action = Some(Action::Menu);
                        }
                    }
                });
            });
            self.hud_rect = Some(hud.response.rect);
            if self.screen != Screen::Playing {
                egui::Window::new(if self.screen == Screen::Title {
                    "오러리 투기장"
                } else {
                    "게임 메뉴"
                })
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(root.ctx(), |ui| {
                    ui.label("로컬 조작만 멈춥니다. 시뮬레이션과 네트워크는 계속됩니다.");
                    #[cfg(not(feature = "player-settings"))]
                    ui.label("이동: WASD / 방향키 · 발사: Space (기본 설정)");
                    #[cfg(feature = "player-settings")]
                    if let Some(settings) = &self.player_settings {
                        use crate::player_settings::FireBinding;
                        ui.set_max_width(600.0);
                        ui.label(if settings.is_external() {
                            "이동과 발사: 외부 입력 파일 설정"
                        } else if settings.active() == FireBinding::Space {
                            "이동: WASD / 방향키 · 발사: Space"
                        } else {
                            "이동: WASD / 방향키 · 발사: 마우스 왼쪽 버튼"
                        });
                        ui.label("발사 조작 설정");
                        ui.horizontal(|ui| {
                            for (kind, value, label) in [
                                (Action::FireSpace, FireBinding::Space, "Space"),
                                (
                                    Action::FireLeftMouse,
                                    FireBinding::LeftMouse,
                                    "마우스 왼쪽 버튼",
                                ),
                            ] {
                                let response = ui.add_enabled(
                                    settings.editable(),
                                    egui::RadioButton::new(
                                        settings.draft() == value && !settings.is_external(),
                                        label,
                                    ),
                                );
                                self.buttons.push((kind, response.rect));
                                if response.clicked() {
                                    action = Some(kind);
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            for (kind, label) in [
                                (Action::SettingsApply, "적용"),
                                (Action::SettingsCancel, "취소"),
                                (Action::SettingsReset, "기본값"),
                            ] {
                                let enabled = settings.editable()
                                    && (kind != Action::SettingsApply || settings.dirty());
                                let response = ui.add_enabled(enabled, egui::Button::new(label));
                                self.buttons.push((kind, response.rect));
                                if response.clicked() {
                                    action = Some(kind);
                                }
                            }
                        });
                        ui.label(settings.status());
                    } else {
                        ui.label("이동: WASD / 방향키 · 발사: Space (기본 설정)");
                    }
                    let primary = if self.screen == Screen::Title {
                        (Action::Play, "플레이")
                    } else {
                        (Action::Continue, "계속하기")
                    };
                    for (kind, label) in [
                        primary,
                        (Action::Restart, "다시 시작"),
                        (Action::Quit, "종료"),
                    ] {
                        let response = ui.add_enabled(
                            kind != Action::Restart || self.restart_allowed,
                            egui::Button::new(label),
                        );
                        self.buttons.push((kind, response.rect));
                        if response.clicked() {
                            action = Some(kind);
                        }
                    }
                    if !self.restart_allowed {
                        ui.label("릴레이에서는 다시 시작할 수 없습니다.");
                    }
                });
            }
        });
        self.pointer_layout_stale = false;
        (output, action)
    }
}

// Additional runtime corpus: the installed immutable font package is not rebuilt.
#[cfg(feature = "player-settings")]
const SETTINGS_TEXT_CORPUS: &str = "발사 조작 설정 마우스 왼쪽 버튼 적용 취소 기본값 외부 입력 파일 사용 중 이 실행에서는 설정을 저장하지 않습니다 저장 불가 컴퓨터의 Arena에서 공유됩니다 적용 실패 이전 조작을 유지합니다 저장하고 적용했습니다 설정은 적용됐지만 저장 안정성을 확인할 수 없습니다 다시 실행해 확인하세요 이동과 발사";

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "player-settings")]
    #[test]
    fn external_map_disables_real_settings_widgets_without_importing() {
        use crate::player_controls::PlayerSettingsSession;
        let mut ui = ui(true);
        ui.set_player_settings(PlayerSettingsSession::external(
            crate::arena_input::default_map(),
        ));
        for action in [
            Action::FireLeftMouse,
            Action::SettingsApply,
            Action::SettingsReset,
            Action::SettingsCancel,
        ] {
            assert_eq!(
                click(&mut ui, action),
                None,
                "external selectors are visibly disabled"
            );
        }
        assert!(ui.player_settings().unwrap().is_external());
        assert!(!ui.player_settings().unwrap().editable());
        assert_eq!(click(&mut ui, Action::Play), Some(Action::Play));
    }
    fn ui(restart: bool) -> GameUi {
        GameUi::from_font(
            include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec(),
            restart,
        )
        .unwrap()
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
    fn click(ui: &mut GameUi, action: Action) -> Option<Action> {
        // Anchored windows need a sizing pass before their stable hit rectangles.
        for _ in 0..3 {
            ui.show(input(vec![]), Hud::default())
                .0
                .drop_without_applying_deltas();
        }
        let point = ui
            .buttons
            .iter()
            .find(|(a, _)| *a == action)
            .unwrap()
            .1
            .center();
        ui.show(
            input(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]),
            Hud::default(),
        )
        .0
        .drop_without_applying_deltas();
        let (output, action) = ui.show(
            input(vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }]),
            Hud::default(),
        );
        output.drop_without_applying_deltas();
        action
    }
    #[test]
    fn real_widgets_title_hud_menu_continue_restart_quit() {
        let mut ui = ui(true);
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
            assert_eq!(click(&mut ui, action), Some(action));
            assert!(ui.apply(action));
        }
    }
    #[test]
    fn queued_menu_click_captures_remapped_fire_before_the_next_ui_frame() {
        use crate::arena_input::{default_map, ArenaControls};
        use orr_input::Button;
        let mut ui = ui(true);
        ui.apply(Action::Play);
        for _ in 0..3 {
            ui.show(
                input(vec![egui::Event::PointerMoved(egui::pos2(800.0, 800.0))]),
                Hud::default(),
            )
            .0
            .drop_without_applying_deltas();
        }
        assert!(!ui.context.egui_wants_pointer_input());
        let point = ui
            .buttons
            .iter()
            .find(|(action, _)| *action == Action::Menu)
            .unwrap()
            .1
            .center();
        let pending = input(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        // No show() call intervenes: this is the same raw event queue that the
        // winit adapter checks before immediately publishing held bridge input.
        assert!(ui.pending_pointer_over_ui(&pending));
        let mut map = default_map();
        map.actions
            .iter_mut()
            .find(|action| action.name == "fire")
            .unwrap()
            .bindings = vec![Button::Mouse { button: 0 }];
        let mut controls = ArenaControls::new(map).unwrap();
        controls.set_ui_capture(ui.pending_pointer_over_ui(&pending));
        controls.button(Button::Mouse { button: 0 }, true, false, false);
        assert!(!controls.keys().fire);
        assert!(!ui.pending_pointer_over_ui(&input(vec![egui::Event::PointerGone])));
        assert!(
            !ui.pending_pointer_over_ui(&input(vec![egui::Event::PointerMoved(egui::pos2(
                800.0, 800.0
            ))]))
        );
        controls.button(Button::Mouse { button: 0 }, false, false, true);
        controls.set_ui_capture(false);
        controls.button(Button::Mouse { button: 0 }, true, true, false);
        assert!(
            !controls.keys().fire,
            "capture release still requires a fresh press"
        );
    }

    #[test]
    fn pointer_layout_invalidates_until_resized_dpi_layout_is_observed() {
        let mut ui = ui(true);
        ui.apply(Action::Play);
        ui.show(input(vec![]), Hud::default())
            .0
            .drop_without_applying_deltas();
        let outside = input(vec![egui::Event::PointerMoved(egui::pos2(400.0, 400.0))]);
        assert!(!ui.pending_pointer_over_ui(&outside));
        ui.invalidate_pointer_layout();
        assert!(ui.pending_pointer_over_ui(&outside));
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
        for _ in 0..3 {
            ui.show(resized.clone(), Hud::default())
                .0
                .drop_without_applying_deltas();
        }
        let menu = ui
            .buttons
            .iter()
            .find(|(action, _)| *action == Action::Menu)
            .unwrap()
            .1
            .center();
        assert!(ui.pending_pointer_over_ui(&input(vec![egui::Event::PointerMoved(menu)])));
        assert!(!ui.pending_pointer_over_ui(&outside));
    }

    #[test]
    fn relay_restart_disabled_in_widget_and_dispatch() {
        let mut ui = ui(false);
        assert_eq!(click(&mut ui, Action::Restart), None);
        assert!(!ui.apply(Action::Restart));
        assert_eq!(ui.screen(), Screen::Title);
    }
    #[test]
    fn font_covers_all_modern_hangul_and_basic_jamo() {
        let bytes = include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf");
        let face = ttf_parser::Face::parse(bytes, 0).unwrap();
        for value in (0xAC00..=0xD7A3)
            .chain(0x1100..=0x11FF)
            .chain(0x3131..=0x318E)
            .chain(0x20..=0x7E)
        {
            let character = char::from_u32(value).unwrap();
            assert!(
                face.glyph_index(character).is_some(),
                "missing U+{value:04X}"
            );
        }
        assert!(GameUi::from_font(vec![0; 20], true).is_err());
    }
    #[test]
    fn font_validator_rejects_a_missing_required_glyph() {
        let bytes = include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf");
        let error = GameUi::validate_font_with_corpus(bytes, "🦄").unwrap_err();
        assert!(error.contains("lacks required glyph"));
    }
    #[test]
    fn font_package_is_required_hash_verified_and_removable() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temp_root = std::env::temp_dir();
        #[cfg(unix)]
        let temp_root = std::fs::canonicalize(temp_root).unwrap();
        let root = temp_root.join(format!(
            "orr-game-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let project =
            orr_package::Project::open(&root, orr_package::Runtime::content_only()).unwrap();
        assert!(GameUi::open(&root, true).is_err());
        let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("assets/game_ui_font");
        project.install(&[source]).unwrap();
        assert!(GameUi::open(&root, true).is_ok());
        let lock = project.list().unwrap();
        let digest = &lock.packages["korean-game-ui"].digest;
        let font = root
            .join(".orr/packages/objects")
            .join(digest)
            .join("OrreryKoreanUI.otf");
        std::fs::write(&font, b"tampered").unwrap();
        assert!(GameUi::open(&root, true).is_err());
        assert!(project.remove("korean-game-ui").is_err());
        std::fs::write(
            &font,
            include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf"),
        )
        .unwrap();
        project.remove("korean-game-ui").unwrap();
        assert!(GameUi::open(&root, true).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn menu_capture_neutralizes_real_bridge_without_stopping_simulation() {
        use crate::{
            arena_input::{default_map, ArenaControls},
            arena_view::{arena_bridge_config, loopback_pair, Keys, Loopback},
        };
        use orr_bridge::{Bridge, InProc};
        use orr_input::{Button, Key};
        let mut controls = ArenaControls::new(default_map()).unwrap();
        let mut bridge = InProc::new(loopback_pair(Loopback::default()), arena_bridge_config());
        let button = Button::Keyboard { key: Key::D };
        controls.button(button, true, false, false);
        controls.publish(&mut bridge).unwrap();
        bridge.update(std::time::Duration::from_millis(50));
        let before = bridge.snapshot().unwrap().tick();
        controls.set_paused(true);
        controls.set_ui_capture(true);
        controls.publish(&mut bridge).unwrap();
        assert_eq!(controls.keys(), Keys::default());
        bridge.update(std::time::Duration::from_millis(50));
        assert!(bridge.snapshot().unwrap().tick() > before);
        controls.button(button, false, false, true); // release remains processed
        controls.set_paused(false);
        controls.set_ui_capture(false);
        controls.button(button, true, true, false); // repeats cannot resume a held key
        assert_eq!(controls.keys(), Keys::default());
        controls.button(button, true, false, false);
        assert!(controls.keys().right);
        controls.set_focused(false);
        controls.set_focused(true);
        assert_eq!(controls.keys(), Keys::default());
    }
}
