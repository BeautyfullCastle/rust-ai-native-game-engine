//! Editor-owned UI history. Saving atomically replaces only the admitted UI
//! document; it is not a transaction with the scene or project manifest.
use orr_sample::{
    authored_ui::{Action, Binding, Document, Kind, Node, Screen, MAX_BYTES, MAX_NODES},
    collect_project::PreparedCollectUi,
    collect_ui::CollectUi,
};
use std::{
    collections::BTreeSet,
    fs::{File, Metadata},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_UNDO: usize = 32;

pub struct Panel {
    path: PathBuf,
    root: Result<RootPin, String>,
    document: Document,
    saved: Document,
    font: Vec<u8>,
    undo: Vec<Document>,
    selected: Option<usize>,
    error: Option<String>,
}

struct RootPin {
    path: PathBuf,
    /// Keep the admitted directory alive and compare its filesystem identity.
    directory: File,
}

impl RootPin {
    fn new(parent: &Path) -> Result<Self, String> {
        let path = checked_directory(parent)?;
        let directory = File::open(&path).map_err(|e| e.to_string())?;
        Ok(Self { path, directory })
    }
    fn recheck(&self, parent: &Path) -> Result<(), String> {
        let current = checked_directory(parent)?;
        let metadata = std::fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
        let pinned = self.directory.metadata().map_err(|e| e.to_string())?;
        if current != self.path || !same_directory(&metadata, &pinned) {
            return Err("UI document directory changed; reopen the project before saving".into());
        }
        Ok(())
    }
}

#[cfg(unix)]
fn same_directory(a: &Metadata, b: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}
#[cfg(not(unix))]
fn same_directory(a: &Metadata, b: &Metadata) -> bool {
    // If the platform cannot report creation identity, fail closed on Save.
    matches!((a.created(), b.created()), (Ok(a), Ok(b)) if a == b)
}

fn checked_directory(path: &Path) -> Result<PathBuf, String> {
    crate::sprite_bindings::resolve_project(path, ".")
        .and_then(|path| std::fs::canonicalize(path).map_err(|e| e.to_string()))
}

fn checked_file(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_BYTES as u64
    {
        return Err("UI document must remain a regular, non-symlink file within 64 KiB".into());
    }
    Ok(())
}

impl Panel {
    pub fn new(prepared: PreparedCollectUi) -> Self {
        let parent = prepared.path.parent().unwrap_or(Path::new("."));
        let root = RootPin::new(parent);
        let error = root.as_ref().err().cloned();
        Self {
            path: prepared.path,
            root,
            saved: prepared.document.clone(),
            document: prepared.document,
            font: prepared.font,
            undo: Vec::new(),
            selected: None,
            error,
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn apply_document(&mut self, candidate: Document) -> Result<(), String> {
        // Validate the entire candidate before changing the live document/history.
        CollectUi::validate_font(&self.font, &candidate)?;
        candidate.to_bytes()?;
        if candidate != self.document {
            if self.undo.len() == MAX_UNDO {
                self.undo.remove(0);
            }
            self.undo
                .push(std::mem::replace(&mut self.document, candidate));
            self.selected = self
                .selected
                .filter(|index| *index < self.document.nodes.len());
        }
        Ok(())
    }

    pub fn undo(&mut self) -> bool {
        if let Some(previous) = self.undo.pop() {
            self.document = previous;
            self.selected = self
                .selected
                .filter(|index| *index < self.document.nodes.len());
            true
        } else {
            false
        }
    }

    pub fn save(&mut self) -> Result<(), String> {
        CollectUi::validate_font(&self.font, &self.document)?;
        let bytes = self.document.to_bytes()?;
        let parent = self.path.parent().unwrap_or(Path::new("."));
        let pin = self.root.as_ref().map_err(Clone::clone)?;
        pin.recheck(parent)?;
        checked_file(&self.path)?;
        // Do not overwrite a separately edited document without reopening it.
        let mut current = Vec::new();
        File::open(&self.path)
            .map_err(|e| e.to_string())?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut current)
            .map_err(|e| e.to_string())?;
        if Document::parse(&current)? != self.saved {
            return Err("UI document changed on disk; reopen the project before saving".into());
        }
        let mut temp = tempfile::NamedTempFile::new_in(&pin.path).map_err(|e| e.to_string())?;
        temp.write_all(&bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        // Recheck after writing, before the single-file atomic replacement. These
        // checks detect changed paths; they do not claim hostile-race isolation.
        pin.recheck(parent)?;
        checked_file(&self.path)?;
        temp.persist(&self.path).map_err(|e| e.to_string())?;
        self.saved = self.document.clone();
        Ok(())
    }

    pub fn show(&mut self, ui: &mut egui::Ui, editable: bool) {
        ui.collapsing("Collect UI document", |ui| {
            ui.label(format!(
                "{} nodes{}",
                self.document.nodes.len(),
                if self.document != self.saved {
                    " (unsaved)"
                } else {
                    ""
                }
            ));
            if !editable {
                ui.label("UI authoring is read-only during Play");
            }
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo UI edit"))
                        .clicked()
                    {
                        self.undo();
                        self.error = None;
                    }
                    if ui.button("Save UI document").clicked() {
                        self.error = self.save().err();
                    }
                });
            });
            egui::ComboBox::new("collect_ui_selected", "UI node")
                .selected_text(
                    self.selected
                        .and_then(|i| self.document.nodes.get(i))
                        .map(|n| n.id.as_str())
                        .unwrap_or("Select node"),
                )
                .show_ui(ui, |ui| {
                    for (index, node) in self.document.nodes.iter().enumerate() {
                        ui.selectable_value(&mut self.selected, Some(index), &node.id);
                    }
                });
            ui.add_enabled_ui(editable, |ui| {
                let mut candidate = self.document.clone();
                let mut changed = false;
                if let Some(index) = self.selected.filter(|i| *i < candidate.nodes.len()) {
                    let node = &mut candidate.nodes[index];
                    ui.label(format!("Stable id: {}", node.id));
                    ui.label(format!(
                        "Parent: {}",
                        node.parent.as_deref().unwrap_or("viewport")
                    ));
                    changed |= choice(
                        ui,
                        "Screen",
                        &mut node.screen,
                        &[
                            Screen::Title,
                            Screen::Playing,
                            Screen::Menu,
                            Screen::Terminal,
                        ],
                    );
                    for (label, values) in [
                        ("Anchor (0..1000)", &mut node.anchor),
                        ("Size (1..4096)", &mut node.size),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            for value in values {
                                changed |= ui.add(egui::DragValue::new(value)).changed();
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label("Offset (-4096..4096)");
                        for value in &mut node.offset {
                            changed |= ui.add(egui::DragValue::new(value)).changed();
                        }
                    });
                    match &mut node.kind {
                        Kind::Label { text, binding } => {
                            ui.label("Literal text (128 characters max)");
                            changed |= ui.text_edit_singleline(text).changed();
                            changed |= choice(
                                ui,
                                "Binding",
                                binding,
                                &[
                                    None,
                                    Some(Binding::Score),
                                    Some(Binding::Phase),
                                    Some(Binding::Best),
                                ],
                            );
                        }
                        Kind::Button { text, action } => {
                            ui.label("Button text (128 characters max)");
                            changed |= ui.text_edit_singleline(text).changed();
                            changed |= choice(
                                ui,
                                "Action",
                                action,
                                &[
                                    Action::Play,
                                    Action::Menu,
                                    Action::Continue,
                                    Action::Restart,
                                    Action::Quit,
                                ],
                            );
                        }
                        Kind::Container => {
                            ui.label("Container: children inherit this coordinate space");
                        }
                    }
                    if ui.button("Remove node and descendants").clicked() {
                        remove_subtree(&mut candidate, index);
                        self.selected = None;
                        changed = true;
                    }
                }
                ui.horizontal(|ui| {
                    for (name, kind) in [
                        (
                            "Add label",
                            Kind::Label {
                                text: "Label".into(),
                                binding: None,
                            },
                        ),
                        (
                            "Add button",
                            Kind::Button {
                                text: "Play".into(),
                                action: Action::Play,
                            },
                        ),
                        ("Add container", Kind::Container),
                    ] {
                        if ui
                            .add_enabled(candidate.nodes.len() < MAX_NODES, egui::Button::new(name))
                            .clicked()
                        {
                            add_node(&mut candidate, self.selected, kind);
                            self.selected = Some(candidate.nodes.len() - 1);
                            changed = true;
                        }
                    }
                });
                if changed {
                    self.error = self.apply_document(candidate).err();
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, error);
            }
            ui.label("Saves this UI file only. Scene and project metadata are unchanged.");
        });
    }
}

