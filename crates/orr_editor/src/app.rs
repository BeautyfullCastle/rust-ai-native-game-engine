//! The egui application: menu and toolbar, hierarchy, inspector, viewport,
//! timeline and history panels (layout of design doc 7.3, simplified).
//!
//! All state changes go through [`Editor`]; this file only draws and
//! translates input into `Editor` calls.

use std::path::PathBuf;

use egui::{Color32, Key, KeyboardShortcut, Modifiers, PointerButton, Pos2, Rect, RichText, Sense, Ui};
use orr_edit::Target;
use orr_fp::FPVec2;
use orr_reflect::Value;
use orr_session::PlayMode;

use crate::editor::{fp_of_f64, Editor, Message, Mode, Owner};
use crate::inspector::{show_component, InspEvent};
use crate::viewport::{self, GpuViewport};

/// Button labels (tests click them by these texts).
pub const LBL_PLAY: &str = "\u{25B6} Play";
/// See [`LBL_PLAY`].
pub const LBL_PAUSE: &str = "\u{23F8} Pause";
/// See [`LBL_PLAY`].
pub const LBL_STEP: &str = "\u{23ED} Step";
/// See [`LBL_PLAY`].
pub const LBL_STOP: &str = "\u{25A0} Stop";

const BODY: &str = "orr_physics::Body";

/// Which tab the bottom panel shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BottomTab {
    /// Play controls and the scrub bar.
    #[default]
    Timeline,
    /// The undo history.
    History,
    /// Proposals from AI agents: review, preview, verify, accept.
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
    /// Diffs of the open proposals (see [`crate::agent_ui::cached_diff`]).
    pub diffs: crate::agent_ui::DiffCache,
    body_drag: Option<BodyDrag>,
}

/// A request to save the app's own framebuffer as a PNG and quit (see [`crate::cli`]).
pub struct ScreenshotJob {
    /// Output file.
    pub path: PathBuf,
    /// Frames to render before asking for the screenshot.
    pub frames: u64,
    requested_at: Option<u64>,
}

