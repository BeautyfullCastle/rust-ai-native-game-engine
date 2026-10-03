//! The egui application: menu and toolbar, hierarchy, inspector, viewport,
//! timeline and history panels (layout of design doc 7.3, simplified).
//!
//! All state changes go through [`Editor`]; this file only draws and
//! translates input into `Editor` calls.

use std::io::{self, Write};
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Instant;

use egui::{Color32, Key, KeyboardShortcut, Modifiers, PointerButton, Pos2, Rect, RichText, Sense, Ui};
use orr_fp::FPVec2;
use orr_reflect::Value;
use orr_remote::{CaptureError, CaptureRequest, CapturedImage, ViewState};

use crate::editor::{fp_of_f64, Editor, Message, Mode, Owner};
use crate::inspector::{show_component, InspEvent};
use crate::model::{EntityRow, Target};
use crate::viewport::{self, GpuViewport};

/// Button labels (tests click them by these texts).
pub const LBL_PLAY: &str = "\u{25B6} Play";
/// See [`LBL_PLAY`].
pub const LBL_PAUSE: &str = "\u{23F8} Pause";
/// See [`LBL_PLAY`].
pub const LBL_STEP: &str = "\u{23ED} Step";
/// See [`LBL_PLAY`].
pub const LBL_STOP: &str = "\u{25A0} Stop";

/// The button that starts a stopped local host again (tests click it by this text).
pub const LBL_RESTART: &str = "Restart simulation host";
/// The button that reconnects to a remote host.
pub const LBL_RECONNECT: &str = "Reconnect";


/// Which tab the bottom panel shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BottomTab {
    /// Play controls and the scrub bar.
    #[default]
    Timeline,
    /// The undo history.
    History,
    /// What AI agents do through ERP: a read-only activity feed.
    Agent,
}

/// A path prompt (no native file dialog in the MVP).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogKind {
    /// Open a scene file.
    Open,
    /// Save under a new name.
    SaveAs,
}

/// An open path prompt.
#[derive(Clone, Debug)]
pub struct Dialog {
    /// What the path is for.
    pub kind: DialogKind,
    /// The text of the path field.
    pub text: String,
}

/// An entity an agent just edited: the viewport outlines it for [`viewport::PULSE_SECONDS`].
#[derive(Clone, Debug, PartialEq)]
pub struct Pulse {
    /// The entity (GUID, or handle in play mode).
    pub target: Target,
    /// egui time (seconds) the pulse started.
    pub started: f64,
}

/// A viewport drag of a body in progress.
#[derive(Clone, Copy, Debug)]
struct BodyDrag {
    /// Body position minus pointer world position at the press.
    offset: [f32; 2],
}

/// Screen state of the window that is not part of the document.
#[derive(Default)]
pub struct UiState {
    /// Bottom panel tab.
    pub bottom_tab: BottomTab,
    /// Hierarchy filter text.
    pub filter: String,
    /// Open path prompt.
    pub dialog: Option<Dialog>,
    /// Where the viewport was drawn last frame, in points.
    pub viewport_rect: Option<Rect>,
    /// Size of the viewport in pixels last frame.
    pub viewport_px: (u32, u32),
    /// Entities an agent just edited, pulsing in the viewport (see [`Pulse`]).
    pub pulses: Vec<Pulse>,
    body_drag: Option<BodyDrag>,
    input_player: u8,
    take_control: bool,
}

/// A request to save the app's own framebuffer as a PNG and quit (see [`crate::cli`]).
pub struct ScreenshotJob {
    /// Output file.
    pub path: PathBuf,
    /// Frames to render before asking for the screenshot.
    pub frames: u64,
    requested_at: Option<u64>,
    settle: bool,
    started_at: Option<f64>,
}

const MAX_CAPTURE_PIXELS: u64 = 1_048_576;
const MAX_CAPTURE_RGBA_BYTES: usize = 4 << 20;

#[derive(Clone, Copy)]
struct FramebufferInfo {
    viewport: egui::ViewportId,
    logical_width: f32,
    logical_height: f32,
    pixels_per_point: f32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
struct CapturePass {
    state: ViewState,
    frame_seq: u64,
    ui_frame: u64,
    framebuffer: FramebufferInfo,
}

struct RemoteCapture {
    request: CaptureRequest,
    pass: Option<CapturePass>,
    screenshot_requested: bool,
    image_received: bool,
    orphaned: bool,
}

struct EncoderWorker {
    serial: u64,
    pass: CapturePass,
    handle: JoinHandle<Result<Vec<u8>, CaptureError>>,
}

struct BoundedPngWriter<'a>(&'a mut Vec<u8>);

