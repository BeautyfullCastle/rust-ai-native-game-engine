//! Reserved native publication. Adapted from the independently reviewed embedded
//! prototype harness at 640c18f; no directory-publisher fallback or extraction.
use super::content::validate_pe;
use super::{basename, expected, hash, validate, Options, Receipt, Result, MAX_RUNTIME_BYTES};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, Write},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf},
    ptr,
};
#[path = "process.rs"]
mod process;
#[path = "win.rs"]
mod win;
fn error(context: &str) -> String {
    format!("{context}: {}", std::io::Error::last_os_error())
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn info(file: &File) -> Result<win::FileInfo> {
    let mut value = win::FileInfo::default();
    if unsafe { win::GetFileInformationByHandle(file.as_raw_handle(), &mut value) } == 0 {
        return Err(error("GetFileInformationByHandle"));
    }
    Ok(value)
}
fn guid(file: &File) -> Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let n = unsafe {
        win::GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            1,
        )
    } as usize;
    if n == 0 || n >= buffer.len() {
        return Err(error("GetFinalPathNameByHandleW"));
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..n])))
}
fn directory(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .access_mode(1 | 0x80)
        .share_mode(win::SHARE_READ)
        .custom_flags(win::BACKUP | win::REPARSE)
        .open(path)
        .map_err(|e| e.to_string())?;
    let i = info(&file)?;
    if i.attributes & 0x400 != 0 || i.attributes & 0x10 == 0 {
        return Err("nonordinary directory".into());
    }
    Ok(file)
}
struct Root {
    path: PathBuf,
    anchors: Vec<(File, win::FileInfo)>,
}
impl Root {
    fn open(path: &Path) -> Result<Self> {
        let mut parts = path.components();
        let Some(Component::Prefix(prefix)) = parts.next() else {
            return Err("absolute local drive required".into());
        };
        if !matches!(prefix.kind(), std::path::Prefix::Disk(_))
            || parts.next() != Some(Component::RootDir)
        {
            return Err("absolute local drive required".into());
        }
        let root = directory(&PathBuf::from(format!(
            "{}\\",
            prefix.as_os_str().to_string_lossy()
        )))?;
        let mut path = guid(&root)?;
        let mut filesystem = [0u16; 32];
        if unsafe { win::GetDriveTypeW(wide(&path).as_ptr()) } != 3 {
            return Err("fixed local drive required".into());
        }
        if unsafe {
            win::GetVolumeInformationByHandleW(
                root.as_raw_handle(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                filesystem.as_mut_ptr(),
                32,
            )
        } == 0
            || filesystem[..5] != [78, 84, 70, 83, 0]
        {
            return Err("NTFS required".into());
        }
        let original = info(&root)?;
        let mut anchors = vec![(root, original)];
        for part in parts {
            let Component::Normal(name) = part else {
                return Err("normal components required".into());
            };
            let text = name.to_str().ok_or("non-ASCII ancestor")?;
            basename(text)?;
            path.push(name);
            let handle = directory(&path)?;
            if guid(&handle)? != path {
                return Err("ancestor resolved elsewhere".into());
            }
            let original = info(&handle)?;
            anchors.push((handle, original));
        }
        Ok(Self { path, anchors })
    }
    fn recheck(&self) -> Result<()> {
        for (f, original) in &self.anchors {
            let i = info(f)?;
            if (i.attributes, i.volume, i.index_hi, i.index_lo)
                != (
                    original.attributes,
                    original.volume,
                    original.index_hi,
                    original.index_lo,
                )
            {
                return Err("ancestor identity/attributes changed".into());
            }
        }
        Ok(())
    }
}
struct Reserved {
    file: File,
    original: win::FileInfo,
    bytes: Vec<u8>,
}
impl Reserved {
    fn open(path: &Path, delete: bool, max: usize) -> Result<Self> {
        let file = OpenOptions::new()
            .access_mode(win::READ | if delete { win::DELETE } else { 0 })
            .share_mode(win::SHARE_READ)
            .custom_flags(win::REPARSE)
            .open(path)
            .map_err(|e| e.to_string())?;
        let original = info(&file)?;
        if original.attributes & 0x410 != 0
            || original.links != 1
            || original.size_hi != 0
            || original.size_lo as usize > max
            || unsafe { windows_sys::Win32::Storage::FileSystem::GetFileType(file.as_raw_handle()) }
                != windows_sys::Win32::Storage::FileSystem::FILE_TYPE_DISK
        {
            return Err("ordinary single-link file required".into());
        }
        let mut bytes = Vec::new();
        (&file)
            .take(max as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > max || bytes.len() != original.size_lo as usize || original.size_hi != 0 {
            return Err("fixture byte bound exceeded".into());
        }
        if guid(&file)? != path {
            return Err("file resolved through an alias or elsewhere".into());
        }
        let result = Self {
            file,
            original,
            bytes,
        };
        result.check()?;
        Ok(result)
    }
    fn check(&self) -> Result<()> {
        let i = info(&self.file)?;
        let a = &self.original;
        if (
            i.attributes,
            i.creation,
            i.write,
            i.volume,
            i.size_hi,
            i.size_lo,
            i.links,
            i.index_hi,
            i.index_lo,
        ) != (
            a.attributes,
            a.creation,
            a.write,
            a.volume,
            a.size_hi,
            a.size_lo,
            a.links,
            a.index_hi,
            a.index_lo,
        ) {
            return Err("reserved file identity/metadata changed".into());
        }
        let mut file = &self.file;
        file.rewind().map_err(|e| e.to_string())?;
        let mut actual = Vec::new();
        file.take(self.bytes.len() as u64 + 1)
            .read_to_end(&mut actual)
            .map_err(|e| e.to_string())?;
        if actual != self.bytes {
            return Err("reserved bytes changed".into());
        }
        Ok(())
    }
}
fn rename(file: &File, name: &str) -> Result<()> {
    basename(name)?;
    let name: Vec<u16> = name.encode_utf16().collect();
    let size = std::mem::size_of::<win::Rename>() + name.len() * 2;
    let mut buffer = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
    let r = buffer.as_mut_ptr().cast::<win::Rename>();
    let mut io = win::IoStatus::default();
    let status = unsafe {
        (*r).length = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*r).name).cast::<u16>(),
            name.len(),
        );
        // Zero-initialized ReplaceIfExists=false, RootDirectory=NULL. Same-parent native basename only.
        win::NtSetInformationFile(file.as_raw_handle(), &mut io, r.cast(), size as u32, 10)
    };
    if status != 0 || io.status != 0 {
        return Err(format!(
            "NTSTATUS={:#010x} IOSB={:#010x}",
            status as u32, io.status
        ));
    }
    Ok(())
}
fn published(item: &Reserved, old: &Path, target: &Path) -> Result<()> {
    item.check()?;
    if old.exists() || !target.is_file() || guid(&item.file)? != target {
        return Err("unexpected published location".into());
    }
    // Additional checking open does not replace/downgrade the original protecting handle.
    let named = OpenOptions::new()
        .read(true)
        .share_mode(7)
        .custom_flags(win::REPARSE)
        .open(target)
        .map_err(|e| e.to_string())?;
    let a = info(&named)?;
    let b = &item.original;
    if (a.volume, a.index_hi, a.index_lo) != (b.volume, b.index_hi, b.index_lo) {
        return Err("destination identity mismatch".into());
    }
    Ok(())
}

