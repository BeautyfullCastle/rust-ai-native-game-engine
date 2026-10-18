//! Read-only, editor-owned inspection of one verified installed static model.
//!
//! Importing and byte verification run on at most one worker. A single latest
//! request may wait behind it; closing never spawns another worker or retains a
//! decoded result. No scene, binding, package, or save operation is exposed here.
//! Package verification has the package store's trusted-directory concurrency
//! limits: the final worker check is not an atomic filesystem snapshot. A small
//! metadata pin rejects ordinary replacement between verification and delivery,
//! but cannot detect adversarial same-size, same-timestamp rewrites.

mod camera;
pub(crate) use camera::{bounded_size, PreviewBounds, PreviewCamera};

use crate::model_bindings::{self, LoadedAsset, ModelKind};
use orr_package::{Lock, Project};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    time::SystemTime,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreviewRequest {
    pub project_root: PathBuf,
    pub package_id: String,
    pub asset_path: String,
    pub digest: String,
    pub generation: u64,
}

pub(crate) struct Ready {
    pub loaded: LoadedAsset,
    pub bounds: PreviewBounds,
    pin: FreshnessPin,
}

struct Job {
    request: PreviewRequest,
    epoch: Arc<()>,
}

struct Completion {
    job: Job,
    result: Result<Ready, String>,
}

#[derive(Default)]
pub(crate) struct State {
    pub status: String,
    active: Option<Job>,
    ready: Option<Ready>,
    pending: Option<Receiver<Completion>>,
    queued: Option<Job>,
}

impl State {
    pub fn request(&mut self, request: PreviewRequest) {
        let epoch = Arc::new(());
        self.ready = None;
        self.queued = Some(Job {
            request: request.clone(),
            epoch: Arc::clone(&epoch),
        });
        self.active = Some(Job { request, epoch });
        self.status = "Loading and verifying static model preview…".into();
        self.start_queued();
    }

    pub fn active_request(&self) -> Option<&PreviewRequest> {
        self.active.as_ref().map(|job| &job.request)
    }

    pub fn ready(&self) -> Option<&Ready> {
        self.ready.as_ref()
    }

    pub fn busy(&self) -> bool {
        self.active.is_some() && (self.pending.is_some() || self.queued.is_some())
    }

    pub fn close(&mut self) {
        self.active = None;
        self.ready = None;
        self.queued = None;
        self.status.clear();
        // Keep the sole running worker's receiver until it finishes. Dropping it
        // here would allow repeated Close/Open clicks to create unbounded workers.
    }

