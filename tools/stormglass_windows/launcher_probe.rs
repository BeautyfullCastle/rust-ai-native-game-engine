//! Test-only argv/exit recorder, not a game runtime or gameplay acceptance.
use std::{env, fs};
fn main() {
    let helper =
        env::current_exe().unwrap().file_stem().unwrap() == "orr_prepare_stormglass_settings";
    let prefix = if helper {
        "ORR_LAUNCHER_HELPER"
    } else {
        "ORR_LAUNCHER_PROBE"
    };
    let path = env::var_os(format!("{prefix}_RECORD")).expect("test record path required");
    let arguments = env::args().skip(1).collect::<Vec<_>>().join("\0");
    fs::write(path, arguments).expect("write test argv");
    if let Ok(message) = env::var(format!("{prefix}_STDERR")) {
        eprintln!("{message}");
    }
    let code = env::var(format!("{prefix}_EXIT"))
        .unwrap_or_else(|_| "0".into())
        .parse::<i32>()
        .expect("test exit code");
    std::process::exit(code);
}