struct Inputs {
    runtime_parent: Root,
    scene_parent: Root,
    output_parent: Root,
    runtime: Reserved,
    scene: Reserved,
}
impl Inputs {
    fn check(&self) -> Result<()> {
        self.runtime_parent.recheck()?;
        self.scene_parent.recheck()?;
        self.output_parent.recheck()?;
        self.runtime.check()?;
        self.scene.check()
    }
}
fn absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
        Ok(_) => Err("output already exists; no overwrite and no execution".into()),
    }
}
pub(super) fn publish(options: &Options) -> Result<Receipt> {
    publish_with(options, process::run)
}
fn publish_with(
    options: &Options,
    mut execute: impl FnMut(&Path, process::Probe) -> Result<process::Output>,
) -> Result<Receipt> {
    validate(options)?;
    // Syntax and acknowledgement are complete before all path I/O.
    let runtime_parent = Root::open(options.runtime.parent().ok_or("runtime parent")?)?;
    let scene_parent = Root::open(options.scene.parent().ok_or("scene parent")?)?;
    let output_parent = Root::open(options.output.parent().ok_or("output parent")?)?;
    let target = output_parent
        .path
        .join(options.output.file_name().ok_or("output name")?);
    absent(&target)?;
    let runtime = Reserved::open(
        &runtime_parent
            .path
            .join(options.runtime.file_name().ok_or("runtime name")?),
        false,
        MAX_RUNTIME_BYTES,
    )?;
    let scene = Reserved::open(
        &scene_parent
            .path
            .join(options.scene.file_name().ok_or("scene name")?),
        false,
        orr_games::coop_defense_scene::MAX_SOURCE_BYTES,
    )?;
    let inputs = Inputs {
        runtime_parent,
        scene_parent,
        output_parent,
        runtime,
        scene,
    };
    let runtime_sha256 = hash(&inputs.runtime.bytes);
    if !runtime_sha256.eq_ignore_ascii_case(&options.runtime_sha256) {
        return Err("runtime SHA256 mismatch".into());
    }
    validate_pe(&inputs.runtime.bytes)?;
    let expected_summary = expected(&inputs.scene.bytes)?;
    let mut receipt = Receipt {
        status: "dry_run",
        output: options.output.clone(),
        runtime_sha256,
        runtime_bytes: inputs.runtime.bytes.len(),
        scene_sha256: hash(&inputs.scene.bytes),
        expected_summary,
        embedded_scene_verified: false,
        smoke_verified: false,
        override_rejected: false,
    };
    inputs.check()?;
    if options.dry_run {
        return Ok(receipt);
    }
    // The sole writer is create-new in a held trusted parent. Keep before writing:
    // every failure retains evidence and no cleanup can delete substituted files.
    let temporary = tempfile::Builder::new()
        .prefix(".stormglass-unvalidated-")
        .suffix(".exe")
        .tempfile_in(&inputs.output_parent.path)
        .map_err(|e| e.to_string())?;
    let (mut writer, staged) = temporary.keep().map_err(|e| e.to_string())?;
    let result = (|| {
        writer
            .write_all(&inputs.runtime.bytes)
            .and_then(|_| writer.sync_all())
            .map_err(|e| e.to_string())?;
        drop(writer);
        let artifact = Reserved::open(&staged, true, MAX_RUNTIME_BYTES)?;
        // Continuously retained READ|DELETE + SHARE_READ through all executions and rename.
        inputs.check()?;
        artifact.check()?;
        if artifact.bytes != inputs.runtime.bytes || hash(&artifact.bytes) != receipt.runtime_sha256
        {
            return Err("source-to-artifact bytes/hash mismatch BEFORE execution".into());
        }
        for probe in [
            process::Probe::SceneHash,
            process::Probe::Smoke,
            process::Probe::Override,
        ] {
            inputs.check()?;
            artifact.check()?;
            let execution = execute(&staged, probe);
            let recheck = inputs.check().and_then(|_| artifact.check());
            let output = match (execution, recheck) {
                (Ok(output), Ok(())) => output,
                (Err(error), Ok(())) => return Err(error),
                (Ok(_), Err(error)) => return Err(format!("post-execution recheck: {error}")),
                (Err(run), Err(check)) => {
                    return Err(format!("{run}; post-execution recheck: {check}"))
                }
            };
            verify_output(&output, probe, &receipt)?;
        }
        // The supervisor returns success only after direct process signal + empty Job.
        inputs.check()?;
        artifact.check()?;
        let name = options
            .output
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("output name")?;
        rename(&artifact.file, name)?;
        published(&artifact, &staged, &target).and_then(|_| inputs.check()).map_err(|e|
            format!("publication may have succeeded at {}; final verification failed: {e}; inspect before retry", target.display()))?;
        receipt.status = "published";
        receipt.embedded_scene_verified = true;
        receipt.smoke_verified = true;
        receipt.override_rejected = true;
        Ok(receipt)
    })();
    result.map_err(|e| {
        format!(
            "{e}; retained artifact location (unless published): {}",
            staged.display()
        )
    })
}
fn verify_output(output: &process::Output, probe: process::Probe, receipt: &Receipt) -> Result<()> {
    let ok = match probe {
        process::Probe::SceneHash => output.exit_code == 0 && output.stderr.is_empty()
            && output.stdout == format!("{}\n", receipt.scene_sha256).as_bytes(),
        process::Probe::Smoke => output.exit_code == 0 && output.stderr.is_empty()
            && output.stdout == format!("{}\n", receipt.expected_summary).as_bytes(),
        process::Probe::Override => output.exit_code == 1 && output.stdout.is_empty()
            && output.stderr == b"Stormglass embedded prototype: embedded game does not accept --scene; rebuild its authored scene\n",
    };
    if ok {
        Ok(())
    } else {
        Err("trusted embedded runtime check failed (scene hash, exact smoke, or override refusal); no publication".into())
    }
}
#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
