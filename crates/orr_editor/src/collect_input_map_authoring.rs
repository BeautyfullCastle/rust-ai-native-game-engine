//! Optional standalone Collect input-map authoring; no project or runtime writes.
//!
//! Only complete maps admitted by Collect's production adapter become accepted
//! documents. A selected action's draft is separate and is never saved implicitly.
use orr_input::{Action, ActionMap, Button, MAX_BINDINGS};
use orr_sample::collect_app::input_actions::{default_map, CollectBindings};
use std::path::Path;

#[path = "collect_input_map_authoring/widgets.rs"]
mod widgets;
pub use widgets::App;

const MAX_HISTORY: usize = 32;
const PENDING_DRAFT: &str =
    "Unapplied action draft: Apply action or Discard draft before continuing.";

/// A validated accepted document and one explicitly applied or discarded draft.
pub struct Panel {
    document: ActionMap,
    undo: Vec<ActionMap>,
    redo: Vec<ActionMap>,
    selected: usize,
    draft: Vec<Button>,
    path: String,
    message: String,
}

impl Default for Panel {
    fn default() -> Self {
        let document = default_map();
        Self {
            draft: document.actions[0].bindings.clone(),
            document,
            undo: Vec::new(),
            redo: Vec::new(),
            selected: 0,
            path: String::new(),
            message: String::new(),
        }
    }
}

impl Panel {
    pub fn document(&self) -> &ActionMap {
        &self.document
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    pub fn selected_action(&self) -> &Action {
        &self.document.actions[self.selected]
    }

    pub fn selected_action_index(&self) -> usize {
        self.selected
    }

    pub fn draft(&self) -> &[Button] {
        &self.draft
    }

    pub fn has_pending_draft(&self) -> bool {
        self.draft != self.selected_action().bindings
    }

    /// Report stored history, even while a pending draft prevents navigating it.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn require_clean_draft(&self) -> Result<(), String> {
        if self.has_pending_draft() {
            Err(PENDING_DRAFT.into())
        } else {
            Ok(())
        }
    }

    /// Changing action never discards a pending draft. Reselecting it is harmless.
    pub fn select_action(&mut self, index: usize) -> Result<(), String> {
        if index >= self.document.actions.len() {
            return Err("unknown Collect action selection".into());
        }
        if index != self.selected {
            self.require_clean_draft()?;
            self.selected = index;
            self.discard_draft();
        }
        Ok(())
    }

    /// Drafts may conflict until Apply; only keyboard choices and size are bounded.
    /// Refusal preserves the previous draft, document and history.
    pub fn replace_draft(&mut self, draft: Vec<Button>) -> Result<(), String> {
        if draft.len() > MAX_BINDINGS {
            return Err(format!("at most {MAX_BINDINGS} keys per Collect action"));
        }
        if draft
            .iter()
            .any(|binding| !matches!(binding, Button::Keyboard { .. }))
        {
            return Err("Collect supports keyboard bindings only".into());
        }
        self.draft = draft;
        Ok(())
    }

    /// The sole explicit operation that discards unaccepted draft changes.
    pub fn discard_draft(&mut self) {
        self.draft = self.selected_action().bindings.clone();
    }

    fn validate(candidate: &ActionMap) -> Result<(), String> {
        CollectBindings::from_map(candidate)?;
        // Keep the shared format's encoded-byte limit as well as its schema limits.
        candidate.save(Vec::new())
    }

    /// Called only after validation. The selected semantic action survives imports
    /// with a different action order, and every accepted replacement is undoable.
    fn accept(&mut self, candidate: ActionMap) {
        let selected_name = self.selected_action().name.clone();
        if candidate != self.document {
            if self.undo.len() == MAX_HISTORY {
                self.undo.remove(0);
            }
            self.undo
                .push(std::mem::replace(&mut self.document, candidate));
            self.redo.clear();
        }
        self.selected = self
            .document
            .actions
            .iter()
            .position(|action| action.name == selected_name)
            .expect("validated Collect action set");
        self.discard_draft();
    }

    /// Validate the entire candidate map before changing accepted state or history.
    /// Failure, including a conflict with another action, also preserves the draft.
    pub fn apply_draft(&mut self) -> Result<(), String> {
        let mut candidate = self.document.clone();
        candidate.actions[self.selected]
            .bindings
            .clone_from(&self.draft);
        Self::validate(&candidate)?;
        self.accept(candidate);
        Ok(())
    }

    /// Explicit complete-map replacement. Pending drafts must be resolved first.
    pub fn apply_document(&mut self, candidate: ActionMap) -> Result<(), String> {
        self.require_clean_draft()?;
        Self::validate(&candidate)?;
        self.accept(candidate);
        Ok(())
    }

    /// Defaults are one undoable accepted-map edit, never an implicit draft discard.
    pub fn use_defaults(&mut self) -> Result<(), String> {
        self.apply_document(default_map())
    }

    fn restore(&mut self, map: ActionMap) -> ActionMap {
        let selected_name = self.selected_action().name.clone();
        let previous = std::mem::replace(&mut self.document, map);
        self.selected = self
            .document
            .actions
            .iter()
            .position(|action| action.name == selected_name)
            .expect("validated Collect action set");
        self.discard_draft();
        previous
    }

    pub fn undo(&mut self) -> Result<bool, String> {
        self.require_clean_draft()?;
        let Some(previous) = self.undo.pop() else {
            return Ok(false);
        };
        let current = self.restore(previous);
        self.redo.push(current);
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool, String> {
        self.require_clean_draft()?;
        let Some(next) = self.redo.pop() else {
            return Ok(false);
        };
        let current = self.restore(next);
        // Undo and Redo only transfer entries in the same bounded history.
        self.undo.push(current);
        Ok(true)
    }

    /// Open is an undoable replacement after full admission. The shared loader
    /// rejects direct symlinks, special files and files over 64 KiB. Concurrent
    /// hostile path replacement is outside that local-file loader's boundary.
    pub fn open(&mut self, path: &Path) -> Result<(), String> {
        self.require_clean_draft()?;
        self.apply_document(ActionMap::load_file(path)?)
    }

    /// Save only accepted state, preserving any pending draft and all history.
    /// Uses same-directory staging and a create-new hard-link publication; existing
    /// paths are never overwritten. Filesystems without hard links return an error.
    /// A cleanup error may be reported after publication, as the shared API states.
    pub fn save_as(&self, path: &Path) -> Result<(), String> {
        Self::validate(&self.document)?;
        self.document.save_new(path)
    }
}

#[cfg(test)]
#[path = "collect_input_map_authoring/tests.rs"]
mod tests;
