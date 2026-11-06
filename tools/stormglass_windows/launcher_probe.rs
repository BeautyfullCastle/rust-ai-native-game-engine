//! Test-only argv/exit recorder, not a game runtime or gameplay acceptance.
use std::{env, fs};
fn main() {
    let path = env::var_os("ORR_LAUNCHER_PROBE_RECORD").expect("test record path required");
    let arguments = env::args().skip(1).collect::<Vec<_>>().join("\0");
    fs::write(path, arguments).expect("write test argv");
    if let Ok(message) = env::var("ORR_LAUNCHER_PROBE_STDERR") {
        eprintln!("{message}");
    }
    let code = env::var("ORR_LAUNCHER_PROBE_EXIT")
        .unwrap_or_else(|_| "0".into())
        .parse::<i32>()
        .expect("test exit code");
    std::process::exit(code);
}
