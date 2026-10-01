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

fn find_lib() -> Option<Lib> {
    let dir = profile_dir();
    let (file, link) = if cfg!(windows) {
        ("orr_ffi.dll", "orr_ffi.dll.lib")
    } else if cfg!(target_os = "macos") {
        ("liborr_ffi.dylib", "liborr_ffi.dylib")
    } else {
        ("liborr_ffi.so", "liborr_ffi.so")
    };
    (dir.join(file).exists() && dir.join(link).exists()).then(|| Lib { link_arg: dir.join(link), dir })
}

/// `cargo test` builds the rlib of this crate, not its cdylib: build it (same
/// profile, same target directory) if it is not there yet.
///
/// Always runs `cargo build` (a no-op when the library is current), so a library left in the
/// target directory by an earlier run never hides a change of the code under test.
pub fn ensure_lib() -> Option<Lib> {
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["build", "-p", "orr_ffi", "--lib"]);
    if profile_dir().file_name().is_some_and(|n| n == "release") {
        cmd.arg("--release");
    }
    let status = cmd.status().ok()?;
    status.success().then(find_lib).flatten()
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

