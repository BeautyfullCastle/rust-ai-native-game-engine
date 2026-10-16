//! Shared by the C ABI tests: finding or building the shared library and compiling a C client
//! of `include/orrery.h` with the system C compiler (the `cc` crate: gcc/clang, MSVC's `cl`).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `target/<profile>`, found from this test binary (`target/<profile>/deps/<test>`).
fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().and_then(Path::parent).unwrap().to_path_buf()
}

pub struct Lib {
    /// What to pass to the linker.
    pub link_arg: PathBuf,
    /// The directory the loader must find the shared library in at run time.
    pub dir: PathBuf,
}

fn library_files() -> (&'static str, &'static str, &'static str) {
    if cfg!(windows) {
        ("orr_ffi.dll", "orr_ffi.dll.lib", "orr_ffi.lib")
    } else if cfg!(target_os = "macos") {
        ("liborr_ffi.dylib", "liborr_ffi.dylib", "liborr_ffi.a")
    } else {
        ("liborr_ffi.so", "liborr_ffi.so", "liborr_ffi.a")
    }
}

fn require_lib(dir: PathBuf) -> Lib {
    let (file, link, staticlib) = library_files();
    for name in [file, link, staticlib] {
        let path = dir.join(name);
        let metadata = std::fs::metadata(&path).unwrap_or_else(|e| panic!("missing orr_ffi library artifact {}: {e}", path.display()));
        assert!(metadata.is_file() && metadata.len() > 0, "invalid orr_ffi library artifact {}: expected a nonempty file", path.display());
    }
    Lib { link_arg: dir.join(link), dir }
}

fn build_lib(cmd: &mut Command, dir: PathBuf) -> Lib {
    let status = cmd.status().expect("could not run `cargo build -p orr_ffi --lib`");
    assert!(status.success(), "`cargo build -p orr_ffi --lib` failed: {status}");
    require_lib(dir)
}

/// `cargo test` does not put the C libraries in the profile directory: build
/// them with the same profile and target directory for the external C clients.
///
/// Always runs `cargo build` (a no-op when the library is current), so a library left in the
/// target directory by an earlier run never hides a change of the code under test.
/// Build and artifact failures are fatal on every platform, independently of
/// the optional C compiler policy. Verify the static library and Windows import
/// library too, even though these tests run clients against the shared library.
pub fn ensure_lib() -> Lib {
    let dir = profile_dir();
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["build", "-p", "orr_ffi", "--lib"]);
    if dir.file_name().is_some_and(|n| n == "release") {
        cmd.arg("--release");
    }
    build_lib(&mut cmd, dir)
}

pub fn compile_c(out_dir: &Path, lib: &Lib, name: &str) -> Result<PathBuf, String> {
    let target = env!("ORR_FFI_TARGET");
    let mut build = cc::Build::new();
    build.target(target).host(target).opt_level(1).debug(false).cargo_metadata(false).cargo_warnings(false).out_dir(out_dir);
    let tool = build.try_get_compiler().map_err(|e| format!("no C compiler: {e}"))?;
    let exe = out_dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() });
    let src = crate_dir().join(format!("tests/c/{name}.c"));
    let include = crate_dir().join("include");
    let mut cmd = tool.to_command();
    if tool.is_like_msvc() {
        cmd.arg("/nologo")
            .arg("/DORR_FFI_DLL_IMPORT")
            .arg(format!("/I{}", include.display()))
            .arg(&src)
            .arg(format!("/Fe{}", exe.display()))
            .arg(format!("/Fo{}\\", out_dir.display()))
            .arg("/link")
            .arg(&lib.link_arg);
    } else {
        cmd.arg("-std=c99")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg(format!("-I{}", include.display()))
            .arg(&src)
            .arg("-o")
            .arg(&exe)
            .arg(&lib.link_arg);
        if !cfg!(windows) {
            cmd.arg(format!("-Wl,-rpath,{}", lib.dir.display()));
        }
    }
    let out = cmd.output().map_err(|e| format!("cannot run the C compiler: {e}"))?;
    if !out.status.success() {
        return Err(format!("the C compiler failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
    }
    Ok(exe)
}

pub fn skip_or_fail(why: &str) {
    if std::env::var("ORR_REQUIRE_C_COMPILER").is_ok_and(|v| v == "1") {
        panic!("{why}");
    }
    eprintln!("SKIPPED: {why} (set ORR_REQUIRE_C_COMPILER=1 to make this an error)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct Artifacts(PathBuf);

    impl Artifacts {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!("orr_ffi_artifacts_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&dir).unwrap();
            let (file, link, staticlib) = library_files();
            for name in [file, link, staticlib] {
                std::fs::write(dir.join(name), b"artifact").unwrap();
            }
            Self(dir)
        }
    }

    impl Drop for Artifacts {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn every_library_artifact_is_required() {
        let (file, link, staticlib) = library_files();
        for name in [file, link, staticlib] {
            let artifacts = Artifacts::new();
            let lib = require_lib(artifacts.0.clone());
            assert_eq!(lib.link_arg, artifacts.0.join(link));
            assert_eq!(lib.dir, artifacts.0);
            let path = artifacts.0.join(name);
            std::fs::remove_file(&path).unwrap();
            assert!(std::panic::catch_unwind(|| require_lib(artifacts.0.clone())).is_err(), "accepted missing {name}");
            std::fs::write(&path, []).unwrap();
            assert!(std::panic::catch_unwind(|| require_lib(artifacts.0.clone())).is_err(), "accepted empty {name}");
            std::fs::remove_file(&path).unwrap();
            std::fs::create_dir(&path).unwrap();
            assert!(std::panic::catch_unwind(|| require_lib(artifacts.0.clone())).is_err(), "accepted directory {name}");
        }
    }

    #[test]
    #[should_panic(expected = "could not run `cargo build -p orr_ffi --lib`")]
    fn cargo_spawn_failure_is_fatal() {
        let artifacts = Artifacts::new();
        build_lib(&mut Command::new(artifacts.0.join("missing-cargo")), artifacts.0.clone());
    }

    #[test]
    #[should_panic(expected = "`cargo build -p orr_ffi --lib` failed")]
    fn cargo_build_failure_is_fatal_even_with_existing_artifacts() {
        let artifacts = Artifacts::new();
        // A portable failing subprocess; an invalid libtest flag exits nonzero.
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.arg("--invalid-orr-ffi-test-option");
        build_lib(&mut cmd, artifacts.0.clone());
    }
}
