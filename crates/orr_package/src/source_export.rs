//! Recover installable sources from identity-checked active installed content.
use crate::{Error, Project, Result};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

/// One portable source directory, relative to the explicitly selected output.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ExportedSource {
    pub directory: String,
    pub version: String,
    pub digest: String,
}

/// Successful content export, not host admission or redistribution authorization.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SourceExportReport {
    pub requested_package: String,
    pub sources: BTreeMap<String, ExportedSource>,
    pub declared_bytes: u64,
}

impl Project {
    /// Export one active package and its exact dependency closure to a new directory.
    ///
    /// Linux only. All payloads are snapshotted and compared with one admitted lock
    /// before output creation. Only declared content is copied. The project is never
    /// modified, and a content-only handle gains no asset-loading authority.
    ///
    /// Directory creation reserves the destination without replacing existing paths;
    /// publication of its contents is NOT atomic. A later failure leaves the partial
    /// output for explicit operator inspection/removal. Trusted, quiescent directories
    /// are required: this is not protection against hostile concurrent path changes.
    pub fn export_sources(
        &self,
        name: &str,
        destination: impl AsRef<Path>,
    ) -> Result<SourceExportReport> {
        #[cfg(target_os = "linux")]
        {
            linux::export(self, name, destination.as_ref())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (name, destination);
            Err(Error(
                "package source export currently requires Linux".into(),
            ))
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::{fail, no_links, snapshot, write_sync, Lock, MANIFEST, MAX_JSON, MAX_TOTAL};
    use std::{
        collections::BTreeSet,
        fs,
        path::{Component, PathBuf},
    };

    struct Source {
        manifest: Vec<u8>,
        files: BTreeMap<String, Vec<u8>>,
    }

    pub(super) fn export(
        project: &Project,
        name: &str,
        destination: &Path,
    ) -> Result<SourceExportReport> {
        crate::name(name)?;
        let destination = checked_destination(project, destination)?;
        let lock = project.list()?;
        let selected = closure(&lock, name)?;
        let mut sources = BTreeMap::new();
        let mut report = SourceExportReport {
            requested_package: name.into(),
            sources: BTreeMap::new(),
            declared_bytes: 0,
        };
        for selected_name in selected {
            let expected = &lock.packages[&selected_name];
            // Each source is read once into a bounded immutable in-memory snapshot.
            // Capability enforcement follows the handle, just like inspect/install.
            let (observed, files) = snapshot(
                &project.object(expected),
                &project.runtime,
                project.enforce_capabilities,
            )?;
            if &observed != expected {
                return fail(format!("installed package changed: {selected_name}"));
            }
            for bytes in files.values() {
                report.declared_bytes =
                    add_payload_bytes(report.declared_bytes, bytes.len() as u64)?;
            }
            let mut manifest = serde_json::to_vec_pretty(&observed.manifest)?;
            manifest.push(b'\n');
            if manifest.len() as u64 > MAX_JSON {
                return fail("exported manifest exceeds 1 MiB");
            }
            report.sources.insert(
                selected_name,
                ExportedSource {
                    directory: observed.digest.clone(),
                    version: observed.manifest.version.clone(),
                    digest: observed.digest.clone(),
                },
            );
            sources.insert(observed.digest, Source { manifest, files });
        }
        if project.list()? != lock {
            return fail("active package lock changed during export preparation");
        }
        if checked_destination(project, &destination)? != destination {
            return fail("export destination identity changed during preparation");
        }
        reserve_and_write(&destination, || {
            for (digest, source) in &sources {
                let root = destination.join(digest);
                fs::create_dir(&root)?;
                write_sync(&root.join(MANIFEST), &source.manifest)?;
                for (relative, bytes) in &source.files {
                    let path = root.join(relative);
                    if let Some(parent) = path.parent() {
                        crate::safe_mkdir(parent)?;
                    }
                    write_sync(&path, bytes)?;
                }
            }
            Ok(())
        })?;
        Ok(report)
    }

    fn add_payload_bytes(current: u64, additional: u64) -> Result<u64> {
        let total = current
            .checked_add(additional)
            .ok_or_else(|| Error("export byte limit exceeded".into()))?;
        if total > MAX_TOTAL {
            return fail("export closure exceeds 256 MiB");
        }
        Ok(total)
    }

    fn closure(lock: &Lock, name: &str) -> Result<BTreeSet<String>> {
        if !lock.packages.contains_key(name) {
            return fail(format!("missing package: {name}"));
        }
        let mut selected = BTreeSet::new();
        let mut pending = vec![name.to_owned()];
        while let Some(name) = pending.pop() {
            if selected.insert(name.clone()) {
                pending.extend(lock.packages[&name].manifest.dependencies.keys().cloned());
            }
        }
        Ok(selected)
    }

    fn checked_destination(project: &Project, path: &Path) -> Result<PathBuf> {
        let text = path
            .to_str()
            .ok_or_else(|| Error("export path must be Unicode".into()))?;
        if !path.is_absolute()
            || text.len() > 4096
            || text.contains('\\')
            || text.chars().any(char::is_control)
            || text
                .split('/')
                .skip(1)
                .any(|part| part.is_empty() || part == "." || part == "..")
            || path.components().count() > 64
            || !matches!(path.components().next_back(), Some(Component::Normal(_)))
        {
            return fail("export path must be bounded, absolute and normalized");
        }
        let parent = path
            .parent()
            .ok_or_else(|| Error("export parent required".into()))?;
        no_links(parent)?;
        if !fs::symlink_metadata(parent)?.is_dir() {
            return fail("export parent must be an existing directory");
        }
        let parent = fs::canonicalize(parent)?;
        if parent.starts_with(project.root()) {
            return fail("export destination must be outside the source project");
        }
        let target = parent.join(
            path.file_name()
                .ok_or_else(|| Error("export name required".into()))?,
        );
        match fs::symlink_metadata(&target) {
            Ok(_) => return fail("export destination already exists"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(target)
    }

    fn reserve_and_write(destination: &Path, write: impl FnOnce() -> Result<()>) -> Result<()> {
        // create_dir is the exclusive reservation, not an atomic tree publication.
        fs::create_dir(destination)?;
        write().map_err(|error| Error(format!(
            "package source export failed; partial output retained at {} (inspect before manually removing): {error}", destination.display()
        )))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn closure_budget_rejects_excess_and_overflow() {
            assert_eq!(add_payload_bytes(MAX_TOTAL - 1, 1).unwrap(), MAX_TOTAL);
            assert!(add_payload_bytes(MAX_TOTAL, 1).is_err());
            assert!(add_payload_bytes(u64::MAX, 1).is_err());
        }

        #[test]
        fn partial_failure_retains_output_and_does_not_retry_existing_directory() {
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("output");
            let error = reserve_and_write(&output, || {
                fs::write(output.join("observed-file"), b"preserve")?;
                fail("injected write failure")
            })
            .unwrap_err();
            assert!(error.to_string().contains("partial output retained"));
            assert_eq!(fs::read(output.join("observed-file")).unwrap(), b"preserve");
            assert!(
                reserve_and_write(&output, || panic!("existing output must not be written"))
                    .is_err()
            );
            assert_eq!(fs::read(output.join("observed-file")).unwrap(), b"preserve");
        }
    }
}
