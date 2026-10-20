//! UI-only durable command evidence. The legacy #279 recorder is unchanged.
use std::{fs, io::Write, path::Path, process::Command};
const CONSUMER_ENV: [&str; 6] = [
    "ORR_UI_CONSUMER_BUNDLE",
    "ORR_UI_CONSUMER_EVIDENCE",
    "ORR_UI_CONSUMER_SOURCE_PROJECT",
    "ORR_UI_CONSUMER_SOURCE_ROOT",
    "ORR_UI_CONSUMER_SOURCE_RUNTIME",
    "ORR_UI_CONSUMER_SOURCE_PRODUCER",
];
const GPU_ENV: [&str; 5] = [
    "VK_ICD_FILENAMES",
    "VK_DRIVER_FILES",
    "WGPU_BACKEND",
    "ORR_REQUIRE_GPU",
    "XDG_RUNTIME_DIR",
];
fn effective_gpu_environment(
    command: &Command,
) -> std::collections::BTreeMap<String, Option<String>> {
    // All callers inherit the process environment and may explicitly override or
    // remove keys. No caller uses env_clear; this covers the actual child values.
    GPU_ENV
        .into_iter()
        .map(|key| {
            let value = match command
                .get_envs()
                .find(|(name, _)| *name == std::ffi::OsStr::new(key))
            {
                Some((_, value)) => value.map(std::ffi::OsStr::to_os_string),
                None => std::env::var_os(key),
            }
            .map(|value| {
                let value = value
                    .to_str()
                    .expect("GPU environment must be UTF-8")
                    .to_owned();
                assert!(value.len() <= 4096, "bounded GPU environment value");
                value
            });
            (key.to_owned(), value)
        })
        .collect()
}
pub fn recorded(root: &Path, stage: &str, command: &mut Command) -> String {
    recorded_status(root, stage, command, 0)
}
pub fn recorded_status(
    root: &Path,
    stage: &str,
    command: &mut Command,
    expected_status: i32,
) -> String {
    assert!(matches!(expected_status, 0 | 1));
    let environment: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(name, value)| {
            let name = name.to_str()?;
            if !CONSUMER_ENV.contains(&name) {
                return None;
            }
            let value = value.map(|value| {
                value
                    .to_str()
                    .expect("consumer environment must be UTF-8")
                    .to_owned()
            });
            assert!(
                value.as_ref().is_none_or(|value| value.len() <= 4096),
                "bounded consumer environment value"
            );
            Some((name.to_owned(), value))
        })
        .collect();
    let gpu_environment = effective_gpu_environment(command);
    let stdout_file = format!("{stage}.stdout.txt");
    let stderr_file = format!("{stage}.stderr.txt");
    let stdout = fs::File::create(root.join(&stdout_file)).unwrap();
    let stderr = fs::File::create(root.join(&stderr_file)).unwrap();
    // Direct file descriptors retain child output even if the outer gate times out.
    command.stdout(stdout.try_clone().unwrap());
    command.stderr(stderr.try_clone().unwrap());
    let mut record = serde_json::json!({"stage":stage,"expected_status":expected_status,"environment":environment,"gpu_environment":gpu_environment,"program":command.get_program().to_string_lossy(),"args":command.get_args().map(|s|s.to_string_lossy().into_owned()).collect::<Vec<_>>(),"cwd":command.get_current_dir().map(|p|p.display().to_string()),"status":null,"stdout_file":stdout_file,"stderr_file":stderr_file});
    let state_path = root.join(format!("{stage}.command-state.json"));
    let persist_state = |state: &str, record: &serde_json::Value, error: Option<String>| {
        let mut state_record = record.clone();
        state_record["state"] = state.into();
        state_record["error"] = error.into();
        let temporary = state_path.with_extension("json.tmp");
        let mut file = fs::File::create(&temporary).unwrap();
        writeln!(file, "{state_record}").unwrap();
        file.sync_all().unwrap();
        fs::rename(temporary, &state_path).unwrap();
        fs::File::open(root).unwrap().sync_all().unwrap();
    };
    // This is durable before spawn, separate from the completed-record gate schema.
    persist_state("started", &record, None);
    let mut child = command.spawn().unwrap_or_else(|error| {
        persist_state("spawn_failed", &record, Some(error.to_string()));
        panic!("launch recorded acceptance command {stage}: {error}");
    });
    let status = child.wait().unwrap_or_else(|error| {
        persist_state("wait_failed", &record, Some(error.to_string()));
        panic!("wait for recorded acceptance command {stage}: {error}");
    });
    stdout.sync_all().unwrap();
    stderr.sync_all().unwrap();
    record["status"] = status.code().into();
    record["stdout_sha256"] = super::evidence::digest(&root.join(&stdout_file)).into();
    record["stderr_sha256"] = super::evidence::digest(&root.join(&stderr_file)).into();
    persist_state("completed", &record, None);
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("commands.jsonl"))
        .unwrap();
    writeln!(log, "{record}").unwrap();
    log.sync_all().unwrap();
    assert!(
        status.code() == Some(expected_status),
        "{stage}: {status}: {}",
        String::from_utf8_lossy(&fs::read(root.join(&stderr_file)).unwrap())
    );
    String::from_utf8(fs::read(root.join(&stdout_file)).unwrap()).unwrap()
}

pub struct ConsumerPaths<'a> {
    pub repository: &'a Path,
    pub project: &'a Path,
    pub runtime: &'a Path,
    pub producer: &'a Path,
    pub bundle: &'a Path,
    pub evidence: &'a Path,
}
pub fn consumer_command<'a>(
    command: &'a mut Command,
    paths: ConsumerPaths<'_>,
    executable: &Path,
    test: &str,
) -> &'a mut Command {
    command.args(["--", "/bin/sh", "-c", "test ! -e \"$1/Cargo.toml\" && test ! -e \"$2/orr.project.json\" && test ! -e \"$3\" && test ! -e \"$4\" || exit 91; if touch \"$5/.ui-consumer-write-probe\" 2>/dev/null; then rm -f \"$5/.ui-consumer-write-probe\"; exit 92; fi; printf 'consumer-source-hidden=true consumer-runtime-hidden=true consumer-producer-hidden=true consumer-bundle-read-only=true\\n'; shift 5; exec \"$@\"", "sh"])
        .arg(paths.repository).arg(paths.project).arg(paths.runtime).arg(paths.producer).arg(paths.bundle)
        .arg(executable).args(["--ignored", "--exact", test, "--nocapture"])
        .env(CONSUMER_ENV[0], paths.bundle).env(CONSUMER_ENV[1], paths.evidence)
        .env(CONSUMER_ENV[2], paths.project).env(CONSUMER_ENV[3], paths.repository)
        .env(CONSUMER_ENV[4], paths.runtime).env(CONSUMER_ENV[5], paths.producer)
}