impl Write for BoundedPngWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_CAPTURE_RGBA_BYTES {
            return Err(io::Error::other("PNG exceeds screenshot byte limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl ScreenshotJob {
    /// Screenshot after `frames` frames.
    pub fn new(path: PathBuf, frames: u64) -> Self {
        Self { path, frames, requested_at: None, settle: false, started_at: None }
    }

    /// Also wait for refreshed panels, a current paused frame and expired
    /// agent pulses. Readiness is checked without blocking the UI; failure
    /// to settle within ten seconds of UI time fails the screenshot job.
    pub fn with_settle(mut self) -> Self {
        self.settle = true;
        self
    }

    fn ready(&mut self, frames: u64, now: f64, waiting_for: Option<&str>) -> Result<bool, String> {
        let started = *self.started_at.get_or_insert(now);
        if !self.settle {
            return Ok(frames >= self.frames);
        }
        if now - started >= 10.0 {
            return Err(format!("settle deadline exceeded waiting for {}", waiting_for.unwrap_or("minimum frame count")));
        }
        Ok(frames >= self.frames && waiting_for.is_none())
    }
}

/// The editor window.
pub struct EditorApp {
    /// The state machine.
    pub editor: Editor,
    /// Window-only state.
    pub ui: UiState,
    render_state: Option<egui_wgpu::RenderState>,
    gpu: Option<GpuViewport>,
    shot: Option<ScreenshotJob>,
    remote_capture: Option<RemoteCapture>,
    encoder: Option<EncoderWorker>,
    ui_settled_reported: bool,
    frames: u64,
    last_title: String,
}

impl EditorApp {
    /// An app on `editor`. `render_state` is eframe's wgpu state (None without
    /// a GPU: the viewport then shows a notice, everything else works).
    pub fn new(editor: Editor, render_state: Option<egui_wgpu::RenderState>) -> Self {
        Self { editor, ui: UiState::default(), render_state, gpu: None, shot: None, remote_capture: None, encoder: None, ui_settled_reported: false, frames: 0, last_title: String::new() }
    }

    /// Asks for a screenshot of the window after some frames, then quits.
    pub fn with_screenshot(mut self, job: ScreenshotJob) -> Self {
        self.shot = Some(job);
        self
    }

    /// Frames drawn so far.
    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    /// True if the viewport draws on the GPU.
    pub fn has_gpu(&self) -> bool {
        self.render_state.is_some()
    }

    /// The viewport's GPU texture target, once it has drawn (readback in tests).
    pub fn viewport_gpu(&self) -> Option<&GpuViewport> {
        self.gpu.as_ref()
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let started = Instant::now();
        let ctx = ui.ctx().clone();
        self.frames += 1;

        // Disarm before ingesting a late claim/update; focus returning is not intent.
        let input_focus = ctx.memory(|m| m.focused()) == Some(egui::Id::new("arena_keyboard_viewport"));
        let outside_click = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !self.ui.viewport_rect.is_some_and(|r| r.contains(p))));
        if !input_focus || outside_click || self.ui.dialog.is_some() || ctx.input(|i| !i.focused || i.key_pressed(Key::Escape)) {
            self.editor.release_control();
        }
        // The host simulates and edits on its own; this takes in what it says (never waits for it).
        self.editor.pump();
        self.poll_remote_capture_request();
        self.start_pulses(ctx.input(|i| i.time));
        if self.editor.agent_mut().take_tab_request() {
            self.ui.bottom_tab = BottomTab::Agent;
        }
        // Notifications and frames arrive any time: look again soon, and at once while playing.
        ctx.request_repaint_after(std::time::Duration::from_millis(40));
        if self.editor.is_playing_mode() && self.editor.timeline().is_some_and(|t| t.playing) {
            ctx.request_repaint();
        }
        if ctx.input(|i| i.key_pressed(Key::Escape) || !i.focused) {
            self.editor.cancel_edit();
            // Focus loss may hide the physical release. Forget egui's drag
            // ownership so later moves cannot revive it; a fresh press starts
            // a fresh gesture even when the old pointer-up never arrived.
            ctx.stop_dragging();
            self.editor.end_edit();
            self.ui.body_drag = None;
        }
        self.shortcuts(&ctx);

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        if self.editor.down().is_some() {
            egui::Panel::top("down").show(ui, |ui| self.down_banner(ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        // The Agent tab needs more room than the timeline; each has its own remembered height.
        let (bottom_id, bottom_h) = if self.ui.bottom_tab == BottomTab::Agent { ("bottom_agent", 470.0) } else { ("bottom", 120.0) };
        egui::Panel::bottom(bottom_id).resizable(true).default_size(bottom_h).show(ui, |ui| self.bottom(ui));
        egui::Panel::left("hierarchy").resizable(true).default_size(230.0).show(ui, |ui| self.hierarchy(ui));
        egui::Panel::right("inspector").resizable(true).default_size(340.0).show(ui, |ui| self.inspector(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui));

        // A scrub widget can disappear after selection/navigation changes.
        // Release still closes its gesture even if that widget saw no End.
        if !ctx.input(|i| i.pointer.button_down(PointerButton::Primary)) {
            self.editor.end_edit();
            self.ui.body_drag = None;
        }
        self.dialogs(&ctx);
        if !self.ui.pulses.is_empty() {
            // The pulses animate until they expire.
            ctx.request_repaint();
        }

        let title = self.editor.title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        self.process_remote_capture(&ctx);
        self.report_ui_settled(&ctx);
        self.screenshot(&ctx);
        self.editor.record_ui_frame(started.elapsed());
    }
}

impl EditorApp {
    /// Turns the entities agents just touched into pulses (and drops the expired ones).
    fn start_pulses(&mut self, now: f64) {
        for name in self.editor.feed_mut().take_touched() {
            if let Some(target) = Target::parse(&name) {
                self.ui.pulses.retain(|p| p.target != target);
                self.ui.pulses.push(Pulse { target, started: now });
            }
        }
        self.ui.pulses.retain(|p| now - p.started < viewport::PULSE_SECONDS);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.editor.input_phase() != crate::editor::input::Phase::Off || ctx.egui_wants_keyboard_input() || !self.editor.can_mutate() || self.editor.previewing().is_some() {
            return;
        }
        let redo_a = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        let redo_b = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        let (do_redo, do_undo, do_save) = ctx.input_mut(|i| {
            let redo = i.consume_shortcut(&redo_a) | i.consume_shortcut(&redo_b);
            let undo = !redo && i.consume_shortcut(&undo);
            (redo, undo, i.consume_shortcut(&save))
        });
        if do_redo {
            self.editor.redo();
        }
        if do_undo {
            self.editor.undo();
        }
        if do_save {
            self.save();
        }
    }

    fn save(&mut self) {
        if self.editor.sim().scene_path.is_some() {
            self.editor.save();
        } else {
            self.ui.dialog = Some(Dialog { kind: DialogKind::SaveAs, text: String::new() });
        }
    }

    // ---- top bar ----

    fn top_bar(&mut self, ui: &mut Ui) {
        let mutable = self.editor.can_mutate() && self.editor.previewing().is_none();
        let live = self.editor.down().is_none();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if !(mutable) { ui.disable(); }
                if ui.button("Open scene\u{2026}").clicked() {
                    let text = self.editor.path().map(|p| p.display().to_string()).unwrap_or_default();
                    self.ui.dialog = Some(Dialog { kind: DialogKind::Open, text });
                    ui.close();
                }
                if ui.add(egui::Button::new("Save").shortcut_text("Ctrl+S")).clicked() {
                    self.save();
                    ui.close();
                }
                if ui.button("Save As\u{2026}").clicked() {
                    let text = self.editor.path().map(|p| p.display().to_string()).unwrap_or_default();
                    self.ui.dialog = Some(Dialog { kind: DialogKind::SaveAs, text });
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                let editing = mutable && self.editor.mode() == Mode::Edit;
                if ui.add_enabled(editing && self.editor.history().can_undo, egui::Button::new("Undo").shortcut_text("Ctrl+Z")).clicked() {
                    self.editor.undo();
                    ui.close();
                }
                if ui.add_enabled(editing && self.editor.history().can_redo, egui::Button::new("Redo").shortcut_text("Ctrl+Y")).clicked() {
                    self.editor.redo();
                    ui.close();
                }
            });
            ui.separator();
            let playing = self.editor.timeline().is_some_and(|t| t.playing);
            if ui.add_enabled(live && !playing, egui::Button::new(LBL_PLAY)).clicked() {
                self.editor.play();
            }
            if ui.add_enabled(live && playing, egui::Button::new(LBL_PAUSE)).clicked() {
                self.editor.pause();
            }
            if ui.add_enabled(live && !playing, egui::Button::new(LBL_STEP)).clicked() {
                self.editor.step(1);
            }
            if ui.add_enabled(live && self.editor.is_playing_mode(), egui::Button::new(LBL_STOP)).clicked() {
                self.editor.stop();
            }
            ui.separator();
            match self.editor.mode() {
                Mode::Edit => ui.label(RichText::new("EDIT").strong()),
                Mode::Play => ui.label(RichText::new("PLAY").strong().color(Color32::from_rgb(255, 150, 60))),
            };
            ui.weak(self.editor.game().name());
            if self.editor.is_viewer() { ui.label("REPLAY VIEWER · read-only"); }
            if self.editor.is_dirty() {
                ui.label(RichText::new("\u{25CF} unsaved").color(Color32::from_rgb(240, 200, 80)));
            }
        });
        if self.editor.game() == crate::game::EditorGame::Arena {
            ui.horizontal_wrapped(|ui| {
                use crate::editor::input::Phase;
                let phase = self.editor.input_phase();
                ui.label("Keyboard player slot");
                ui.add_enabled(phase == Phase::Off, egui::DragValue::new(&mut self.ui.input_player).range(0..=self.editor.sim().player_count.saturating_sub(1)));
                if ui.add_enabled(phase == Phase::Off && self.editor.can_take_control(), egui::Button::new("Take control")).on_hover_text("Replaces this slot's current held input. Does not cancel commands or take over the simulation.").clicked() {
                    self.ui.take_control = true;
                }
                if ui.add_enabled(phase != Phase::Off, egui::Button::new("Release control")).clicked() { self.editor.release_control(); }
                if self.editor.input_cleanup_pending() && ui.button("Reconnect control").clicked() { self.editor.restart(); }
                ui.label(match phase { Phase::Off => self.editor.input_hint(), Phase::Claiming => "Claiming…", Phase::Active => "Active · WASD/arrows · hold Space to fire · Escape releases", Phase::Releasing => "Releasing…" });
            });
        }
    }

