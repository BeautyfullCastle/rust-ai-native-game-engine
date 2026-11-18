//! The Forest Valley scene inside the editor: a live view drawn by
//! `orr_scenery` and a panel that edits every setting of its look. Presets
//! are saved to and loaded from a JSON file that `forest_valley --settings`
//! also reads.
//!
//! The same settings can also edit the scene that is open in the editor, when
//! its game keeps `orr_gfx` components (see [`crate::scenery_scene`]). Then the
//! window shows no view of its own: the editor's viewport is the view.
#![allow(clippy::float_arithmetic)]

use crate::editor::Editor;
use crate::scenery_scene;
use crate::viewport::TARGET_FORMAT;
use orr_render::orr_rhi::{Rhi, Wgpu};
use orr_render::OffscreenTarget;
use orr_scenery::settings::{COLORS, GROUPS, SLIDERS, TOGGLES};
use orr_scenery::{viewpoints, Camera, Environment, Motion, Preset, Quality, Renderer, Settings, World};
use std::path::{Path, PathBuf};

const DEFAULT_FILE: &str = "forest_valley.preset.json";
/// Largest side of the view texture, in pixels.
const MAX_VIEW: f32 = 4096.0;
const PANEL_WIDTH: f32 = 340.0;

/// The renderer and its texture on eframe's device.
struct Gpu {
    state: egui_wgpu::RenderState,
    target: OffscreenTarget<Wgpu>,
    renderer: Renderer<Wgpu>,
    id: egui::TextureId,
    generation: u64,
    adapter: String,
}

impl Gpu {
    fn new(state: &egui_wgpu::RenderState, world: &World, quality: Quality, size: (u32, u32)) -> Self {
        let rhi = Wgpu::from_parts(state.instance.clone(), state.adapter.clone(), state.device.clone(), state.queue.clone());
        let target = OffscreenTarget::new(&rhi, size.0, size.1, TARGET_FORMAT);
        let renderer = Renderer::new(rhi.clone(), TARGET_FORMAT, world, quality);
        let id = state.renderer.write().register_native_texture(&state.device, target.sample_view(), egui_wgpu::wgpu::FilterMode::Linear);
        Self { generation: target.generation(), adapter: rhi.adapter_name(), state: state.clone(), target, renderer, id }
    }

    fn draw(&mut self, size: (u32, u32), camera: &Camera, environment: &Environment, settings: &Settings) -> egui::TextureId {
        self.target.resize(size.0, size.1);
        let renderer = &mut self.renderer;
        self.target.render_into(|_, view, size| renderer.render(view, size, camera, environment, settings));
        if self.target.generation() != self.generation {
            self.generation = self.target.generation();
            self.state.renderer.write().update_egui_texture_from_wgpu_texture(
                &self.state.device,
                self.target.sample_view(),
                egui_wgpu::wgpu::FilterMode::Linear,
                self.id,
            );
        }
        self.id
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        self.state.renderer.write().free_texture(&self.id);
    }
}

/// The valley that is on screen and the camera in it.
struct Scene {
    seed: u32,
    world: World,
    views: Vec<Camera>,
    camera: Camera,
    walking: bool,
}

impl Scene {
    fn new(seed: u32) -> Self {
        let world = World::generate(seed);
        let views = viewpoints(&world);
        let camera = views[0];
        Self { seed, walking: camera.height_above(&world) < 3.0, world, views, camera }
    }

    fn go_to(&mut self, view: usize) {
        self.camera = self.views[view];
        // Viewpoints above the ground are flying viewpoints.
        self.walking = self.camera.height_above(&self.world) < 3.0;
    }
}

/// What the settings of the window change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    /// The generated valley that the window draws itself.
    #[default]
    Valley,
    /// The scene that is open in the editor, through its components.
    OpenScene,
}

/// State of the Forest Valley window. The window keeps its preset while closed.
pub struct Panel {
    pub open: bool,
    /// What the controls change. `OpenScene` works only while the open scene
    /// has the components; otherwise the window acts as `Valley`.
    pub target: Target,
    /// What the controls show. `preset.seed` and `preset.quality` take effect
    /// on the next frame by rebuilding the scene or the renderer.
    preset: Preset,
    /// File that Save writes and Load reads.
    file: String,
    /// Result of the last Save, Load or Reset.
    status: String,
    paused: bool,
    time: f32,
    frames_per_second: f32,
    scene: Option<Scene>,
    gpu: Option<Gpu>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            target: Target::Valley,
            preset: Preset::default(),
            file: DEFAULT_FILE.into(),
            status: String::new(),
            paused: false,
            time: 20.0,
            frames_per_second: 0.0,
            scene: None,
            gpu: None,
        }
    }
}

