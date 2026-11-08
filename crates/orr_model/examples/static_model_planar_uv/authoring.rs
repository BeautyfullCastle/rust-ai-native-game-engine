//! Private GPU-free cooked-model planar UV authoring. No runtime changes.
//! Paths require trusted stable ancestry; these Linux CLI checks are not a
//! sandbox against hostile concurrent directory replacement. Never overwrites.
#![allow(clippy::float_arithmetic)]
pub use orr_model::static_planar_uv::{project_model_uv, MAX_RECIPE_BYTES};
use orr_model::MAX_FILE_BYTES;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static STAGE_ID: AtomicU64 = AtomicU64::new(0);

fn checked_path(path: &Path, output: bool) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        return Err("empty, parent-traversal or platform-prefixed paths unsupported".into());
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let parts: Vec<_> = absolute
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    let mut checked = PathBuf::new();
    for (i, c) in parts.iter().enumerate() {
        if let Component::Normal(name) = c {
            let name = name.to_str().ok_or("filesystem paths must be UTF-8")?;
            if name.ends_with(['.', ' ']) || {
                let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
                ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                    || (stem.len() == 4
                        && (stem.starts_with("COM") || stem.starts_with("LPT"))
                        && stem.as_bytes()[3].is_ascii_digit())
            } {
                return Err("nonportable path component".into());
            }
            if output && name.eq_ignore_ascii_case(".orr") {
                return Err("output cannot be inside an .orr package store".into());
            }
        }
        checked.push(c.as_os_str());
        match fs::symlink_metadata(&checked) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err("symlink path/ancestry unsupported".into())
            }
            Ok(m) if i + 1 < parts.len() && !m.is_dir() => {
                return Err("path ancestor is not a directory".into())
            }
            Ok(_) => {}
            Err(e) if output && i + 1 == parts.len() && e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("path metadata: {e}")),
        }
    }
    Ok(checked)
}
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}
fn owned_path(path: &Path, file: &File) -> Result<(), String> {
    let path_meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let file_meta = file.metadata().map_err(|e| e.to_string())?;
    if !path_meta.file_type().is_file()
        || !file_meta.is_file()
        || !same_file(&path_meta, &file_meta)
    {
        return Err("file path identity changed; refusing to operate on replacement".into());
    }
    Ok(())
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn safe_open(path: &Path) -> Result<File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    // Linux x86_64 O_NOFOLLOW | O_NONBLOCK: reject final symlinks atomically and
    // avoid blocking if a regular input was replaced by a FIFO before open.
    OpenOptions::new()
        .read(true)
        .custom_flags(0x20000 | 0x800)
        .open(path)
        .map_err(|e| format!("nofollow input open: {e}"))
}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn safe_open(_path: &Path) -> Result<File, String> {
    Err("safe nofollow/nonblocking file operations currently require Linux x86_64; pure planar UV API is portable".into())
}
fn read_input(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let path = checked_path(path, false)?;
    let before = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !before.file_type().is_file() || before.len() > limit as u64 {
        return Err("input must be a bounded regular file".into());
    }
    let file = safe_open(&path)?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if !opened.is_file()
        || !same_file(&before, &opened)
        || before.len() != opened.len()
        || before.modified().ok() != opened.modified().ok()
    {
        return Err("input changed during open".into());
    }
    owned_path(&path, &file)?;
    let mut bounded = file.take(limit as u64 + 1);
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    bounded.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let after = bounded.get_ref().metadata().map_err(|e| e.to_string())?;
    if bytes.len() > limit
        || bytes.len() as u64 != opened.len()
        || after.len() != opened.len()
        || after.modified().ok() != opened.modified().ok()
    {
        return Err("input changed during bounded read".into());
    }
    checked_path(&path, false)?;
    owned_path(&path, bounded.get_ref())?;
    Ok(bytes)
}
fn output_path(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("output needs portable filename")?;
    logical_path(name, ".model.json")?;
    if name.len() > 100 {
        return Err("output filename exceeds 100 bytes".into());
    }
    let path = checked_path(path, true)?;
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(path),
        Ok(_) => Err("output already exists; no overwrite/force supported".into()),
        Err(e) => Err(e.to_string()),
    }
}
fn publish(output: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("output exceeds 64 MiB limit".into());
    }
    let destination = output_path(output)?;
    let parent = destination
        .parent()
        .ok_or("output requires an existing parent")?;
    let mut stage = None;
    for _ in 0..32 {
        let path = parent.join(format!(
            ".orr-static-planar-uv-{}-{}.tmp",
            std::process::id(),
            STAGE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                stage = Some((path, file));
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("create owned output stage: {e}")),
        }
    }
    let (path, mut file) = stage.ok_or("stage-name budget exhausted")?;
    let commit = (|| {
        owned_path(&path, &file)?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut readback = Vec::with_capacity(bytes.len());
        (&mut file)
            .take(bytes.len() as u64 + 1)
            .read_to_end(&mut readback)
            .map_err(|e| e.to_string())?;
        if readback != bytes {
            return Err("owned stage readback differs".into());
        }
        output_path(&destination)?;
        checked_path(&path, false)?;
        owned_path(&path, &file)?;
        fs::hard_link(&path, &destination)
            .map_err(|e| format!("no-replace hard-link publication required: {e}"))?;
        Ok::<(), String>(())
    })();
    // Only remove the entry if it still identifies our open stage. A replacement
    // is never unlinked, including after a failed publication attempt.
    let cleanup =
        owned_path(&path, &file).and_then(|()| fs::remove_file(&path).map_err(|e| e.to_string()));
    if let Err(error) = commit {
        return Err(format!(
            "not published: {error}{}",
            cleanup
                .err()
                .map(|e| format!("; stage cleanup refused/failed: {e}"))
                .unwrap_or_default()
        ));
    }
    if let Err(error) = cleanup {
        let _ = writeln!(
            io::stderr(),
            "warning: published; stage cleanup refused/failed: {error}"
        );
    }
    if let Err(error) = File::open(parent).and_then(|f| f.sync_all()) {
        let _ = writeln!(
            io::stderr(),
            "warning: published; directory durability uncertain: {error}"
        );
    }
    Ok(())
}
/// Strict bounded CLI. The recipe parent anchors the normalized cooked-source input.
pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args: Vec<_> = args.into_iter().take(8).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        return writeln!(std::io::stdout(), "Usage: static_model_planar_uv --recipe FILE.planar-uv.json --asset-id variants/tile.planar-uv.json --output NEW.model.json\nRequires import. Project model-space positions onto xy/xz/yz to generate UV0 for one selected material; not unwrap or seam generation. Linux x86_64 filesystem CLI; recipe-relative inputs; trusted stable link-free parents; no overwrite/install. Exit 0: help/published (check warnings); 2: error/not published.").map_err(|e| e.to_string());
    }
    if args.len() != 6 {
        return Err("expected exactly three unique option/value pairs; use --help".into());
    }
    let mut values = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        let key = pair[0].to_str().ok_or("option names must be UTF-8")?;
        if !["--recipe", "--asset-id", "--output"].contains(&key) {
            return Err(format!("unknown option {key}"));
        }
        if pair[1].as_os_str().len() > 4096 {
            return Err(format!("{key} value is too long"));
        }
        if values.insert(key, &pair[1]).is_some() {
            return Err(format!("duplicate option {key}"));
        }
    }
    let get = |key: &str| {
        values
            .get(key)
            .copied()
            .ok_or_else(|| format!("missing {key}"))
    };
    let asset_id = get("--asset-id")?
        .to_str()
        .ok_or("--asset-id must be UTF-8")?;
    let output = output_path(Path::new(get("--output")?))?;
    let recipe_path = checked_path(Path::new(get("--recipe")?), false)?;
    let bytes = read_input(&recipe_path, MAX_RECIPE_BYTES)?;
    let parent = recipe_path
        .parent()
        .ok_or("recipe requires a trusted parent")?;
    let mut remaining = MAX_FILE_BYTES - bytes.len();
    let model = project_model_uv(&bytes, asset_id, |relative| {
        let input = read_input(&parent.join(relative), remaining)?;
        remaining -= input.len();
        Ok(input)
    })?;
    let cooked = model.to_bytes().map_err(|e| e.to_string())?;
    publish(&output, &cooked)?;
    let _ = writeln!(
        std::io::stdout(),
        "published {} bytes to {}",
        cooked.len(),
        output.display()
    );
    Ok(())
}