    /// The banner of a host that is gone: why, and the way back.
    fn down_banner(&mut self, ui: &mut Ui) {
        let Some(down) = self.editor.down().cloned() else { return };
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(ui.visuals().error_fg_color, RichText::new(&down.reason).strong());
            let label = if down.local { LBL_RESTART } else { LBL_RECONNECT };
            if ui.button(label).clicked() {
                self.editor.restart();
            }
            ui.weak(if down.local {
                "Restart opens the scene file again in a new simulation host: edits that were not saved are lost."
            } else {
                "Reconnect attaches to the host again and shows its scene (edits the host kept are still there)."
            });
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            match self.editor.status() {
                Some(Message { text, error: true }) => ui.colored_label(ui.visuals().error_fg_color, text),
                Some(Message { text, error: false }) => ui.label(text),
                None => ui.weak("ready"),
            };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let gpu = self.gpu.as_ref().map_or_else(|| "no GPU".to_string(), |g| g.gpu().adapter_name());
                ui.weak(gpu);
                ui.weak(format!("checksum {:#018x}", self.editor.checksum()));
                if let Some((url, clients)) = self.editor.erp_status() {
                    ui.weak(format!("ERP {url} ({clients} connected)"));
                }
                if let Some(action) = self.editor.feed().last_action() {
                    let short: String = if action.chars().count() > 70 { action.chars().take(69).chain(std::iter::once('\u{2026}')).collect() } else { action.to_string() };
                    ui.label(RichText::new(short).color(Color32::from_rgb(120, 200, 255)));
                }
            });
        });
    }

    // ---- hierarchy ----

    fn hierarchy(&mut self, ui: &mut Ui) {
        ui.heading("Hierarchy");
        ui.horizontal(|ui| {
            if !(self.editor.can_mutate() && self.editor.previewing().is_none()) { ui.disable(); }
            if self.editor.game() == crate::game::EditorGame::PhysGame && ui.button("+ Body").on_hover_text("Spawn a dynamic circle at the view center").clicked() {
                let c = self.editor.camera.center;
                self.editor.spawn_body(c);
            }
            if ui.add_enabled(self.editor.selection().is_some(), egui::Button::new("Delete")).clicked() {
                self.editor.delete_selected();
            }
        });
        ui.add(egui::TextEdit::singleline(&mut self.ui.filter).hint_text("filter").desired_width(f32::INFINITY));
        ui.separator();
        let filter = self.ui.filter.to_lowercase();
        let rows: Vec<(String, Target, String)> = self
            .editor
            .rows()
            .iter()
            .filter_map(|row| {
                let label = row.label();
                if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                    return None;
                }
                Some((label, row.target(), row.components.join(", ")))
            })
            .collect();
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
        let mut clicked: Option<Target> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, rows.len(), |ui, range| {
            for (label, target, tip) in &rows[range] {
                let selected = self.editor.selection() == Some(target);
                if ui.selectable_label(selected, label).on_hover_text(tip).clicked() {
                    clicked = Some(target.clone());
                }
            }
        });
        if let Some(t) = clicked {
            self.editor.select(Some(t));
        }
    }

    // ---- inspector ----

    fn inspector(&mut self, ui: &mut Ui) {
        ui.heading("Inspector");
        if self.editor.previewing().is_some() {
            ui.weak("Live document inspector · read-only during proposal preview");
        } else if self.editor.is_viewer() {
            ui.weak("Replay Viewer · read-only");
        }
        let mut events: Vec<(Owner, InspEvent)> = Vec::new();
        let mut add: Option<String> = None;
        let mut remove: Option<String> = None;
        let selection = self.editor.selection().cloned();
        let inspect = self.editor.inspect().cloned();
        let editor = &self.editor;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if !(editor.can_mutate() && editor.previewing().is_none()) { ui.disable(); }
            let types = editor.types();
            match (&selection, &inspect) {
                (Some(_), Some(ins)) => {
                    let row = ins.row.as_ref();
                    let key = ins.target.param();
                    ui.label(RichText::new(row.map_or_else(|| key.clone(), EntityRow::label)).strong());
                    ui.weak(&key);
                    ui.separator();
                    for (name, value) in &ins.components {
                        let Some(ty) = types.get(name) else { continue };
                        let base = egui::Id::new(("inspector", &key, name));
                        egui::CollapsingHeader::new(RichText::new(name).strong()).id_salt(base).default_open(true).show(ui, |ui| {
                            let mut evs = Vec::new();
                            show_component(ui, base, ty.desc(), value, &mut evs);
                            events.extend(evs.into_iter().map(|e| (Owner::Component(name.clone()), e)));
                            if ui.small_button("Remove component").clicked() {
                                remove = Some(name.clone());
                            }
                        });
                    }
                    let have: Vec<&str> = row.map(|r| r.components.iter().map(String::as_str).collect()).unwrap_or_default();
                    let addable: Vec<&str> = types.components().map(|t| t.name()).filter(|n| !have.contains(n) && !ins.components.iter().any(|(c, _)| c == n)).collect();
                    ui.add_enabled_ui(!addable.is_empty(), |ui| {
                        egui::ComboBox::from_id_salt("add_component").selected_text("+ Add component").show_ui(ui, |ui| {
                            for n in addable {
                                if ui.selectable_label(false, n).clicked() {
                                    add = Some(n.to_string());
                                }
                            }
                        });
                    });
                }
                (Some(t), None) => {
                    if editor.row_of(t).is_some() || editor.rows().is_empty() {
                        ui.weak("loading\u{2026}");
                    } else {
                        ui.weak("selection no longer exists");
                    }
                }
                (None, _) => {
                    ui.weak("Select an entity in the hierarchy or click an entity in the viewport.");
                }
            }
            ui.separator();
            ui.label(RichText::new("Singletons").strong());
            for (name, value) in editor.singletons() {
                let Some(ty) = types.get(name) else { continue };
                let base = egui::Id::new(("singleton", name));
                egui::CollapsingHeader::new(name).id_salt(base).default_open(editor.game() == crate::game::EditorGame::Arena && name == "Score").show(ui, |ui| {
                    let mut evs = Vec::new();
                    show_component(ui, base, ty.desc(), value, &mut evs);
                    events.extend(evs.into_iter().map(|e| (Owner::Singleton(name.clone()), e)));
                });
            }
        });
        for (owner, ev) in events {
            match ev {
                InspEvent::Begin(label) => self.editor.begin_edit(&label),
                InspEvent::Set { path, value } => {
                    self.editor.set_field(&owner, &path, value);
                }
                InspEvent::End => self.editor.end_edit(),
                InspEvent::Error(m) => self.editor.error(m),
            }
        }
        if let Some(n) = add {
            self.editor.add_component(&n);
        }
        if let Some(n) = remove {
            self.editor.remove_component(&n);
        }
    }

    // ---- viewport ----

    fn viewport(&mut self, ui: &mut Ui) {
        let (rect, resp) = if self.editor.game() == crate::game::EditorGame::Arena {
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
            (rect, ui.interact(rect, egui::Id::new("arena_keyboard_viewport"), Sense::click_and_drag()))
        } else {
            ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag())
        };
        if self.ui.take_control {
            self.ui.take_control = false;
            if self.ui.dialog.is_none() && ui.input(|i| i.focused) {
                resp.request_focus();
                self.editor.take_control(self.ui.input_player, true);
            }
        }
        let focused = resp.has_focus() && self.ui.dialog.is_none() && ui.input(|i| i.focused && !i.key_pressed(Key::Escape));
        let keys = ui.input(|i| crate::editor::input::Keys {
            x: i8::from(i.key_down(Key::D) || i.key_down(Key::ArrowRight)) - i8::from(i.key_down(Key::A) || i.key_down(Key::ArrowLeft)),
            y: i8::from(i.key_down(Key::W) || i.key_down(Key::ArrowUp)) - i8::from(i.key_down(Key::S) || i.key_down(Key::ArrowDown)),
            fire: i.key_down(Key::Space),
        });
        self.editor.arena_keys(focused, keys);
        if focused && self.editor.input_phase() == crate::editor::input::Phase::Active {
            ui.input_mut(|i| i.events.retain(|event| !matches!(event, egui::Event::Key { key: Key::W | Key::A | Key::S | Key::D | Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight | Key::Space, .. })));
        }
        let ppp = ui.ctx().pixels_per_point();
        let px = (((rect.width() * ppp).round() as u32).clamp(1, 8192), ((rect.height() * ppp).round() as u32).clamp(1, 8192));
        self.ui.viewport_rect = Some(rect);
        self.ui.viewport_px = px;
        let to_px = |p: Pos2| [(p.x - rect.min.x) * ppp, (p.y - rect.min.y) * ppp];

        // Camera: pan with middle or right drag, zoom with the wheel.
        if resp.dragged_by(PointerButton::Middle) || resp.dragged_by(PointerButton::Secondary) {
            let d = resp.drag_delta();
            self.editor.camera.pan_pixels([d.x * ppp, d.y * ppp], px);
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if let (true, Some(at)) = (scroll != 0.0, resp.hover_pos()) {
                self.editor.camera.zoom_at((scroll * 0.0015).exp(), to_px(at), px);
            }
        }

        // While a proposal is previewed the viewport is read-only (the frame on screen is not the document's).
        let previewing = self.editor.previewing().map(str::to_string);
        let live = self.editor.down().is_none();
        // Select and move bodies with the primary button.
        if self.editor.can_mutate() && previewing.is_none() && resp.drag_started_by(PointerButton::Primary) {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                let world = self.editor.camera.screen_to_world(to_px(origin), px);
                if let Some(target) = self.editor.pick(world) {
                    let pos = self.editor.body_pos(&target);
                    self.editor.select(Some(target));
                    if let Some(pos) = pos.filter(|_| self.editor.movement_owner().is_some()) {
                        self.ui.body_drag = Some(BodyDrag { offset: [pos[0] - world[0], pos[1] - world[1]] });
                        self.editor.begin_edit("move entity");
                    }
                }
            }
        }
        if let (Some(drag), true) = (self.ui.body_drag, resp.dragged_by(PointerButton::Primary)) {
            if let Some(at) = resp.interact_pointer_pos() {
                let w = self.editor.camera.screen_to_world(to_px(at), px);
                let p = [w[0] + drag.offset[0], w[1] + drag.offset[1]];
                if let (Some(x), Some(y)) = (fp_of_f64(f64::from(p[0])), fp_of_f64(f64::from(p[1]))) {
                    if let Some(owner) = self.editor.movement_owner() {
                        self.editor.set_field(&owner, "pos", Value::Vec2(FPVec2::new(x, y)));
                    }
                }
            }
        }
        if resp.drag_stopped() && self.ui.body_drag.take().is_some() {
            self.editor.end_edit();
        }
        if live && previewing.is_none() && resp.clicked_by(PointerButton::Primary) {
            if let Some(at) = resp.interact_pointer_pos() {
                let world = self.editor.camera.screen_to_world(to_px(at), px);
                let hit = self.editor.pick(world);
                self.editor.select(hit);
            }
        }

        // Draw.
        let now = ui.ctx().input(|i| i.time);
        let pulses: Vec<(Target, f32)> = self.ui.pulses.iter().map(|p| (p.target.clone(), (1.0 - (now - p.started) / viewport::PULSE_SECONDS).clamp(0.0, 1.0) as f32)).collect();
        let list = self.editor.viewport_list(px, &pulses);
        match &self.render_state {
            Some(rs) => {
                let gpu = self.gpu.get_or_insert_with(|| GpuViewport::new(rs, px));
                let tex = gpu.render(px, &list, &self.editor.camera);
                ui.painter().image(tex, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(10, 10, 16));
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "viewport needs a GPU (wgpu adapter)", egui::FontId::proportional(14.0), Color32::GRAY);
            }
        }
        if !live {
            ui.painter().rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 120));
        }
        let label = match self.editor.timeline() {
            Some(t) => format!("{}  tick {}", if self.editor.is_viewer() { "REPLAY VIEWER" } else { "PLAY" }, t.tick),
            None => "EDIT".to_string(),
        };
        ui.painter().text(rect.min + egui::vec2(8.0, 6.0), egui::Align2::LEFT_TOP, label, egui::FontId::monospace(13.0), Color32::from_gray(200));
        if let Some(id) = previewing {
            let font = egui::FontId::monospace(15.0);
            let text = format!("PREVIEW {id} · read-only");
            let galley = ui.painter().layout_no_wrap(text, font, Color32::BLACK);
            let box_rect = Rect::from_min_size(rect.min + egui::vec2(8.0, 26.0), galley.size() + egui::vec2(14.0, 6.0));
            ui.painter().rect_filled(box_rect, 3.0, Color32::from_rgb(90, 200, 240));
            ui.painter().galley(box_rect.min + egui::vec2(7.0, 3.0), galley, Color32::BLACK);
            let hint = "cyan = changed, green = added, ghost = removed / old place";
            let hint_galley = ui.painter().layout_no_wrap(hint.to_string(), egui::FontId::monospace(11.0), Color32::from_gray(210));
            let hint_rect = Rect::from_min_size(box_rect.left_bottom() + egui::vec2(0.0, 4.0), hint_galley.size() + egui::vec2(8.0, 4.0));
            ui.painter().rect_filled(hint_rect, 3.0, Color32::from_rgba_unmultiplied(10, 10, 16, 200));
            ui.painter().galley(hint_rect.min + egui::vec2(4.0, 2.0), hint_galley, Color32::from_gray(210));
            ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(2.0, Color32::from_rgb(90, 200, 240)), egui::StrokeKind::Inside);
        }
        if self.editor.is_playing_mode() {
            ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(2.0, Color32::from_rgb(255, 150, 60)), egui::StrokeKind::Inside);
        }
    }

    // ---- bottom panel: timeline and history ----

    fn bottom(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.ui.bottom_tab, BottomTab::Timeline, "Timeline");
            ui.selectable_value(&mut self.ui.bottom_tab, BottomTab::History, "History");
            ui.selectable_value(&mut self.ui.bottom_tab, BottomTab::Agent, "Agent");
            let unseen = self.editor.feed().unseen();
            if unseen > 0 && self.ui.bottom_tab != BottomTab::Agent {
                // Badge: what agents did since the tab was last shown (reads excluded).
                ui.label(RichText::new(format!(" {unseen} ")).strong().color(Color32::BLACK).background_color(Color32::from_rgb(240, 170, 60)))
                    .on_hover_text(format!("{unseen} new agent action(s)"));
            }
        });
        ui.separator();
        match self.ui.bottom_tab {
            BottomTab::Timeline => self.timeline(ui),
            BottomTab::History => self.history(ui),
            BottomTab::Agent => {
                self.agent_tab(ui);
                self.editor.feed_mut().mark_seen();
            }
        }
    }

    fn timeline(&mut self, ui: &mut Ui) {
        let Some(tl) = self.editor.timeline() else {
            ui.weak("Edit mode. Press Play to bake the scene and start a session; Step runs one tick.");
            return;
        };
        ui.horizontal(|ui| {
            if !(self.editor.down().is_none()) { ui.disable(); }
            if ui.button("\u{23EE}").on_hover_text("Rewind to the first tick").clicked() {
                self.editor.seek(tl.first_tick);
            }
            if ui.add_enabled(!tl.playing, egui::Button::new("\u{25B6}")).on_hover_text(if self.editor.is_viewer() { "Play recorded ticks only" } else { "Play by the clock (from a rewound tick this branches)" }).clicked() {
                self.editor.play();
            }
            if ui.add_enabled(tl.playing, egui::Button::new("\u{23F8}")).on_hover_text("Pause").clicked() {
                self.editor.pause();
            }
            if ui.add_enabled(!tl.playing, egui::Button::new("\u{23ED}")).on_hover_text("Step one tick").clicked() {
                self.editor.step(1);
            }
            ui.separator();
            ui.label("speed");
            let mut speed = tl.speed.permille() as f32 / 1000.0;
            if ui.add(egui::Slider::new(&mut speed, 0.25..=4.0).logarithmic(true).suffix("x")).changed() {
                self.editor.set_speed(speed);
            }
            ui.separator();
            let can_branch = !self.editor.is_viewer() && tl.tick < tl.last_tick;
            if ui.add_enabled(can_branch, egui::Button::new("Branch")).on_hover_text("Drop the recorded ticks after the current one").clicked() {
                self.editor.branch();
            }
        });
        ui.horizontal(|ui| {
            let mut tick = tl.tick;
            ui.label("tick");
            let slider = egui::Slider::new(&mut tick, tl.first_tick..=tl.last_tick.max(tl.first_tick)).integer();
            ui.spacing_mut().slider_width = (ui.available_width() - 330.0).max(120.0);
            if ui.add(slider).changed() && tick != tl.tick {
                self.editor.seek(tick);
            }
            ui.monospace(format!("{} / {}", tl.tick, tl.last_tick));
        });
        ui.horizontal(|ui| {
            ui.monospace(format!("checksum {:#018x}", tl.checksum));
            ui.weak(format!("keyframes {}", tl.keyframes.len()));
            ui.weak(format!("branches {}", tl.branches));
            if tl.pending_edits > 0 {
                ui.weak(format!("{} edit(s) pending", tl.pending_edits));
            }
            ui.weak(if tl.playing { "playing" } else { "paused" });
        });
    }

    fn history(&mut self, ui: &mut Ui) {
        let history = self.editor.history().entries.clone();
        if history.is_empty() {
            ui.weak("No edits yet.");
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for h in history.iter().rev() {
                let text = format!("#{}  {}  [{}]{}", h.id, h.label, h.origin, if h.op_count > 1 { format!("  ({} ops)", h.op_count) } else { String::new() });
                if h.undone {
                    ui.label(RichText::new(text).weak().strikethrough());
                } else {
                    ui.label(text);
                }
            }
        });
    }

    // ---- dialogs and screenshot ----

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.ui.dialog.take() else { return };
        let title = match dialog.kind {
            DialogKind::Open => "Open scene",
            DialogKind::SaveAs => "Save scene as",
        };
        let mut keep = true;
        let mut accept = false;
        egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label("File path:");
            let r = ui.add(egui::TextEdit::singleline(&mut dialog.text).desired_width(420.0));
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                accept = true;
            }
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    accept = true;
                }
                if ui.button("Cancel").clicked() {
                    keep = false;
                }
            });
        });
        if accept && !dialog.text.trim().is_empty() {
            let path = PathBuf::from(dialog.text.trim());
            let ok = match dialog.kind {
                DialogKind::Open => self.editor.open_path(&path),
                DialogKind::SaveAs => self.editor.save_as(&path),
            };
            keep = !ok;
        }
        if keep {
            self.ui.dialog = Some(dialog);
        }
    }

    fn poll_remote_capture_request(&mut self) {
        let mut clear_unrequested = false;
        if let Some(capture) = self.remote_capture.as_mut() {
            if !self.editor.screenshot_is_active(capture.request.serial) {
                self.editor.cancel_screenshot_validation(capture.request.serial);
                if capture.screenshot_requested && !capture.image_received {
                    // Keep the old ticket/pass until its matching GPU event is
                    // consumed; this remains true across a backend restart in
                    // the same App because the physical renderer is unchanged.
                    capture.orphaned = true;
                } else if !capture.screenshot_requested {
                    clear_unrequested = true;
                }
            }
        }
        if clear_unrequested {
            self.remote_capture = None;
        }
        if self.remote_capture.is_some() {
            return;
        }
        let Some(request) = self.editor.take_screenshot_request() else { return };
        if self.shot.is_some() {
            self.editor.screenshot_complete(request.serial, Err(CaptureError::Unavailable));
            return;
        }
        match self.editor.begin_screenshot_validation(&request) {
            Ok(()) => {
                self.remote_capture = Some(RemoteCapture { request, pass: None, screenshot_requested: false, image_received: false, orphaned: false });
            }
            Err(error) => self.editor.screenshot_complete(request.serial, Err(error)),
        }
    }

    fn process_remote_capture(&mut self, ctx: &egui::Context) {
        self.reap_capture_encoder(ctx);
        self.receive_capture_event(ctx);
        self.request_capture_frame(ctx);
        if self.remote_capture.as_ref().is_some_and(|capture| !capture.orphaned) || self.encoder.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn request_capture_frame(&mut self, ctx: &egui::Context) {
        let Some(capture) = self.remote_capture.as_ref() else { return };
        if capture.orphaned || capture.pass.is_some() || !self.editor.screenshot_is_active(capture.request.serial) {
            return;
        }
        if self.encoder.is_some() {
            return;
        }
        let serial = capture.request.serial;
        let request = capture.request.clone();
        let state = match self.editor.screenshot_capture_ready(serial) {
            Ok(Some(state)) => state,
            Ok(None) => return,
            Err(error) => {
                self.finish_remote_capture(serial, Err(error));
                return;
            }
        };
        if !self.ui.pulses.is_empty() || self.editor.previewing().is_some() || self.editor.screenshot_gesture_busy() {
            return;
        }
        let Some(frame_seq) = self.editor.capture_snapshot_seq().filter(|seq| *seq > 0) else { return };
        let Some(framebuffer) = framebuffer_info(ctx) else {
            self.finish_remote_capture(serial, Err(CaptureError::Unavailable));
            return;
        };
        let pixels = u64::from(framebuffer.width) * u64::from(framebuffer.height);
        if framebuffer.width > request.options.max_width
            || framebuffer.height > request.options.max_height
            || pixels > MAX_CAPTURE_PIXELS
            || pixels.saturating_mul(4) > MAX_CAPTURE_RGBA_BYTES as u64
        {
            self.finish_remote_capture(serial, Err(CaptureError::Failed));
            return;
        }
        let pass = CapturePass { state, frame_seq, ui_frame: self.frames, framebuffer };
        if self.editor.capture_view_state() != state || !self.editor.capture_snapshot_matches(state) {
            self.finish_remote_capture(serial, Err(CaptureError::Stale));
            return;
        }
        if let Err(error) = self.editor.screenshot_begin_capture(serial) {
            self.finish_remote_capture(serial, Err(error));
            return;
        }
        if let Some(capture) = self.remote_capture.as_mut().filter(|capture| capture.request.serial == serial) {
            capture.pass = Some(pass);
            capture.screenshot_requested = true;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(serial)));
    }

    fn receive_capture_event(&mut self, ctx: &egui::Context) {
        let Some(capture) = self.remote_capture.as_ref() else { return };
        if !capture.screenshot_requested || capture.image_received {
            return;
        }
        let serial = capture.request.serial;
        let Some(pass) = capture.pass else { return };
        let event = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if screenshot_event_matches(event, pass.framebuffer.viewport, serial) {
                    match event {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    }
                } else {
                    None
                }
            })
        });
        let Some(image) = event else { return };
        if let Some(capture) = self.remote_capture.as_mut().filter(|capture| capture.request.serial == serial) {
            capture.image_received = true;
        }
        if !self.editor.screenshot_is_active(serial) {
            self.editor.screenshot_end_capture(serial);
            self.editor.cancel_screenshot_validation(serial);
            self.remote_capture = None;
            return;
        }
        if !self.capture_pass_still_current(ctx, pass) {
            self.editor.screenshot_end_capture(serial);
            self.finish_remote_capture(serial, Err(CaptureError::Stale));
            return;
        }
        let (Ok(width), Ok(height)) = (u32::try_from(image.width()), u32::try_from(image.height())) else {
            self.editor.screenshot_end_capture(serial);
            self.finish_remote_capture(serial, Err(CaptureError::Stale));
            return;
        };
        let pixels = u64::from(width) * u64::from(height);
        if width != pass.framebuffer.width
            || height != pass.framebuffer.height
            || pixels > MAX_CAPTURE_PIXELS
            || pixels.saturating_mul(4) > MAX_CAPTURE_RGBA_BYTES as u64
            || image.pixels.len() as u64 != pixels
        {
            self.editor.screenshot_end_capture(serial);
            self.finish_remote_capture(serial, Err(CaptureError::Stale));
            return;
        }
        if let Err(error) = self.editor.screenshot_begin_encoder(serial) {
            self.editor.screenshot_end_capture(serial);
            self.finish_remote_capture(serial, Err(error));
            return;
        }
        let handle = std::thread::Builder::new()
            .name("orr-screenshot-png".into())
            .spawn(move || encode_png_bounded(&image));
        match handle {
            Ok(handle) => self.encoder = Some(EncoderWorker { serial, pass, handle }),
            Err(_) => {
                self.editor.screenshot_end_encoder(serial);
                self.finish_remote_capture(serial, Err(CaptureError::Failed));
            }
        }
    }

    fn reap_capture_encoder(&mut self, ctx: &egui::Context) {
        if !self.encoder.as_ref().is_some_and(|worker| worker.handle.is_finished()) {
            return;
        }
        let worker = self.encoder.take().expect("finished screenshot worker exists");
        let result = worker.handle.join().unwrap_or(Err(CaptureError::Failed));
        if !self.editor.screenshot_is_active(worker.serial) {
            self.editor.screenshot_end_encoder(worker.serial);
            self.editor.cancel_screenshot_validation(worker.serial);
            if self.remote_capture.as_ref().is_some_and(|capture| capture.request.serial == worker.serial) {
                self.remote_capture = None;
            }
            return;
        }
        if !self.capture_pass_still_current(ctx, worker.pass) {
            self.editor.screenshot_end_encoder(worker.serial);
            self.finish_remote_capture(worker.serial, Err(CaptureError::Stale));
            return;
        }
        match result {
            Ok(png) => {
                let image = CapturedImage {
                    png,
                    width: worker.pass.framebuffer.width,
                    height: worker.pass.framebuffer.height,
                    captured: worker.pass.state,
                    frame_seq: worker.pass.frame_seq,
                    ui_frame: worker.pass.ui_frame,
                };
                self.editor.cancel_screenshot_validation(worker.serial);
                // Publish while the just-finished worker still owns the shared
                // capture permit; release only after its real exit is observed.
                self.editor.screenshot_complete(worker.serial, Ok(image));
                self.editor.screenshot_end_encoder(worker.serial);
                self.remote_capture = None;
            }
            Err(error) => {
                self.editor.screenshot_end_encoder(worker.serial);
                self.finish_remote_capture(worker.serial, Err(error));
            }
        }
    }

    fn capture_pass_still_current(&self, ctx: &egui::Context, pass: CapturePass) -> bool {
        let Some(current) = framebuffer_info(ctx) else { return false };
        same_framebuffer(current, pass.framebuffer)
            && self.editor.capture_view_state() == pass.state
            && self.editor.capture_snapshot_seq() == Some(pass.frame_seq)
            && self.editor.capture_snapshot_matches(pass.state)
            && self.editor.previewing().is_none()
            && !self.editor.screenshot_gesture_busy()
            && self.ui.pulses.is_empty()
            && self.editor.screenshot_waiting_for().is_none()
    }

    fn finish_remote_capture(&mut self, serial: u64, result: Result<CapturedImage, CaptureError>) {
        self.editor.cancel_screenshot_validation(serial);
        self.editor.screenshot_complete(serial, result);
        if self.remote_capture.as_ref().is_some_and(|capture| capture.request.serial == serial) {
            self.remote_capture = None;
        }
    }

    fn report_ui_settled(&mut self, ctx: &egui::Context) {
        if self.ui_settled_reported || !self.editor.has_local_screenshot_owner() || self.render_state.is_none() {
            return;
        }
        if !self.ui.pulses.is_empty() || self.editor.previewing().is_some() || self.editor.screenshot_gesture_busy() {
            return;
        }
        if framebuffer_info(ctx).is_none() || self.editor.screenshot_waiting_for().is_some() {
            return;
        }
        let Some((url, _)) = self.editor.erp_status() else { return };
        eprintln!("Editor UI settled: {url}");
        self.ui_settled_reported = true;
    }

    /// `--screenshot`: after N frames ask the window for a screenshot of its
    /// own framebuffer, write the PNG and quit.
    fn screenshot(&mut self, ctx: &egui::Context) {
        let frames = self.frames;
        let waiting_for = if self.ui.pulses.is_empty() { self.editor.screenshot_waiting_for() } else { Some("agent pulses to expire") };
        let Some(job) = &mut self.shot else { return };
        ctx.request_repaint();
        match job.requested_at {
            None => match job.ready(frames, ctx.input(|i| i.time), waiting_for) {
                Ok(true) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                    job.requested_at = Some(frames);
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!("screenshot failed: {e}");
                    std::process::exit(2);
                }
            }
            Some(at) => {
                let image = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { viewport_id, user_data, image }
                            if *viewport_id == egui::ViewportId::ROOT && user_data.data.is_none() => Some(image.clone()),
                        _ => None,
                    })
                });
                if let Some(image) = image {
                    match write_png(&job.path, &image) {
                        Ok(()) => eprintln!("screenshot written: {} ({}x{})", job.path.display(), image.width(), image.height()),
                        Err(e) => {
                            eprintln!("screenshot failed: {e}");
                            std::process::exit(2);
                        }
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    self.shot = None;
                } else if frames > at + 600 {
                    eprintln!("screenshot failed: the window never answered");
                    std::process::exit(2);
                }
            }
        }
    }
}

