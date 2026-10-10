//! Small explicit recipe editor, separate from model binding and scene histories.
#![allow(clippy::float_arithmetic)]
use crate::blockout::{BakeResult, Blockout, Face, FaceUv};
use std::path::PathBuf;

pub struct BlockoutPanel {
    pub document: Option<Blockout>,
    pub path: String,
    pub face: Face,
    pub move_distance: f32,
    pub uv: FaceUv,
    pub material: u8,
    error: Option<String>,
    last_bake: Option<BakeResult>,
    discard: bool,
}
impl Default for BlockoutPanel {
    fn default() -> Self {
        Self { document: None, path: "box.blockout.json".into(), face: Face::PosX,
            move_distance: 0.25, uv: FaceUv::default(), material: 0,
            error: None, last_bake: None, discard: false }
    }
}
impl BlockoutPanel {
    pub fn error(&self) -> Option<&str> { self.error.as_deref() }
    pub fn last_bake(&self) -> Option<&BakeResult> { self.last_bake.as_ref() }
    fn sync_face(&mut self) {
        if let Some(doc) = &self.document { let settings = doc.recipe().faces[self.face.index()]; self.uv = settings.uv; self.material = settings.material; }
    }
    fn replace(&mut self, root: &std::path::Path, create: bool) -> Result<(), String> {
        if self.document.as_ref().is_some_and(Blockout::dirty) { return Err("Save or explicitly discard recipe changes before New/Open".into()); }
        let next = if create { Blockout::create(root, &self.path)? } else { Blockout::open(root, &self.path)? };
        self.document = Some(next); self.discard = false; self.last_bake = None; self.sync_face(); Ok(())
    }
    /// Called only with the admitted model sidecar's root. A baked asset is a
    /// picker suggestion: existing assigned immutable geometry never changes.
    pub fn show(&mut self, ui: &mut egui::Ui, root: Result<PathBuf, String>, editable: bool) -> Option<BakeResult> {
        let mut baked = None;
        ui.collapsing("Box blockout", |ui| {
            ui.weak("Six faces · presentation only · no collision generation");
            let root = match root { Ok(root) => root, Err(ref error) => { ui.label(error); return; } };
            let matches = self.document.as_ref().is_none_or(|doc| root.canonicalize().is_ok_and(|root| root == doc.root()));
            if !editable { ui.label("Box authoring requires the admitted local scene in Edit mode"); }
            if !matches { ui.label("Recipe belongs to another project; return there to save, or explicitly discard it"); }
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal(|ui| {
                    let label = ui.label("Box recipe path");
                    ui.add(egui::TextEdit::singleline(&mut self.path).hint_text("box.blockout.json")).labelled_by(label.id);
                });
                ui.horizontal(|ui| {
                    if ui.button("New box recipe").clicked() { self.error = self.replace(&root, true).err(); }
                    if ui.button("Open box recipe").clicked() { self.error = self.replace(&root, false).err(); }
                });
                ui.add_enabled_ui(matches, |ui| {
                    if self.document.is_none() { return; }
                    ui.horizontal(|ui| {
                        for face in Face::ALL { if ui.selectable_value(&mut self.face, face, face.name()).clicked() { self.sync_face(); } }
                    });
                    ui.horizontal(|ui| {
                        let label = ui.label("Move face delta");
                        ui.add(egui::DragValue::new(&mut self.move_distance).speed(0.05).range(-200.0..=200.0)).labelled_by(label.id);
                        if ui.button("Apply face move").clicked() { self.error = self.document.as_mut().expect("document").move_face(self.face, self.move_distance).err(); }
                    });
                    ui.horizontal(|ui| {
                        let label = ui.label("Face U offset");
                        ui.add(egui::DragValue::new(&mut self.uv.offset[0]).speed(0.05)).labelled_by(label.id);
                        let label = ui.label("Face V offset");
                        ui.add(egui::DragValue::new(&mut self.uv.offset[1]).speed(0.05)).labelled_by(label.id);
                    });
                    ui.horizontal(|ui| {
                        let label = ui.label("Face U scale");
                        ui.add(egui::DragValue::new(&mut self.uv.scale[0]).speed(0.05)).labelled_by(label.id);
                        let label = ui.label("Face V scale");
                        ui.add(egui::DragValue::new(&mut self.uv.scale[1]).speed(0.05)).labelled_by(label.id);
                    });
                    ui.horizontal(|ui| {
                        ui.label("UV quarter turn");
                        for turn in 0..4 { ui.selectable_value(&mut self.uv.quarter_turn, turn, format!("{} degrees", u16::from(turn) * 90)); }
                        if ui.button("Apply face UV").clicked() { self.error = self.document.as_mut().expect("document").set_uv(self.face, self.uv).err(); }
                    });
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.material, 0, "Checker white");
                        ui.selectable_value(&mut self.material, 1, "Checker blue");
                        if ui.button("Apply face material").clicked() { self.error = self.document.as_mut().expect("document").set_material(self.face, self.material).err(); }
                    });
                    self.preview(ui);
                    ui.horizontal(|ui| {
                        if ui.button("Undo blockout").clicked() { self.document.as_mut().expect("document").undo(); self.sync_face(); }
                        if ui.button("Redo blockout").clicked() { self.document.as_mut().expect("document").redo(); self.sync_face(); }
                        if ui.button("Save box recipe").clicked() { self.error = self.document.as_mut().expect("document").save().err(); }
                    });
                    if ui.button("Bake immutable box").clicked() {
                        let result = crate::model_bindings::open_project(&root).and_then(|project| self.document.as_ref().expect("document").prepare_bake(&project)?.commit(&project, || Ok(())));
                        match result { Ok(result) => { self.error = None; self.last_bake = Some(result.clone()); baked = Some(result); }, Err(error) => self.error = Some(error) }
                    }
                    if let Some(result) = &self.last_bake { ui.label(format!("Baked {}. Use Load model asset, Assign static model, then Save model bindings.", result.package)); }
                    if self.document.as_ref().is_some_and(Blockout::dirty) { ui.colored_label(egui::Color32::YELLOW, "Recipe has unsaved changes; baking requires Save"); }
                });
                if self.document.is_some() {
                    if self.discard {
                        ui.horizontal(|ui| {
                            if ui.button("Confirm discard box recipe").clicked() { self.document = None; self.discard = false; self.last_bake = None; self.error = None; }
                            if ui.button("Keep box recipe").clicked() { self.discard = false; }
                        });
                    } else if ui.button("Discard box recipe").clicked() { self.discard = true; }
                }
            });
            if let Some(error) = &self.error { ui.colored_label(egui::Color32::RED, error); }
        });
        baked
    }
    fn preview(&self, ui: &mut egui::Ui) {
        let Some(document) = &self.document else { return; };
        let recipe = document.recipe();
        let uv = recipe.face_uvs(self.face);
        ui.label(format!("{} planar UV preview: {:?}", self.face.name(), uv));
        // Show the applied result, not uncommitted control values. A fixed 32x32
        // sampling grid bounds UI work regardless of offset/scale or file input.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(160.0, 160.0), egui::Sense::hover());
        let image = crate::blockout::checker();
        let material = recipe.faces[self.face.index()].material;
        let factor = if material == 0 { [1.0, 1.0, 1.0] } else { [0.3, 0.65, 1.0] };
        for row in 0..32 { for col in 0..32 {
            let (s,t) = ((col as f32 + 0.5) / 32.0, (row as f32 + 0.5) / 32.0);
            let (u,v) = (uv[0][0] + s * (uv[1][0] - uv[0][0]) + t * (uv[3][0] - uv[0][0]), uv[0][1] + s * (uv[1][1] - uv[0][1]) + t * (uv[3][1] - uv[0][1]));
            let index = ((v.rem_euclid(1.0) * 4.0) as usize * 4 + (u.rem_euclid(1.0) * 4.0) as usize) * 4;
            let color = egui::Color32::from_rgb((f32::from(image.rgba8[index]) * factor[0]) as u8, (f32::from(image.rgba8[index+1]) * factor[1]) as u8, (f32::from(image.rgba8[index+2]) * factor[2]) as u8);
            let min = rect.min + egui::vec2(col as f32 * 5.0, row as f32 * 5.0);
            ui.painter().rect_filled(egui::Rect::from_min_size(min, egui::vec2(5.0,5.0)), 0.0, color);
        }}
    }
}
