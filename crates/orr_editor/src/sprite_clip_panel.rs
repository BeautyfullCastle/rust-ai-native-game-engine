//! Optional standalone clip panel. Source-only staging: no workspace registration.
//! Caller must first perform real package/PNG admission; this panel owns that
//! admitted snapshot and never activates a project lock or edits the source package.
use crate::sprite_clip_authoring::{ClipDocument, SaveAsPlan};
use orr_package::LockedPackage;
use orr_sprite::{PlaybackMode, SpriteSource};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug)]
pub enum SaveOutcome {
    Never,
    Published { destination: PathBuf },
    Failed { message: String, leftover_stage: Option<PathBuf>, published: bool },
}

pub struct ClipPanel {
    document: ClipDocument,
    clip: usize,
    frame: usize,
    region: u32,
    duration: String,
    mode: PlaybackMode,
    package: LockedPackage,
    snapshot: BTreeMap<String, Vec<u8>>,
    document_path: String,
    next_version: String,
    destination: String,
    message: String,
    outcome: SaveOutcome,
}

impl ClipPanel {
    /// Admit an installed dependency-free sprite package before opening the panel.
    /// Reads are hash-checked against the owned lock snapshot; PNG decoding uses
    /// the same loader as the runtime. No install, activation or source write occurs.
    pub fn open_installed(project_root: &std::path::Path, package_name: &str,
        document_path: &str, clip_id: &str, next_version: &str) -> Result<Self, String> {
        let mut runtime = orr_package::Runtime::content_only();
        runtime.capabilities.insert("sprite".into());
        let project = orr_package::Project::open(project_root, runtime).map_err(|e| e.to_string())?;
        let lock = project.verify().map_err(|e| e.to_string())?;
        let package = lock.packages.get(package_name).ok_or("installed sprite package missing")?.clone();
        if !(3..=4).contains(&package.files.len()) || !package.manifest.dependencies.is_empty() {
            return Err("clip authoring requires one dependency-free closed sprite package".into());
        }
        let mut snapshot = BTreeMap::new();
        let mut total = 0usize;
        for path in package.files.keys() {
            let bytes = project.read_asset(package_name, path).map_err(|e| e.to_string())?;
            total = total.checked_add(bytes.len()).ok_or("sprite snapshot size overflow")?;
            if total > crate::sprite_clip_authoring::MAX_PACKAGE_BYTES {
                return Err("sprite snapshot exceeds authoring byte limit".into());
            }
            snapshot.insert(path.clone(), bytes);
        }
        let bytes = snapshot.get(document_path).ok_or("sprite document missing")?;
        let document = ClipDocument::open(bytes, clip_id)?;
        let atlas = &document.source().atlas;
        let image = snapshot.get(&atlas.image).ok_or("sprite atlas missing")?;
        orr_sample::project_sprites::decode_atlas(image, atlas.width, atlas.height)?;
        Self::from_admitted_snapshot(package, snapshot, document_path.into(), clip_id, next_version.into())
    }