fn framebuffer_info(ctx: &egui::Context) -> Option<FramebufferInfo> {
    let (viewport, inner_rect) = ctx.input(|input| (input.raw.viewport_id, input.raw.viewport().inner_rect));
    framebuffer_info_from(viewport, inner_rect?, ctx.pixels_per_point())
}

fn framebuffer_info_from(viewport: egui::ViewportId, rect: Rect, ppp: f32) -> Option<FramebufferInfo> {
    if viewport != egui::ViewportId::ROOT { return None; }
    let physical_width = rect.width() * ppp;
    let physical_height = rect.height() * ppp;
    if !rect.width().is_finite()
        || !rect.height().is_finite()
        || !ppp.is_finite()
        || !physical_width.is_finite()
        || !physical_height.is_finite()
        || rect.width() <= 0.0
        || rect.height() <= 0.0
        || ppp <= 0.0
        || physical_width < 1.0
        || physical_height < 1.0
        || physical_width >= u32::MAX as f32
        || physical_height >= u32::MAX as f32
    {
        return None;
    }
    Some(FramebufferInfo {
        viewport,
        logical_width: rect.width(),
        logical_height: rect.height(),
        pixels_per_point: ppp,
        width: physical_width.round() as u32,
        height: physical_height.round() as u32,
    })
}

