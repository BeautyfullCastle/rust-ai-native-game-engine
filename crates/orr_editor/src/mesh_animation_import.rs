//! Bounded raw GLB or explicitly cooked animation packaging for trusted local directories.
//! Path checks and no-replace publication are not a hostile-filesystem sandbox.
use crate::mesh_animation::Snapshot;
use crate::mesh_art_import::{
    checked_directory, checked_new_destination, publish_new_directory, read_input_file,
    recheck_input, refuse_existing, validate_package_identity, write_new_file,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::{Path, PathBuf},
};

#[cfg(test)]
#[path = "mesh_animation_import/tests.rs"]
mod tests;

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LICENSE_BYTES: u64 = 1024 * 1024;
const ASSET: &str = "animated.glb";
const COOKED_ASSET: &str = "model.animated.json";
fn source_asset(path: &Path) -> Result<&'static str, String> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("animation source filename must be UTF-8")?;
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".glb") {
        Ok(ASSET)
    } else if lower.ends_with(".animated.json") || lower.ends_with(".orrmodel.json") {
        Ok(COOKED_ASSET)
    } else {
        Err("mesh animation import requires .glb, .animated.json or .orrmodel.json source".into())
    }
}
const LICENSE: &str = "LICENSE.txt";
const MANIFEST: &str = "orr.package.json";

pub(crate) struct Request {
    pub source: PathBuf,
    pub license: PathBuf,
    pub destination: PathBuf,
    pub name: String,
    pub version: String,
    pub license_acknowledged: bool,
    pub graph_revision: [u8; 32],
}

pub(crate) struct Report {
    pub root: PathBuf,
    pub asset: String,
    pub license: String,
    pub source_sha256: String,
    pub license_sha256: String,
    pub snapshot: Snapshot,
    /// Publication succeeded, but durability or readback could not be confirmed.
    pub publication_warning: Option<String>,
}

// Holding handles prevents identity reuse after an unlinked inode is replaced.
struct Retained {
    handle: File,
    identity: (u64, u64),
}

impl Retained {
    fn open(path: &Path) -> Result<Self, String> {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0200_0000 | 0x0020_0000);
        }
        let handle = options
            .open(path)
            .map_err(|error| format!("retain file identity: {error}"))?;
        let identity = file_identity(&handle)?;
        Ok(Self { handle, identity })
    }

    fn matches(&self, path: &Path) -> bool {
        file_identity(&self.handle).ok() == Some(self.identity)
            && Self::open(path).is_ok_and(|current| current.identity == self.identity)
    }
}

#[cfg(unix)]
fn file_identity(file: &File) -> Result<(u64, u64), String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn file_identity(file: &File) -> Result<(u64, u64), String> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    #[derive(Default)]
    struct Information {
        attributes: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        written: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Information)
            -> i32;
    }
    let mut info = Information::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(format!(
            "read file identity: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok((
        u64::from(info.volume),
        (u64::from(info.index_high) << 32) | u64::from(info.index_low),
    ))
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_file: &File) -> Result<(u64, u64), String> {
    Err("retained file identity is unavailable on this platform".into())
}

struct Stage {
    path: PathBuf,
    directory: Retained,
    parent_path: PathBuf,
    parent: Retained,
    files: Vec<(String, Retained, Vec<u8>)>,
}

impl Stage {
    fn create(parent_path: &Path) -> Result<Self, String> {
        let parent = Retained::open(parent_path)?;
        let temporary = tempfile::Builder::new()
            .prefix(".orr-mesh-animation-import-")
            .tempdir_in(parent_path)
            .map_err(|error| format!("create animation staging directory: {error}"))?;
        // Disable TempDir's recursive destructor before any fallible inspection.
        let path = temporary.keep();
        let directory = Retained::open(&path)?;
        Ok(Self {
            path,
            directory,
            parent_path: parent_path.into(),
            parent,
            files: Vec::new(),
        })
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.check_identity()?;
        let path = self.path.join(name);
        write_new_file(&path, bytes)?;
        self.files
            .push((name.into(), Retained::open(&path)?, bytes.to_vec()));
        Ok(())
    }

    fn check_identity(&self) -> Result<(), String> {
        if checked_directory(&self.parent_path, "animation destination parent")? != self.parent_path
            || !self.parent.matches(&self.parent_path)
            || checked_directory(&self.path, "animation staging directory")? != self.path
            || !self.directory.matches(&self.path)
        {
            return Err("animation package parent or staging identity changed".into());
        }
        Ok(())
    }

