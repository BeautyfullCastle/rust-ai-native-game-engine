//! Optional prospective ramp overlay. The only document write is one explicit Apply.
use super::EditorApp;
use crate::{
    model::Target,
    terrain_document::{format_fixed, TerrainSource},
    terrain_pick::{self, TerrainSelectionMode},
    terrain_ramp::{RampCandidate, RampSpec},
};
use egui::{Color32, Context, Painter, Rect, Stroke, Ui};
use orr_fp::FP;
use orr_render::Camera3D;
use orr_terrain::Edit;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, PartialEq, Eq)]
struct Pin {
    scene_epoch: u64,
    host_epoch: u64,
    scene: PathBuf,
    source: TerrainSource,
    revision: [u8; 32],
    selection: Option<Target>,
    selected_guids: Vec<String>,
    vertex: [u32; 2],
    cell: [u32; 2],
    mode: TerrainSelectionMode,
    height: String,
    hole: bool,
    endpoints: [Option<[u32; 2]>; 2],
    half_width: u32,
}

pub(crate) struct Staged {
    pub(crate) candidate: RampCandidate,
    pin: Pin,
}

pub(crate) struct State {
    pub(crate) endpoints: [Option<[u32; 2]>; 2],
    pub(crate) half_width: u32,
    pub(crate) staged: Option<Staged>,
    pub(crate) status: String,
    armed: Option<usize>,
    pin: Option<Pin>,
    focused: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            endpoints: [None; 2],
            half_width: 1,
            staged: None,
            status: String::new(),
            armed: None,
            pin: None,
            focused: true,
        }
    }
}
impl State {
    fn cancel(&mut self, reason: &str) {
        self.staged = None;
        self.armed = None;
        self.pin = None;
        self.endpoints = [None; 2];
        self.status = reason.into();
    }
    fn active(&self) -> bool {
        self.staged.is_some() || self.armed.is_some() || self.endpoints.iter().any(Option::is_some)
    }
}

impl EditorApp {
    fn terrain_ramp_pin(&self) -> Option<Pin> {
        if !self.terrain_ramp.focused
            || !self.terrain.authoring_active(&self.editor)
            || self.terrain.document.read_only()
        {
            return None;
        }
        let document = &self.terrain.document;
        document.terrain()?;
        Some(Pin {
            scene_epoch: self.editor.scene_load_epoch(),
            host_epoch: self.editor.sim().epoch,
            scene: document.scene()?.to_owned(),
            source: document.source()?.clone(),
            revision: document.revision()?,
            selection: self.editor.selection().cloned(),
            selected_guids: self
                .editor
                .selected_guids()
                .iter()
                .map(ToString::to_string)
                .collect(),
            vertex: self.terrain.selected_vertex,
            cell: self.terrain.selected_cell,
            mode: self.terrain.selection_mode,
            height: self.terrain.height.clone(),
            hole: self.terrain.hole,
            endpoints: self.terrain_ramp.endpoints,
            half_width: self.terrain_ramp.half_width,
        })
    }

    fn terrain_ramp_validate(&mut self) {
        if !self.terrain_ramp.active() {
            return;
        }
        let current = self.terrain_ramp_pin();
        if current.is_none() || self.terrain_ramp.pin.as_ref() != current.as_ref() {
            self.terrain_ramp
                .cancel("Ramp cancelled: terrain, selection, settings or editing context changed");
        }
    }