fn same_framebuffer(a: FramebufferInfo, b: FramebufferInfo) -> bool {
    a.viewport == b.viewport
        && a.width == b.width
        && a.height == b.height
        && a.logical_width.to_bits() == b.logical_width.to_bits()
        && a.logical_height.to_bits() == b.logical_height.to_bits()
        && a.pixels_per_point.to_bits() == b.pixels_per_point.to_bits()
}

fn screenshot_event_matches(event: &egui::Event, viewport: egui::ViewportId, serial: u64) -> bool {
    matches!(event,
        egui::Event::Screenshot { viewport_id, user_data, .. }
            if *viewport_id == viewport
                && user_data.data.as_ref().and_then(|data| data.downcast_ref::<u64>()).copied() == Some(serial)
    )
}

fn encode_png_bounded(image: &egui::ColorImage) -> Result<Vec<u8>, CaptureError> {
    let width = u32::try_from(image.width()).map_err(|_| CaptureError::Failed)?;
    let height = u32::try_from(image.height()).map_err(|_| CaptureError::Failed)?;
    let pixels = u64::from(width) * u64::from(height);
    let rgba_len = usize::try_from(pixels.checked_mul(4).ok_or(CaptureError::Failed)?)
        .map_err(|_| CaptureError::Failed)?;
    if width == 0 || height == 0 || pixels > MAX_CAPTURE_PIXELS || rgba_len > MAX_CAPTURE_RGBA_BYTES || image.pixels.len() as u64 != pixels {
        return Err(CaptureError::Failed);
    }
    let mut rgba = Vec::with_capacity(rgba_len);
    for pixel in &image.pixels {
        rgba.extend_from_slice(&pixel.to_array());
    }
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(BoundedPngWriter(&mut png), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|_| CaptureError::Failed)?;
    writer.write_image_data(&rgba).map_err(|_| CaptureError::Failed)?;
    writer.finish().map_err(|_| CaptureError::Failed)?;
    Ok(png)
}

