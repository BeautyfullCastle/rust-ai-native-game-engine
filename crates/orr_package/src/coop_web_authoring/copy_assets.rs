//! Non-atomic generated-build copy. Trusted quiescent directories are required.
//! Copies bytes, not source modes/mtime. Unrelated output files are preserved.
use crate::authoring_io::{directory, error, read_at, recheck_directory, same_snapshot, DIR_FLAGS};
use rustix::fs::{self as unix, AtFlags, Dir, FileType, Mode, OFlags, RenameFlags};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::File,
    io::Write,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_TOTAL: usize = 128 * 1024 * 1024;
const MAX_COUNT: usize = 4096;
const MAX_DEPTH: usize = 64;
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Options {
    source: PathBuf,
    output: PathBuf,
    wasm: Option<PathBuf>,
}
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty()
        || path.as_os_str().as_bytes().len() > 4096
        || path.as_os_str().as_bytes().contains(&0)
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        return Err("invalid bounded path".into());
    }
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().map_err(error)?.join(path)
    };
    let mut normalized = PathBuf::new();
    for part in path.components() {
        match part {
            Component::RootDir => normalized.push("/"),
            Component::Normal(p) => normalized.push(p),
            Component::CurDir => {}
            _ => return Err("unsafe path".into()),
        }
    }
    if normalized.as_os_str().as_bytes().len() > 4096 || normalized.components().count() > MAX_DEPTH
    {
        return Err("absolute path exceeds bound".into());
    }
    Ok(normalized)
}
struct Snapshot {
    files: BTreeMap<PathBuf, Vec<u8>>,
    dirs: BTreeSet<PathBuf>,
    total: usize,
    count: usize,
}
impl Snapshot {
    fn count(&mut self) -> Result<(), String> {
        self.count += 1;
        if self.count > MAX_COUNT {
            return Err("source entry count exceeds4096".into());
        }
        Ok(())
    }
    fn file(
        &mut self,
        relative: PathBuf,
        parent: &File,
        name: &std::ffi::OsStr,
    ) -> Result<(), String> {
        if self.files.contains_key(&relative) {
            return Ok(());
        }
        self.count()?;
        let before = unix::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if FileType::from_raw_mode(before.st_mode) != FileType::RegularFile {
            return Err("source must be regular and non-symlink".into());
        }
        let data = read_at(parent, name, MAX_FILE)?;
        self.total = self
            .total
            .checked_add(data.len())
            .ok_or("source byte overflow")?;
        if self.total > MAX_TOTAL {
            return Err("source total exceeds128MiB".into());
        }
        self.files.insert(relative, data);
        Ok(())
    }
    fn walk(&mut self, fd: &File, relative: &Path, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH || relative.as_os_str().as_bytes().len() > 4096 {
            return Err("asset path exceeds bounds".into());
        }
        if self.dirs.insert(relative.into()) {
            self.count()?;
        }
        let before = fd.metadata().map_err(error)?;
        for entry in Dir::read_from(fd).map_err(error)? {
            let entry = entry.map_err(error)?;
            let name = std::ffi::OsStr::from_bytes(entry.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            let path = relative.join(name);
            if path.as_os_str().as_bytes().len() > 4096 || depth + 1 > MAX_DEPTH {
                return Err("asset path exceeds bounds".into());
            }
            let info = unix::statat(fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
            if FileType::from_raw_mode(info.st_mode) == FileType::Directory {
                let child =
                    File::from(unix::openat(fd, name, DIR_FLAGS, Mode::empty()).map_err(error)?);
                let opened = child.metadata().map_err(error)?;
                if info.st_dev != opened.dev() || info.st_ino != opened.ino() {
                    return Err("source directory changed".into());
                }
                self.walk(&child, &path, depth + 1)?;
                let after = unix::statat(fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
                if after.st_dev != opened.dev() || after.st_ino != opened.ino() {
                    return Err("source directory replaced".into());
                }
            } else {
                self.file(path, fd, name)?;
            }
        }
        if !same_snapshot(&before, &fd.metadata().map_err(error)?) {
            return Err("source directory changed during scan".into());
        }
        Ok(())
    }
}
fn snapshot(options: &Options) -> Result<Snapshot, String> {
    let root = directory(&options.source)?;
    let mut result = Snapshot {
        files: BTreeMap::new(),
        dirs: BTreeSet::new(),
        total: 0,
        count: 0,
    };
    for name in super::offline::asset_names()
        .iter()
        .copied()
        .filter(|n| *n != "game.wasm")
    {
        let path = Path::new(name);
        let parent_path = options
            .source
            .join(path.parent().ok_or("asset parent missing")?);
        let parent = directory(&parent_path)?;
        result.file(
            path.into(),
            &parent,
            path.file_name().ok_or("asset filename missing")?,
        )?;
        recheck_directory(&parent_path, &parent)?;
        let mut ancestor = path.parent();
        while let Some(p) = ancestor {
            if p.as_os_str().is_empty() {
                break;
            }
            if result.dirs.insert(p.into()) {
                result.count()?;
            }
            ancestor = p.parent();
        }
    }
    let asset_root =
        File::from(unix::openat(&root, "assets", DIR_FLAGS, Mode::empty()).map_err(error)?);
    result.walk(&asset_root, Path::new("assets"), 1)?;
    recheck_directory(&options.source.join("assets"), &asset_root)?;
    if let Some(path) = &options.wasm {
        let parent_path = path.parent().ok_or("WASM parent missing")?;
        let parent = directory(parent_path)?;
        result.file(
            "game.wasm".into(),
            &parent,
            path.file_name().ok_or("WASM filename missing")?,
        )?;
        recheck_directory(parent_path, &parent)?;
    }
    recheck_directory(&options.source, &root)?;
    Ok(result)
}
// Inspect every existing component without following links. A missing component
// means the remaining path is absent; mkdir later uses no-follow held FDs.
fn existing(path: &Path, want_dir: bool) -> Result<Option<unix::Stat>, String> {
    let mut parent = directory(Path::new("/"))?;
    let parts: Vec<_> = path
        .components()
        .filter_map(|c| {
            if let Component::Normal(n) = c {
                Some(n)
            } else {
                None
            }
        })
        .collect();
    if parts.is_empty() {
        return if want_dir {
            Ok(Some(unix::fstat(&parent).map_err(error)?))
        } else {
            Err("output file cannot be root".into())
        };
    }
    for (i, name) in parts.iter().enumerate() {
        let info = match unix::statat(&parent, *name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(v) => v,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(e) => return Err(error(e)),
        };
        let directory_required = i + 1 < parts.len() || want_dir;
        let expected = if directory_required {
            FileType::Directory
        } else {
            FileType::RegularFile
        };
        if FileType::from_raw_mode(info.st_mode) != expected {
            return Err(format!("unsafe destination component: {}", path.display()));
        }
        if i + 1 == parts.len() {
            return Ok(Some(info));
        }
        let child =
            File::from(unix::openat(&parent, *name, DIR_FLAGS, Mode::empty()).map_err(error)?);
        let opened = child.metadata().map_err(error)?;
        if opened.dev() != info.st_dev || opened.ino() != info.st_ino {
            return Err("destination ancestor changed".into());
        }
        parent = child;
    }
    unreachable!()
}
fn identity(a: &unix::Stat, b: &unix::Stat) -> bool {
    a.st_dev == b.st_dev
        && a.st_ino == b.st_ino
        && a.st_mode == b.st_mode
        && a.st_size == b.st_size
        && a.st_mtime == b.st_mtime
        && a.st_mtime_nsec == b.st_mtime_nsec
        && a.st_ctime == b.st_ctime
        && a.st_ctime_nsec == b.st_ctime_nsec
}
fn ensure_directory(path: &Path) -> Result<File, String> {
    let mut fd = directory(Path::new("/"))?;
    for part in path.components() {
        if let Component::Normal(name) = part {
            match unix::mkdirat(&fd, name, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                Ok(()) => {
                    fd.sync_all().map_err(error)?;
                }
                Err(rustix::io::Errno::EXIST) => {}
                Err(e) => return Err(error(e)),
            }
            fd = File::from(unix::openat(&fd, name, DIR_FLAGS, Mode::empty()).map_err(error)?);
        }
    }
    recheck_directory(path, &fd)?;
    Ok(fd)
}
fn write_file(
    path: &Path,
    bytes: &[u8],
    expected: Option<&unix::Stat>,
    checkpoint: &mut impl FnMut(&str, &Path) -> Result<(), String>,
    published: &mut usize,
) -> Result<(), String> {
    let root = path.parent().ok_or("output parent missing")?;
    let parent = directory(root)?;
    let target = path.file_name().ok_or("output filename missing")?;
    let mut allocated = None;
    for _ in 0..128 {
        let name = format!(
            ".orr-web-copy-{}-{}.tmp",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        match unix::openat(
            &parent,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => {
                allocated = Some((name, File::from(fd)));
                break;
            }
            Err(rustix::io::Errno::EXIST) => {}
            Err(e) => return Err(error(e)),
        }
    }
    let (name, mut stage) = allocated.ok_or("cannot claim private copy sibling")?;
    let stage_path = root.join(&name);
    let result = (|| {
        let first = stage.metadata().map_err(error)?;
        stage.write_all(bytes).map_err(error)?;
        stage.sync_all().map_err(error)?;
        checkpoint("before-replace", path)?;
        recheck_directory(root, &parent)?;
        let staged =
            unix::statat(&parent, name.as_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if staged.st_dev != first.dev()
            || staged.st_ino != first.ino()
            || read_at(&parent, name.as_ref(), MAX_FILE)? != bytes
        {
            return Err("copy sibling identity or bytes changed".into());
        }
        let current = match unix::statat(&parent, target, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(v) => Some(v),
            Err(rustix::io::Errno::NOENT) => None,
            Err(e) => return Err(error(e)),
        };
        match (expected, current.as_ref()) {
            (Some(a), Some(b)) if identity(a, b) => {
                unix::renameat(&parent, name.as_str(), &parent, target).map_err(error)?
            }
            (None, None) => unix::renameat_with(
                &parent,
                name.as_str(),
                &parent,
                target,
                RenameFlags::NOREPLACE,
            )
            .map_err(error)?,
            _ => return Err("destination changed since batch preflight".into()),
        }
        *published += 1;
        checkpoint("after-replace", path)?;
        let out = unix::statat(&parent, target, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if out.st_dev != first.dev() || out.st_ino != first.ino() {
            return Err("published copy identity changed".into());
        }
        parent.sync_all().map_err(error)?;
        recheck_directory(root, &parent)?;
        Ok(())
    })();
    result.map_err(|e: String| {
        format!(
            "{e}; inspect owned sibling {} (may already be renamed); no automatic cleanup",
            stage_path.display()
        )
    })
}
fn copy(
    options: Options,
    mut checkpoint: impl FnMut(&str, &Path) -> Result<(), String>,
) -> Result<(), String> {
    let options = Options {
        source: absolute(&options.source)?,
        output: absolute(&options.output)?,
        wasm: options.wasm.as_deref().map(absolute).transpose()?,
    };
    if options.source.starts_with(&options.output) || options.output.starts_with(&options.source) {
        return Err("source/output overlap refused".into());
    }
    if options
        .wasm
        .as_ref()
        .is_some_and(|p| p.starts_with(&options.output))
    {
        return Err("WASM source must be outside output".into());
    }
    let snapshot = snapshot(&options)?;
    existing(&options.output, true)?;
    for dir in &snapshot.dirs {
        existing(&options.output.join(dir), true)?;
    }
    let mut expected = BTreeMap::new();
    for relative in snapshot.files.keys() {
        expected.insert(
            relative.clone(),
            existing(&options.output.join(relative), false)?,
        );
    }
    // No output creation or writes have occurred before all source admission and
    // destination preflight above. Subsequent failure may leave a partial batch.
    let mut published = 0;
    let result = (|| {
        ensure_directory(&options.output)?;
        for relative in &snapshot.dirs {
            ensure_directory(&options.output.join(relative))?;
        }
        for (relative, bytes) in &snapshot.files {
            write_file(
                &options.output.join(relative),
                bytes,
                expected[relative].as_ref(),
                &mut checkpoint,
                &mut published,
            )?;
        }
        directory(&options.output)?.sync_all().map_err(error)?;
        Ok(())
    })();
    result.map_err(|e:String|format!("copy failed after {published} file publications; generated directories/stages or a partial batch may remain; durability may be uncertain; preserve and inspect, do not rollback blindly: {e}"))
}
pub(super) fn run_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    let args: Vec<_> = args.into_iter().take(8).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("orr_pkg copy-coop-web-assets --source-dir DIR --output DIR [--wasm TRUSTED_COMPILED_FILE]\nNon-atomic bytes-only generated-build overwrite; trusted quiescent directories. Preserves unrelated files and existing game.wasm unless --wasm supplied. Does not compile or execute WASM.");
        return 0;
    }
    let parsed = (|| {
        if !matches!(args.len(), 4 | 6) || args.iter().any(|a| a.as_bytes().len() > 4096) {
            return Err("expected --source-dir DIR --output DIR [--wasm FILE]".to_owned());
        }
        let (mut source, mut output, mut wasm) = (None, None, None);
        for pair in args.chunks_exact(2) {
            match pair[0].to_str() {
                Some("--source-dir") if source.is_none() => source = Some(PathBuf::from(&pair[1])),
                Some("--output") if output.is_none() => output = Some(PathBuf::from(&pair[1])),
                Some("--wasm") if wasm.is_none() => wasm = Some(PathBuf::from(&pair[1])),
                _ => return Err("unknown or duplicate copy option".into()),
            }
        }
        Ok(Options {
            source: source.ok_or("missing source-dir")?,
            output: output.ok_or("missing output")?,
            wasm,
        })
    })();
    match parsed {
        Err(e) => {
            eprintln!("copy usage: {e}");
            2
        }
        Ok(options) => match copy(options, |_, _| Ok(())) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("copy failed: {e}");
                1
            }
        },
    }
}
#[cfg(test)]
#[path = "copy_assets_tests.rs"]
mod tests;