impl ScreenshotJob {
    /// Screenshot after `frames` frames.
    pub fn new(path: PathBuf, frames: u64) -> Self {
        Self { path, frames, requested_at: None }
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
    frames: u64,
    last_title: String,
}

impl EditorApp {
    /// An app on `editor`. `render_state` is eframe's wgpu state (None without
    /// a GPU: the viewport then shows a notice, everything else works).
    pub fn new(editor: Editor, render_state: Option<egui_wgpu::RenderState>) -> Self {
        Self { editor, ui: UiState::default(), render_state, gpu: None, shot: None, frames: 0, last_title: String::new() }
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
        let ctx = ui.ctx().clone();
        self.frames += 1;

        let dt = f64::from(ctx.input(|i| i.unstable_dt));
        self.editor.poll_erp();
        self.editor.poll_verify();
        if self.editor.agent().running().is_some() {
            // The spinner animates and the result is collected on a later frame.
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        if self.editor.agent_mut().take_tab_request() {
            self.ui.bottom_tab = BottomTab::Agent;
        }
        self.editor.advance(dt);
        if self.editor.erp_status().is_some() {
            // Keep polling ERP requests while the window is idle.
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        self.editor.sanitize_selection();
        if self.editor.is_playing_mode() && self.editor.timeline().is_some_and(|t| t.playing) {
            ctx.request_repaint();
        }
        self.shortcuts(&ctx);

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        // The Agent tab needs more room than the timeline; each has its own remembered height.
        let (bottom_id, bottom_h) = if self.ui.bottom_tab == BottomTab::Agent { ("bottom_agent", 390.0) } else { ("bottom", 120.0) };
        egui::Panel::bottom(bottom_id).resizable(true).default_size(bottom_h).show(ui, |ui| self.bottom(ui));
        egui::Panel::left("hierarchy").resizable(true).default_size(230.0).show(ui, |ui| self.hierarchy(ui));
        egui::Panel::right("inspector").resizable(true).default_size(340.0).show(ui, |ui| self.inspector(ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui));

        self.dialogs(&ctx);

        let title = self.editor.title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        self.screenshot(&ctx);
    }
}

impl EditorApp {
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
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
        if self.editor.path().is_some() {
            self.editor.save();
        } else {
            self.ui.dialog = Some(Dialog { kind: DialogKind::SaveAs, text: String::new() });
        }
    }

    // ---- top bar ----

    fn top_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
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
                let editing = self.editor.mode() == Mode::Edit;
                if ui.add_enabled(editing && self.editor.doc().can_undo(), egui::Button::new("Undo").shortcut_text("Ctrl+Z")).clicked() {
                    self.editor.undo();
                    ui.close();
                }
                if ui.add_enabled(editing && self.editor.doc().can_redo(), egui::Button::new("Redo").shortcut_text("Ctrl+Y")).clicked() {
                    self.editor.redo();
                    ui.close();
                }
            });
            ui.separator();
            let playing = self.editor.timeline().is_some_and(|t| t.playing);
            if ui.add_enabled(!playing, egui::Button::new(LBL_PLAY)).clicked() {
                self.editor.play();
            }
            if ui.add_enabled(playing, egui::Button::new(LBL_PAUSE)).clicked() {
                self.editor.pause();
            }
            if ui.add_enabled(!playing, egui::Button::new(LBL_STEP)).clicked() {
                self.editor.step(1);
            }
            if ui.add_enabled(self.editor.is_playing_mode(), egui::Button::new(LBL_STOP)).clicked() {
                self.editor.stop();
            }
            ui.separator();
            match self.editor.mode() {
                Mode::Edit => ui.label(RichText::new("EDIT").strong()),
                Mode::Play => ui.label(RichText::new("PLAY").strong().color(Color32::from_rgb(255, 150, 60))),
            };
            if self.editor.is_dirty() {
                ui.label(RichText::new("\u{25CF} unsaved").color(Color32::from_rgb(240, 200, 80)));
            }
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
            });
        });
    }

    // ---- hierarchy ----

    fn hierarchy(&mut self, ui: &mut Ui) {
        ui.heading("Hierarchy");
        ui.horizontal(|ui| {
            if ui.button("+ Body").on_hover_text("Spawn a dynamic circle at the view center").clicked() {
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
            .view()
            .entities()
            .into_iter()
            .filter_map(|info| {
                let label = Editor::entity_label(&info);
                if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                    return None;
                }
                let tip = info.components.join(", ");
                let target = info.guid.clone().map_or(Target::Entity(info.entity), Target::Guid);
                Some((label, target, tip))
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
        let mut events: Vec<(Owner, InspEvent)> = Vec::new();
        let mut add: Option<String> = None;
        let mut remove: Option<String> = None;
        let selection = self.editor.selection().cloned();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let view = self.editor.view();
            if let Some(t) = &selection {
                match view.entity(t) {
                    Ok(info) => {
                        let key = match &info.guid {
                            Some(g) => g.to_string(),
                            None => format!("e{}v{}", info.entity.index, info.entity.version),
                        };
                        ui.label(RichText::new(Editor::entity_label(&info)).strong());
                        ui.weak(&key);
                        ui.separator();
                        match view.components(t) {
                            Ok(comps) => {
                                for (name, value) in comps {
                                    let Some(ty) = view.types().get(&name) else { continue };
                                    let base = egui::Id::new(("inspector", &key, &name));
                                    egui::CollapsingHeader::new(RichText::new(&name).strong()).id_salt(base).default_open(true).show(ui, |ui| {
                                        let mut evs = Vec::new();
                                        show_component(ui, base, ty.desc(), &value, &mut evs);
                                        events.extend(evs.into_iter().map(|e| (Owner::Component(name.clone()), e)));
                                        if ui.small_button("Remove component").clicked() {
                                            remove = Some(name.clone());
                                        }
                                    });
                                }
                            }
                            Err(e) => {
                                ui.colored_label(ui.visuals().error_fg_color, e.to_string());
                            }
                        }
                        let addable: Vec<&str> = view.types().components().map(|t| t.name()).filter(|n| !info.components.iter().any(|c| c == n)).collect();
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
                    Err(_) => {
                        ui.weak("selection no longer exists");
                    }
                }
            } else {
                ui.weak("Select an entity in the hierarchy or click a body in the viewport.");
            }
            ui.separator();
            ui.label(RichText::new("Singletons").strong());
            for (name, value) in view.singletons() {
                let Some(ty) = view.types().get(&name) else { continue };
                let base = egui::Id::new(("singleton", &name));
                egui::CollapsingHeader::new(&name).id_salt(base).default_open(false).show(ui, |ui| {
                    let mut evs = Vec::new();
                    show_component(ui, base, ty.desc(), &value, &mut evs);
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
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
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
        let previewing = self.editor.previewing();
        // Select and move bodies with the primary button.
        if previewing.is_none() && resp.drag_started_by(PointerButton::Primary) {
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                let world = self.editor.camera.screen_to_world(to_px(origin), px);
                let hit = viewport::pick(&self.editor.view(), world);
                if let Some(target) = hit {
                    let pos = viewport::body_pos(&self.editor.view(), &target);
                    self.editor.select(Some(target));
                    if let Some(pos) = pos {
                        self.ui.body_drag = Some(BodyDrag { offset: [pos[0] - world[0], pos[1] - world[1]] });
                        self.editor.begin_edit("move body");
                    }
                }
            }
        }
        if let (Some(drag), true) = (self.ui.body_drag, resp.dragged_by(PointerButton::Primary)) {
            if let Some(at) = resp.interact_pointer_pos() {
                let w = self.editor.camera.screen_to_world(to_px(at), px);
                let p = [w[0] + drag.offset[0], w[1] + drag.offset[1]];
                if let (Some(x), Some(y)) = (fp_of_f64(f64::from(p[0])), fp_of_f64(f64::from(p[1]))) {
                    self.editor.set_field(&Owner::Component(BODY.to_string()), "pos", Value::Vec2(FPVec2::new(x, y)));
                }
            }
        }
        if resp.drag_stopped() && self.ui.body_drag.take().is_some() {
            self.editor.end_edit();
        }
        if previewing.is_none() && resp.clicked_by(PointerButton::Primary) {
            if let Some(at) = resp.interact_pointer_pos() {
                let world = self.editor.camera.screen_to_world(to_px(at), px);
                let hit = viewport::pick(&self.editor.view(), world);
                self.editor.select(hit);
            }
        }

        // Draw.
        let list = match previewing.and_then(|id| crate::agent_ui::cached_diff(&self.editor, &mut self.ui.diffs, id)) {
            Some(diff) => {
                viewport::build_preview_list(&self.editor.viewport_view(), &self.editor.doc().view(), &diff.summary, self.editor.selection(), &self.editor.camera, px)
            }
            None => viewport::build_list(&self.editor.view(), self.editor.selection(), &self.editor.camera, px),
        };
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
        let label = match self.editor.timeline() {
            Some(t) => format!("PLAY  tick {}", t.tick),
            None => "EDIT".to_string(),
        };
        ui.painter().text(rect.min + egui::vec2(8.0, 6.0), egui::Align2::LEFT_TOP, label, egui::FontId::monospace(13.0), Color32::from_gray(200));
        if let Some(id) = previewing {
            let font = egui::FontId::monospace(15.0);
            let text = format!("PREVIEW {id}");
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
            let waiting = self.proposal_count();
            if waiting > 0 {
                // Badge: proposals are waiting for a decision.
                ui.label(RichText::new(format!(" {waiting} ")).strong().color(Color32::BLACK).background_color(Color32::from_rgb(240, 170, 60)))
                    .on_hover_text(format!("{waiting} proposal(s) waiting"));
            }
        });
        ui.separator();
        match self.ui.bottom_tab {
            BottomTab::Timeline => self.timeline(ui),
            BottomTab::History => self.history(ui),
            BottomTab::Agent => self.agent_tab(ui),
        }
    }

    fn timeline(&mut self, ui: &mut Ui) {
        let Some(tl) = self.editor.timeline() else {
            ui.weak("Edit mode. Press Play to bake the scene and start a session; Step runs one tick.");
            return;
        };
        ui.horizontal(|ui| {
            if ui.button("\u{23EE}").on_hover_text("Rewind to the first tick").clicked() {
                self.editor.seek(tl.first_tick);
            }
            if ui.add_enabled(!tl.playing, egui::Button::new("\u{25B6}")).on_hover_text("Play by the clock (from a rewound tick this branches)").clicked() {
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
            let can_branch = tl.mode == PlayMode::Viewer || tl.tick < tl.last_tick;
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
        let history = self.editor.doc().history();
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

    /// `--screenshot`: after N frames ask the window for a screenshot of its
    /// own framebuffer, write the PNG and quit.
    fn screenshot(&mut self, ctx: &egui::Context) {
        let frames = self.frames;
        let Some(job) = &mut self.shot else { return };
        ctx.request_repaint();
        match job.requested_at {
            None if frames >= job.frames => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                job.requested_at = Some(frames);
            }
            None => {}
            Some(at) => {
                let image = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
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