/// Writes a `ColorImage` as an RGBA PNG.
pub fn write_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), image.width() as u32, image.height() as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Best);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    writer.write_image_data(&bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod screenshot_tests {
    use super::*;

    #[test]
    fn ordinary_capture_keeps_frame_count_behavior() {
        let mut job = ScreenshotJob::new("unused.png".into(), 30);
        assert!(!job.ready(29, 0.0, Some("agent pulses")).unwrap());
        assert!(job.ready(30, 0.0, Some("agent pulses")).unwrap());
    }

    #[test]
    fn settle_is_condition_based_and_has_a_deadline() {
        let mut job = ScreenshotJob::new("unused.png".into(), 30).with_settle();
        assert!(!job.ready(30, 3.0, Some("agent pulses")).unwrap());
        assert!(!job.ready(3000, 4.0, Some("model refresh")).unwrap());
        assert!(job.ready(3001, 4.1, None).unwrap());
        assert!(job.ready(3002, 13.0, Some("model refresh")).unwrap_err().contains("deadline exceeded waiting for model refresh"));
    }

    #[test]
    fn settled_capture_waits_for_actual_pulse_expiry_on_egui_clock() {
        let mut editor = Editor::open(&crate::editor::default_scene_path()).unwrap();
        editor.select_named("body_05");
        editor.sync();
        assert_eq!(editor.screenshot_waiting_for(), None);
        let target = editor.selection().unwrap().clone();
        let mut app = EditorApp::new(editor, None).with_screenshot(ScreenshotJob::new("unused.png".into(), 30).with_settle());
        app.frames = 30;
        app.ui.pulses.push(Pulse { target: target.clone(), started: 0.0 });
        let ctx = egui::Context::default();
        let draw = |app: &mut EditorApp, now| {
            let mut output = ctx.run_ui(egui::RawInput { time: Some(now), ..Default::default() }, |ui| {
                let ctx = ui.ctx();
                app.start_pulses(ctx.input(|i| i.time));
                app.screenshot(ctx);
            });
            output.textures_delta.clear(); // This clock/command test deliberately has no GPU.
            output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::Screenshot(_))))
        };
        assert!(!draw(&mut app, 0.0));
        assert!(!draw(&mut app, viewport::PULSE_SECONDS - 0.01));
        assert_eq!(app.ui.pulses.len(), 1, "waiting must not remove agent activity");
        // A later edit extends readiness according to its real pulse, not a fixed delay.
        app.ui.pulses.push(Pulse { target, started: 1.0 });
        assert!(!draw(&mut app, viewport::PULSE_SECONDS));
        assert_eq!(app.ui.pulses.len(), 1);
        assert!(draw(&mut app, 1.0 + viewport::PULSE_SECONDS));
        assert!(app.ui.pulses.is_empty());
        assert_eq!(app.shot.as_ref().unwrap().requested_at, Some(30));
    }

    #[test]
    fn remote_screenshot_event_requires_its_ticket_and_root_viewport() {
        let image = std::sync::Arc::new(egui::ColorImage::filled([1, 1], Color32::BLACK));
        let matching = egui::Event::Screenshot {
            viewport_id: egui::ViewportId::ROOT,
            user_data: egui::UserData::new(42_u64),
            image: image.clone(),
        };
        let duplicate = egui::Event::Screenshot {
            viewport_id: egui::ViewportId::ROOT,
            user_data: egui::UserData::new(41_u64),
            image: image.clone(),
        };
        let foreign_viewport = egui::Event::Screenshot {
            viewport_id: egui::ViewportId::from_hash_of("secondary"),
            user_data: egui::UserData::new(42_u64),
            image,
        };
        assert!(screenshot_event_matches(&matching, egui::ViewportId::ROOT, 42));
        assert!(!screenshot_event_matches(&duplicate, egui::ViewportId::ROOT, 42));
        assert!(!screenshot_event_matches(&foreign_viewport, egui::ViewportId::ROOT, 42));
    }

    #[test]
    fn physical_framebuffer_identity_invalidates_resize_and_dpi_changes() {
        let root = egui::ViewportId::ROOT;
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let original = framebuffer_info_from(root, rect, 1.0).unwrap();
        assert!(same_framebuffer(original, framebuffer_info_from(root, rect, 1.0).unwrap()));
        assert!(!same_framebuffer(original, framebuffer_info_from(root, Rect::from_min_size(Pos2::ZERO, egui::vec2(801.0, 600.0)), 1.0).unwrap()));
        assert!(!same_framebuffer(original, framebuffer_info_from(root, rect, 1.25).unwrap()));
        assert!(framebuffer_info_from(egui::ViewportId::from_hash_of("secondary"), rect, 1.0).is_none());
    }

    #[test]
    fn bounded_png_encoder_and_writer_enforce_allocation_limits() {
        let image = egui::ColorImage::filled([2, 2], Color32::from_rgb(10, 20, 30));
        let png = encode_png_bounded(&image).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() <= MAX_CAPTURE_RGBA_BYTES);

        let mut output = vec![0; MAX_CAPTURE_RGBA_BYTES];
        let mut writer = BoundedPngWriter(&mut output);
        assert!(writer.write_all(b"x").is_err());
        assert_eq!(writer.0.len(), MAX_CAPTURE_RGBA_BYTES);

        let oversized = egui::ColorImage::filled([1025, 1024], Color32::BLACK);
        assert_eq!(encode_png_bounded(&oversized), Err(CaptureError::Failed));
    }
}
