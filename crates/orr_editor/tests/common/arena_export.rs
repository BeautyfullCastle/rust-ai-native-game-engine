//! Shared read-only evidence helpers. Each caller supplies its own evidence root.
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::EditorApp;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path, process::Command};
pub fn field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(2);
}
pub fn digest(path: &Path) -> String {
    Sha256::digest(fs::read(path).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub fn recorded(root: &Path, stage: &str, command: &mut Command) -> String {
    let stdout_file = format!("{stage}.stdout.txt");
    let stderr_file = format!("{stage}.stderr.txt");
    let stdout = fs::File::create(root.join(&stdout_file)).unwrap();
    let stderr = fs::File::create(root.join(&stderr_file)).unwrap();
    // Direct file descriptors retain child output even if the outer gate times out.
    command.stdout(stdout.try_clone().unwrap());
    command.stderr(stderr.try_clone().unwrap());
    let mut record = serde_json::json!({"stage":stage,"program":command.get_program().to_string_lossy(),"args":command.get_args().map(|s|s.to_string_lossy().into_owned()).collect::<Vec<_>>(),"cwd":command.get_current_dir().map(|p|p.display().to_string()),"status":null,"stdout_file":stdout_file,"stderr_file":stderr_file});
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
    persist_state("completed", &record, None);
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("commands.jsonl"))
        .unwrap();
    writeln!(log, "{record}").unwrap();
    log.sync_all().unwrap();
    assert!(
        status.success(),
        "{stage}: {status}: {}",
        String::from_utf8_lossy(&fs::read(root.join(&stderr_file)).unwrap())
    );
    String::from_utf8(fs::read(root.join(&stdout_file)).unwrap()).unwrap()
}
pub fn inventory(root: &Path) -> Vec<serde_json::Value> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<serde_json::Value>) {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            let meta = fs::symlink_metadata(&path).unwrap();
            assert!(!meta.file_type().is_symlink());
            if meta.is_dir() {
                visit(root, &path, out);
            } else {
                assert!(meta.is_file());
                out.push(serde_json::json!({"path":path.strip_prefix(root).unwrap().to_str().unwrap(),"bytes":meta.len(),"sha256":digest(&path)}));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out
}
pub fn git(repo: &Path, arg: &str) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", arg])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().into()
}
pub fn png(path: &Path) -> Vec<u8> {
    let mut reader = png::Decoder::new(fs::File::open(path).unwrap())
        .read_info()
        .unwrap();
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!((info.width, info.height), (512, 512));
    bytes.truncate(info.buffer_size());
    bytes
}
