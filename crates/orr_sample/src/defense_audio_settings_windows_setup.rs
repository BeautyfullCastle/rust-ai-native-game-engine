//! Explicit one-directory provisioning; never modifies an existing ACL or settings file.
use super::*;

/// Read-only startup admission. This does not attempt or promise a future save.
pub(crate) fn check(path: &Path, forbidden_roots: &[PathBuf]) -> Result<(bool, bool), String> {
    let (audio, audio_bytes) = Store::open_profile(path, forbidden_roots, StorageProfile::Audio)
        .map_err(|e| format!("audio settings admission failed: {e}"))?;
    if !audio.writable() {
        return Err("audio settings unreadable or unsupported; existing data preserved".into());
    }
    let (controls, controls_bytes) =
        Store::open_profile(path, forbidden_roots, StorageProfile::Controls)
            .map_err(|e| format!("controls settings admission failed: {e}"))?;
    if !controls.writable() {
        return Err("controls settings unreadable or unsupported; existing data preserved".into());
    }
    Ok((audio_bytes.is_some(), controls_bytes.is_some()))
}

pub(crate) fn prepare(path: &Path, forbidden_roots: &[PathBuf]) -> Result<(), String> {
    let paths = validate_path(path)?;
    let leaf = paths
        .last()
        .unwrap()
        .file_name()
        .ok_or("missing directory name")?;
    let mut anchors = vec![open_directory(&paths[0])?];
    // Resolve the opened root once, then descend only through its stable volume GUID.
    let mut parent = volume_path(&anchors[0])?;
    let root_name = security::wide(parent.as_os_str());
    // SAFETY: a live root handle and a NUL-terminated volume GUID path.
    if unsafe { GetDriveTypeW(root_name.as_ptr()) } != 3 {
        return Err(error("settings require a fixed local NTFS drive"));
    }
    let mut filesystem = [0u16; 32];
    // SAFETY: live root handle, bounded output; unused outputs are null.
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            anchors[0].as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    };
    if ok == 0 || filesystem[..5] != [b'N' as u16, b'T' as u16, b'F' as u16, b'S' as u16, 0] {
        return Err(error("settings require a local NTFS filesystem"));
    }
    for part in &paths[1..paths.len() - 1] {
        parent.push(part.file_name().ok_or("missing parent name")?);
        anchors.push(open_directory(&parent)?);
    }
    for root in forbidden_roots {
        let canonical = std::fs::canonicalize(root).map_err(error)?;
        let forbidden = identity(&info(&open_directory(&canonical)?)?);
        if anchors
            .iter()
            .any(|f| info(f).map(|i| identity(&i) == forbidden).unwrap_or(true))
        {
            return Err(error("settings must be outside the executable/source root"));
        }
    }
    let security = Security::new().map_err(error)?;
    let attributes = security.attributes();
    let target = parent.join(leaf);
    let target_name = security::wide(target.as_os_str());
    // SAFETY: all parents are pinned against rename/delete and checked for reparse points;
    // private owner/DACL exist at creation, not in a later permission repair step.
    if unsafe { CreateDirectoryW(target_name.as_ptr(), &attributes) } == 0 {
        return Err(error(format!(
            "new directory was not created: {}",
            security::last()
        )));
    }
    // Keep ancestors pinned through the postcondition check. Never remove/repair on failure.
    let created = open_directory(&target).and_then(|f| security.private(&f).map_err(error));
    if let Err(e) = created {
        return Err(error(format!(
            "directory was created but verification failed; inspect it manually: {e}"
        )));
    }
    drop(anchors);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defense_controls::{default_map, Document};

    fn acl(file: &File) -> Vec<u8> {
        use windows_sys::Win32::Security::{
            Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
            GetSecurityDescriptorLength, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        };
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: live handle; returned self-relative descriptor is owned through Allocation.
        unsafe {
            assert_eq!(
                GetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut descriptor
                ),
                0
            );
            let _allocation = security::Allocation(descriptor);
            std::slice::from_raw_parts(
                descriptor.cast::<u8>(),
                GetSecurityDescriptorLength(descriptor) as usize,
            )
            .to_vec()
        }
    }

    #[test]
    fn setup_creates_private_empty_directory_admitted_by_both_stores() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings [shared] & space");
        prepare(&path, &[]).unwrap();
        assert_eq!(check(&path, &[]).unwrap(), (false, false));
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
        let mut audio = Store::open(&path, &[]).unwrap();
        let (mut controls, bytes) =
            Store::open_profile(&path, &[], StorageProfile::Controls).unwrap();
        assert!(bytes.is_none());
        assert!(audio.store.writable() && controls.writable());
        let preferences = Preferences::new(37, false).unwrap();
        assert!(matches!(audio.store.save(&preferences), Outcome::Saved));
        let controls_bytes = serde_json::to_vec(&Document::new(default_map()).unwrap()).unwrap();
        assert!(matches!(
            controls.save_bytes(controls_bytes.clone()),
            Outcome::Saved
        ));
        drop((audio, controls));
        assert_eq!(Store::open(&path, &[]).unwrap().preferences, preferences);
        assert_eq!(
            Store::open_profile(&path, &[], StorageProfile::Controls)
                .unwrap()
                .1
                .unwrap(),
            controls_bytes
        );
        let count = std::fs::read_dir(&path).unwrap().count();
        assert_eq!(check(&path, &[]).unwrap(), (true, true));
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), count);
    }

    #[test]
    fn check_rejects_invalid_profiles_without_creating_locks_or_changing_data_or_acl() {
        let root = tempfile::tempdir().unwrap();
        for (profile, filename) in [
            ("audio", "audio-settings.json"),
            ("controls", "controls-settings.json"),
        ] {
            let path = root.path().join(profile);
            prepare(&path, &[]).unwrap();
            let original = b"{malformed profile data";
            super::super::tests::write_private(&path.join(filename), original);
            let before = acl(&open_directory(&path).unwrap());
            let error = check(&path, &[]).unwrap_err();
            assert!(error.contains(profile), "{error}");
            assert_eq!(std::fs::read(path.join(filename)).unwrap(), original);
            assert_eq!(acl(&open_directory(&path).unwrap()), before);
            assert_eq!(std::fs::read_dir(path).unwrap().count(), 1);
        }
    }

    #[test]
    fn check_never_creates_missing_directory_or_repairs_broad_acl() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        assert!(check(&missing, &[]).is_err());
        assert!(!missing.exists());
        let broad = root.path().join("broad");
        let security = Security::test_acl("D:P(A;;FA;;;WD)");
        let attributes = security.attributes();
        let name = security::wide(broad.as_os_str());
        // SAFETY: valid owned descriptor and NUL-terminated fresh test path.
        assert_ne!(unsafe { CreateDirectoryW(name.as_ptr(), &attributes) }, 0);
        let before = acl(&open_directory(&broad).unwrap());
        assert!(check(&broad, &[]).is_err());
        assert_eq!(acl(&open_directory(&broad).unwrap()), before);
        assert_eq!(std::fs::read_dir(broad).unwrap().count(), 0);
    }

    #[test]
    fn setup_refuses_existing_directory_and_file_without_acl_or_content_changes() {
        let (root, private) = super::super::tests::private_directory();
        super::super::tests::write_private(
            &private.join("audio-settings.json"),
            b"preserve malformed data",
        );
        let before = std::fs::read(private.join("audio-settings.json")).unwrap();
        let private_acl = acl(&open_directory(&private).unwrap());
        assert!(prepare(&private, &[]).is_err());
        assert_eq!(acl(&open_directory(&private).unwrap()), private_acl);
        assert_eq!(
            std::fs::read(private.join("audio-settings.json")).unwrap(),
            before
        );
        let existing = root.path().join("existing");
        std::fs::create_dir(&existing).unwrap();
        let check = open_directory(&existing).unwrap();
        let security = Security::new().unwrap();
        let was_private = security.private(&check).is_ok();
        let before_acl = acl(&check);
        assert!(prepare(&existing, &[]).is_err());
        assert_eq!(acl(&check), before_acl);
        assert_eq!(security.private(&check).is_ok(), was_private);
        let file = root.path().join("file");
        std::fs::write(&file, b"untouched").unwrap();
        assert!(prepare(&file, &[]).is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"untouched");
    }

    #[test]
    fn setup_refuses_missing_parent_without_recursive_creation() {
        let root = tempfile::tempdir().unwrap();
        assert!(prepare(&root.path().join("missing").join("child"), &[]).is_err());
        assert!(!root.path().join("missing").exists());
    }

    #[test]
    fn setup_reuses_strict_path_refusal_without_io() {
        for path in [
            "relative",
            "C:relative",
            "C:\\",
            "C:\\a\\..\\b",
            "C:\\a\\CON",
            "C:\\a\\b.",
            "\\\\server\\share\\new",
            "C:\\a:stream",
        ] {
            assert!(prepare(Path::new(path), &[]).is_err(), "{path}");
        }
    }

    #[test]
    fn setup_refuses_forbidden_parent_identity_before_creation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new");
        assert!(prepare(&path, &[root.path().to_owned()]).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn setup_refuses_junction_ancestor_without_creating_in_target() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        let junction = root.path().join("junction");
        std::fs::create_dir(&target).unwrap();
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(prepare(&junction.join("new"), &[]).is_err());
        assert!(!target.join("new").exists());
    }
}
