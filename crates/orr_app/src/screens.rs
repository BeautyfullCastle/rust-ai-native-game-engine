//! Game screens on top of the overlay: title, pause, win, lose and restart.
//!
//! [`Screens`] is a small state machine plus the text of each screen:
//!
//! ```text
//! Title --Confirm--> Playing <--Pause/Confirm--> Paused --Restart--> Playing (new round)
//!                       |
//!                       +--win()--> Win  --Confirm/Restart--> Playing (new round)
//!                       +--lose()-> Lose --Confirm/Restart--> Playing (new round)
//! ```
//!
//! Input actions (`Confirm`, `Pause`, `Restart`) come from keys and pad buttons ([`ActionMap`]),
//! one step per press. The game wires three things:
//!
//! 1. **Input.** Its `.input(..)` closure calls [`Screens::input`] with the function that reads
//!    the keys. While any screen but `Playing` is shown, `input` returns `Input::default()` (the
//!    neutral input) and never calls that function. The key that closed a menu stays hidden from
//!    the game until it is let go, so a menu press never reaches the game as a move. The
//!    simulation only sees `Input` values, so this is deterministic: the same inputs make the
//!    same checksum, whatever menus the window showed. The simulation keeps ticking while a
//!    screen is shown (the pause stops the local controls, not the clock).
//! 2. **Result.** A view reads the game (an event, the verified frame) and calls
//!    [`Screens::win`] or [`Screens::lose`].
//! 3. **Restart.** A restart is a command to the simulation, not a new session. The game
//!    defines a `Command` that resets its frame (despawn, reset scores), and a view sends it
//!    when [`Screens::take_restart`] says `true`. The reset is part of the recorded game, so
//!    a replay and a networked match restart the same way. It keeps the random generator
//!    of the frame running, so a new round deals new random numbers from the same seed.
//!    A new `PlaySession` with the same seed would replay round one exactly; the app does not
//!    build one while it runs.
//!
//! A load of a save is not a new round: from any screen, [`Screens::resume`] goes to `Playing` and
//! drops a pending new round. A game calls it from `SceneView::on_load`.
//!
//! The words are centered on the window and wrap to its width. They are drawn with the overlay
//! font: any text the font has glyphs for (Korean with the default system font). Without a font
//! the 5x7 bitmap font draws ASCII capitals, digits and a few signs.

use std::cell::RefCell;
use std::rc::Rc;

use winit::keyboard::KeyCode;

use crate::gamepad::PadButton;
use crate::input::InputContext;
use crate::overlay::Overlay;
use crate::typeset::{Align, ScreenRect};

/// Which screen is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Title,
    /// No screen: the game takes input.
    Playing,
    Paused,
    Win,
    Lose,
}

/// What a player asks of the screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Start, resume, or play again.
    Confirm,
    /// Pause or resume.
    Pause,
    /// A new round, from the pause screen or after the end.
    Restart,
}

const ACTIONS: [Action; 3] = [Action::Confirm, Action::Pause, Action::Restart];

impl Action {
    fn index(self) -> usize {
        self as usize
    }
}

/// The keys and pad buttons of each [`Action`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionMap {
    pub confirm_keys: Vec<KeyCode>,
    pub pause_keys: Vec<KeyCode>,
    pub restart_keys: Vec<KeyCode>,
    pub confirm_buttons: Vec<PadButton>,
    pub pause_buttons: Vec<PadButton>,
    pub restart_buttons: Vec<PadButton>,
}

impl Default for ActionMap {
    /// Enter and Space or the south button confirm; P or Start pauses; R or Select restarts.
    fn default() -> Self {
        Self {
            confirm_keys: vec![KeyCode::Enter, KeyCode::Space],
            pause_keys: vec![KeyCode::KeyP],
            restart_keys: vec![KeyCode::KeyR],
            confirm_buttons: vec![PadButton::South],
            pause_buttons: vec![PadButton::Start],
            restart_buttons: vec![PadButton::Select],
        }
    }
}

impl ActionMap {
    fn keys(&self, action: Action) -> &[KeyCode] {
        match action {
            Action::Confirm => &self.confirm_keys,
            Action::Pause => &self.pause_keys,
            Action::Restart => &self.restart_keys,
        }
    }

    fn buttons(&self, action: Action) -> &[PadButton] {
        match action {
            Action::Confirm => &self.confirm_buttons,
            Action::Pause => &self.pause_buttons,
            Action::Restart => &self.restart_buttons,
        }
    }

