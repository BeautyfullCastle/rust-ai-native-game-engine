use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const CREATE_ATTEMPTS: usize = 32;
static TEMP_FILE_SERIAL: AtomicU64 = AtomicU64::new(0);

struct TempFileGuard {
    path: Option<PathBuf>,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("temporary file guard is armed")
    }

    fn disarm(mut self) {
        self.path.take();
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

#[allow(deprecated)] // Keep the repository's Rust 1.85 minimum; newer rustc renames this API.
fn next_serial(counter: &AtomicU64) -> io::Result<u64> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |serial| {
            serial.checked_add(1)
        })
        .map_err(|_| io::Error::other("scene-save temporary filename serial exhausted"))
}

fn temporary_path(parent: &Path, process_id: u32, serial: u64) -> PathBuf {
    parent.join(format!(".orr-scene-save-{process_id}-{serial}.tmp"))
}

fn is_destination_path(candidate: &Path, destination: &Path) -> bool {
    candidate == destination
        || (candidate.parent() == destination.parent()
            && candidate.file_name() == destination.file_name())
}

fn create_temporary(destination: &Path, counter: &AtomicU64) -> io::Result<(File, TempFileGuard)> {
    let parent = destination.parent().unwrap_or_else(|| Path::new(""));
    let process_id = std::process::id();

    for _ in 0..CREATE_ATTEMPTS {
        let serial = next_serial(counter)?;
        let candidate = temporary_path(parent, process_id, serial);
        if is_destination_path(&candidate, destination) {
            continue;
        }

        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((file, TempFileGuard::new(candidate))),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("could not reserve a scene-save temporary file after {CREATE_ATTEMPTS} attempts"),
    ))
}

pub(super) fn write_atomic(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with_counter(destination, bytes, &TEMP_FILE_SERIAL)
}

fn write_atomic_with_counter(
    destination: &Path,
    bytes: &[u8],
    counter: &AtomicU64,
) -> io::Result<()> {
    let (mut file, temporary) = create_temporary(destination, counter)?;
    let write_result = file.write_all(bytes).and_then(|()| file.sync_all());

    // Close before the guard removes a failed temporary file; this also permits cleanup on Windows.
    drop(file);
    write_result?;

    fs::rename(temporary.path(), destination)?;
    // The old name is gone. Do not let Drop remove a later file created at that name.
    temporary.disarm();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{create_temporary, temporary_path, write_atomic_with_counter, CREATE_ATTEMPTS};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    struct TestRoot(PathBuf);

    impl TestRoot {
        #[allow(deprecated)] // Keep test fixture code compatible with the repository's Rust MSRV.
        fn new() -> Self {
            for _ in 0..CREATE_ATTEMPTS {
                let serial = NEXT_TEST_ROOT
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                        value.checked_add(1)
                    })
                    .expect("test root serial does not exhaust");
                let path = std::env::temp_dir().join(format!(
                    "orr-scene-save-test-{}-{serial}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot create scene-save test root: {error}"),
                }
            }
            panic!("scene-save test root creation attempts exhausted")
        }

        fn destination(&self) -> PathBuf {
            self.0.join("level.scene.yaml")
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn candidate(destination: &Path, serial: u64) -> PathBuf {
        temporary_path(
            destination.parent().unwrap_or_else(|| Path::new("")),
            std::process::id(),
            serial,
        )
    }

    #[test]
    fn create_new_skips_existing_regular_file_and_guard_cleans_only_owned_temp() {
        let root = TestRoot::new();
        let destination = root.destination();
        let existing = candidate(&destination, 0);
        fs::write(&existing, b"foreign regular file").unwrap();
        let counter = AtomicU64::new(0);

        let (file, guard) = create_temporary(&destination, &counter).unwrap();
        assert_eq!(guard.path(), candidate(&destination, 1));
        drop(file);
        drop(guard);

        assert_eq!(fs::read(existing).unwrap(), b"foreign regular file");
        assert!(!candidate(&destination, 1).exists());
    }

    #[cfg(unix)]
    #[test]
    fn create_new_skips_existing_symlink_without_following_or_removing_it() {
        let root = TestRoot::new();
        let destination = root.destination();
        let sentinel = root.0.join("sentinel");
        fs::write(&sentinel, b"preserve sentinel").unwrap();
        let existing = candidate(&destination, 0);
        std::os::unix::fs::symlink(&sentinel, &existing).unwrap();
        let counter = AtomicU64::new(0);

        let (file, guard) = create_temporary(&destination, &counter).unwrap();
        assert_eq!(guard.path(), candidate(&destination, 1));
        drop(file);
        drop(guard);

        assert!(fs::symlink_metadata(&existing)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(sentinel).unwrap(), b"preserve sentinel");
        assert!(!candidate(&destination, 1).exists());
    }

    #[test]
    fn create_new_stops_after_32_collisions_without_changing_existing_files() {
        let root = TestRoot::new();
        let destination = root.destination();
        for serial in 0..CREATE_ATTEMPTS as u64 {
            fs::write(
                candidate(&destination, serial),
                format!("collision-{serial}"),
            )
            .unwrap();
        }
        let counter = AtomicU64::new(0);

        let error = create_temporary(&destination, &counter).err().unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(counter.load(Ordering::Relaxed), CREATE_ATTEMPTS as u64);
        for serial in 0..CREATE_ATTEMPTS as u64 {
            assert_eq!(
                fs::read(candidate(&destination, serial)).unwrap(),
                format!("collision-{serial}").as_bytes()
            );
        }
        assert!(!candidate(&destination, CREATE_ATTEMPTS as u64).exists());
    }

    #[test]
    fn create_new_reports_counter_exhaustion_without_wrapping() {
        let root = TestRoot::new();
        let destination = root.destination();
        let counter = AtomicU64::new(u64::MAX);

        let error = create_temporary(&destination, &counter).err().unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        assert!(!candidate(&destination, 0).exists());
    }

    #[test]
    fn destination_equal_to_generated_temp_name_is_skipped() {
        let root = TestRoot::new();
        let destination = candidate(&root.destination(), 0);
        fs::write(&destination, b"destination bytes").unwrap();
        let counter = AtomicU64::new(0);

        let (file, guard) = create_temporary(&destination, &counter).unwrap();
        assert_eq!(guard.path(), candidate(&root.destination(), 1));
        drop(file);
        drop(guard);

        assert_eq!(fs::read(destination).unwrap(), b"destination bytes");
        assert!(!candidate(&root.destination(), 1).exists());
    }

    #[test]
    fn successful_rename_replaces_destination_and_leaves_no_owned_temp() {
        let root = TestRoot::new();
        let destination = root.destination();
        fs::write(&destination, b"old scene").unwrap();
        let counter = AtomicU64::new(0);

        write_atomic_with_counter(&destination, b"new scene", &counter).unwrap();

        assert_eq!(fs::read(destination).unwrap(), b"new scene");
        assert!(!candidate(&root.destination(), 0).exists());
    }

    #[test]
    fn failed_rename_preserves_destination_and_cleans_owned_temp() {
        let root = TestRoot::new();
        let destination = root.destination();
        fs::create_dir(&destination).unwrap();
        let witness = destination.join("witness");
        fs::write(&witness, b"keep destination directory").unwrap();
        let counter = AtomicU64::new(0);

        assert!(write_atomic_with_counter(&destination, b"new scene", &counter).is_err());

        assert!(destination.is_dir());
        assert_eq!(fs::read(witness).unwrap(), b"keep destination directory");
        assert!(!candidate(&root.destination(), 0).exists());
    }
}
