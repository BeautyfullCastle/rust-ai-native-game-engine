pub(crate) mod authoring {
    include!("windows_room/authoring.rs");

    #[cfg(feature = "project-export")]
    pub(crate) mod export_platform {
        use std::{
            fs::{File, Metadata},
            path::{Path, PathBuf},
        };

        pub(crate) fn destination(path: &Path) -> Result<PathBuf, String> {
            super::platform::destination(path)
        }

        pub(crate) fn publish(stage: &Path, output: &Path) -> Result<(), String> {
            super::platform::publish(stage, output)
        }

        pub(crate) fn open_read(path: &Path) -> Result<File, String> {
            super::platform::open_read(path)
        }

        pub(crate) fn is_regular(metadata: &Metadata) -> bool {
            super::platform::is_regular(metadata)
        }

        pub(crate) fn is_directory(metadata: &Metadata) -> bool {
            super::platform::is_directory(metadata)
        }
    }
}

use crate::room_project::PreparedProject;
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf, Prefix},
};

const MAX_ROOT_PATH_BYTES: usize = 4096;
const MAX_ROOT_PATH_UNITS: usize = 32_760;
const MAX_ROOT_COMPONENTS: usize = 64;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

pub fn create_room_project(output: &Path, seed: &str) -> Result<PreparedProject, String> {
    let options = authoring::Options::parse(vec![
        OsString::from("--output"),
        output.as_os_str().to_os_string(),
        OsString::from("--seed"),
        OsString::from(seed),
    ])?;
    let report = authoring::create(&options)?;
    let prepared = PreparedProject::open(&report.output).map_err(|error| {
        format!(
            "Room project was published at {} but post-publication admission failed: {error}",
            report.output.display()
        )
    })?;
    if prepared.scene().frame().checksum() != report.initial_checksum {
        return Err(format!(
            "Room project was published at {} but its checksum differs from the validated stage",
            report.output.display()
        ));
    }
    let expected_guids = report
        .entity_guids
        .iter()
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let actual_guids = prepared
        .scene()
        .frame()
        .entities()
        .map(|entity| {
            prepared
                .scene()
                .index()
                .guid(entity)
                .map(ToString::to_string)
                .ok_or_else(|| "published Room actor has no authored GUID".to_owned())
        })
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| {
            format!(
                "Room project was published at {} but post-publication admission failed: {error}",
                report.output.display()
            )
        })?;
    if actual_guids != expected_guids {
        return Err(format!(
            "Room project was published at {} but its actor GUID set differs from the validated stage",
            report.output.display()
        ));
    }
    Ok(prepared)
}

pub fn open_room_project(root: &Path) -> Result<PreparedProject, String> {
    let root = validate_existing_root(root)?;
    PreparedProject::open(root)
}

fn validate_existing_root(root: &Path) -> Result<PathBuf, String> {
    let text = root.to_str().ok_or("Room project path must be Unicode")?;
    if text.is_empty()
        || text.len() > MAX_ROOT_PATH_BYTES
        || text
            .chars()
            .any(|character| character.is_control() || character == '\0')
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err("Room project path must be a bounded absolute path without traversal".into());
    }
    let components: Vec<_> = root.components().collect();
    if components.len() < 3 || components.len() > MAX_ROOT_COMPONENTS {
        return Err("Room project path has an invalid component count".into());
    }
    let mut components = components.into_iter();
    let prefix = match components.next() {
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
        {
            prefix
        }
        _ => return Err("Room project path must use an absolute local disk path".into()),
    };
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err("Room project path must be absolute".into());
    }

    let mut current = PathBuf::from(prefix.as_os_str());
    current.push(Path::new("\\"));
    let root_metadata = fs::symlink_metadata(&current).map_err(|error| error.to_string())?;
    validate_directory(&root_metadata)?;
    for component in components {
        let Component::Normal(name) = component else {
            return Err("Room project path must be normalized".into());
        };
        current.push(name);
        let metadata = fs::symlink_metadata(&current).map_err(|error| error.to_string())?;
        validate_directory(&metadata)?;
    }

    let canonical = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let canonical_text = canonical
        .to_str()
        .ok_or("canonical Room project path must be Unicode")?;
    if canonical_text.encode_utf16().count() >= MAX_ROOT_PATH_UNITS {
        return Err("Room project path exceeds Windows path limits".into());
    }
    Ok(canonical)
}

fn validate_directory(metadata: &std::fs::Metadata) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;

    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("Room project path must contain only existing non-reparse directories".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn create_path(root: &Path, leaf: &str) -> PathBuf {
        root.join(leaf)
    }

    #[test]
    fn creation_returns_admitted_project_and_never_overwrites() {
        let temporary = tempfile::tempdir().unwrap();
        let output = create_path(temporary.path(), "new-room");
        let prepared = create_room_project(&output, "acceptance-1").unwrap();
        assert_eq!(prepared.root(), fs::canonicalize(&output).unwrap());
        assert_eq!(prepared.scene().frame().entities().count(), 7);

        let reopened = open_room_project(&output).unwrap();
        assert_eq!(
            reopened.scene().frame().checksum(),
            prepared.scene().frame().checksum()
        );
        assert!(create_room_project(&output, "acceptance-2").is_err());
        assert_eq!(
            open_room_project(&output)
                .unwrap()
                .scene()
                .frame()
                .checksum(),
            prepared.scene().frame().checksum()
        );
        assert!(create_room_project(&create_path(temporary.path(), "bad-seed"), "../bad").is_err());
        assert!(!temporary.path().join("bad-seed").exists());
    }

    #[test]
    fn opening_rejects_a_malformed_room_entry() {
        let temporary = tempfile::tempdir().unwrap();
        let output = create_path(temporary.path(), "malformed-room");
        create_room_project(&output, "malformed").unwrap();
        fs::write(
            output.join("orr.project.json"),
            br#"{"schema":2,"engine":"^0.0.1","entry":{"game":"room-escape-v1","scene":"missing.scene.yaml","models":"room.models.json","camera":"room.camera.json"}}"#,
        )
        .unwrap();
        assert!(open_room_project(&output).is_err());
    }

    #[test]
    fn opening_rejects_changed_package_closure() {
        let temporary = tempfile::tempdir().unwrap();
        let output = create_path(temporary.path(), "tampered-room");
        create_room_project(&output, "tampered").unwrap();
        let objects = output.join(".orr/packages/objects");
        let object = fs::read_dir(objects)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .next()
            .unwrap();
        let mut license = fs::OpenOptions::new()
            .append(true)
            .open(object.join("LICENSE.txt"))
            .unwrap();
        license.write_all(b"changed").unwrap();
        assert!(open_room_project(&output).is_err());
    }
}