    fn is_down(&self, action: Action, input: &InputContext) -> bool {
        input.any_held(self.keys(action)) || self.buttons(action).iter().any(|b| input.any_pad_button(*b))
    }
}

/// The words of the screens. The defaults are ASCII capitals, so they also work with the 5x7
/// bitmap font; with a font any text works (see the overlay font).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Labels {
    pub title: String,
    pub title_hint: String,
    pub paused: String,
    pub paused_hint: String,
    pub win: String,
    pub lose: String,
    pub end_hint: String,
}

impl Default for Labels {
    fn default() -> Self {
        Self {
            title: "ORRERY".to_string(),
            title_hint: "ENTER OR A: START".to_string(),
            paused: "PAUSED".to_string(),
            paused_hint: "P: RESUME   R: RESTART".to_string(),
            win: "YOU WIN".to_string(),
            lose: "GAME OVER".to_string(),
            end_hint: "ENTER OR R: PLAY AGAIN".to_string(),
        }
    }
}

/// The state machine without keys, text or sharing.
#[derive(Debug)]
struct Machine {
    screen: Screen,
    restart: bool,
}

impl Machine {
    /// `true` when the action changed the screen.
    fn act(&mut self, action: Action) -> bool {
        let (next, restart) = match (self.screen, action) {
            (Screen::Title, Action::Confirm) => (Screen::Playing, true),
            (Screen::Playing, Action::Pause) => (Screen::Paused, false),
            (Screen::Paused, Action::Pause | Action::Confirm) => (Screen::Playing, false),
            (Screen::Paused, Action::Restart) => (Screen::Playing, true),
            (Screen::Win | Screen::Lose, Action::Confirm | Action::Restart) => (Screen::Playing, true),
            _ => return false,
        };
        self.screen = next;
        self.restart |= restart;
        true
    }

    /// Plays on from the state the game is in: no screen, and no new round.
    fn resume(&mut self) {
        self.screen = Screen::Playing;
        self.restart = false;
    }

    fn end(&mut self, screen: Screen) -> bool {
        let playing = self.screen == Screen::Playing;
        if playing {
            self.screen = screen;
        }
        playing
    }
}

#[derive(Debug)]
struct State {
    machine: Machine,
    map: ActionMap,
    labels: Labels,
    /// Was each action down at the last poll? A press is a change from up to down.
    down: [bool; 3],
    /// Keys and buttons that closed a screen and are still held: hidden from the game.
    masked_keys: Vec<KeyCode>,
    masked_buttons: Vec<PadButton>,
}

/// The title, pause, win, lose and restart screens of a game. Cloning gives another handle to
/// the same screens: the input closure and a view each keep one. See the module docs.
#[derive(Clone, Debug)]
pub struct Screens(Rc<RefCell<State>>);

impl Screens {
    /// Screens that start on the title screen, with the default keys and the words `labels`.
    pub fn new(labels: Labels) -> Self {
        Self::from_screen(Screen::Title, labels)
    }

    /// Screens that start in the game, with no title.
    pub fn playing(labels: Labels) -> Self {
        Self::from_screen(Screen::Playing, labels)
    }

    fn from_screen(screen: Screen, labels: Labels) -> Self {
        Self(Rc::new(RefCell::new(State {
            machine: Machine { screen, restart: false },
            map: ActionMap::default(),
            labels,
            down: [false; 3],
            masked_keys: Vec::new(),
            masked_buttons: Vec::new(),
        })))
    }

    /// Other keys and buttons for the actions.
    pub fn with_map(self, map: ActionMap) -> Self {
        self.0.borrow_mut().map = map;
        self
    }

    pub fn screen(&self) -> Screen {
        self.0.borrow().machine.screen
    }

    /// `true` while the game takes input (no screen is shown).
    pub fn is_playing(&self) -> bool {
        self.screen() == Screen::Playing
    }

    /// Applies one action. `true` when it changed the screen. [`Screens::input`] calls this for
    /// the presses it sees; a program with its own controls can call it directly.
    pub fn act(&self, action: Action) -> bool {
        self.0.borrow_mut().machine.act(action)
    }

    /// The game was won: shows the win screen. Only from `Playing`; `true` when it did.
    pub fn win(&self) -> bool {
        self.0.borrow_mut().machine.end(Screen::Win)
    }

    /// The game was lost: shows the lose screen. Only from `Playing`; `true` when it did.
    pub fn lose(&self) -> bool {
        self.0.borrow_mut().machine.end(Screen::Lose)
    }

