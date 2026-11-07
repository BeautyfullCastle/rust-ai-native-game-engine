//! Explicit read-only preparation; no license, destination, lock or publication.
use super::*;
use crate::model_package_import::variant_preview::{self as backend, Prepared, Spec};
use std::sync::Arc;
#[derive(Clone)]
pub(crate) struct PreviewFrame {
    pub ticket: u64,
    pub prepared: Arc<Prepared>,
}
#[derive(Clone, Debug, PartialEq)]
struct Identity {
    root: PathBuf,
    project: u64,
    generation: u64,
    request: backend::Request,
}
struct Completion {
    identity: Identity,
    ticket: u64,
    result: Result<Prepared, String>,
}
#[derive(Default)]
pub(super) struct State {
    pending: Option<Receiver<Completion>>,
    identity: Option<Identity>,
    ready: Option<PreviewFrame>,
    retired: bool,
    ticket: u64,
    status: String,
}
impl State {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
}
impl Panel {
    fn preview_request(&self) -> Result<backend::Request, String> {
        if self.source_format != SourceFormat::CookedAnimated
            || self.kind != ModelKind::Animated
            || self.source.trim().is_empty()
        {
            return Err("Choose a cooked animated source before previewing".into());
        }
        let spec = match (self.color_request()?, self.texture_request()?) {
            (Some(color), None) => Spec::Color(color),
            (None, Some(texture)) => Spec::Texture(texture),
            _ => {
                return Err(
                    "Choose one animated color variant or PNG skin before previewing".into(),
                )
            }
        };
        Ok(backend::Request {
            source: PathBuf::from(self.source.trim()),
            spec,
            expected_source_sha256: self.inspected_hash.clone(),
        })
    }
    fn preview_identity(
        &self,
        root: Option<&Path>,
        project: u64,
        generation: u64,
    ) -> Option<Identity> {
        Some(Identity {
            root: root?.into(),
            project,
            generation,
            request: self.preview_request().ok()?,
        })
    }
    pub(crate) fn preview_frame(&mut self) -> Option<PreviewFrame> {
        let request = self.preview_request().ok();
        if self
            .variant_preview
            .identity
            .as_ref()
            .is_some_and(|id| Some(&id.request) != request.as_ref())
        {
            self.close_variant_preview();
        }
        if self.variant_preview.retired {
            return None;
        }
        self.variant_preview.ready.clone()
    }
    pub(crate) fn close_variant_preview(&mut self) {
        self.variant_preview.ready = None;
        self.variant_preview.identity = None;
        self.variant_preview.retired = true;
        self.variant_preview.status.clear();
        // Keep an in-flight receiver until completion: closing cannot spawn a
        // second worker or accept a retired result.
    }
    pub(super) fn service_variant_preview(
        &mut self,
        root: Option<&Path>,
        project: u64,
        generation: u64,
    ) {
        let current = self.preview_identity(root, project, generation);
        if self.variant_preview.retired || self.variant_preview.identity != current {
            self.variant_preview.ready = None;
            self.variant_preview.identity = None;
            self.variant_preview.status.clear();
        }
        let completion = match self
            .variant_preview
            .pending
            .as_ref()
            .map(|receiver| receiver.try_recv())
        {
            Some(Ok(value)) => Some(value),
            Some(Err(TryRecvError::Disconnected)) => {
                self.variant_preview.pending = None;
                self.variant_preview.status =
                    "Variant preview worker disconnected; retry explicitly.".into();
                None
            }
            _ => None,
        };
        if let Some(completion) = completion {
            self.variant_preview.pending = None;
            if self.variant_preview.retired
                || current.as_ref() != Some(&completion.identity)
                || self.variant_preview.identity.as_ref() != Some(&completion.identity)
            {
                return;
            }
            match completion.result {
                Ok(prepared) => {
                    self.variant_preview.ready = Some(PreviewFrame {
                        ticket: completion.ticket,
                        prepared: Arc::new(prepared),
                    });
                    self.variant_preview.status="Read-only variant preview ready; nothing was imported or assigned. License acknowledgement is still required for import.".into();
                }
                Err(error) => {
                    self.variant_preview.ready = None;
                    self.variant_preview.status =
                        format!("Animated variant preview failed: {error}");
                }
            }
        }
    }
    fn start_variant_preview(&mut self, root: Option<&Path>, project: u64, generation: u64) {
        if self.busy() {
            return;
        }
        let Some(identity) = self.preview_identity(root, project, generation) else {
            return;
        };
        let worker = identity.clone();
        let Some(ticket) = self.variant_preview.ticket.checked_add(1) else {
            self.variant_preview.status = "Variant preview ticket exhausted".into();
            return;
        };
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("orr-animated-variant-preview".into())
            .spawn(move || {
                let result = backend::prepare(worker.request.clone());
                let _ = sender.send(Completion {
                    identity: worker,
                    ticket,
                    result,
                });
            }) {
            Ok(_) => {
                self.variant_preview.pending = Some(receiver);
                self.variant_preview.identity = Some(identity);
                self.variant_preview.ready = None;
                self.variant_preview.retired = false;
                self.variant_preview.ticket = ticket;
                self.variant_preview.status =
                    "Preparing animated variant on a read-only worker...".into();
            }
            Err(error) => {
                self.variant_preview.status = format!("Cannot start variant preview: {error}")
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn hold_variant_preview_for_test(&mut self) -> Box<dyn FnOnce()> {
        self.source_format = SourceFormat::CookedAnimated;
        self.kind = ModelKind::Animated;
        self.source = "/held-preview.animated.json".into();
        self.color_variant_enabled = true;
        self.texture_variant_enabled = false;
        self.color_variant = super::default_color_variant();
        let (sender, receiver) = mpsc::channel::<Completion>();
        self.variant_preview.pending = Some(receiver);
        self.variant_preview.retired = false;
        Box::new(move || drop(sender))
    }

    pub(super) fn show_variant_preview(
        &mut self,
        ui: &mut Ui,
        enabled: bool,
        root: Option<&Path>,
        project: u64,
        generation: u64,
    ) {
        self.service_variant_preview(root, project, generation);
        if (self.variant_preview.pending.is_some() || self.variant_preview.ready.is_some())
            && ui.button("Cancel variant preview").clicked()
        {
            self.close_variant_preview();
        }
        if self.source_format != SourceFormat::CookedAnimated || self.kind != ModelKind::Animated {
            return;
        }
        if ui
            .add_enabled(
                enabled
                    && !self.busy()
                    && self.preview_identity(root, project, generation).is_some(),
                egui::Button::new("Preview animated variant"),
            )
            .clicked()
        {
            self.start_variant_preview(root, project, generation);
        }
        if !self.variant_preview.status.is_empty() {
            ui.label(&self.variant_preview.status);
        }
    }
}
#[cfg(test)]
#[path = "variant_preview_tests.rs"]
mod tests;