    /// Snapshot identity/size checks are repeated, but decoder/runtime admission
    /// remains the caller's responsibility. No filesystem write occurs here.
    pub fn from_admitted_snapshot(package: LockedPackage, snapshot: BTreeMap<String, Vec<u8>>,
        document_path: String, clip_id: &str, next_version: String) -> Result<Self, String> {
        let bytes = snapshot.get(&document_path).ok_or("sprite document missing from snapshot")?;
        let document = ClipDocument::open(bytes, clip_id)?;
        SaveAsPlan::prepare(&document, &package, &snapshot, &document_path, &next_version)?;
        let clip = document.source().clips.iter().position(|c| c.id == clip_id).ok_or("clip missing")?;
        let mut panel = Self { document, clip, frame: 0, region: 0, duration: String::new(),
            mode: PlaybackMode::Loop, package, snapshot, document_path, next_version,
            destination: String::new(), message: String::new(), outcome: SaveOutcome::Never };
        panel.reset_draft();
        Ok(panel)
    }
    pub fn source(&self) -> &SpriteSource { self.document.source() }
    pub fn message(&self) -> &str { &self.message }
    pub fn save_outcome(&self) -> &SaveOutcome { &self.outcome }
    pub fn can_undo(&self) -> bool { self.document.can_undo() }
    pub fn can_redo(&self) -> bool { self.document.can_redo() }
    pub fn has_pending_draft(&self) -> bool {
        let clip = &self.source().clips[self.clip]; let frame = &clip.frames[self.frame];
        self.region != frame.region || self.duration != frame.duration_ms.to_string() || self.mode != clip.mode
    }
    fn reset_draft(&mut self) {
        let clip = &self.document.source().clips[self.clip];
        self.frame = self.frame.min(clip.frames.len() - 1);
        self.region = clip.frames[self.frame].region;
        self.duration = clip.frames[self.frame].duration_ms.to_string();
        self.mode = clip.mode;
    }
    fn apply_draft(&mut self) -> Result<(), String> {
        if self.duration.is_empty() || self.duration.len() > 20 || !self.duration.bytes().all(|c| c.is_ascii_digit()) {
            return Err("Duration must be a positive integer number of milliseconds".into());
        }
        let duration_ms: u64 = self.duration.parse().map_err(|_| "Duration exceeds u64")?;
        if duration_ms == 0 { return Err("Duration must be positive".into()); }
        let mut frames = self.source().clips[self.clip].frames.clone();
        frames[self.frame].region = self.region; frames[self.frame].duration_ms = duration_ms;
        self.document.replace_frames(frames, self.mode)?;
        self.reset_draft();
        Ok(())
    }
    fn save_as(&mut self) {
        let result = SaveAsPlan::prepare(&self.document, &self.package, &self.snapshot,
            &self.document_path, &self.next_version);
        self.outcome = match result {
            Err(message) => SaveOutcome::Failed { message, leftover_stage: None, published: false },
            Ok(plan) => {
                let destination = PathBuf::from(&self.destination);
                match plan.publish_new(&destination) {
                    Ok(()) => SaveOutcome::Published { destination },
                    Err(error) => SaveOutcome::Failed { message: error.message,
                        leftover_stage: error.leftover_stage, published: error.published },
                }
            }
        };
        self.message = match &self.outcome {
            SaveOutcome::Never => String::new(),
            SaveOutcome::Published { destination } => format!("Saved new package at {}. Install/activation is a separate action.", destination.display()),
            SaveOutcome::Failed { message, leftover_stage, published } => format!(
                "Save As failed: {message}; published={published}; private stage={leftover_stage:?}. Do not delete an existing destination or retry by replacing it."),
        };
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sprite clip authoring");
        ui.label(format!("Package {} · Clip {}", self.package.manifest.name, self.source().clips[self.clip].id));
        ui.label("Edits affect only this existing clip. Save As creates a new package; original files remain unchanged.");
        let pending = self.has_pending_draft();
        if pending { ui.label("Pending frame draft: Apply frame draft or Revert frame draft before navigating or saving."); }
        let old_frame = self.frame;
        ui.add_enabled_ui(!pending, |ui| {
            egui::ComboBox::from_label("Frame").selected_text(format!("Frame {}", self.frame + 1)).show_ui(ui, |ui| {
                for index in 0..self.document.source().clips[self.clip].frames.len() {
                    ui.selectable_value(&mut self.frame, index, format!("Frame {}", index + 1));
                }
            });
        });
        if self.frame != old_frame { self.reset_draft(); }
        egui::ComboBox::from_label("Region").selected_text(format!("Region {}", self.region)).show_ui(ui, |ui| {
            for region in &self.document.source().regions {
                ui.selectable_value(&mut self.region, region.id, format!("Region {}", region.id));
            }
        });
        let label = ui.label("Duration ms");
        ui.add(egui::TextEdit::singleline(&mut self.duration).char_limit(20)).labelled_by(label.id);
        egui::ComboBox::from_label("Playback").selected_text(match self.mode { PlaybackMode::Loop => "Loop", PlaybackMode::Once => "Once" }).show_ui(ui, |ui| {
            ui.selectable_value(&mut self.mode, PlaybackMode::Loop, "Loop");
            ui.selectable_value(&mut self.mode, PlaybackMode::Once, "Once");
        });
        ui.horizontal(|ui| {
            if ui.button("Apply frame draft").clicked() {
                self.message = match self.apply_draft() { Ok(()) => "Frame draft applied".into(), Err(error) => error };
            }
            if ui.button("Revert frame draft").clicked() { self.reset_draft(); self.message = "Frame draft reverted".into(); }
        });
        let clean = !self.has_pending_draft();
        ui.horizontal(|ui| {
            for (label, to) in [("Move frame earlier", self.frame.checked_sub(1)),
                ("Move frame later", (self.frame + 1 < self.source().clips[self.clip].frames.len()).then_some(self.frame + 1))] {
                if ui.add_enabled(clean && to.is_some(), egui::Button::new(label)).clicked() {
                    if let Some(to) = to {
                        match self.document.move_frame(self.frame, to) {
                            Ok(_) => { self.frame = to; self.reset_draft(); self.message = "Frame reordered".into(); }
                            Err(error) => self.message = error,
                        }
                    }
                }
            }
            if ui.add_enabled(clean && self.can_undo(), egui::Button::new("Undo clip edit")).clicked() {
                self.document.undo(); self.reset_draft(); self.message = "Clip edit undone".into();
            }
            if ui.add_enabled(clean && self.can_redo(), egui::Button::new("Redo clip edit")).clicked() {
                self.document.redo(); self.reset_draft(); self.message = "Clip edit redone".into();
            }
        });
        let version_label = ui.label("New package version");
        ui.add(egui::TextEdit::singleline(&mut self.next_version).char_limit(32)).labelled_by(version_label.id);
        let path_label = ui.label("New package directory");
        ui.add(egui::TextEdit::singleline(&mut self.destination).char_limit(4096)).labelled_by(path_label.id);
        if ui.add_enabled(clean, egui::Button::new("Save As new package")).clicked() { self.save_as(); }
        ui.label(&self.message);
        ui.label("Filesystem publication currently requires Linux. Failed private stages are retained for explicit recovery.");
    }
}