    /// The game goes on from a loaded state (see `SceneView::on_load`): from any screen to `Playing`,
    /// and a new round that was asked for but not taken yet is dropped, so no reset command follows.
    pub fn resume(&self) {
        self.0.borrow_mut().machine.resume();
    }

    /// `true` once for each new round (the start from the title, a restart): the view then
    /// sends the game's reset command.
    pub fn take_restart(&self) -> bool {
        std::mem::take(&mut self.0.borrow_mut().machine.restart)
    }

    /// The input of one local player for this frame. Polls the action keys and buttons first
    /// (one action per press). Then, while the game is playing, it returns what `read` makes
    /// of the keys, without the keys that just closed a screen. On any screen it returns
    /// `I::default()` and does not call `read`.
    pub fn input<I: Default>(&self, keys: &InputContext, read: impl FnOnce(&InputContext) -> I) -> I {
        let mut state = self.0.borrow_mut();
        state.poll(keys);
        if state.machine.screen != Screen::Playing {
            return I::default();
        }
        if state.masked_keys.is_empty() && state.masked_buttons.is_empty() {
            return read(keys);
        }
        read(&keys.without(&state.masked_keys, &state.masked_buttons))
    }

    /// Draws the words of the screen that is shown (nothing while playing): the heading centered
    /// in the upper middle of the window, the hint below it. Both wrap to the window width. A
    /// window-free overlay (see [`Overlay::screen_size`]) has no size, so the text starts at
    /// the left edge there.
    pub fn draw(&self, overlay: &mut Overlay) {
        let state = self.0.borrow();
        let labels = &state.labels;
        const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
        const SOFT: [f32; 4] = [0.8, 0.85, 1.0, 0.95];
        const GOLD: [f32; 4] = [1.0, 0.85, 0.25, 1.0];
        let (head, hint, color) = match state.machine.screen {
            Screen::Playing => return,
            Screen::Title => (&labels.title, &labels.title_hint, WHITE),
            Screen::Paused => (&labels.paused, &labels.paused_hint, WHITE),
            Screen::Win => (&labels.win, &labels.end_hint, GOLD),
            Screen::Lose => (&labels.lose, &labels.end_hint, [0.95, 0.4, 0.35, 1.0]),
        };
        const MARGIN: f32 = 48.0;
        const HEAD: f32 = 56.0;
        const HINT: f32 = 21.0;
        let screen = overlay.screen_rect();
        let top = if screen.h > 0.0 { screen.h * 0.38 - HEAD } else { 150.0 };
        let area = ScreenRect::new(MARGIN, top, (screen.w - 2.0 * MARGIN).max(0.0), 0.0);
        let [_, used] = overlay.text_box(area, HEAD, color, head, Align::Center);
        overlay.text_box(ScreenRect { y: top + used + HEAD * 0.4, ..area }, HINT, SOFT, hint, Align::Center);
    }
}

impl Default for Screens {
    fn default() -> Self {
        Self::new(Labels::default())
    }
}

