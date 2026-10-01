//! The interactive terminal view: crossterm raw mode on the alternate screen, about 30 frames a
//! second, keys to input bytes (through the schema's input layout) and timeline controls.

use std::io::{BufWriter, Stdout, Write};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use orr_viewstream::FLAG_PAUSED;

use crate::input::{encode, Controls};
use crate::render::{render, Camera};
use crate::schema::ViewSchema;
use crate::source::{Control, Source};
use crate::state::ViewState;

/// Terminals report no key releases: a pressed direction counts as held for this long (the
/// key repeat of the terminal keeps it alive).
const HOLD: Duration = Duration::from_millis(300);

pub struct UiOpts {
    pub player: u8,
    /// Redraws per second (at most 30 is plenty).
    pub fps: u32,
    /// Send `Play` at the start (a host of our own starts paused).
    pub autoplay: bool,
}

/// Puts the terminal back, also when the loop ends with an error or a panic.
struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), Show, LeaveAlternateScreen, ResetColor);
        let _ = terminal::disable_raw_mode();
    }
}

#[derive(Default)]
struct Held {
    x: Option<(i8, Instant)>,
    y: Option<(i8, Instant)>,
    spin: Option<(i8, Instant)>,
    fire: Option<Instant>,
}

impl Held {
    fn controls(&self, now: Instant) -> Controls {
        let live = |h: Option<(i8, Instant)>| h.filter(|(_, until)| *until > now).map_or(0, |(v, _)| v);
        Controls { x: live(self.x), y: live(self.y), spin: live(self.spin), fire: self.fire.is_some_and(|u| u > now) }
    }

    fn press(&mut self, code: KeyCode, now: Instant) -> bool {
        let until = now + HOLD;
        match code {
            KeyCode::Left | KeyCode::Char('a') => self.x = Some((-1, until)),
            KeyCode::Right | KeyCode::Char('d') => self.x = Some((1, until)),
            KeyCode::Up | KeyCode::Char('w') => self.y = Some((1, until)),
            KeyCode::Down => self.y = Some((-1, until)),
            KeyCode::Char('z') => self.spin = Some((-1, until)),
            KeyCode::Char('c') => self.spin = Some((1, until)),
            KeyCode::Char('f') | KeyCode::Enter => self.fire = Some(until),
            _ => return false,
        }
        true
    }
}

fn color_of(rgba: [u8; 4]) -> Color {
    Color::Rgb { r: rgba[0], g: rgba[1], b: rgba[2] }
}

fn clip(s: &str, w: usize) -> String {
    s.chars().take(w).collect()
}

fn draw(out: &mut BufWriter<Stdout>, state: &mut ViewState, help: &str, now: Instant) -> std::io::Result<()> {
    let (tw, th) = terminal::size().unwrap_or((80, 24));
    let (tw, th) = (tw as usize, th as usize);
    let events_h = if th >= 20 { 4 } else if th >= 12 { 2 } else { 0 };
    let grid_h = th.saturating_sub(2 + events_h).max(1);
    // The newest frame, with the correction of a rollback still fading out (about 0.1 s).
    if let (Some(f), Some(cam)) = (state.display_frame(now), &state.camera) {
        let grid = render(&f, &state.schema, cam, state.alpha(now), tw, grid_h, true);
        let mut last = None;
        for y in 0..grid.h {
            queue!(out, MoveTo(0, y as u16))?;
            for c in grid.row(y) {
                let color = (c.ch != ' ').then(|| color_of(c.rgba));
                if color != last {
                    match color {
                        Some(col) => queue!(out, SetForegroundColor(col))?,
                        None => queue!(out, ResetColor)?,
                    }
                    last = color;
                }
                queue!(out, Print(c.ch))?;
            }
        }
        queue!(out, ResetColor)?;
    } else {
        queue!(out, Clear(ClearType::All))?;
    }
    let status = state.status_line(now);
    queue!(out, MoveTo(0, grid_h as u16), Clear(ClearType::CurrentLine), Print(clip(&status, tw)))?;
    queue!(out, MoveTo(0, grid_h as u16 + 1), Clear(ClearType::CurrentLine), Print(clip(help, tw)))?;
    let shown = state.notice.clone();
    for i in 0..events_h {
        let line = if i == 0 {
            shown.as_ref().map(|n| format!("! {n}")).unwrap_or_else(|| "events:".to_string())
        } else {
            state
                .recent_events
                .iter()
                .rev()
                .nth(i - 1)
                .map(|e| format!("  tick {} {} [{}]", e.tick, e.name, e.state_name()))
                .unwrap_or_default()
        };
        queue!(out, MoveTo(0, (grid_h + 2 + i) as u16), Clear(ClearType::CurrentLine), Print(clip(&line, tw)))?;
    }
    out.flush()
}