fn choice<T: Copy + PartialEq + std::fmt::Debug>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    options: &[T],
) -> bool {
    let before = *value;
    egui::ComboBox::from_id_salt(("collect_ui", label))
        .selected_text(format!("{label}: {value:?}"))
        .show_ui(ui, |ui| {
            for option in options {
                ui.selectable_value(value, *option, format!("{option:?}"));
            }
        });
    *value != before
}

fn add_node(document: &mut Document, selected: Option<usize>, kind: Kind) {
    let selected = selected.and_then(|index| document.nodes.get(index));
    let parent = selected
        .filter(|node| matches!(node.kind, Kind::Container))
        .map(|node| node.id.clone());
    let screen = selected.map(|node| node.screen).unwrap_or(Screen::Title);
    let id = (1..=MAX_NODES + 1)
        .map(|index| format!("ui_{index}"))
        .find(|id| document.nodes.iter().all(|node| node.id != *id))
        .expect("bounded nodes leave a free id");
    document.nodes.push(Node {
        id,
        parent,
        kind,
        screen,
        anchor: [0, 0],
        offset: [16, 16],
        size: [200, 36],
    });
}

fn remove_subtree(document: &mut Document, index: usize) {
    let mut removed = BTreeSet::from([document.nodes[index].id.clone()]);
    // Validated order guarantees every parent is encountered before children.
    document.nodes.retain(|node| {
        if removed.contains(&node.id) || node.parent.as_ref().is_some_and(|id| removed.contains(id))
        {
            removed.insert(node.id.clone());
            false
        } else {
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PreparedCollectUi) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("collect.ui.json");
        let document = Document::default_collect();
        std::fs::write(&path, document.to_bytes().unwrap()).unwrap();
        let font = include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec();
        (
            dir,
            PreparedCollectUi {
                path,
                document,
                font,
            },
        )
    }
    /// Acceptance through rendered widgets, not calls to apply_document/save.
    #[test]
    fn headless_widgets_edit_undo_save_reopen_and_share_runtime_document() {
        use egui::accesskit::Role;
        use egui_kittest::{kittest::Queryable, Harness};
        let (_dir, prepared) = fixture();
        let path = prepared.path.clone();
        let font = prepared.font.clone();
        let original = prepared.document.clone();
        let panel = Panel::new(prepared);
        let mut installed = false;
        let installed_font = font.clone();
        let mut h = Harness::builder()
            .with_size([1000.0, 1000.0])
            .build_ui_state(
                move |ui, panel: &mut Panel| {
                    if !installed {
                        let mut definitions = egui::FontDefinitions::default();
                        definitions.font_data.insert(
                            "collect-editor-test".into(),
                            egui::FontData::from_owned(installed_font.clone()).into(),
                        );
                        definitions
                            .families
                            .entry(egui::FontFamily::Proportional)
                            .or_default()
                            .insert(0, "collect-editor-test".into());
                        ui.ctx().set_fonts(definitions);
                        installed = true;
                    }
                    panel.show(ui, true);
                },
                panel,
            );
        h.run_steps(3);
        h.get_by_label("Collect UI document").click();
        h.run_steps(3);
        h.get_by_role_and_label(Role::ComboBox, "UI node").click();
        h.run_steps(3);
        h.get_by_label("title").click();
        h.run_steps(3);

        // Select-all and type through the accessible TextInput. DragValue
        // controls have their own spin-button role, so this is the text field.
        h.get_by_role(Role::TextInput).focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(2);
        h.get_by_role(Role::TextInput)
            .type_text("Edited through UI");
        h.run_steps(3);
        let edited = h.state().document().clone();
        assert!(
            matches!(&edited.nodes[0].kind, Kind::Label { text, .. } if text == "Edited through UI")
        );
        assert_ne!(edited, original);
        assert!(h.state().error.is_none(), "{:?}", h.state().error);

        h.get_by_label("Undo UI edit").click();
        h.run_steps(3);
        assert_eq!(h.state().document(), &original);

        h.get_by_role(Role::TextInput).focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(2);
        h.get_by_role(Role::TextInput)
            .type_text("Edited through UI");
        h.run_steps(3);
        assert_eq!(h.state().document(), &edited);
        h.get_by_label("Save UI document").click();
        h.run_steps(3);
        assert!(h.state().error.is_none(), "{:?}", h.state().error);
        let reopened = Document::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(reopened, edited);
        let runtime = CollectUi::new(reopened.clone(), font.clone()).unwrap();
        assert_eq!(runtime.document(), &edited);
        let reopened_panel = Panel::new(PreparedCollectUi {
            path,
            document: reopened,
            font,
        });
        assert_eq!(reopened_panel.document(), runtime.document());

        // Actual Add/Remove widgets maintain the same bounded history.
        h.get_by_label("Add label").click();
        h.run_steps(3);
        assert_eq!(h.state().document().nodes.len(), edited.nodes.len() + 1);
        h.get_by_label("Remove node and descendants").click();
        h.run_steps(3);
        assert_eq!(h.state().document(), &edited);
        h.get_by_label("Undo UI edit").click();
        h.run_steps(3);
        assert_eq!(h.state().document().nodes.len(), edited.nodes.len() + 1);
    }

    #[test]
    fn property_edit_undo_save_and_reopen() {
        let (_dir, prepared) = fixture();
        let path = prepared.path.clone();
        let mut panel = Panel::new(prepared.clone());
        let mut edited = panel.document().clone();
        if let Kind::Label { text, .. } = &mut edited.nodes[0].kind {
            *text = "New title".into();
        }
        panel.apply_document(edited.clone()).unwrap();
        assert!(panel.undo());
        assert_eq!(panel.document(), &prepared.document);
        panel.apply_document(edited.clone()).unwrap();
        panel.save().unwrap();
        let reopened = Document::parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(reopened, edited);
        assert!(panel.undo());
        panel.save().unwrap();
    }
    #[test]
    fn invalid_or_missing_font_glyph_edit_is_atomic_and_history_bounded() {
        let (_dir, prepared) = fixture();
        let mut panel = Panel::new(prepared);
        let before = panel.document().clone();
        let mut invalid = before.clone();
        invalid.nodes[0].size = [0, 1];
        assert!(panel.apply_document(invalid).is_err());
        let mut unsupported = before.clone();
        unsupported.nodes[0].kind = Kind::Label {
            text: "🦄".into(),
            binding: None,
        };
        assert!(panel.apply_document(unsupported).is_err());
        assert_eq!(panel.document(), &before);
        assert!(!panel.undo());
        for index in 0..40 {
            let mut candidate = panel.document().clone();
            candidate.nodes[0].offset[0] = index;
            panel.apply_document(candidate).unwrap();
        }
        assert_eq!(panel.undo.len(), MAX_UNDO);
    }
    #[test]
    fn subtree_removal_and_bounded_add_are_validated() {
        let (_dir, prepared) = fixture();
        let mut panel = Panel::new(prepared);
        let mut document = Document {
            schema: 1,
            nodes: Vec::new(),
        };
        add_node(&mut document, None, Kind::Container);
        add_node(&mut document, Some(0), Kind::Container);
        add_node(
            &mut document,
            Some(1),
            Kind::Label {
                text: "child".into(),
                binding: None,
            },
        );
        panel.apply_document(document.clone()).unwrap();
        remove_subtree(&mut document, 0);
        assert!(document.nodes.is_empty());
        panel.apply_document(document).unwrap();
        assert!(panel.undo());
        assert_eq!(panel.document().nodes.len(), 3);
    }
    #[test]
    fn save_rejects_external_edit_and_nonregular_target() {
        let (_dir, prepared) = fixture();
        let path = prepared.path.clone();
        let mut panel = Panel::new(prepared);
        std::fs::write(&path, br#"{"schema":1,"nodes":[]}"#).unwrap();
        assert!(panel.save().is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(panel.save().is_err());
    }
    #[cfg(unix)]
    #[test]
    fn save_rejects_symlink_and_replaced_directory() {
        let (dir, prepared) = fixture();
        let path = prepared.path.clone();
        let mut panel = Panel::new(prepared.clone());
        let other = dir.path().join("other.json");
        std::fs::rename(&path, &other).unwrap();
        std::os::unix::fs::symlink(&other, &path).unwrap();
        assert!(panel.save().is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(&other, &path).unwrap();
        let moved = dir.path().with_extension("moved");
        std::fs::rename(dir.path(), &moved).unwrap();
        std::fs::create_dir(dir.path()).unwrap();
        std::fs::write(&path, prepared.document.to_bytes().unwrap()).unwrap();
        assert!(panel.save().is_err());
        std::fs::remove_dir_all(moved).unwrap();
    }
}