impl State {
    /// Reads the presses of this frame and lets go of the masks of keys that are up again.
    fn poll(&mut self, input: &InputContext) {
        for action in ACTIONS {
            let down = self.map.is_down(action, input);
            let pressed = down && !self.down[action.index()];
            self.down[action.index()] = down;
            if pressed && self.machine.act(action) {
                // The press closed or opened a screen. The game must not see it, even when
                // the same key is a move or a jump in the game.
                for key in self.map.keys(action).iter().filter(|k| input.held(**k)) {
                    if !self.masked_keys.contains(key) {
                        self.masked_keys.push(*key);
                    }
                }
                for button in self.map.buttons(action).iter().filter(|b| input.any_pad_button(**b)) {
                    if !self.masked_buttons.contains(button) {
                        self.masked_buttons.push(*button);
                    }
                }
            }
        }
        self.masked_keys.retain(|k| input.held(*k));
        self.masked_buttons.retain(|b| input.any_pad_button(*b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamepad::PadEvent;

    fn keys(down: &[KeyCode]) -> InputContext {
        let mut input = InputContext::default();
        for key in down {
            input.set(*key, true);
        }
        input
    }

    /// The read function of a game where Space is also the action key.
    fn read(input: &InputContext) -> (i32, bool) {
        let stick = input.stick(&crate::KeyLayout::WASD);
        (stick.x, stick.action)
    }

    #[test]
    fn the_screens_follow_the_actions() {
        let screens = Screens::default();
        assert_eq!(screens.screen(), Screen::Title);
        assert!(!screens.act(Action::Pause) && !screens.act(Action::Restart), "nothing to pause on the title");
        assert!(!screens.win(), "a title cannot be won");
        assert!(screens.act(Action::Confirm));
        assert!(screens.is_playing() && screens.take_restart(), "the first round starts clean");
        assert!(!screens.take_restart(), "a restart is taken once");

        assert!(!screens.act(Action::Confirm) && !screens.act(Action::Restart), "no menu action while playing");
        assert!(screens.act(Action::Pause));
        assert_eq!(screens.screen(), Screen::Paused);
        assert!(screens.act(Action::Confirm), "confirm resumes");
        assert!(screens.is_playing() && !screens.take_restart(), "a resume is not a restart");
        screens.act(Action::Pause);
        assert!(screens.act(Action::Pause), "pause toggles");
        screens.act(Action::Pause);
        assert!(screens.act(Action::Restart));
        assert!(screens.is_playing() && screens.take_restart());

        assert!(screens.win());
        assert_eq!(screens.screen(), Screen::Win);
        assert!(!screens.lose() && !screens.act(Action::Pause), "the end screen only offers a new round");
        assert!(screens.act(Action::Restart));
        assert!(screens.take_restart());
        assert!(screens.lose());
        assert_eq!(screens.screen(), Screen::Lose);
        assert!(screens.act(Action::Confirm) && screens.is_playing() && screens.take_restart());
    }

    #[test]
    fn a_resume_plays_on_from_any_screen_without_a_new_round() {
        for start in [Screen::Title, Screen::Paused, Screen::Win, Screen::Lose] {
            let screens = Screens::from_screen(start, Labels::default());
            screens.resume();
            assert!(screens.is_playing() && !screens.take_restart(), "{start:?}: a loaded game is not a new round");
            assert!(!screens.act(Action::Confirm) && !screens.take_restart(), "{start:?}: Enter does nothing in play");
        }
        // A round that was asked for and not taken yet is dropped too.
        let screens = Screens::default();
        screens.act(Action::Confirm);
        screens.resume();
        assert!(!screens.take_restart());
    }

    #[test]
    fn a_press_acts_once_and_a_held_key_does_not_repeat() {
        let screens = Screens::default();
        let enter = keys(&[KeyCode::Enter]);
        screens.input(&enter, |_| 0);
        screens.input(&enter, |_| 0);
        assert!(screens.is_playing(), "one press started the game");
        screens.input(&keys(&[KeyCode::KeyP]), |_| 0);
        assert_eq!(screens.screen(), Screen::Paused);
        screens.input(&keys(&[KeyCode::KeyP]), |_| 0);
        assert_eq!(screens.screen(), Screen::Paused, "P held: still the same press");
        screens.input(&keys(&[]), |_| 0);
        screens.input(&keys(&[KeyCode::KeyP]), |_| 0);
        assert!(screens.is_playing(), "a new press resumes");
    }

    #[test]
    fn a_screen_gives_the_neutral_input_and_never_reads_the_keys() {
        let screens = Screens::default();
        let walking = keys(&[KeyCode::KeyD]);
        let mut reads = 0;
        let mut poll = |input: &InputContext| {
            screens.input(input, |k| {
                reads += 1;
                read(k)
            })
        };
        assert_eq!(poll(&walking), (0, false), "title");
        screens.act(Action::Confirm);
        assert_eq!(poll(&walking), (1, false), "playing: the keys count");
        screens.act(Action::Pause);
        assert_eq!(poll(&walking), (0, false), "paused");
        screens.act(Action::Pause);
        screens.win();
        assert_eq!(poll(&walking), (0, false), "won");
        assert_eq!(reads, 1, "only the playing frame read the keys");
    }

    #[test]
    fn the_key_that_closed_a_screen_does_not_reach_the_game_until_it_is_let_go() {
        // Space confirms and is the action key of the game too.
        let screens = Screens::default();
        let space = keys(&[KeyCode::Space]);
        assert_eq!(screens.input(&space, read), (0, false), "the press that starts the game is not an action");
        assert_eq!(screens.input(&space, read), (0, false), "held: still hidden");
        assert_eq!(screens.input(&keys(&[]), read), (0, false));
        assert_eq!(screens.input(&space, read), (0, true), "pressed again in the game: it counts");

        // A key that is not a menu action in the game is not hidden.
        let both = keys(&[KeyCode::Space, KeyCode::KeyD]);
        assert_eq!(screens.input(&both, read), (1, true));
    }

    #[test]
    fn pad_buttons_drive_the_screens_and_the_press_that_closed_one_is_hidden() {
        let screens = Screens::default();
        let mut input = InputContext::default();
        input.apply_pad(PadEvent::Button { pad: 0, button: PadButton::South, down: true });
        let mut seen = Vec::new();
        screens.input(&input, |k| seen.push(k.pad_button(0, PadButton::South)));
        assert!(screens.is_playing());
        screens.input(&input, |k| seen.push(k.pad_button(0, PadButton::South)));
        assert_eq!(seen, vec![false, false], "the confirm button is hidden from the game while it is held");
        input.apply_pad(PadEvent::Button { pad: 0, button: PadButton::South, down: false });
        input.apply_pad(PadEvent::Button { pad: 0, button: PadButton::Start, down: true });
        screens.input(&input, |_| ());
        assert_eq!(screens.screen(), Screen::Paused, "Start pauses");
    }

    #[test]
    fn the_words_of_the_shown_screen_are_drawn_and_a_game_in_play_draws_nothing() {
        let screens = Screens::new(Labels { title: "STAR PICK".to_string(), ..Labels::default() });
        let texts = |screens: &Screens| {
            let mut overlay = Overlay::default();
            screens.draw(&mut overlay);
            overlay.texts().iter().map(|t| t.text.clone()).collect::<Vec<_>>()
        };
        assert_eq!(texts(&screens)[0], "STAR PICK");
        screens.act(Action::Confirm);
        assert!(texts(&screens).is_empty());
        screens.act(Action::Pause);
        assert_eq!(texts(&screens)[0], "PAUSED");
        screens.act(Action::Pause);
        screens.win();
        assert_eq!(texts(&screens)[0], "YOU WIN");
        screens.act(Action::Restart);
        screens.lose();
        assert_eq!(texts(&screens)[0], "GAME OVER");
    }

    #[test]
    fn every_default_word_has_a_glyph() {
        let labels = Labels::default();
        let mut overlay = Overlay::default();
        for word in [&labels.title, &labels.title_hint, &labels.paused, &labels.paused_hint, &labels.win, &labels.lose, &labels.end_hint] {
            overlay.text([0.0, 0.0], 14.0, [1.0; 4], word);
        }
        assert!(overlay.missing_glyphs().is_empty(), "{:?}", overlay.missing_glyphs());
    }

    #[test]
    fn the_words_are_centered_on_the_window_and_the_hint_sits_below_the_heading() {
        let screens = Screens::new(Labels { title: "ABCD".to_string(), title_hint: "AB".to_string(), ..Labels::default() });
        let mut overlay = Overlay::default();
        overlay.set_screen_size([800.0, 600.0]);
        screens.draw(&mut overlay);

        let [head, hint] = [&overlay.texts()[0], &overlay.texts()[1]];
        // The bitmap font: 6 pixels per font pixel, so "ABCD" at 56 is 4 * 48 = 192 pixels wide, "AB" at 21 is 2 * 18 = 36.
        assert_eq!(head.pos[0], (800.0 - 192.0) / 2.0);
        assert_eq!(hint.pos[0], (800.0 - 36.0) / 2.0);
        assert!(hint.pos[1] > head.pos[1] + 56.0, "the hint is below the heading");
        assert!(head.pos[1] > 100.0 && head.pos[1] < 300.0, "in the upper middle: {}", head.pos[1]);
    }

    #[test]
    fn a_heading_wider_than_the_window_wraps_and_the_hint_follows_the_last_line() {
        let screens = Screens::new(Labels { title: "가나다라마바사아자차".to_string(), title_hint: "AB".to_string(), ..Labels::default() });
        let mut overlay = Overlay::default();
        overlay.set_screen_size([400.0, 300.0]);
        screens.draw(&mut overlay);

        // 400 - 2 * 48 = 304 pixels of text width, 48 per character at size 56: six per line.
        let lines: Vec<&str> = overlay.texts().iter().map(|t| t.text.as_str()).collect();
        assert_eq!(lines, ["가나다라마바", "사아자차", "AB"]);
        let ys: Vec<f32> = overlay.texts().iter().map(|t| t.pos[1]).collect();
        assert!(ys[0] < ys[1] && ys[1] < ys[2]);
    }
}
