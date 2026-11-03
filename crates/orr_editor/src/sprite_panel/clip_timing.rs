//! Private, presentation-only clone of an admitted clip. Nothing here assigns a
//! binding, refreshes a texture, or edits the source document.
use super::*;
use orr_sprite::{PlaybackMode, SpriteDocument, SpriteSource, MAX_CLIP_ID_BYTES};
pub(super) mod output;

#[derive(Default)]
pub(super) struct State {
    draft: Option<Draft>,
    message: Option<String>,
}
struct Pin {
    epoch: u64,
    scene: Option<String>,
    key: (String, String),
    source: Source,
    binding_path: PathBuf,
    project: String,
    texture: egui::TextureId,
}
struct Draft {
    pin: Pin,
    original: SpriteSource,
    original_json: String,
    clip_index: usize,
    id: String,
    durations: Vec<String>,
    mode: PlaybackMode,
    output: String,
    preview: Option<SpriteDocument>,
    playing: bool,
    started: f64,
    elapsed: u64,
}
impl Draft {
    fn candidate(&self) -> Result<SpriteDocument, String> {
        if self.id.trim().is_empty() || self.id.len() > MAX_CLIP_ID_BYTES {
            return Err("Timing variant ID must be nonempty and at most 128 bytes".into());
        }
        if self.original.clips.iter().any(|c| c.id == self.id) {
            return Err(
                "Timing variant ID must be new; existing clips cannot be overwritten".into(),
            );
        }
        let mut clip = self.original.clips[self.clip_index].clone();
        if self.durations.len() != clip.frames.len() {
            return Err("Frame layout changed".into());
        }
        let mut total = 0u64;
        for (frame, text) in clip.frames.iter_mut().zip(&self.durations) {
            if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err(
                    "Frame duration must be a positive decimal u64 (at most 20 digits)".into(),
                );
            }
            let duration = text
                .parse::<u64>()
                .map_err(|_| "Frame duration exceeds u64")?;
            if duration == 0 {
                return Err("Frame duration must be positive".into());
            }
            total = total
                .checked_add(duration)
                .ok_or("Total clip duration exceeds u64")?;
            frame.duration_ms = duration;
        }
        clip.id.clone_from(&self.id);
        clip.mode = self.mode;
        let mut source = self.original.clone();
        source.clips.push(clip);
        let document = SpriteDocument::new(source).map_err(|e| e.to_string())?;
        let json = document.to_json().map_err(|e| e.to_string())?;
        SpriteDocument::from_json(&json).map_err(|e| e.to_string())
    }
}
impl SpritePanel {
    fn timing_pin_matches(&self, editor: &Editor, pin: &Pin) -> bool {
        !self.tilemap_pinned
            && editor.scene_load_epoch() == pin.epoch
            && editor.sim().scene_path == pin.scene
            && self.scene_matches(editor)
            && self.package == pin.key.0
            && self.document == pin.key.1
            && self.source == pin.source
            && self
                .bindings
                .as_ref()
                .is_some_and(|b| b.path == pin.binding_path && b.document().project == pin.project)
            && self
                .loaded
                .get(&pin.key)
                .is_some_and(|a| a.texture.id() == pin.texture)
    }
    pub(super) fn update_timing(&mut self, editor: &Editor) {
        if self
            .timing
            .draft
            .as_ref()
            .is_some_and(|d| !self.timing_pin_matches(editor, &d.pin))
        {
            self.timing.draft = None;
            self.timing.message =
                Some("Timing variant discarded because its source context changed".into());
        }
        if !self.editable(editor) {
            if let Some(draft) = &mut self.timing.draft {
                draft.playing = false;
            }
        }
    }
    fn verify_timing_source(&self, editor: &Editor, draft: Option<&Draft>) -> Result<(), String> {
        self.require_unpinned()?;
        if !self.editable(editor)
            || !editor.spec().is_local()
            || !editor.can_mutate()
            || editor.sim().in_tx
        {
            return Err("Clip timing requires the matching settled local scene in Edit".into());
        }
        if let Some(draft) = draft {
            if !self.timing_pin_matches(editor, &draft.pin) {
                return Err("Timing source context changed; begin again".into());
            }
        }
        let bindings = self.bindings.as_ref().ok_or("Open sprite bindings first")?;
        let cached = self
            .loaded
            .get(&(self.package.clone(), self.document.clone()))
            .ok_or("Load the source clip first")?;
        let root = sprite_bindings::resolve_project(bindings.base(), &bindings.document().project)?;
        let current = sprite_bindings::load_asset(&root, &self.package, &self.document)?;
        let json = current.document.to_json().map_err(|e| e.to_string())?;
        if json != cached.asset.document.to_json().map_err(|e| e.to_string())?
            || current.rgba != cached.asset.rgba
            || draft.is_some_and(|d| d.original_json != json)
        {
            return Err("Installed sprite document or pixels changed; explicitly reload before beginning a variant".into());
        }
        Ok(())
    }
    fn begin_timing(&mut self, editor: &Editor) -> Result<(), String> {
        self.verify_timing_source(editor, None)?;
        let Source::Clip(id) = &self.source else {
            return Err("Select an existing Clip source first".into());
        };
        let key = (self.package.clone(), self.document.clone());
        let loaded = self.loaded.get(&key).ok_or("Load source first")?;
        let original_json = loaded.asset.document.to_json().map_err(|e| e.to_string())?;
        let original: SpriteSource =
            serde_json::from_str(&original_json).map_err(|e| e.to_string())?;
        let clip_index = original
            .clips
            .iter()
            .position(|c| &c.id == id)
            .ok_or("Source clip missing")?;
        let clip = &original.clips[clip_index];
        let bindings = self.bindings.as_ref().unwrap();
        let draft = Draft {
            pin: Pin {
                epoch: editor.scene_load_epoch(),
                scene: editor.sim().scene_path.clone(),
                key,
                source: self.source.clone(),
                binding_path: bindings.path.clone(),
                project: bindings.document().project.clone(),
                texture: loaded.texture.id(),
            },
            durations: clip
                .frames
                .iter()
                .map(|f| f.duration_ms.to_string())
                .collect(),
            mode: clip.mode,
            original,
            original_json,
            clip_index,
            id: String::new(),
            output: String::new(),
            preview: None,
            playing: false,
            started: 0.0,
            elapsed: 0,
        };
        self.timing.draft = Some(draft);
        self.timing.message = None;
        Ok(())
    }
    fn export_timing(&self, editor: &Editor) -> Result<String, String> {
        let draft = self
            .timing
            .draft
            .as_ref()
            .ok_or("Begin a timing variant first")?;
        self.verify_timing_source(editor, Some(draft))?;
        let candidate = draft.candidate()?;
        let result = output::publish_new(std::path::Path::new(&draft.output), &candidate, || {
            self.verify_timing_source(editor, Some(draft))
        })?;
        Ok(format!(
            "Exported {} bytes. {}",
            result.bytes,
            result.warning.unwrap_or_default()
        ))
    }
    pub(super) fn show_timing(&mut self, ui: &mut Ui, editor: &Editor) {
        self.update_timing(editor);
        ui.separator();
        ui.label("Clip timing variant (new JSON only)");
        ui.weak("Clone an existing Clip source. Frames and regions stay fixed. JSON requires the original exact PNG and a license when packaging.");
        let enabled = self.editable(editor)
            && editor.spec().is_local()
            && editor.can_mutate()
            && !editor.sim().in_tx
            && matches!(self.source, Source::Clip(_));
        if ui
            .add_enabled(enabled, egui::Button::new("Begin clip timing variant"))
            .clicked()
        {
            let result = self.begin_timing(editor);
            self.report(result);
        }
        let mut export = false;
        let mut reset = false;
        if let Some(draft) = &mut self.timing.draft {
            ui.add_enabled_ui(enabled, |ui| {
                let mut changed = false;
                ui.horizontal(|ui| {
                    let label = ui.label("Timing variant ID");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut draft.id).char_limit(MAX_CLIP_ID_BYTES),
                        )
                        .labelled_by(label.id)
                        .changed();
                });
                while draft.id.len() > MAX_CLIP_ID_BYTES {
                    draft.id.pop();
                }
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .id_salt("timing_frames")
                    .show(ui, |ui| {
                        for (index, duration) in draft.durations.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                let label = ui.label(format!("Frame {index} duration ms"));
                                changed |= ui
                                    .add(egui::TextEdit::singleline(duration).char_limit(20))
                                    .labelled_by(label.id)
                                    .changed();
                                ui.label(format!(
                                    "Region {}",
                                    draft.original.clips[draft.clip_index].frames[index].region
                                ));
                            });
                        }
                    });
                ui.horizontal(|ui| {
                    changed |= ui
                        .radio_value(&mut draft.mode, PlaybackMode::Loop, "Timing Loop")
                        .changed();
                    changed |= ui
                        .radio_value(&mut draft.mode, PlaybackMode::Once, "Timing Once")
                        .changed();
                });
                if changed {
                    draft.preview = None;
                    draft.playing = false;
                    draft.elapsed = 0;
                }
                if ui.button("Preview timing variant").clicked() {
                    match draft.candidate() {
                        Ok(candidate) => {
                            draft.preview = Some(candidate);
                            draft.started = ui.input(|i| i.time);
                            draft.elapsed = 0;
                            draft.playing = true;
                            self.error = None;
                        }
                        Err(error) => {
                            self.error = Some(error);
                            draft.playing = false;
                        }
                    }
                }
                if ui.button("Stop timing variant preview").clicked() {
                    draft.playing = false;
                    draft.elapsed = 0;
                }
                if let Some(document) = &draft.preview {
                    let elapsed = if draft.playing {
                        ((ui.input(|i| i.time) - draft.started).max(0.0) * 1000.0) as u64
                    } else {
                        draft.elapsed
                    };
                    draft.elapsed = elapsed;
                    let clip = document.clip(&draft.id).unwrap();
                    let sample = clip.sample(elapsed);
                    if sample.finished {
                        draft.playing = false;
                    }
                    if draft.playing {
                        ui.ctx().request_repaint();
                    }
                    let region = document.region(sample.region).unwrap();
                    let size = egui::vec2(region.width as f32, region.height as f32);
                    let size = size * (128.0 / size.x.max(size.y));
                    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                    paint_oriented_preview(
                        ui,
                        draft.pin.texture,
                        rect,
                        size,
                        document.uv_rect(sample.region).unwrap(),
                        Orientation::default(),
                    );
                }
                ui.horizontal(|ui| {
                    let label = ui.label("Timing variant output path");
                    ui.add(egui::TextEdit::singleline(&mut draft.output).char_limit(4096))
                        .labelled_by(label.id);
                });
                export = ui.button("Export timing variant JSON").clicked();
            });
            reset = ui.button("Discard timing variant").clicked();
        }
        if export {
            match self.export_timing(editor) {
                Ok(message) => {
                    self.timing.message = Some(message);
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        if reset {
            self.timing = State::default();
        }
        if let Some(message) = &self.timing.message {
            ui.label(message);
        }
    }
}
#[cfg(test)]
mod tests;