    pub fn poll(&mut self, current_project: Option<&Path>) {
        if self
            .active_request()
            .is_some_and(|request| current_project != Some(request.project_root.as_path()))
        {
            self.close();
        }
        let completion = self.pending.as_ref().map(Receiver::try_recv);
        match completion {
            Some(Ok(completion)) => {
                self.pending = None;
                self.accept(completion);
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.pending = None;
                if self.active.is_some() && self.queued.is_none() {
                    self.status = "Static preview worker stopped before completing.".into();
                }
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }
        self.start_queued();
    }

    fn accept(&mut self, completion: Completion) {
        let Some(active) = &self.active else { return };
        if active.request != completion.job.request
            || !Arc::ptr_eq(&active.epoch, &completion.job.epoch)
        {
            return;
        }
        let result = completion.result.and_then(|ready| {
            ready.pin.check()?;
            Ok(ready)
        });
        match result {
            Ok(ready) => {
                self.status =
                    "Verified static model preview. Drag to orbit; scroll to zoom.".into();
                self.ready = Some(ready);
            }
            Err(error) => {
                self.ready = None;
                self.status = format!("Static preview unavailable: {error}");
            }
        }
    }

    fn start_queued(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let Some(job) = self.queued.take() else {
            return;
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("package-static-preview".into())
            .spawn(move || {
                let result = load(&job.request);
                let _ = sender.send(Completion { job, result });
            }) {
            Ok(_) => self.pending = Some(receiver),
            Err(error) => self.status = format!("Could not start static preview: {error}"),
        }
    }
}

fn open(request: &PreviewRequest) -> Result<Project, String> {
    if request.project_root.as_os_str().as_encoded_bytes().len() > 4096
        || !request.project_root.is_absolute()
        || request.package_id.len() > 128
        || request.asset_path.len() > 2048
        || request.digest.len() != 64
        || !request.digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("invalid bounded static preview request".into());
    }
    let project = Project::open(
        &request.project_root,
        orr_sample::room_project::compiled_runtime(),
    )
    .map_err(|error| format!("preview project: {error}"))?;
    if project.root() != request.project_root {
        return Err("preview project path identity changed".into());
    }
    Ok(project)
}

fn check_identity(request: &PreviewRequest, lock: &Lock) -> Result<(), String> {
    let package = lock
        .packages
        .get(&request.package_id)
        .ok_or("preview package was removed; verify packages again")?;
    if package.digest != request.digest {
        return Err("preview package digest changed; verify packages again".into());
    }
    if !package.manifest.capabilities.contains("models")
        || !package.manifest.files.contains(&request.asset_path)
    {
        return Err("preview requires a declared model asset in a models package".into());
    }
    if !Path::new(&request.asset_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["orrmodel", "glb", "gltf"]
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
    {
        return Err(
            "preview supports declared static .orrmodel, .glb, or .gltf assets only".into(),
        );
    }
    Ok(())
}

fn load(request: &PreviewRequest) -> Result<Ready, String> {
    let project = open(request)?;
    let lock = project.list().map_err(|error| error.to_string())?;
    check_identity(request, &lock)?;
    let loaded = model_bindings::load_asset_from_project(
        &project,
        &request.package_id,
        &request.asset_path,
        ModelKind::Static,
    )?;
    let model = loaded.static_model().ok_or("preview asset is not static")?;
    let bounds = PreviewBounds::from_model(model)?;
    let pin = verify_loaded(request, &loaded)?;
    Ok(Ready {
        loaded,
        bounds,
        pin,
    })
}

/// Run only on the worker. Reopen and verify after decoding, then pin the result
/// to the exact requested package and the loaded source hash. This also covers
/// glTF external resources through whole-project package verification.
fn verify_loaded(request: &PreviewRequest, loaded: &LoadedAsset) -> Result<FreshnessPin, String> {
    if loaded.kind() != ModelKind::Static
        || loaded.package() != request.package_id
        || loaded.asset() != request.asset_path
        || loaded.package_digest() != request.digest
    {
        return Err("decoded preview identity no longer matches its request".into());
    }
    let project = open(request)?;
    let before = project.list().map_err(|error| error.to_string())?;
    check_identity(request, &before)?;
    let pin = FreshnessPin::capture(request)?;
    let verified = project
        .verify()
        .map_err(|error| format!("preview verification: {error}"))?;
    check_identity(request, &verified)?;
    if verified != before
        || verified.packages[&request.package_id]
            .files
            .get(&request.asset_path)
            .map(String::as_str)
            != Some(loaded.source_hash())
    {
        return Err("preview package changed during final verification".into());
    }
    pin.check()?;
    Ok(pin)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    length: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
    directory: bool,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl FileStamp {
    fn read(path: &Path, directory: bool) -> Result<Self, String> {
        let metadata =
            fs::symlink_metadata(path).map_err(|error| format!("preview file changed: {error}"))?;
        #[cfg(windows)]
        let reparse = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x0400 != 0
        };
        #[cfg(not(windows))]
        let reparse = false;
        if metadata.file_type().is_symlink()
            || reparse
            || (directory && !metadata.is_dir())
            || (!directory && !metadata.is_file())
        {
            return Err("preview path changed to a link or unexpected file type".into());
        }
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified().map_err(|error| error.to_string())?,
            created: metadata.created().ok(),
            directory,
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (metadata.dev(), metadata.ino())
            },
        })
    }
}

struct FreshnessPin(Vec<(PathBuf, FileStamp)>);

impl FreshnessPin {
    fn capture(request: &PreviewRequest) -> Result<Self, String> {
        let object = request
            .project_root
            .join(".orr/packages/objects")
            .join(&request.digest);
        [
            (request.project_root.clone(), true),
            (request.project_root.join("orr.packages.lock.json"), false),
            (object.join("orr.package.json"), false),
            (object.join(&request.asset_path), false),
        ]
        .into_iter()
        .map(|(path, directory)| Ok((path.clone(), FileStamp::read(&path, directory)?)))
        .collect::<Result<Vec<_>, String>>()
        .map(Self)
    }

    fn check(&self) -> Result<(), String> {
        for (path, stamp) in &self.0 {
            if FileStamp::read(path, stamp.directory)? != *stamp {
                return Err(
                    "preview files changed after verification; preview the asset again".into(),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