    pub(crate) fn terrain_ramp_sync(&mut self, ctx: &Context) {
        self.terrain_ramp.focused = ctx.input(|input| input.focused);
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) && self.terrain_ramp.active() {
            self.terrain_ramp.cancel("Ramp cancelled");
        }
        self.terrain_ramp_validate();
    }

    pub(crate) fn terrain_ramp_stage(&mut self) -> Result<(), String> {
        self.terrain_ramp_validate();
        let result: Result<(), String> = (|| {
            let pin = self
                .terrain_ramp_pin()
                .ok_or("Ramp requires focused, writable local terrain in Edit mode")?;
            let [Some(start), Some(end)] = self.terrain_ramp.endpoints else {
                return Err("Pick both ramp endpoints in the viewport first".into());
            };
            let candidate = RampCandidate::build(
                self.terrain
                    .document
                    .terrain()
                    .ok_or("Open terrain first")?,
                RampSpec {
                    start,
                    end,
                    half_width: self.terrain_ramp.half_width,
                },
            )?;
            self.terrain_ramp.status = format!(
                "Prospective ramp: {} changed vertices; not applied",
                candidate.changed_vertices()
            );
            self.terrain_ramp.pin = Some(pin.clone());
            self.terrain_ramp.staged = Some(Staged { candidate, pin });
            self.terrain_ramp.armed = None;
            Ok(())
        })();
        if let Err(error) = &result {
            self.terrain_ramp.status = error.clone();
        }
        result
    }

    pub(crate) fn terrain_ramp_apply(&mut self) -> Result<(), String> {
        self.terrain_ramp_validate();
        let result: Result<(), String> = (|| {
            let current = self
                .terrain_ramp_pin()
                .ok_or("Ramp editing context is no longer available")?;
            let staged = self
                .terrain_ramp
                .staged
                .as_ref()
                .ok_or("Stage a ramp before applying")?;
            if current != staged.pin
                || !staged
                    .candidate
                    .is_current(self.terrain.document.terrain().ok_or("Terrain closed")?)
            {
                self.terrain_ramp
                    .cancel("Ramp cancelled: staged source no longer matches");
                return Err("Ramp cancelled: staged source no longer matches".into());
            }
            let count = staged.candidate.changed_vertices();
            // Empty candidates must not destroy a redo branch or add history.
            if count != 0 {
                self.terrain.document.apply(staged.candidate.edits())?;
            }
            let [x, z] = self.terrain.selected_vertex;
            if let Some(terrain) = self.terrain.document.terrain() {
                if x < terrain.width() && z < terrain.depth() {
                    self.terrain.height =
                        format_fixed(terrain.heights()[(z * terrain.width() + x) as usize]);
                }
            }
            self.terrain_ramp.cancel(if count == 0 {
                "Ramp already matches terrain; no history entry added"
            } else {
                "Ramp applied as one Terrain Undo step; Save terrain to persist"
            });
            Ok(())
        })();
        if let Err(error) = &result {
            self.terrain_ramp.status = error.clone();
        }
        result
    }

    pub(crate) fn terrain_ramp_show(&mut self, ui: &mut Ui) {
        self.terrain_ramp_validate();
        if self.terrain.document.terrain().is_none() {
            return;
        }
        egui::CollapsingHeader::new("Terrain ramp").default_open(true).show(ui, |ui| {
            ui.weak("Axis-aligned finite ramp · cyan is prospective geometry, not applied terrain");
            let writable = self.terrain_ramp_pin().is_some();
            ui.add_enabled_ui(writable, |ui| {
                ui.horizontal(|ui| {
                    for (index, label) in [(0, "Pick ramp start"), (1, "Pick ramp end")] {
                        if ui.button(label).clicked() {
                            self.terrain_ramp.staged = None;
                            self.terrain.selection_mode = TerrainSelectionMode::Vertex;
                            self.terrain_ramp.armed = Some(index);
                            self.terrain_ramp.status = format!("Click a terrain vertex for ramp {}", if index == 0 { "start" } else { "end" });
                            self.terrain_ramp.pin = self.terrain_ramp_pin();
                        }
                    }
                });
                for (index, name) in [(0, "Start"), (1, "End")] {
                    ui.label(match self.terrain_ramp.endpoints[index] {
                        Some([x,z]) => format!("Ramp {name}: vertex ({x}, {z})"),
                        None => format!("Ramp {name}: not selected"),
                    });
                }
                let changed = ui.horizontal(|ui| {
                    let label = ui.label("Ramp half-width (cells)");
                    ui.add(egui::DragValue::new(&mut self.terrain_ramp.half_width).range(1..=orr_terrain::MAX_SIDE).speed(1)).labelled_by(label.id).changed()
                }).inner;
                if changed {
                    self.terrain_ramp.staged = None;
                    self.terrain_ramp.armed = None;
                    self.terrain_ramp.status = "Width changed; Stage ramp to preview again".into();
                    self.terrain_ramp.pin = self.terrain_ramp_pin();
                }
                ui.weak("Full width = 2 × half-width cells. End heights come from terrain; no clipping or hole bridging");
                ui.horizontal(|ui| {
                    if ui.add_enabled(self.terrain_ramp.endpoints.iter().all(Option::is_some), egui::Button::new("Stage ramp")).clicked() { let _ = self.terrain_ramp_stage(); }
                    if ui.add_enabled(self.terrain_ramp.staged.is_some(), egui::Button::new("Apply ramp")).clicked() { let _ = self.terrain_ramp_apply(); }
                });
            });
            if ui.add_enabled(self.terrain_ramp.active(), egui::Button::new("Cancel ramp")).clicked() { self.terrain_ramp.cancel("Ramp cancelled; terrain unchanged"); }
            if let Some(staged) = &self.terrain_ramp.staged {
                let footprint = staged.candidate.footprint();
                let heights = staged.candidate.endpoint_heights();
                ui.label(format!("Preview footprint: ({}, {})–({}, {}) · {} vertices · {} changed", footprint.min[0], footprint.min[1], footprint.max[0], footprint.max[1], staged.candidate.footprint_vertices(), staged.candidate.changed_vertices()));
                ui.label(format!("Endpoint heights: {} → {}", format_fixed(heights[0]), format_fixed(heights[1])));
            }
            if !self.terrain_ramp.status.is_empty() { ui.label(&self.terrain_ramp.status); }
        });
    }

    /// Reuses the existing terrain vertex picker and panel selection route.
    pub(crate) fn terrain_ramp_pick(
        &mut self,
        camera: &Camera3D,
        pixel: [f32; 2],
        size: (u32, u32),
    ) -> bool {
        self.terrain_ramp_validate();
        let Some(index) = self.terrain_ramp.armed else {
            return false;
        };
        let hit = self
            .terrain
            .document
            .terrain()
            .and_then(|terrain| terrain_pick::pick_vertex(terrain, camera, pixel, size, 12.0));
        self.terrain.pick(&self.editor, camera, pixel, size);
        if let Some(vertex) = hit {
            self.terrain_ramp.endpoints[index] = Some(vertex);
            self.terrain_ramp.armed = None;
            self.terrain_ramp.staged = None;
            self.terrain_ramp.status =
                "Endpoint captured; pick the other endpoint, then Stage ramp".into();
        }
        self.terrain_ramp.pin = self.terrain_ramp_pin();
        true
    }

    pub(crate) fn terrain_ramp_paint(&self, painter: &Painter, camera: &Camera3D, rect: Rect) {
        if self.terrain_ramp_pin().as_ref() != self.terrain_ramp.pin.as_ref() {
            return;
        }
        let Some(terrain) = self.terrain.document.terrain() else {
            return;
        };
        let size = (rect.width().max(1.0) as u32, rect.height().max(1.0) as u32);
        let screen = |position: [FP; 3]| {
            camera
                .world_to_screen(position.map(FP::to_f32), size)
                .map(|p| rect.min + egui::vec2(p[0], p[1]))
        };
        let painter = painter.with_clip_rect(rect);
        let cyan = Color32::from_rgb(60, 240, 255);
        for (index, endpoint) in self.terrain_ramp.endpoints.iter().enumerate() {
            if let Some([x, z]) = endpoint {
                if let Some(point) = terrain
                    .vertex_position(z * terrain.width() + x)
                    .and_then(screen)
                {
                    painter.circle_stroke(point, 10.0, Stroke::new(2.0, cyan));
                    painter.text(
                        point + egui::vec2(12.0, -12.0),
                        egui::Align2::LEFT_BOTTOM,
                        if index == 0 { "Ramp start" } else { "Ramp end" },
                        egui::FontId::proportional(13.0),
                        cyan,
                    );
                }
            }
        }
        let Some(staged) = &self.terrain_ramp.staged else {
            return;
        };
        let heights: BTreeMap<_, _> = staged
            .candidate
            .edits()
            .iter()
            .filter_map(|edit| match edit {
                Edit::SetHeight { x, z, height } => Some(([*x, *z], *height)),
                _ => None,
            })
            .collect();
        let position = |x: u32, z: u32| {
            let mut point = terrain.vertex_position(z * terrain.width() + x)?;
            if let Some(height) = heights.get(&[x, z]) {
                point[1] = *height;
            }
            Some(point)
        };
        let bounds = staged.candidate.footprint();
        for z in bounds.min[1]..=bounds.max[1] {
            for x in bounds.min[0]..=bounds.max[0] {
                let Some(point) = position(x, z).and_then(screen) else {
                    continue;
                };
                for (nx, nz) in [(x.saturating_add(1), z), (x, z.saturating_add(1))] {
                    if nx <= bounds.max[0] && nz <= bounds.max[1] {
                        if let Some(next) = position(nx, nz).and_then(screen) {
                            painter.line_segment([point, next], Stroke::new(2.0, cyan));
                        }
                    }
                }
                if heights.contains_key(&[x, z]) {
                    if let Some(old) = terrain
                        .vertex_position(z * terrain.width() + x)
                        .and_then(screen)
                    {
                        painter.line_segment(
                            [old, point],
                            Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 240, 255, 100)),
                        );
                    }
                }
            }
        }
        painter.text(
            rect.left_top() + egui::vec2(12.0, 12.0),
            egui::Align2::LEFT_TOP,
            "RAMP PREVIEW · not applied",
            egui::FontId::proportional(15.0),
            cyan,
        );
    }
}