/// Writes the whole file or leaves the old one: the text goes to a temporary
/// file in the same folder, which then replaces the destination.
fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write as _;
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    temporary.write_all(text.as_bytes()).and_then(|()| temporary.as_file().sync_all()).map_err(|e| e.to_string())?;
    temporary.persist(path).map(drop).map_err(|e| format!("{}: {}", path.display(), e.error))
}

impl Panel {
    fn path(&self) -> Result<PathBuf, String> {
        let name = self.file.trim();
        if name.is_empty() {
            return Err("Enter a file name first.".into());
        }
        Ok(PathBuf::from(name))
    }

    /// Writes the preset to [`Panel::file`].
    fn save(&mut self) {
        let result = self.path().and_then(|path| write_whole(&path, &self.preset.to_json()).map(|()| path));
        self.status = match result {
            Ok(path) => format!("Saved {}", path.display()),
            Err(error) => format!("Not saved: {error}"),
        };
    }

    /// Replaces the preset with the one in [`Panel::file`]. A file that cannot
    /// be read leaves the preset as it is.
    fn load(&mut self) {
        let result = self.path().and_then(|path| {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            Preset::from_json(&text).map(|preset| (path, preset))
        });
        self.status = match result {
            Ok((path, preset)) => {
                self.preset = preset;
                format!("Loaded {}", path.display())
            }
            Err(error) => format!("Not loaded: {error}"),
        };
    }

    /// Returns every setting to its default. The valley seed stays.
    fn reset(&mut self) {
        self.preset = Preset { seed: self.preset.seed, ..Preset::default() };
        self.status = "All settings are back to their defaults.".into();
    }

    /// Whether the controls now edit the open scene: chosen, and the scene can hold them.
    pub fn edits_open_scene(&self, editor: &Editor) -> bool {
        self.target == Target::OpenScene && scenery_scene::available(editor)
    }

    pub fn show(&mut self, ctx: &egui::Context, render_state: Option<&egui_wgpu::RenderState>, editor: &mut Editor) {
        if !self.open {
            // A closed window holds no GPU memory. The preset stays.
            self.gpu = None;
            self.scene = None;
            return;
        }
        let on_scene = self.edits_open_scene(editor);
        if on_scene {
            // The editor's viewport is the view. The valley needs no memory.
            self.gpu = None;
            self.scene = None;
        }
        let mut open = true;
        egui::Window::new("Forest Valley scene")
            .id(egui::Id::new("forest-valley-scene"))
            .open(&mut open)
            .default_pos(egui::pos2(60.0, 50.0))
            .default_size(if on_scene { egui::vec2(PANEL_WIDTH + 40.0, 760.0) } else { egui::vec2(1280.0, 760.0) })
            .min_size(egui::vec2(if on_scene { PANEL_WIDTH } else { 640.0 }, 360.0))
            .resizable(true)
            .show(ctx, |ui| {
                if on_scene {
                    egui::ScrollArea::vertical().id_salt("forest-valley-settings").auto_shrink([false, false]).show(ui, |ui| self.controls(ui, editor));
                    return;
                }
                let available = ui.available_size();
                let panel_width = PANEL_WIDTH.min(available.x * 0.5);
                let view = egui::vec2((available.x - panel_width - ui.spacing().item_spacing.x).max(160.0), available.y.max(160.0));
                ui.horizontal_top(|ui| {
                    self.view(ui, view, render_state);
                    ui.allocate_ui_with_layout(egui::vec2(panel_width, view.y), egui::Layout::top_down(egui::Align::Min), |ui| {
                        egui::ScrollArea::vertical().id_salt("forest-valley-settings").auto_shrink([false, false]).show(ui, |ui| self.controls(ui, editor));
                    });
                });
            });
        self.open = open;
        // Wind, water and clouds move all the time.
        ctx.request_repaint();
    }

