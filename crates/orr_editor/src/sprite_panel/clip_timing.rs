//! Private, presentation-only timing or sequence drafts from an admitted atlas.
//! Nothing here assigns a binding, refreshes a texture, or edits the source document.
use super::*;
use orr_sprite::{
    ClipSource, Frame, PlaybackMode, SpriteDocument, SpriteSource, MAX_CLIPS, MAX_CLIP_ID_BYTES,
    MAX_FRAMES_PER_CLIP, MAX_TOTAL_FRAMES,
};
pub(super) mod output;

#[derive(Default)]
pub(super) struct State {
    draft: Option<Draft>,
    message: Option<String>,
    sequence_inputs: SequenceInputs,
}
// Text inputs are staging buffers, not committed draft data. Text edits clear
// preview/playback immediately; invalid Apply preserves the last valid draft.
#[derive(Default)]
struct SequenceInputs {
    id: String,
    duration: String,
    region: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum DraftKind {
    Timing { clip_index: usize },
    Sequence { regions: Vec<u32>, selected: usize },
}
enum SequenceEdit {
    Id(String),
    Duration(String),
    Mode(PlaybackMode),
    Append(u32),
    Remove,
    MoveUp,
    MoveDown,
    Reverse,
}
impl SequenceInputs {
    fn matches(&self, draft: &Draft) -> bool {
        match &draft.kind {
            DraftKind::Timing { .. } => true,
            DraftKind::Sequence { selected, .. } => {
                self.id == draft.id && draft.durations.get(*selected) == Some(&self.duration)
            }
        }
    }
}
impl State {
    fn sequence_inputs_match(&self) -> bool {
        self.draft
            .as_ref()
            .is_none_or(|draft| self.sequence_inputs.matches(draft))
    }
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
    kind: DraftKind,
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
    fn frame_budget(original: &SpriteSource, count: usize) -> Result<(), String> {
        if original.clips.len() >= MAX_CLIPS {
            return Err("Adding a clip would exceed the 256 clip limit".into());
        }
        if count == 0 || count > MAX_FRAMES_PER_CLIP {
            return Err("A clip sequence needs 1 to 4096 frames".into());
        }
        let total = original
            .clips
            .iter()
            .try_fold(count, |total, clip| total.checked_add(clip.frames.len()));
        if total.is_none_or(|total| total > MAX_TOTAL_FRAMES) {
            return Err("Adding these frames would exceed the 16384 total frame limit".into());
        }
        Ok(())
    }
    fn candidate(&self) -> Result<SpriteDocument, String> {
        self.candidate_with(&self.kind, &self.id, &self.durations, self.mode)
    }
    fn candidate_with(
        &self,
        kind: &DraftKind,
        id: &str,
        durations: &[String],
        mode: PlaybackMode,
    ) -> Result<SpriteDocument, String> {
        let label = match kind {
            DraftKind::Timing { .. } => "Timing variant",
            DraftKind::Sequence { .. } => "Sequence clip",
        };
        if id.trim().is_empty() || id.len() > MAX_CLIP_ID_BYTES {
            return Err(format!("{label} ID must be nonempty and at most 128 bytes"));
        }
        if self.original.clips.iter().any(|c| c.id == id) {
            return Err(format!(
                "{label} ID must be new; existing clips cannot be overwritten"
            ));
        }
        let frame_count = match kind {
            DraftKind::Timing { clip_index } => self
                .original
                .clips
                .get(*clip_index)
                .ok_or("Source clip missing")?
                .frames
                .len(),
            DraftKind::Sequence { regions, selected } => {
                if *selected >= regions.len() {
                    return Err("Selected sequence frame is missing".into());
                }
                regions.len()
            }
        };
        Self::frame_budget(&self.original, frame_count)?;
        let regions: Vec<_> = match kind {
            DraftKind::Timing { clip_index } => self.original.clips[*clip_index]
                .frames
                .iter()
                .map(|frame| frame.region)
                .collect(),
            DraftKind::Sequence { regions, .. } => regions.clone(),
        };
        if durations.len() != regions.len() {
            return Err("Frame layout changed".into());
        }
        let mut total = 0u64;
        let mut frames = Vec::with_capacity(regions.len());
        for (region, text) in regions.into_iter().zip(durations) {
            if !self.original.regions.iter().any(|r| r.id == region) {
                return Err("Sequence frame must reference an admitted existing region".into());
            }
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
            frames.push(Frame {
                region,
                duration_ms: duration,
            });
        }
        let mut source = self.original.clone();
        source.clips.push(ClipSource {
            id: id.into(),
            mode,
            frames,
        });
        let document = SpriteDocument::new(source).map_err(|e| e.to_string())?;
        let json = document.to_json().map_err(|e| e.to_string())?;
        SpriteDocument::from_json(&json).map_err(|e| e.to_string())
    }
    fn invalidate_preview(&mut self) {
        self.preview = None;
        self.playing = false;
        self.started = 0.0;
        self.elapsed = 0;
    }
    /// Validate a complete candidate before committing any sequence change.
    /// Selection and staged text are UI-only; boundary moves are true no-ops.
    fn edit_sequence(&mut self, edit: SequenceEdit) -> Result<bool, String> {
        let DraftKind::Sequence { regions, selected } = &self.kind else {
            return Err("Begin a clip sequence first".into());
        };
        Self::frame_budget(&self.original, regions.len())?;
        if *selected >= regions.len() || self.durations.len() != regions.len() {
            return Err("Selected sequence frame is missing".into());
        }
        if let SequenceEdit::Append(region) = &edit {
            Self::frame_budget(
                &self.original,
                regions.len().checked_add(1).ok_or("Frame count overflow")?,
            )?;
            if !self.original.regions.iter().any(|r| r.id == *region) {
                return Err("Sequence frame must reference an admitted existing region".into());
            }
        }
        let mut kind = self.kind.clone();
        let DraftKind::Sequence { regions, selected } = &mut kind else {
            unreachable!()
        };
        let mut durations = self.durations.clone();
        let mut id = self.id.clone();
        let mut mode = self.mode;
        match edit {
            SequenceEdit::Id(value) => id = value,
            SequenceEdit::Duration(value) => durations[*selected] = value,
            SequenceEdit::Mode(value) => mode = value,
            SequenceEdit::Append(region) => {
                regions.push(region);
                durations.push("100".into());
                *selected = regions.len() - 1;
            }
            SequenceEdit::Remove => {
                if regions.len() == 1 {
                    return Err("Cannot remove the last sequence frame".into());
                }
                regions.remove(*selected);
                durations.remove(*selected);
                *selected = (*selected).min(regions.len() - 1);
            }
            SequenceEdit::MoveUp if *selected > 0 => {
                regions.swap(*selected, *selected - 1);
                durations.swap(*selected, *selected - 1);
                *selected -= 1;
            }
            SequenceEdit::MoveDown if *selected + 1 < regions.len() => {
                regions.swap(*selected, *selected + 1);
                durations.swap(*selected, *selected + 1);
                *selected += 1;
            }
            SequenceEdit::Reverse => {
                regions.reverse();
                durations.reverse();
                *selected = regions.len() - 1 - *selected;
            }
            SequenceEdit::MoveUp | SequenceEdit::MoveDown => {}
        }
        self.candidate_with(&kind, &id, &durations, mode)?;
        if self.kind == kind && self.id == id && self.durations == durations && self.mode == mode {
            return Ok(false);
        }
        self.kind = kind;
        self.id = id;
        self.durations = durations;
        self.mode = mode;
        self.invalidate_preview();
        Ok(true)
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
            let sequence = self
                .timing
                .draft
                .as_ref()
                .is_some_and(|d| matches!(d.kind, DraftKind::Sequence { .. }));
            self.timing.draft = None;
            self.timing.sequence_inputs = SequenceInputs::default();
            self.timing.message = Some(
                if sequence {
                    "Clip sequence discarded because its source context changed"
                } else {
                    "Timing variant discarded because its source context changed"
                }
                .into(),
            );
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
            kind: DraftKind::Timing { clip_index },
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
    fn begin_sequence(&mut self, editor: &Editor) -> Result<(), String> {
        self.verify_timing_source(editor, None)?;
        let Source::Region(region) = self.source else {
            return Err("Select an existing Region source first".into());
        };
        let key = (self.package.clone(), self.document.clone());
        let loaded = self.loaded.get(&key).ok_or("Load source first")?;
        let original_json = loaded.asset.document.to_json().map_err(|e| e.to_string())?;
        let original: SpriteSource =
            serde_json::from_str(&original_json).map_err(|e| e.to_string())?;
        Draft::frame_budget(&original, 1)?;
        if !original.regions.iter().any(|r| r.id == region) {
            return Err("Select an admitted existing Region source first".into());
        }
        let mut id = "sequence".to_owned();
        for suffix in 2..=MAX_CLIPS + 1 {
            if !original.clips.iter().any(|clip| clip.id == id) {
                break;
            }
            id = format!("sequence-{suffix}");
        }
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
            original,
            original_json,
            kind: DraftKind::Sequence {
                regions: vec![region],
                selected: 0,
            },
            id: id.clone(),
            durations: vec!["100".into()],
            mode: PlaybackMode::Loop,
            output: String::new(),
            preview: None,
            playing: false,
            started: 0.0,
            elapsed: 0,
        };
        draft.candidate()?;
        self.timing.draft = Some(draft);
        self.timing.sequence_inputs = SequenceInputs {
            id,
            duration: "100".into(),
            region,
        };
        self.timing.message = None;
        Ok(())
    }
    fn export_timing(&self, editor: &Editor) -> Result<String, String> {
        let draft = self
            .timing
            .draft
            .as_ref()
            .ok_or("Begin a timing variant or clip sequence first")?;
        if !self.timing.sequence_inputs_match() {
            return Err("Apply valid sequence ID and duration before preview or export".into());
        }
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
    fn edit_sequence(&mut self, editor: &Editor, edit: SequenceEdit) -> Result<bool, String> {
        if !matches!(&edit, SequenceEdit::Id(_) | SequenceEdit::Duration(_))
            && !self.timing.sequence_inputs_match()
        {
            return Err(
                "Apply valid sequence ID and duration before changing frames or playback mode"
                    .into(),
            );
        }
        let draft = self
            .timing
            .draft
            .as_ref()
            .ok_or("Begin a clip sequence first")?;
        self.verify_timing_source(editor, Some(draft))?;
        let sync_id = matches!(&edit, SequenceEdit::Id(_));
        let sync_duration = !matches!(&edit, SequenceEdit::Id(_) | SequenceEdit::Mode(_));
        let draft = self.timing.draft.as_mut().unwrap();
        let changed = draft.edit_sequence(edit)?;
        if sync_id {
            self.timing.sequence_inputs.id.clone_from(&draft.id);
        }
        if changed && sync_duration {
            if let DraftKind::Sequence { selected, .. } = &draft.kind {
                self.timing
                    .sequence_inputs
                    .duration
                    .clone_from(&draft.durations[*selected]);
            }
        }
        Ok(changed)
    }
    fn preview_timing(&mut self, editor: &Editor, started: f64) -> Result<(), String> {
        let draft = self
            .timing
            .draft
            .as_ref()
            .ok_or("Begin a clip draft first")?;
        let sequence = matches!(draft.kind, DraftKind::Sequence { .. });
        let result = if sequence {
            self.verify_timing_source(editor, Some(draft))
                .and_then(|()| {
                    if self.timing.sequence_inputs_match() {
                        draft.candidate()
                    } else {
                        Err("Apply valid sequence ID and duration before preview or export".into())
                    }
                })
        } else {
            draft.candidate()
        };
        let draft = self.timing.draft.as_mut().unwrap();
        match result {
            Ok(candidate) => {
                draft.preview = Some(candidate);
                draft.started = started;
                draft.elapsed = 0;
                draft.playing = true;
                Ok(())
            }
            Err(error) => {
                if sequence {
                    draft.invalidate_preview();
                } else {
                    draft.playing = false;
                }
                Err(error)
            }
        }
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
            && !self.tilemap_pinned;
        if ui
            .add_enabled(
                enabled && matches!(self.source, Source::Clip(_)),
                egui::Button::new("Begin clip timing variant"),
            )
            .clicked()
        {
            let result = self.begin_timing(editor);
            self.report(result);
        }
        ui.label("Clip sequence (new JSON only)");
        ui.weak("Begin from a Region source and arrange existing regions, including repeats. Append uses 100 ms. At least one frame is required. JSON requires the original exact PNG and a license when packaging.");
        if ui
            .add_enabled(
                enabled && matches!(self.source, Source::Region(_)),
                egui::Button::new("Begin clip sequence"),
            )
            .clicked()
        {
            let result = self.begin_sequence(editor);
            self.report(result);
        }
        let mut export = false;
        let mut reset = false;
        let mut sequence_edit = None;
        let mut preview = false;
        if let Some(draft) = &mut self.timing.draft {
            let inputs = &mut self.timing.sequence_inputs;
            let sequence = matches!(draft.kind, DraftKind::Sequence { .. });
            let message = &self.timing.message;
            let mut show_draft = |ui: &mut Ui| {
                ui.add_enabled_ui(enabled, |ui| {
                let mut pending_changed = false;
                match &mut draft.kind {
                    DraftKind::Timing { clip_index } => {
                        let mut changed = false;
                        ui.horizontal(|ui| {
                            let label = ui.label("Timing variant ID");
                            changed |= ui.add(egui::TextEdit::singleline(&mut draft.id).char_limit(MAX_CLIP_ID_BYTES))
                                .labelled_by(label.id).changed();
                        });
                        while draft.id.len() > MAX_CLIP_ID_BYTES { draft.id.pop(); }
                        egui::ScrollArea::vertical().max_height(160.0).id_salt("timing_frames").show(ui, |ui| {
                            for (index, duration) in draft.durations.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    let label = ui.label(format!("Frame {index} duration ms"));
                                    changed |= ui.add(egui::TextEdit::singleline(duration).char_limit(20))
                                        .labelled_by(label.id).changed();
                                    ui.label(format!("Region {}", draft.original.clips[*clip_index].frames[index].region));
                                });
                            }
                        });
                        ui.horizontal(|ui| {
                            changed |= ui.radio_value(&mut draft.mode, PlaybackMode::Loop, "Timing Loop").changed();
                            changed |= ui.radio_value(&mut draft.mode, PlaybackMode::Once, "Timing Once").changed();
                        });
                        if changed { draft.invalidate_preview(); }
                    }
                    DraftKind::Sequence { regions, selected } => {
                        ui.vertical(|ui| {
                            let label = ui.label("Sequence clip ID");
                            pending_changed |= ui.add(egui::TextEdit::singleline(&mut inputs.id).char_limit(MAX_CLIP_ID_BYTES))
                                .labelled_by(label.id).changed();
                            if ui.button("Apply sequence ID").clicked() {
                                sequence_edit = Some(SequenceEdit::Id(inputs.id.clone()));
                            }
                        });
                        let fields_applied = inputs.id == draft.id
                            && draft.durations.get(*selected) == Some(&inputs.duration);
                        ui.add_enabled_ui(fields_applied, |ui| {
                        egui::ComboBox::from_label("Sequence append region")
                            .selected_text(format!("Sequence region {}", inputs.region))
                            .show_ui(ui, |ui| {
                                for region in &draft.original.regions {
                                    ui.selectable_value(&mut inputs.region, region.id, format!("Sequence region {}", region.id));
                                }
                            });
                        if ui.button("Append sequence frame").clicked() {
                            sequence_edit = Some(SequenceEdit::Append(inputs.region));
                        }
                        egui::ScrollArea::vertical().max_height(160.0).id_salt("sequence_frames").show(ui, |ui| {
                            for (index, region) in regions.iter().enumerate() {
                                if ui.selectable_label(*selected == index, format!("Sequence frame {index} (region {region})")).clicked() {
                                    *selected = index;
                                    inputs.duration.clone_from(&draft.durations[index]);
                                }
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Move sequence frame up").clicked() { sequence_edit = Some(SequenceEdit::MoveUp); }
                            if ui.button("Move sequence frame down").clicked() { sequence_edit = Some(SequenceEdit::MoveDown); }
                            if ui.button("Reverse clip sequence").clicked() { sequence_edit = Some(SequenceEdit::Reverse); }
                            if ui.button("Remove sequence frame").clicked() { sequence_edit = Some(SequenceEdit::Remove); }
                        });
                        });
                        ui.vertical(|ui| {
                            let label = ui.label("Selected sequence frame duration ms");
                            pending_changed |= ui.add(egui::TextEdit::singleline(&mut inputs.duration).char_limit(20)).labelled_by(label.id).changed();
                            if ui.button("Apply sequence frame duration").clicked() {
                                sequence_edit = Some(SequenceEdit::Duration(inputs.duration.clone()));
                            }
                        });
                        let fields_applied = inputs.id == draft.id
                            && draft.durations.get(*selected) == Some(&inputs.duration);
                        ui.add_enabled_ui(fields_applied, |ui| { ui.horizontal(|ui| {
                            let mut mode = draft.mode;
                            let changed_loop = ui.radio_value(&mut mode, PlaybackMode::Loop, "Sequence Loop").changed();
                            let changed_once = ui.radio_value(&mut mode, PlaybackMode::Once, "Sequence Once").changed();
                            if changed_loop || changed_once { sequence_edit = Some(SequenceEdit::Mode(mode)); }
                        }); });
                    }
                }
                if pending_changed { draft.invalidate_preview(); }
                let applied = inputs.matches(draft);
                if !applied {
                    ui.weak("Unapplied sequence fields: apply valid ID and duration before preview or export");
                }
                preview = ui.add_enabled(applied, egui::Button::new(if sequence { "Preview clip sequence" } else { "Preview timing variant" })).clicked();
                if ui.button(if sequence { "Stop clip sequence preview" } else { "Stop timing variant preview" }).clicked() {
                    draft.playing = false;
                    draft.elapsed = 0;
                }
                if let Some(document) = &draft.preview {
                    let elapsed = if draft.playing {
                        ((ui.input(|i| i.time) - draft.started).max(0.0) * 1000.0) as u64
                    } else { draft.elapsed };
                    draft.elapsed = elapsed;
                    let clip = document.clip(&draft.id).unwrap();
                    let sample = clip.sample(elapsed);
                    if sample.finished { draft.playing = false; }
                    if draft.playing { ui.ctx().request_repaint(); }
                    let region = document.region(sample.region).unwrap();
                    let size = egui::vec2(region.width as f32, region.height as f32);
                    let size = size * (128.0 / size.x.max(size.y));
                    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                    paint_oriented_preview(ui, draft.pin.texture, rect, size, document.uv_rect(sample.region).unwrap(), Orientation::default());
                }
                if sequence {
                    ui.vertical(|ui| {
                        let label = ui.label("Sequence output path");
                        ui.add(egui::TextEdit::singleline(&mut draft.output).char_limit(4096).desired_width(ui.available_width())).labelled_by(label.id);
                    });
                } else {
                    ui.horizontal(|ui| {
                        let label = ui.label("Timing variant output path");
                        ui.add(egui::TextEdit::singleline(&mut draft.output).char_limit(4096)).labelled_by(label.id);
                    });
                }
                export = ui.add_enabled(applied, egui::Button::new(if sequence { "Export clip sequence JSON" } else { "Export timing variant JSON" })).clicked();
            });
                reset = ui
                    .button(if sequence {
                        "Discard clip sequence"
                    } else {
                        "Discard timing variant"
                    })
                    .clicked();
                if sequence {
                    if let Some(message) = message {
                        ui.label(message);
                    }
                }
            };
            if sequence {
                // Keep the full sequence workflow reachable in a narrow inspector,
                // including when adding the private preview increases its height.
                let height = ui.available_height().clamp(160.0, 520.0);
                egui::ScrollArea::vertical()
                    .id_salt("clip_sequence_draft")
                    .max_height(height)
                    .auto_shrink([false, true])
                    .show(ui, &mut show_draft);
            } else {
                show_draft(ui);
            }
        }
        if let Some(edit) = sequence_edit {
            let result = self.edit_sequence(editor, edit).map(|_| ());
            self.report(result);
        }
        if preview {
            let result = self.preview_timing(editor, ui.input(|i| i.time));
            self.report(result);
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
        if !self
            .timing
            .draft
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DraftKind::Sequence { .. }))
        {
            if let Some(message) = &self.timing.message {
                ui.label(message);
            }
        }
    }
}
#[cfg(test)]
mod tests;
