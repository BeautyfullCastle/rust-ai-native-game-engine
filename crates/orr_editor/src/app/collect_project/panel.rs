//! A bounded explicit request, never an automatic retry or generated identity.
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    New,
    Open,
}

#[derive(Clone, Debug)]
pub(super) struct Request {
    pub path: PathBuf,
    pub create: Option<(String, String)>,
}

#[derive(Default)]
pub(in crate::app) struct Panel {
    kind: Option<Kind>,
    path: String,
    seed: String,
    game_id: String,
    error: Option<String>,
    created: Option<PathBuf>,
}

impl Panel {
    pub(super) fn open(&mut self, kind: Kind) {
        if self.kind.is_some() {
            return;
        }
        self.kind = Some(kind);
        self.error = None;
        // The recovery path remains visible until the user explicitly leaves it.
        // A fresh New request never inherits a successful creation for retry.
        self.created = None;
    }

    pub(super) fn is_open(&self) -> bool {
        self.kind.is_some()
    }

    pub(super) fn created(&mut self, path: PathBuf) {
        self.created = Some(path);
    }

    pub(super) fn failed(&mut self, error: String) {
        self.error = Some(error);
    }

    pub(super) fn request(&self) -> Result<Request, String> {
        if let Some(path) = &self.created {
            return Ok(Request {
                path: path.clone(),
                create: None,
            });
        }
        let path = PathBuf::from(&self.path);
        if self.path.is_empty()
            || self.path.len() > 4096
            || !path.is_absolute()
            || self.path.chars().any(char::is_control)
            || self.path.split('/').any(|part| matches!(part, "." | ".."))
            || path.components().count() > 64
        {
            return Err("Choose an absolute project directory (at most 4096 bytes and 64 components), without control characters or dot segments.".into());
        }
        let create = if self.kind == Some(Kind::New) {
            if self.seed.is_empty()
                || self.seed.len() > orr_sample::project_create::MAX_SEED_BYTES
                || !self
                    .seed
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            {
                return Err(
                    "Seed requires 1–128 ASCII letters, digits, dots, underscores or hyphens."
                        .into(),
                );
            }
            orr_package::ProjectProgress {
                schema: 1,
                game_id: self.game_id.clone(),
                profile: orr_package::ProgressProfile::CollectDodgeHighscoreV1,
            }
            .validate()?;
            Some((self.seed.clone(), self.game_id.clone()))
        } else {
            None
        };
        Ok(Request { path, create })
    }

    pub(super) fn show(
        &mut self,
        context: &egui::Context,
        blocked: Option<&str>,
    ) -> Option<Request> {
        let kind = self.kind?;
        let mut request = None;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("collect_project_dialog")).show(context, |ui| {
            ui.set_width(520.0);
            ui.heading(match kind { Kind::New => "New Collect project", Kind::Open => "Open Collect project" });
            if kind == Kind::New {
                ui.label("Create a playable Collect starter with sprites and Korean HUD/menu screens.");
                ui.label("Choose an absent absolute directory with an existing, non-symlink parent. Existing files are never overwritten.");
            } else {
                ui.label("Open an existing Collect project directory with its installed packages, sprites, UI and progress identity.");
            }
            if let Some(path) = &self.created {
                ui.label("Project created and retained at:");
                ui.label(path.display().to_string());
                ui.label("Opening failed. Use Open created project after resolving the error; creation will not run again.");
            } else {
                field(ui, "Project directory", "collect_project_path", &mut self.path, 4097);
                if kind == Kind::New {
                    field(ui, "Seed", "collect_project_seed", &mut self.seed, 129);
                    field(ui, "Game UUIDv4", "collect_project_game_id", &mut self.game_id, 37);
                    ui.label("Supply your own canonical lowercase UUIDv4. Reusing a UUID shares the high-score identity; the seed does not create a new identity.");
                    ui.weak("The editor reads identity metadata only. It does not read or write player high scores.");
                }
            }
            if let Some(reason) = blocked {
                ui.colored_label(ui.visuals().warn_fg_color, reason);
            }
            let parsed = self.request();
            if let Err(error) = &parsed {
                ui.label(error);
            }
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|ui| {
                let label = if self.created.is_some() { "Open created project" } else if kind == Kind::New { "Create and open" } else { "Open project" };
                if ui.add_enabled(blocked.is_none() && parsed.is_ok(), egui::Button::new(label)).clicked() {
                    request = parsed.ok();
                }
                cancel = ui.button("Cancel").clicked();
            });
        });
        // A dismissal consumes the frame. It never submits the request behind it.
        if cancel || context.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.kind = None;
            return None;
        }
        request
    }
}

fn field(ui: &mut egui::Ui, label: &str, id: &str, text: &mut String, limit: usize) {
    let labelled = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(text)
            .id_salt(id)
            .hint_text(label)
            .char_limit(limit)
            .desired_width(f32::INFINITY),
    )
    .labelled_by(labelled.id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Panel {
        Panel {
            kind: Some(Kind::New),
            path: "/tmp/collect-project".into(),
            seed: "project-1".into(),
            game_id: "bf3b65a7-752c-4c56-91bb-d76c5de01ce3".into(),
            ..Default::default()
        }
    }

    #[test]
    fn validates_explicit_seed_path_and_canonical_identity_without_repair() {
        assert!(valid().request().is_ok());
        for path in [
            "",
            "relative/project",
            "/tmp/../project",
            "/tmp/./project",
            "/tmp/line\nname",
        ] {
            let mut panel = valid();
            panel.path = path.into();
            assert!(panel.request().is_err());
        }
        for seed in ["", "with spaces", "한글"] {
            let mut panel = valid();
            panel.seed = seed.into();
            assert!(panel.request().is_err());
        }
        for uuid in [
            "",
            "BF3B65A7-752C-4C56-91BB-D76C5DE01CE3",
            "bf3b65a7-752c-5c56-91bb-d76c5de01ce3",
            "bf3b65a7-752c-4c56-71bb-d76c5de01ce3",
        ] {
            let mut panel = valid();
            panel.game_id = uuid.into();
            assert!(panel.request().is_err());
            assert_eq!(panel.game_id, uuid);
        }
        let mut panel = valid();
        panel.seed = "a".repeat(129);
        assert!(panel.request().is_err());
        panel = valid();
        panel.path = format!("/{}", "a".repeat(4096));
        assert!(panel.request().is_err());
    }

    #[test]
    fn repeated_open_preserves_pending_request_and_created_recovery() {
        let mut panel = valid();
        panel.created(PathBuf::from("/tmp/already-created"));
        panel.open(Kind::Open);
        assert_eq!(panel.kind, Some(Kind::New));
        let request = panel.request().unwrap();
        assert_eq!(request.path, PathBuf::from("/tmp/already-created"));
        assert!(request.create.is_none());
    }

    #[test]
    fn open_uses_explicit_path_without_seed_or_uuid() {
        let mut panel = valid();
        panel.kind = Some(Kind::Open);
        panel.seed.clear();
        panel.game_id.clear();
        let request = panel.request().unwrap();
        assert!(request.create.is_none());
        assert_eq!(request.path, PathBuf::from("/tmp/collect-project"));
    }
}