    /// The live view. Drag to look, W A S D to move while the pointer is over it.
    fn view(&mut self, ui: &mut egui::Ui, size: egui::Vec2, render_state: Option<&egui_wgpu::RenderState>) {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let Some(render_state) = render_state else {
            ui.painter().rect_filled(rect, 4.0, egui::Color32::from_gray(24));
            ui.put(rect, egui::Label::new("A GPU is required to draw the Forest Valley scene. The settings can still be edited, saved and loaded."));
            return;
        };
        // A new seed needs a new valley; a new valley or quality needs a new renderer.
        if self.scene.as_ref().is_none_or(|scene| scene.seed != self.preset.seed) {
            self.scene = Some(Scene::new(self.preset.seed));
            self.gpu = None;
        }
        self.preset.quality = self.preset.quality.sanitized();
        if self.gpu.as_ref().is_some_and(|gpu| gpu.renderer.quality() != self.preset.quality) {
            self.gpu = None;
        }
        let Some(scene) = self.scene.as_mut() else { return };

        let seconds = ui.input(|input| input.stable_dt).min(0.1);
        if !self.paused {
            self.time += seconds;
        }
        if seconds > 0.0 {
            // A running average steadies the number; the first frame sets it.
            let now = 1.0 / seconds;
            self.frames_per_second = if self.frames_per_second > 0.0 { self.frames_per_second + (now - self.frames_per_second) * 0.1 } else { now };
        }
        if response.dragged() {
            let delta = response.drag_delta();
            scene.camera.turn(-delta.x * 0.005, -delta.y * 0.005);
        }
        // Keys move the camera only while the pointer is over the view and no text field has the keyboard.
        let motion = if response.hovered() && !ui.ctx().egui_wants_keyboard_input() {
            ui.input(|input| {
                let axis = |positive: egui::Key, negative: egui::Key| f32::from(u8::from(input.key_down(positive))) - f32::from(u8::from(input.key_down(negative)));
                Motion { ahead: axis(egui::Key::W, egui::Key::S), side: axis(egui::Key::D, egui::Key::A), lift: axis(egui::Key::E, egui::Key::Q), fast: input.modifiers.shift }
            })
        } else {
            Motion::default()
        };
        scene.camera.travel(&scene.world, motion, scene.walking, seconds);

        let scale = ui.ctx().pixels_per_point();
        let pixels = ((rect.width() * scale).clamp(16.0, MAX_VIEW) as u32, (rect.height() * scale).clamp(16.0, MAX_VIEW) as u32);
        let gpu = self.gpu.get_or_insert_with(|| Gpu::new(render_state, &scene.world, self.preset.quality, pixels));
        let environment = Environment::at(&scene.world, &scene.camera, self.time, seconds);
        let texture = gpu.draw(pixels, &scene.camera, &environment, &self.preset.settings);
        ui.painter().image(texture, rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }

    fn controls(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let can_edit_scene = scenery_scene::available(editor);
        if !can_edit_scene {
            // The target can only be the valley now. It is kept for a scene that has the components.
            ui.weak("The open scene has no sun, sky and post process components, so these settings change only the generated valley.");
        } else {
            ui.horizontal_wrapped(|ui| {
                let before = self.target;
                ui.selectable_value(&mut self.target, Target::Valley, "Edit the generated valley").on_hover_text("The window draws its own valley with these settings.");
                ui.selectable_value(&mut self.target, Target::OpenScene, "Edit the open scene")
                    .on_hover_text("Each change sets the matching fields of the scene's sun, sky, water and post process. Undo reverses it. The viewport shows the result.");
                if before != self.target && self.target == Target::OpenScene {
                    // The controls show the values of the scene.
                    self.preset.settings = scenery_scene::read(editor, self.preset.settings);
                    self.status = "The controls show the values of the open scene.".into();
                }
            });
        }
        let on_scene = self.edits_open_scene(editor);
        match (&self.gpu, &self.scene) {
            _ if on_scene => {
                ui.weak("Each change edits the open scene. Undo reverses it. Dimmed controls have no field in the scene.");
            }
            (Some(gpu), Some(scene)) => {
                let stats = gpu.renderer.last_frame_stats();
                ui.weak(format!(
                    "{:.0} frames per second · {:.1} million triangles · {} trees · {}",
                    self.frames_per_second,
                    stats.triangles as f32 / 1.0e6,
                    scene.world.conifers.len() + scene.world.broadleaves.len(),
                    gpu.adapter
                ));
            }
            _ => {
                ui.weak("The scene is not drawn.");
            }
        }
        if on_scene {
            ui.separator();
            if ui.button("Read scene values").on_hover_text("Shows the values that the open scene has now, for example after an undo.").clicked() {
                self.preset.settings = scenery_scene::read(editor, self.preset.settings);
                self.status = "The controls show the values of the open scene.".into();
            }
            if !self.status.is_empty() {
                ui.label(&self.status);
            }
            ui.separator();
            self.setting_controls(ui, Some(editor));
            return;
        }
        ui.weak("Drag in the view to look. With the pointer over it: W A S D to move, Shift to move fast, Q and E to fly down and up.");
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            ui.label("Viewpoint");
            for view in 0..6 {
                if ui.button(format!("{}", view + 1)).on_hover_text("Move the camera to a prepared viewpoint.").clicked() {
                    if let Some(scene) = self.scene.as_mut() {
                        scene.go_to(view);
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            if let Some(scene) = self.scene.as_mut() {
                ui.selectable_value(&mut scene.walking, true, "Walk");
                ui.selectable_value(&mut scene.walking, false, "Fly");
            }
            ui.checkbox(&mut self.paused, "Pause motion").on_hover_text("Stops wind, water and clouds.");
        });
        ui.horizontal(|ui| {
            ui.label("Valley seed");
            ui.add(egui::DragValue::new(&mut self.preset.seed).speed(0.2)).on_hover_text("Each seed gives a different valley. A new valley takes a moment to build.");
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Preset file");
            ui.add(egui::TextEdit::singleline(&mut self.file).desired_width(f32::INFINITY));
        });
        ui.horizontal(|ui| {
            if ui.button("Save preset").on_hover_text("Writes the seed, the quality and all settings to the preset file.").clicked() {
                self.save();
            }
            if ui.button("Load preset").clicked() {
                self.load();
            }
            if ui.button("Reset settings").on_hover_text("Returns every setting to its default. The valley seed stays.").clicked() {
                self.reset();
            }
        });
        if !self.status.is_empty() {
            ui.label(&self.status);
        }

        ui.separator();
        crate::theme::header("Quality").default_open(true).show(ui, |ui| {
            let quality = &mut self.preset.quality;
            egui::ComboBox::from_label("Antialiasing").selected_text(format!("{}x", quality.antialiasing)).show_ui(ui, |ui| {
                for samples in Quality::ANTIALIASING {
                    ui.selectable_value(&mut quality.antialiasing, samples, format!("{samples}x"));
                }
            });
            egui::ComboBox::from_label("Shadow resolution").selected_text(format!("{}", quality.shadow_resolution)).show_ui(ui, |ui| {
                for resolution in Quality::SHADOW_RESOLUTION {
                    ui.selectable_value(&mut quality.shadow_resolution, resolution, format!("{resolution}"));
                }
            });
        });
        self.setting_controls(ui, None);
    }

    /// The groups of toggles, sliders and colors. With an `editor`, every change
    /// is also written to the open scene as one undoable edit: a drag is one edit.
    fn setting_controls(&mut self, ui: &mut egui::Ui, mut editor: Option<&mut Editor>) {
        let on_scene = editor.is_some();
        let settings = &mut self.preset.settings;
        /// Writes the control to the open scene. The edit stays open while the pointer drags.
        fn write(editor: &mut Editor, group: &str, name: &str, settings: &Settings, response: &egui::Response) {
            if response.changed() {
                editor.begin_edit(&format!("scene window: {group} {name}"));
                let changes = scenery_scene::changes(editor, group, name, settings);
                scenery_scene::apply(editor, &changes);
                if !response.dragged() {
                    editor.end_edit();
                }
            } else if response.drag_stopped() {
                editor.end_edit();
            }
        }
        for group in GROUPS {
            crate::theme::header(group).default_open(true).show(ui, |ui| {
                // A control with no field in the scene is dimmed while the scene is edited.
                let enabled = |name: &str| !on_scene || scenery_scene::applies(group, name);
                for toggle in TOGGLES.iter().filter(|toggle| toggle.group == group) {
                    let mut value = (toggle.get)(settings);
                    let response = ui.add_enabled(enabled(toggle.name), egui::Checkbox::new(&mut value, toggle.name)).on_hover_text(toggle.help);
                    if response.changed() {
                        (toggle.set)(settings, value);
                    }
                    if let Some(editor) = editor.as_deref_mut() {
                        write(editor, group, toggle.name, settings, &response);
                    }
                }
                for slider in SLIDERS.iter().filter(|slider| slider.group == group) {
                    let mut value = (slider.get)(settings);
                    let response = ui.add_enabled(enabled(slider.name), egui::Slider::new(&mut value, slider.min..=slider.max).text(slider.name)).on_hover_text(slider.help);
                    if response.changed() {
                        (slider.set)(settings, value);
                    }
                    if let Some(editor) = editor.as_deref_mut() {
                        write(editor, group, slider.name, settings, &response);
                    }
                }
                for color in COLORS.iter().filter(|color| color.group == group) {
                    let mut value = (color.get)(settings);
                    let (mut changed, mut dragging, mut stopped) = (false, false, false);
                    ui.add_enabled_ui(enabled(color.name), |ui| {
                        ui.horizontal(|ui| {
                            // Linear values that may pass 1, so three numbers and not a display color picker.
                            for (channel, label) in value.iter_mut().zip(["R", "G", "B"]) {
                                let response = ui.add(egui::DragValue::new(channel).speed(0.005).range(0.0..=color.max).prefix(format!("{label} ")));
                                changed |= response.changed();
                                dragging |= response.dragged();
                                stopped |= response.drag_stopped();
                            }
                            ui.label(color.name).on_hover_text(color.help);
                        });
                    });
                    if changed {
                        (color.set)(settings, value);
                    }
                    if let Some(editor) = editor.as_deref_mut() {
                        if changed {
                            editor.begin_edit(&format!("scene window: {group} {}", color.name));
                            let changes = scenery_scene::changes(editor, group, color.name, settings);
                            scenery_scene::apply(editor, &changes);
                            if !dragging {
                                editor.end_edit();
                            }
                        } else if stopped {
                            editor.end_edit();
                        }
                    }
                }
            });
        }
    }
}
