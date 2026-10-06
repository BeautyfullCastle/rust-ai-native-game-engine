//! Explicit view-sidecar workflow and egui atlas preview. Shape picking remains
//! the host game's existing collider picking; sprites do not change colliders.
use crate::{
    editor::Editor,
    game::EditorGame,
    model::Mode,
    sprite_bindings::{self, Asset, Binding, Bindings, Source},
};
use egui::{Color32, Pos2, Rect, TextureHandle, Ui};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

struct Loaded {
    asset: Asset,
    texture: TextureHandle,
}

pub struct SpritePanel {
    pub bindings: Option<Bindings>,
    path: String,
    scene: String,
    project: String,
    package: String,
    document: String,
    source: Source,
    scale: f32,
    loaded: BTreeMap<(String, String), Loaded>,
    error: Option<String>,
    preview_playing: bool,
    preview_started: f64,
}
impl Default for SpritePanel {
    fn default() -> Self {
        Self {
            bindings: None,
            path: String::new(),
            scene: String::new(),
            project: ".".into(),
            package: String::new(),
            document: String::new(),
            source: Source::Region(0),
            scale: 0.05,
            loaded: BTreeMap::new(),
            error: None,
            preview_playing: false,
            preview_started: 0.0,
        }
    }
}
impl SpritePanel {
    fn scene_matches(&self, editor: &Editor) -> bool {
        self.bindings.as_ref().is_some_and(|b| {
            editor
                .sim()
                .scene_path
                .as_ref()
                .is_some_and(|scene| b.matches_scene(scene))
        })
    }
    fn load(&mut self, ctx: &egui::Context, package: &str, document: &str) -> Result<(), String> {
        let key = (package.to_string(), document.to_string());
        if self.loaded.contains_key(&key) {
            return Ok(());
        }
        if self.loaded.len() >= 8 {
            return Err(
                "atlas cache limit reached (8 documents); close and reopen with fewer documents"
                    .into(),
            );
        }
        let bindings = self
            .bindings
            .as_ref()
            .ok_or("open or create a view sidecar first")?;
        let asset = sprite_bindings::load_asset(
            &sprite_bindings::resolve_project(bindings.base(), &bindings.document().project)?,
            package,
            document,
        )?;
        self.cache_asset(ctx, key, asset);
        Ok(())
    }
    fn cache_asset(&mut self, ctx: &egui::Context, key: (String, String), asset: Asset) {
        let atlas = asset.document.atlas();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [atlas.width as usize, atlas.height as usize],
            &asset.rgba,
        );
        let texture = ctx.load_texture(
            format!("sprite:{}:{}", key.0, key.1),
            image,
            egui::TextureOptions::NEAREST,
        );
        self.loaded.insert(key, Loaded { asset, texture });
    }
    fn reload(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.loaded.clear();
        let references: BTreeSet<_> = self
            .bindings
            .as_ref()
            .into_iter()
            .flat_map(|b| b.document().bindings.values())
            .map(|b| (b.package.clone(), b.document.clone()))
            .collect();
        if references.is_empty() {
            return Ok(());
        }
        if references.len() > 8 {
            return Err("sidecar references more than 8 sprite documents; reduce references before reloading".into());
        }
        let bindings = self.bindings.as_ref().ok_or("open a view sidecar first")?;
        let root = sprite_bindings::resolve_project(bindings.base(), &bindings.document().project)?;
        // Verify the project once per reload, then read each distinct asset once.
        let project = sprite_bindings::open_project(&root)?;
        let mut errors = Vec::new();
        for (package, document) in references {
            match sprite_bindings::load_project_asset(&project, &package, &document) {
                Ok(asset) => self.cache_asset(ctx, (package, document), asset),
                Err(e) => errors.push(format!("{package}/{document}: {e}")),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }
    fn report(&mut self, result: Result<(), String>) {
        self.error = result.err();
    }
    pub fn show(&mut self, ui: &mut Ui, editor: &Editor) {
        if editor.game() != EditorGame::Arena {
            return;
        }
        ui.collapsing("Sprite bindings (view only)", |ui| {
            ui.weak("Separate sidecar save and undo. Scene Save does not save bindings. Picking uses existing collider shapes.");
            if self.bindings.is_none() {
                ui.label("Local sidecar path"); ui.text_edit_singleline(&mut self.path);
                ui.label("Scene path relative to sidecar"); ui.text_edit_singleline(&mut self.scene);
                ui.label("Package project relative to sidecar"); ui.text_edit_singleline(&mut self.project);
                ui.weak("Choose local scene/project files. Remote host paths are not local filesystem authority.");
                ui.horizontal(|ui| {
                    if ui.button("Open bindings").clicked() {
                        match Bindings::open(PathBuf::from(&self.path)) {
                            Ok(b) => { self.bindings = Some(b); let result = self.reload(ui.ctx()); self.report(result); }
                            Err(e) => self.error = Some(e),
                        }
                    }
                    if ui.button("Create bindings").clicked() {
                        match Bindings::create(PathBuf::from(&self.path), self.scene.clone(), self.project.clone()) {
                            Ok(b) => { self.bindings = Some(b); self.loaded.clear(); self.error = None; }
                            Err(e) => self.error = Some(e),
                        }
                    }
                });
            } else {
                let b = self.bindings.as_ref().unwrap();
                ui.label(format!("{}{}", b.path.display(), if b.dirty() { " · UNSAVED bindings" } else { " · saved" }));
                ui.label(format!("Scene: {} · Project: {}", b.document().scene, b.document().project));
                let matches = self.scene_matches(editor);
                if !matches { ui.colored_label(Color32::YELLOW, "Scene changed or unavailable. Bindings retained; preview/assignment disabled. Save the old sidecar or explicitly discard/close. Save As does not retarget this sidecar."); }
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Save bindings").clicked() { let result = self.bindings.as_mut().unwrap().save(); self.report(result); }
                    if ui.button("Undo binding").clicked() { self.bindings.as_mut().unwrap().undo(); }
                    if ui.button("Redo binding").clicked() { self.bindings.as_mut().unwrap().redo(); }
                    if ui.button("Reload assets").clicked() { let result = self.reload(ui.ctx()); self.report(result); }
                });
                let dirty = self.bindings.as_ref().unwrap().dirty();
                if ui.button(if dirty { "Discard unsaved bindings and close" } else { "Close bindings" }).clicked() {
                    self.bindings = None; self.loaded.clear(); self.error = None; return;
                }
                ui.separator();
                ui.label("Package name"); ui.text_edit_singleline(&mut self.package);
                ui.label("Sprite document path in package"); ui.text_edit_singleline(&mut self.document);
                if ui.button("Load sprite document").clicked() {
                    let package = self.package.clone(); let document = self.document.clone();
                    let result = self.load(ui.ctx(), &package, &document);
                    if result.is_ok() {
                        if let Some(first) = self.loaded[&(package, document)].asset.document.regions().first() { self.source = Source::Region(first.id); }
                    }
                    self.report(result);
                }
                let key = (self.package.clone(), self.document.clone());
                if let Some(loaded) = self.loaded.get(&key) {
                    egui::ComboBox::from_id_salt("sprite-source").selected_text(format!("{:?}", self.source)).show_ui(ui, |ui| {
                        for region in loaded.asset.document.regions() { ui.selectable_value(&mut self.source, Source::Region(region.id), format!("Region {}", region.id)); }
                        for clip in loaded.asset.document.clips() { ui.selectable_value(&mut self.source, Source::Clip(clip.id().into()), format!("Clip {}", clip.id())); }
                    });
                    ui.add(egui::Slider::new(&mut self.scale, 0.0001..=2.0).logarithmic(true).text("world units/pixel"));
                    ui.horizontal(|ui| {
                        if ui.button("Play sprite preview").clicked() { self.preview_playing = true; self.preview_started = ui.input(|i| i.time); }
                        if ui.button("Stop sprite preview").clicked() { self.preview_playing = false; }
                    });
                    let binding = Binding { package: self.package.clone(), document: self.document.clone(), source: self.source.clone(), units_per_pixel: self.scale };
                    let elapsed = if self.preview_playing { ((ui.input(|i| i.time) - self.preview_started).max(0.0) * 1000.0) as u64 } else { 0 };
                    if self.preview_playing { ui.ctx().request_repaint(); }
                    if let Ok(region) = binding.region(&loaded.asset.document, elapsed) {
                        let uv = loaded.asset.document.uv_rect(region).unwrap();
                        let region = loaded.asset.document.region(region).unwrap();
                        let size = egui::vec2(region.width as f32, region.height as f32);
                        let size = size * (128.0 / size.x.max(size.y));
                        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                        ui.painter().image(loaded.texture.id(), rect, Rect::from_min_max(Pos2::new(uv[0], uv[1]), Pos2::new(uv[2], uv[3])), Color32::WHITE);
                    }
                    let enabled = matches && editor.mode() == Mode::Edit && !editor.is_viewer() && editor.previewing().is_none() && !editor.selected_guids().is_empty();
                    if ui.add_enabled(enabled, egui::Button::new("Assign sprite to selection")).clicked() {
                        let result = binding.region(&loaded.asset.document, 0).and_then(|_| self.bindings.as_mut().unwrap().assign(editor.selected_guids(), Some(binding)));
                        self.report(result);
                    }
                }
                let enabled = matches && editor.mode() == Mode::Edit && !editor.is_viewer() && editor.previewing().is_none() && !editor.selected_guids().is_empty();
                if ui.add_enabled(enabled, egui::Button::new("Remove selected sprite bindings")).clicked() {
                    let result = self.bindings.as_mut().unwrap().assign(editor.selected_guids(), None); self.report(result);
                }
                for guid in editor.selected_guids() {
                    if let Some(binding) = self.bindings.as_ref().unwrap().document().bindings.get(&guid.to_string()) { ui.label(format!("{guid}: {}/{} {:?}", binding.package, binding.document, binding.source)); }
                }
                for error in self.diagnostics(editor) { ui.colored_label(Color32::YELLOW, error); }
            }
            if let Some(error) = &self.error { ui.colored_label(Color32::LIGHT_RED, error); }
        });
    }
    /// Errors remain visible until repaired; dangling GUIDs are never reassigned
    /// to an entity that happens to occupy an old frame handle.
    pub fn diagnostics(&self, editor: &Editor) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(bindings) = &self.bindings {
            for (guid, binding) in &bindings.document().bindings {
                if !editor
                    .rows()
                    .iter()
                    .any(|row| row.guid.as_ref().is_some_and(|g| g.to_string() == *guid))
                {
                    errors.push(format!("Orphan or unavailable scene GUID: {guid}"));
                }
                match self
                    .loaded
                    .get(&(binding.package.clone(), binding.document.clone()))
                {
                    None => errors.push(format!(
                        "{guid}: sprite asset unavailable; Reload assets for details"
                    )),
                    Some(asset) => {
                        if let Err(e) = binding.region(&asset.asset.document, 0) {
                            errors.push(format!("{guid}: {e}"));
                        }
                    }
                }
            }
        }
        errors
    }
    pub fn paint(&self, ui: &Ui, editor: &Editor, rect: Rect, pixels: (u32, u32)) {
        if editor.game() != EditorGame::Arena
            || !self.scene_matches(editor)
            || editor.previewing().is_some()
        {
            return;
        }
        let Some(bindings) = &self.bindings else {
            return;
        };
        // The bodies and timeline are extracted from the same displayed snapshot.
        // ERP sim.state may be ahead or behind during Play/Stop/seek transitions.
        let elapsed = editor.snapshot().map_or(0, |snapshot| {
            snapshot.timeline().map_or(0, |timeline| {
                scene_elapsed_ms(Mode::Play, timeline.tick, snapshot.tick_rate())
            })
        });
        let painter = ui.painter().with_clip_rect(rect);
        for (guid, binding) in &bindings.document().bindings {
            let Some(row) = binding_row(editor.rows(), guid) else {
                continue;
            };
            let Some(body) = editor
                .bodies()
                .iter()
                .find(|body| body.entity == row.entity)
            else {
                continue;
            };
            let Some(asset) = self
                .loaded
                .get(&(binding.package.clone(), binding.document.clone()))
            else {
                continue;
            };
            let Ok(id) = binding.region(&asset.asset.document, elapsed) else {
                continue;
            };
            let region = asset.asset.document.region(id).unwrap();
            let uv = asset.asset.document.uv_rect(id).unwrap();
            let half = [
                region.width as f32 * binding.units_per_pixel * 0.5,
                region.height as f32 * binding.units_per_pixel * 0.5,
            ];
            let (sin, cos) = body.angle.sin_cos();
            let mut mesh = egui::Mesh::with_texture(asset.texture.id());
            for ([x, y], tex) in [
                ([-half[0], half[1]], [uv[0], uv[1]]),
                ([half[0], half[1]], [uv[2], uv[1]]),
                ([half[0], -half[1]], [uv[2], uv[3]]),
                ([-half[0], -half[1]], [uv[0], uv[3]]),
            ] {
                let screen = editor.camera.world_to_screen(
                    [
                        body.pos[0] + x * cos - y * sin,
                        body.pos[1] + x * sin + y * cos,
                    ],
                    pixels,
                );
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: rect.min
                        + egui::vec2(
                            screen[0] * rect.width() / pixels.0 as f32,
                            screen[1] * rect.height() / pixels.1 as f32,
                        ),
                    uv: Pos2::new(tex[0], tex[1]),
                    color: Color32::WHITE,
                });
            }
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}
fn binding_row<'a>(
    rows: &'a [crate::model::EntityRow],
    guid: &str,
) -> Option<&'a crate::model::EntityRow> {
    rows.iter()
        .find(|row| row.guid.as_ref().is_some_and(|g| g.as_str() == guid))
}
/// Pausing/rewinding follows the host tick. Stop returns exactly to first frame.
pub fn scene_elapsed_ms(mode: Mode, tick: u64, tick_rate: u32) -> u64 {
    if mode == Mode::Edit {
        0
    } else {
        tick.saturating_mul(1000) / u64::from(tick_rate.max(1))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn play_pause_rewind_stop_use_host_time() {
        assert_eq!(scene_elapsed_ms(Mode::Play, 120, 60), 2000);
        assert_eq!(scene_elapsed_ms(Mode::Play, 120, 60), 2000);
        assert_eq!(scene_elapsed_ms(Mode::Play, 30, 60), 500);
        assert_eq!(scene_elapsed_ms(Mode::Edit, 120, 60), 0);
    }
    #[test]
    fn recycled_handle_never_steals_guid_binding() {
        let entity = orr_ecs::Entity {
            index: 3,
            version: 0,
        };
        let original = crate::model::EntityRow {
            entity,
            guid: Some(orr_reflect::Guid::from_u32(1)),
            name: None,
            components: vec![],
        };
        assert_eq!(
            binding_row(std::slice::from_ref(&original), "e_00000001")
                .unwrap()
                .entity,
            entity
        );
        let replacement = crate::model::EntityRow {
            guid: Some(orr_reflect::Guid::from_u32(2)),
            ..original.clone()
        };
        assert!(binding_row(&[replacement], "e_00000001").is_none());
        assert!(binding_row(&[], "e_00000001").is_none());
        assert!(binding_row(&[original], "e_00000001").is_some());
    }
    /// Real egui widgets and input events, CPU-only: no eframe renderer/window.
    /// Atlas loading is injected; verified package loading has separate tests.
    #[test]
    fn headless_widgets_assign_remove_save_and_guard_dirty_scene_switch() {
        use egui_kittest::{kittest::Queryable, Harness};
        let dir = tempfile::tempdir().unwrap();
        let scene = dir.path().join("arena.yaml");
        let other = dir.path().join("other.yaml");
        let scene_text = include_str!("../../../scenes/arena_blank.scene.yaml");
        std::fs::write(&scene, scene_text).unwrap();
        std::fs::write(&other, scene_text).unwrap();
        let mut editor = Editor::open_game(&scene, EditorGame::Arena).unwrap();
        editor.sync();
        assert!(editor.spawn_arena_player(0, [0.0, 0.0]));
        editor.sync();
        let checksum = editor.checksum();
        let history_len = editor.history().entries.len();
        let path = dir.path().join("arena.sprites.json");
        let panel = SpritePanel {
            bindings: Some(
                Bindings::create(path.clone(), "arena.yaml".into(), ".".into()).unwrap(),
            ),
            package: "fixture".into(),
            document: "sprites.json".into(),
            source: Source::Region(10),
            ..Default::default()
        };
        let mut h = Harness::builder()
            .with_size([1200.0, 1400.0])
            .build_ui_state(
                |ui, (panel, editor): &mut (SpritePanel, Editor)| {
                    if panel.loaded.is_empty() && panel.bindings.is_some() {
                        let document = orr_sprite::SpriteDocument::from_json(include_str!(
                            "../../../assets/sprite_demo/sprites.json"
                        ))
                        .unwrap();
                        panel.cache_asset(
                            ui.ctx(),
                            ("fixture".into(), "sprites.json".into()),
                            Asset {
                                document,
                                rgba: vec![255; 64 * 16 * 4],
                            },
                        );
                    }
                    panel.show(ui, editor);
                },
                (panel, editor),
            );
        h.get_by_label("Sprite bindings (view only)").click();
        h.run_steps(3);
        h.get_by_label("Assign sprite to selection").click();
        h.run_steps(3);
        assert_eq!(
            h.state()
                .0
                .bindings
                .as_ref()
                .unwrap()
                .document()
                .bindings
                .len(),
            1
        );
        assert!(h.state().0.bindings.as_ref().unwrap().dirty());
        h.get_by_label("Save bindings").click();
        h.run_steps(3);
        assert!(!h.state().0.bindings.as_ref().unwrap().dirty());
        assert_eq!(
            Bindings::open(path.clone())
                .unwrap()
                .document()
                .bindings
                .len(),
            1
        );
        h.get_by_label("Remove selected sprite bindings").click();
        h.run_steps(3);
        assert!(h
            .state()
            .0
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings
            .is_empty());
        h.get_by_label("Undo binding").click();
        h.run_steps(3);
        assert_eq!(
            h.state()
                .0
                .bindings
                .as_ref()
                .unwrap()
                .document()
                .bindings
                .len(),
            1
        );
        h.get_by_label("Redo binding").click();
        h.run_steps(3);
        assert!(h.state().0.bindings.as_ref().unwrap().dirty());
        assert_eq!(h.state().1.checksum(), checksum);
        assert_eq!(h.state().1.history().entries.len(), history_len);
        assert_eq!(std::fs::read_to_string(&scene).unwrap(), scene_text);
        assert!(h.state_mut().1.open_path(&other));
        h.state_mut().1.sync();
        h.run_steps(3);
        assert!(!h.state().0.scene_matches(&h.state().1));
        assert!(h.state().0.bindings.as_ref().unwrap().dirty());
        h.get_by_label("Assign sprite to selection").click();
        h.run_steps(3);
        assert!(h
            .state()
            .0
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings
            .is_empty());
        assert_eq!(Bindings::open(path).unwrap().document().bindings.len(), 1);
        h.get_by_label("Discard unsaved bindings and close").click();
        h.run_steps(3);
        assert!(h.state().0.bindings.is_none());
        assert!(h.query_by_label("Create bindings").is_some());
    }
}
