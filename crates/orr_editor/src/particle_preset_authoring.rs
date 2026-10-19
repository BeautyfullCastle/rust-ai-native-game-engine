//! Default-off standalone particle authoring, separate from project/runtime state.
use orr_sample::particle_preset::{ParticlePreset, PresetBlend};
use std::path::Path;

#[path = "particle_preset_authoring/widgets.rs"]
mod widgets;
pub use widgets::App;
#[cfg(feature = "particle-preset-preview")]
#[path = "particle_preset_authoring/preview.rs"]
pub mod preview;
#[cfg(feature = "particle-preset-preview")]
#[path = "particle_preset_authoring/preview_gpu.rs"]
pub mod preview_gpu;
const MAX_HISTORY: usize = 32;

/// Text remains untouched until explicit Apply, including malformed numeric input.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub schema: u32,
    pub rate: String,
    pub duration: String,
    pub burst: String,
    pub lifetime: String,
    pub velocity: [String; 3],
    pub velocity_jitter: [String; 3],
    pub size_start: String,
    pub size_end: String,
    pub color_start: [String; 4],
    pub color_end: [String; 4],
    pub blend: PresetBlend,
}
impl From<&ParticlePreset> for Draft {
    fn from(p: &ParticlePreset) -> Self {
        Self {
            schema: p.schema,
            rate: p.rate.to_string(),
            duration: p.duration.to_string(),
            burst: p.burst.to_string(),
            lifetime: p.lifetime.to_string(),
            velocity: p.velocity.map(|v| v.to_string()),
            velocity_jitter: p.velocity_jitter.map(|v| v.to_string()),
            size_start: p.size_start.to_string(),
            size_end: p.size_end.to_string(),
            color_start: p.color_start.map(|v| v.to_string()),
            color_end: p.color_end.map(|v| v.to_string()),
            blend: p.blend,
        }
    }
}
impl Draft {
    fn candidate(&self) -> Result<ParticlePreset, String> {
        fn number<T: std::str::FromStr>(s: &str, label: &str) -> Result<T, String> {
            s.trim()
                .parse()
                .map_err(|_| format!("Invalid {label}: enter a number."))
        }
        fn vector<const N: usize>(s: &[String; N], label: &str) -> Result<[f32; N], String> {
            let mut values = [0.0; N];
            for (index, text) in s.iter().enumerate() {
                values[index] = number(text, label)?;
            }
            Ok(values)
        }
        let candidate = ParticlePreset {
            schema: self.schema,
            rate: number(&self.rate, "rate")?,
            duration: number(&self.duration, "emitter duration")?,
            burst: number(&self.burst, "burst")?,
            lifetime: number(&self.lifetime, "lifetime")?,
            velocity: vector(&self.velocity, "velocity")?,
            velocity_jitter: vector(&self.velocity_jitter, "velocity jitter")?,
            size_start: number(&self.size_start, "start size")?,
            size_end: number(&self.size_end, "end size")?,
            color_start: vector(&self.color_start, "start RGBA")?,
            color_end: vector(&self.color_end, "end RGBA")?,
            blend: self.blend,
        };
        candidate.validate().map_err(|e| e.to_string())?;
        Ok(candidate)
    }
}

/// Accepted state is always validated; drafts are never implicitly saved.
pub struct Panel {
    document: ParticlePreset,
    draft: Draft,
    undo: Vec<ParticlePreset>,
    redo: Vec<ParticlePreset>,
    path: String,
    message: String,
}
impl Default for Panel {
    fn default() -> Self {
        let document = ParticlePreset::default();
        Self {
            draft: Draft::from(&document),
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            path: String::new(),
            message: String::new(),
        }
    }
}
impl Panel {
    pub fn document(&self) -> &ParticlePreset {
        &self.document
    }
    pub fn draft(&self) -> &Draft {
        &self.draft
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn has_pending_draft(&self) -> bool {
        *self.draft() != Draft::from(&self.document)
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    fn require_clean(&self) -> Result<(), String> {
        if self.has_pending_draft() {
            Err("Unapplied draft: Apply or Discard draft before continuing.".into())
        } else {
            Ok(())
        }
    }
    pub fn discard_draft(&mut self) {
        self.draft = Draft::from(&self.document);
    }
    fn accept(&mut self, candidate: ParticlePreset) {
        if candidate != self.document {
            if self.undo.len() == MAX_HISTORY {
                self.undo.remove(0);
            }
            self.undo
                .push(std::mem::replace(&mut self.document, candidate));
            self.redo.clear();
        }
        self.discard_draft();
    }
    pub fn apply_draft(&mut self) -> Result<(), String> {
        let candidate = self.draft.candidate()?;
        self.accept(candidate);
        Ok(())
    }
    pub fn use_defaults(&mut self) -> Result<(), String> {
        self.require_clean()?;
        self.accept(ParticlePreset::default());
        Ok(())
    }
    pub fn undo(&mut self) -> Result<bool, String> {
        self.require_clean()?;
        let Some(previous) = self.undo.pop() else {
            return Ok(false);
        };
        self.redo
            .push(std::mem::replace(&mut self.document, previous));
        self.discard_draft();
        Ok(true)
    }
    pub fn redo(&mut self) -> Result<bool, String> {
        self.require_clean()?;
        let Some(next) = self.redo.pop() else {
            return Ok(false);
        };
        self.undo.push(std::mem::replace(&mut self.document, next));
        self.discard_draft();
        Ok(true)
    }
    pub fn open(&mut self, path: &Path) -> Result<(), String> {
        self.require_clean()?;
        let candidate = ParticlePreset::open(path).map_err(|e| e.to_string())?;
        self.accept(candidate);
        Ok(())
    }
    /// Shared create-new publication never overwrites. Only accepted bytes are saved.
    pub fn save_as(&self, path: &Path) -> Result<(), String> {
        self.document.save_as(path).map_err(|e| e.to_string())
    }
}
#[cfg(test)]
#[path = "particle_preset_authoring/tests.rs"]
mod tests;
