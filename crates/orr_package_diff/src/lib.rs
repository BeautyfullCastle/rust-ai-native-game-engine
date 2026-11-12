//! Read-only, bounded-output comparison of installed and authored package content.
//! Verification uses installation-tool APIs; it grants no runtime capabilities.
use orr_package::{inspect_for_install, LockedPackage, Manifest, Project, Runtime};
use serde::Serialize;
use std::{collections::BTreeSet, fmt, io::Write, path::Path};

pub const MAX_INITIAL_PACKAGES: usize = 8;
pub const MAX_INITIAL_FILES: usize = 8192;
pub const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub struct Error(String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
impl From<orr_package::Error> for Error {
    fn from(e: orr_package::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self(e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Added,
    Removed,
    Changed,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub change: Change,
    pub before_sha256: Option<String>,
    pub after_sha256: Option<String>,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Counts {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub unchanged: usize,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Report {
    pub schema: u32,
    pub package: String,
    pub status: &'static str,
    pub installed_scope: &'static str,
    pub candidate_scope: &'static str,
    pub before_digest: String,
    pub after_digest: String,
    pub before_manifest: Manifest,
    pub after_manifest: Manifest,
    pub counts: Counts,
    pub files: Vec<FileChange>,
}
impl Report {
    pub fn different(&self) -> bool {
        self.status == "changed"
    }
}

/// Compare observed validated content. Inputs are never written.
///
/// The 8-package/8192-file limit is initial admission only: `verify` rereads the
/// lock and may verify up to the inherited 128 packages before a changed-lock
/// rejection. This is not an atomic snapshot or an update-feasibility check.
pub fn compare(project: &Path, package: &str, source: &Path) -> Result<Report> {
    let runtime = Runtime::content_only();
    let project = Project::open_for_install(project, runtime.engine_version.clone())?;
    let admitted = project.list()?;
    admission(&admitted)?;
    let old = admitted
        .packages
        .get(package)
        .ok_or_else(|| Error("selected package is not installed".into()))?;
    let verified = project.verify()?;
    if verified != admitted {
        return Err(Error("installed lock changed during verification".into()));
    }
    let candidate = inspect_for_install(source, runtime.engine_version)?;
    if candidate.manifest.name != package {
        return Err(Error(
            "candidate package name differs from selected package".into(),
        ));
    }
    if project.list()? != admitted {
        return Err(Error(
            "installed lock changed during source inspection".into(),
        ));
    }
    Ok(report(old, &candidate))
}
fn admission(lock: &orr_package::Lock) -> Result<()> {
    let files: usize = lock.packages.values().map(|p| p.files.len()).sum();
    if lock.packages.len() > MAX_INITIAL_PACKAGES || files > MAX_INITIAL_FILES {
        return Err(Error(
            "initial project exceeds diff limit: 8 packages / 8192 declared files".into(),
        ));
    }
    Ok(())
}
fn report(old: &LockedPackage, new: &LockedPackage) -> Report {
    let paths: BTreeSet<_> = old.files.keys().chain(new.files.keys()).collect();
    let mut counts = Counts {
        added: 0,
        removed: 0,
        changed: 0,
        unchanged: 0,
    };
    let mut files = Vec::new();
    for path in paths {
        let before = old.files.get(path);
        let after = new.files.get(path);
        let change = match (before, after) {
            (None, Some(_)) => {
                counts.added += 1;
                Change::Added
            }
            (Some(_), None) => {
                counts.removed += 1;
                Change::Removed
            }
            (Some(a), Some(b)) if a != b => {
                counts.changed += 1;
                Change::Changed
            }
            _ => {
                counts.unchanged += 1;
                continue;
            }
        };
        files.push(FileChange {
            path: path.clone(),
            change,
            before_sha256: before.cloned(),
            after_sha256: after.cloned(),
        });
    }
    Report {
        schema: 1,
        package: old.manifest.name.clone(),
        status: if old == new { "same" } else { "changed" },
        installed_scope: "all declared installed files verified; observed lock unchanged",
        candidate_scope:
            "declared source files inspected; dependency resolution and host capabilities deferred",
        before_digest: old.digest.clone(),
        after_digest: new.digest.clone(),
        before_manifest: old.manifest.clone(),
        after_manifest: new.manifest.clone(),
        counts,
        files,
    }
}
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_OUTPUT_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("diff output exceeds 8 MiB limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
/// Construct the complete bounded output before touching a caller's sink.
pub fn encode(report: &Report, json: bool) -> Result<Vec<u8>> {
    let mut out = Bounded(Vec::new());
    if json {
        serde_json::to_writer(&mut out, report)?;
        writeln!(out)?;
    } else {
        writeln!(out, "Package {:?}: {}", report.package, report.status)?;
        writeln!(
            out,
            "{} added, {} removed, {} changed, {} unchanged",
            report.counts.added,
            report.counts.removed,
            report.counts.changed,
            report.counts.unchanged
        )?;
        writeln!(
            out,
            "Digest: {} -> {}",
            report.before_digest, report.after_digest
        )?;
        // Serialized manifests make every metadata difference visible, with escaped strings.
        write!(out, "Before manifest: ")?;
        serde_json::to_writer(&mut out, &report.before_manifest)?;
        writeln!(out)?;
        write!(out, "After manifest: ")?;
        serde_json::to_writer(&mut out, &report.after_manifest)?;
        writeln!(out)?;
        for file in &report.files {
            writeln!(
                out,
                "{:?} {:?}: {} -> {}",
                file.change,
                file.path,
                file.before_sha256.as_deref().unwrap_or("-"),
                file.after_sha256.as_deref().unwrap_or("-")
            )?;
        }
        writeln!(out, "{}", report.installed_scope)?;
        writeln!(out, "{}", report.candidate_scope)?;
    }
    Ok(out.0)
}
/// A sink failure can leave partial bytes. Callers must treat the report as failed.
pub fn emit(mut sink: impl Write, bytes: &[u8]) -> Result<()> {
    sink.write_all(bytes)?;
    sink.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_bound_and_partial_sink_are_errors() {
        let mut buffer = Bounded(vec![0; MAX_OUTPUT_BYTES - 1]);
        assert_eq!(buffer.write(b"x").unwrap(), 1);
        assert!(buffer.write(b"y").is_err());
        assert_eq!(buffer.0.len(), MAX_OUTPUT_BYTES);
        struct Partial(Vec<u8>);
        impl Write for Partial {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    self.0.push(b[0]);
                    Ok(1)
                } else {
                    Err(std::io::Error::other("closed sink"))
                }
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut sink = Partial(Vec::new());
        assert!(emit(&mut sink, b"abc").is_err());
        assert_eq!(sink.0, b"a");
    }
    #[test]
    fn initial_admission_boundaries() {
        let mut lock = orr_package::Lock::default();
        let package = LockedPackage {
            manifest: Manifest {
                schema: 1,
                name: "a".into(),
                version: "1.0.0".into(),
                engine: "*".into(),
                capabilities: Default::default(),
                dependencies: Default::default(),
                files: Default::default(),
            },
            digest: String::new(),
            files: Default::default(),
        };
        for i in 0..8 {
            lock.packages.insert(i.to_string(), package.clone());
        }
        assert!(admission(&lock).is_ok());
        lock.packages.insert("8".into(), package.clone());
        assert!(admission(&lock).is_err());
        lock.packages.clear();
        let mut package = package;
        for i in 0..8192 {
            package.files.insert(i.to_string(), String::new());
        }
        lock.packages.insert("a".into(), package);
        assert!(admission(&lock).is_ok());
        lock.packages
            .get_mut("a")
            .unwrap()
            .files
            .insert("8192".into(), String::new());
        assert!(admission(&lock).is_err());
    }
}