/// Runs the viewer until `q`. The caller has connected `src` (schema known).
pub fn run(src: &mut dyn Source, opts: &UiOpts) -> Result<(), String> {
    let schema = ViewSchema::parse(src.schema_text())?;
    let mut state = ViewState::new(schema.clone(), Instant::now());
    // A network client plays the slot it joined (the server chose it); a local host the one asked for.
    let mut player = opts.player;
    if let Some(net) = src.net_status() {
        player = net.slot as u8;
        state.set_net(net, Instant::now());
    }
    if opts.autoplay {
        src.control(Control::Play)?;
    }
    terminal::enable_raw_mode().map_err(|e| format!("cannot use the terminal (is stdout a TTY?): {e}"))?;
    let _restore = Restore;
    execute!(std::io::stdout(), EnterAlternateScreen, Hide, Clear(ClearType::All)).map_err(|e| e.to_string())?;
    let mut out = BufWriter::new(std::io::stdout());
    let help = format!(
        "{} player {} | arrows/a/d move, z/c spin, f fire | space/p play-pause, s step, r refit, q quit",
        src.describe(),
        player
    );
    let frame_dt = Duration::from_micros(1_000_000 / u64::from(opts.fps.clamp(1, 60)));
    let (mut held, mut last_sent) = (Held::default(), None::<Controls>);
    let mut last_status = Instant::now();
    loop {
        let deadline = Instant::now() + frame_dt;
        loop {
            let now = Instant::now();
            while event::poll(Duration::ZERO).map_err(|e| e.to_string())? {
                let Event::Key(KeyEvent { code, modifiers, kind, .. }) = event::read().map_err(|e| e.to_string())? else { continue };
                if kind == KeyEventKind::Release {
                    continue;
                }
                match code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                    KeyCode::Char(' ' | 'p') => {
                        let paused = state.frame.as_ref().is_some_and(|f| f.has(FLAG_PAUSED));
                        if let Err(e) = src.control(if paused { Control::Play } else { Control::Pause }) {
                            state.notice = Some(e);
                        }
                    }
                    KeyCode::Char('s') => {
                        if let Err(e) = src.control(Control::Step(1)) {
                            state.notice = Some(e);
                        }
                    }
                    KeyCode::Char('r') => state.camera = state.frame.as_ref().map(Camera::fit),
                    other => {
                        held.press(other, now);
                    }
                }
            }
            let controls = held.controls(now);
            if last_sent != Some(controls) {
                match src.set_input(player, &encode(&schema, &controls)) {
                    Ok(()) => last_sent = Some(controls),
                    // A dropped connection must not close the viewer: say so and keep showing the game.
                    Err(e) if src.net_status().is_some() => state.notice = Some(e),
                    Err(e) => return Err(e),
                }
            }
            if last_status.elapsed() >= Duration::from_millis(100) {
                last_status = Instant::now();
                if let Some(net) = src.net_status() {
                    state.set_net(net, now);
                }
            }
            let left = deadline.saturating_duration_since(now);
            if left.is_zero() {
                break;
            }
            match src.recv(left.min(Duration::from_millis(4))) {
                Ok(Some(msg)) => state.ingest(&msg, Instant::now()),
                Ok(None) => {}
                // After a join failure or a lost connection a client keeps its last picture and says why.
                Err(e) if state.net.is_some() => {
                    state.notice = Some(e);
                    std::thread::sleep(Duration::from_millis(4));
                }
                Err(e) => return Err(e),
            }
        }
        draw(&mut out, &mut state, &help, Instant::now()).map_err(|e| e.to_string())?;
    }
}