    fn check_bytes(&self) -> Result<(), String> {
        self.check_identity()?;
        if fs::read_dir(&self.path)
            .map_err(|error| error.to_string())?
            .count()
            != self.files.len()
        {
            return Err("animation staging directory contents changed".into());
        }
        for (name, retained, expected) in &self.files {
            let path = self.path.join(name);
            let current = read_input_file(&path, expected.len() as u64, "staged animation file")?;
            if !retained.matches(&path) || current.bytes != *expected {
                return Err("animation staged file bytes or identity changed".into());
            }
        }
        Ok(())
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        // Never recursively delete: a replaced directory or file belongs to somebody else.
        if self.check_identity().is_err() {
            return;
        }
        for (name, retained, expected) in &self.files {
            let path = self.path.join(name);
            if retained.matches(&path)
                && read_input_file(&path, expected.len() as u64, "staged cleanup file")
                    .is_ok_and(|current| current.bytes == *expected)
            {
                let _ = fs::remove_file(path);
            }
        }
        let _ = fs::remove_dir(&self.path);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Staged,
    BeforePublish,
    Published,
}

pub(crate) fn import_package(request: Request) -> Result<Report, String> {
    import_with_hook(request, |_, _, _| Ok(()))
}

fn import_with_hook(
    request: Request,
    mut hook: impl FnMut(Phase, &Path, &Path) -> Result<(), String>,
) -> Result<Report, String> {
    if !request.license_acknowledged {
        return Err("explicit license acknowledgement is required".into());
    }
    validate_package_identity(&request.name, &request.version)?;
    let asset = source_asset(&request.source)?;
    let source = read_input_file(&request.source, MAX_SOURCE_BYTES, "animation source")?;
    if asset == COOKED_ASSET {
        // Recognized cooked inputs never fall back to a raw importer or resolve
        // provenance paths. Preserve their exact admitted bytes in the package.
        crate::mesh_animation::cooked_provenance::decode(asset, &source.bytes)?;
    }
    let source_identity = Retained::open(&source.path)?;
    let license = read_input_file(&request.license, MAX_LICENSE_BYTES, "license source")?;
    let license_identity = Retained::open(&license.path)?;
    if source.path == license.path || source_identity.identity == license_identity.identity {
        return Err("Animation source and license must be separate files".into());
    }
    if std::str::from_utf8(&license.bytes)
        .map_err(|_| "license must be nonempty UTF-8 text")?
        .trim()
        .is_empty()
    {
        return Err("license must be nonempty UTF-8 text".into());
    }
    let (parent, destination) = checked_new_destination(&request.destination)?;
    let mut stage = Stage::create(&parent)?;
    let runtime = orr_package::Runtime::content_only();
    let manifest = orr_package::Manifest {
        schema: 1,
        name: request.name,
        version: request.version,
        engine: format!("={}", runtime.engine_version),
        capabilities: BTreeSet::from(["animation".into()]),
        dependencies: BTreeMap::new(),
        files: BTreeSet::from([asset.into(), LICENSE.into()]),
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|error| error.to_string())?;
    if manifest_bytes.len() > 1024 * 1024 {
        return Err("animation manifest exceeds 1 MiB".into());
    }
    stage.write(asset, &source.bytes)?;
    stage.write(LICENSE, &license.bytes)?;
    stage.write(MANIFEST, &manifest_bytes)?;
    hook(Phase::Staged, &stage.path, &destination)?;
    stage.check_bytes()?;
    let snapshot = Snapshot::from_package(&stage.path, asset, LICENSE, request.graph_revision)?;
    hook(Phase::BeforePublish, &stage.path, &destination)?;
    recheck_input(&source, MAX_SOURCE_BYTES, "animation source")?;
    recheck_input(&license, MAX_LICENSE_BYTES, "license source")?;
    if !source_identity.matches(&source.path) || !license_identity.matches(&license.path) {
        return Err("animation source identity changed before publication".into());
    }
    stage.check_bytes()?;
    refuse_existing(&destination)?;
    publish_new_directory(&stage.path, &destination)?;

    // From here forward output exists. Never report rollback on durability/readback failure.
    let verification = (|| {
        hook(Phase::Published, &stage.path, &destination)?;
        if !stage.parent.matches(&parent) || !stage.directory.matches(&destination) {
            return Err("published package or parent identity changed".into());
        }
        #[cfg(unix)]
        stage
            .parent
            .handle
            .sync_all()
            .map_err(|error| format!("sync destination parent: {error}"))?;
        #[cfg(unix)]
        stage
            .directory
            .handle
            .sync_all()
            .map_err(|error| format!("sync published directory: {error}"))?;
        let published =
            Snapshot::from_package(&destination, asset, LICENSE, request.graph_revision)?;
        if published.encode_embedded("mesh.scene.yaml")?
            != snapshot.encode_embedded("mesh.scene.yaml")?
        {
            return Err("published animation snapshot differs from validated snapshot".into());
        }
        Ok::<(), String>(())
    })();
    let publication_warning = verification.err().map(|error| format!(
        "Package was published at {}; output exists or may have changed. Durability/readback verification failed: {error}", destination.display()
    ));
    Ok(Report {
        root: destination,
        asset: asset.into(),
        license: LICENSE.into(),
        source_sha256: source.sha256,
        license_sha256: license.sha256,
        snapshot,
        publication_warning,
    })
}

#[cfg(test)]
#[path = "mesh_animation_import/cooked_tests.rs"]
pub(crate) mod cooked_tests;
