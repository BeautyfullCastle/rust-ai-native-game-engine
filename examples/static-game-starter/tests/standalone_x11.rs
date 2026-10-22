//! Required dev-only source-hidden native acceptance. Missing capabilities fail.
#[path = "support/standalone_x11.rs"]
mod x11;
use crate::x11::Deadline;
use orr_static_starter::{delivery::DeliveryGame, switchboard::SwitchGame};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const LIMIT: Duration = Duration::from_secs(20);
const ENTER: u32 = 0xff0d;
const ESCAPE: u32 = 0xff1b;
const SPACE: u32 = 0x20;
const RIGHT: u32 = 0x64;
const SCHEMA: &str = "orr.static-standalone-native/1";

fn hash(path: &Path) -> Result<String> {
    Ok(Sha256::digest(fs::read(path)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(&serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
struct OwnedProcess(Child, Vec<PathBuf>);
impl OwnedProcess {
    fn check_logs(&self) -> Result<()> {
        for path in &self.1 {
            if fs::metadata(path)?.len() > 8 * 1024 * 1024 {
                return Err(format!("owned process output overflow: {}", path.display()).into());
            }
        }
        Ok(())
    }
    fn wait(&mut self) -> Result<()> {
        let until = Deadline::after(LIMIT);
        loop {
            self.check_logs()?;
            if let Some(status) = self.0.try_wait()? {
                if !status.success() {
                    return Err(format!("owned process failed: {status}").into());
                }
                return Ok(());
            }
            if until.expired() {
                return Err("owned process timeout".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Observations {
    path: PathBuf,
    values: Vec<Value>,
}
impl Observations {
    fn refresh(&mut self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        if fs::symlink_metadata(&self.path)?.file_type().is_symlink()
            || fs::metadata(&self.path)?.len() > 32 * 1024 * 1024
        {
            return Err("unsafe/oversize observation stream".into());
        }
        let bytes = fs::read(&self.path)?;
        let end = bytes.iter().rposition(|v| *v == b'\n').map_or(0, |i| i + 1);
        let values: Vec<Value> = bytes[..end]
            .split(|v| *v == b'\n')
            .filter(|line| !line.is_empty())
            .map(serde_json::from_slice)
            .collect::<std::result::Result<_, _>>()?;
        if values.len() < self.values.len() || values[..self.values.len()] != self.values {
            return Err("observation prefix was rewritten".into());
        }
        for pair in values.windows(2) {
            if number(&pair[1], "sequence")? != number(&pair[0], "sequence")? + 1
                || number(&pair[1], "tick")? < number(&pair[0], "tick")?
            {
                return Err("nonmonotone presentation stream".into());
            }
        }
        self.values = values;
        Ok(())
    }
    fn until(
        &mut self,
        process: &mut OwnedProcess,
        predicate: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        let until = Deadline::after(LIMIT);
        loop {
            process.check_logs()?;
            self.refresh()?;
            if let Some(value) = self.values.last().filter(|v| predicate(v)) {
                return Ok(value.clone());
            }
            if process.0.try_wait()?.is_some() {
                return Err("player exited before required presentation".into());
            }
            if until.expired() {
                return Err("required presentation timed out".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
fn number(v: &Value, key: &str) -> Result<u64> {
    v[key]
        .as_u64()
        .ok_or_else(|| format!("missing integer {key}").into())
}
fn tap(observer: &x11::Observer, key: u32) -> Result<()> {
    observer.key(key, true)?;
    observer.key(key, false)
}
fn neutral(alias: &str) -> Value {
    if alias == "delivery" {
        json!({"dx":0,"dy":0,"deliver":0})
    } else {
        json!({"selection":0,"toggle":0})
    }
}
fn state_without_held(v: &Value) -> Value {
    let mut v = v.clone();
    if let Some(map) = v.as_object_mut() {
        map.remove("held");
    }
    v
}
fn paused_proof(
    obs: &mut Observations,
    process: &mut OwnedProcess,
    before: &Value,
) -> Result<Value> {
    let seq = number(before, "sequence")?;
    let after = obs.until(process, |v| {
        v["sequence"].as_u64().is_some_and(|s| s >= seq + 6)
    })?;
    for v in obs
        .values
        .iter()
        .filter(|v| v["sequence"].as_u64().is_some_and(|s| s > seq))
    {
        if v["paused"] != true
            || v["tick"] != before["tick"]
            || v["state"] != before["state"]
            || v["consumed"].as_array().is_none_or(|a| !a.is_empty())
        {
            return Err("paused/focus-regained player secretly advanced or resumed".into());
        }
    }
    Ok(after)
}
fn clear_proof(
    alias: &str,
    observer: &x11::Observer,
    window: u32,
    process: &mut OwnedProcess,
    obs: &mut Observations,
    focus_loss: bool,
) -> Result<Value> {
    if alias == "switchboard" {
        let deadline = Deadline::after(LIMIT);
        loop {
            let stable = obs.until(process, |v| v["paused"] == true)?;
            if stable["state"]["selected"].as_u64().is_some_and(|n| n != 0) {
                break;
            }
            if deadline.expired() {
                return Err("could not physically select non-target switch".into());
            }
            tap(observer, ENTER)?;
            obs.until(process, |v| {
                v["paused"] == false && v["resume_neutral_pending"] == false
            })?;
            observer.key(RIGHT, true)?;
            obs.until(process, |v| {
                v["state"]["selected"].as_u64().is_some_and(|n| n != 0)
            })?;
            tap(observer, ESCAPE)?;
            obs.until(process, |v| v["paused"] == true)?;
            observer.key(RIGHT, false)?;
        }
    }
    tap(observer, ENTER)?;
    obs.until(process, |v| {
        v["paused"] == false && v["resume_neutral_pending"] == false
    })?;
    observer.key(SPACE, true)?;
    obs.until(process, |v| {
        v["state"]["held"] == 1 && v["state"]["won"] == 0
    })?;
    observer.key(RIGHT, true)?;
    obs.until(process, |v| {
        v["consumed"].as_array().is_some_and(|a| {
            a.iter().any(|t| {
                t["input"]
                    == if alias == "delivery" {
                        json!({"dx":1,"dy":0,"deliver":1})
                    } else {
                        json!({"selection":1,"toggle":1})
                    }
            })
        })
    })?;
    if focus_loss {
        observer.blur()?;
    } else {
        tap(observer, ESCAPE)?;
    }
    let paused = obs.until(process, |v| v["paused"] == true)?;
    if paused["state"]["won"] != 0 || paused["state"]["held"] != 1 {
        return Err("terminal state masked clear proof".into());
    }
    observer.key(RIGHT, false)?;
    observer.key(SPACE, false)?;
    if focus_loss {
        observer.focus(window)?;
    }
    let no_auto = paused_proof(obs, process, &paused)?;
    tap(observer, ENTER)?;
    let resumed = obs.until(process, |v| {
        v["paused"] == false
            && v["neutral_consumed_tick"]
                .as_u64()
                .is_some_and(|t| t > paused["tick"].as_u64().unwrap_or(u64::MAX))
    })?;
    let first_tick = number(&paused, "tick")? + 1;
    let first = obs
        .values
        .iter()
        .flat_map(|v| v["consumed"].as_array().into_iter().flatten())
        .find(|v| v["tick"] == first_tick)
        .ok_or("missing first naturally resumed consumed tick")?
        .clone();
    if first["input"] != neutral(alias)
        || state_without_held(&first["state"]) != state_without_held(&paused["state"])
        || first["state"]["held"] != 0
        || resumed["resume_from_tick"] != paused["tick"]
    {
        return Err("first actual resumed tick was not neutral and stationary".into());
    }
    tap(observer, ESCAPE)?;
    obs.until(process, |v| v["paused"] == true)?;
    Ok(json!({"paused":paused,"no_auto_resume":no_auto,"resumed":resumed,"first_consumed":first}))
}
fn capture(
    observer: &x11::Observer,
    window: u32,
    obs: &mut Observations,
    process: &mut OwnedProcess,
    out: &Path,
    phase: &str,
) -> Result<Value> {
    let paused = obs.until(process, |v| v["paused"] == true)?;
    let before = paused_proof(obs, process, &paused)?;
    let deadline = Deadline::after(LIMIT);
    let mut image = observer.capture(window)?;
    loop {
        let sequence = number(obs.values.last().ok_or("empty capture stream")?, "sequence")?;
        obs.until(process, |v| {
            v["sequence"].as_u64().is_some_and(|s| s >= sequence + 2)
        })?;
        let next = observer.capture(window)?;
        if next.spec.width == image.spec.width
            && next.spec.height == image.spec.height
            && next.visual == image.visual
            && next.rgba == image.rgba
        {
            break;
        }
        if deadline.expired() {
            return Err("presented native framebuffer did not stabilize".into());
        }
        image = next;
    }
    let path = out.join(format!("{phase}.png"));
    image.png(&path)?;
    obs.refresh()?;
    let after = obs.values.last().ok_or("empty stream")?;
    if before["state"] != after["state"]
        || before["tick"] != after["tick"]
        || after["paused"] != true
    {
        return Err("capture crossed a simulation transition".into());
    }
    Ok(
        json!({"phase":phase,"observation":before,"png":format!("{phase}.png"),"sha256":hash(&path)?,
        "x11":{"visual":image.visual,"depth":image.spec.depth,"bits_per_pixel":image.spec.bits,
        "scanline_pad":image.spec.pad,"lsb_first":image.spec.lsb,"masks":image.spec.masks,
        "width":image.spec.width,"height":image.spec.height}}),
    )
}

fn isolated(bundle: &Path, out: &Path, display: &str, hidden: &[PathBuf]) -> Result<OwnedProcess> {
    let socket = format!(
        "/tmp/.X11-unix/X{}",
        display.strip_prefix(':').ok_or("display format")?
    );
    let mut command = Command::new("/usr/bin/bwrap");
    command.env_clear().args([
        "--die-with-parent",
        "--new-session",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--clearenv",
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind",
        "/bin",
        "/bin",
        "--ro-bind",
        "/lib",
        "/lib",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/tmp/.X11-unix",
        "--ro-bind",
        &socket,
        &socket,
        "--dir",
        "/tmp/runtime",
        "--dir",
        "/empty",
        "--setenv",
        "PATH",
        "/usr/bin:/bin",
        "--setenv",
        "DISPLAY",
        display,
        "--setenv",
        "WGPU_BACKEND",
        "vulkan",
        "--setenv",
        "XDG_RUNTIME_DIR",
        "/tmp/runtime",
    ]);
    if Path::new("/lib64").exists() {
        command.args(["--ro-bind", "/lib64", "/lib64"]);
    }
    if Path::new("/etc/ld.so.cache").exists() {
        command.args(["--ro-bind", "/etc/ld.so.cache", "/etc/ld.so.cache"]);
    }
    command.arg("--ro-bind").arg(bundle).arg("/bundle with spaces")
        .arg("--bind").arg(out).arg("/evidence").args(["--remount-ro","/","--chdir","/empty","/bin/sh","-eu","-c",
        "ulimit -f 16384; for path do test ! -e \"$path\" || exit 91; done; test \"$PWD\" = /empty; test ! -e /usr/bin/cargo; test ! -e /usr/bin/rustc; cat /proc/self/mountinfo > /evidence/mountinfo.txt; cat /proc/$$/status > /evidence/process-status.txt; printf '%s\\n' source-hidden > /evidence/isolation.txt; exec '/bundle with spaces/run-game' --record /evidence/record.orrp --observe /evidence/observations.ndjson", "isolation-preflight"]);
    command
        .args(hidden)
        .stdin(Stdio::null())
        .stdout(
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(out.join("player.stdout"))?,
        )
        .stderr(
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(out.join("player.stderr"))?,
        );
    Ok(OwnedProcess(
        command.spawn()?,
        vec![
            out.join("player.stdout"),
            out.join("player.stderr"),
            out.parent()
                .and_then(Path::parent)
                .ok_or("evidence ancestry")?
                .join("xvfb.stderr"),
        ],
    ))
}

fn process_binding(
    process: &OwnedProcess,
    namespace_pid: u32,
    expected_hash: &str,
) -> Result<Value> {
    let deadline = Deadline::after(LIMIT);
    loop {
        let mut queue = vec![process.0.id()];
        let mut visited = 0;
        while let Some(pid) = queue.pop() {
            visited += 1;
            if visited > 64 {
                return Err("owned process descendant bound".into());
            }
            let directory = PathBuf::from(format!("/proc/{pid}"));
            let Ok(status) = fs::read_to_string(directory.join("status")) else {
                continue;
            };
            let ids: Vec<u32> = status
                .lines()
                .find_map(|line| line.strip_prefix("NSpid:"))
                .ok_or("kernel lacks namespace PID evidence")?
                .split_whitespace()
                .map(str::parse)
                .collect::<std::result::Result<_, _>>()?;
            if ids.last() == Some(&namespace_pid) {
                let executable = directory.join("exe");
                if fs::read_link(&executable).ok().as_deref()
                    == Some(Path::new("/bundle with spaces/bin/standalone"))
                    && hash(&executable)? == expected_hash
                {
                    let mut namespaces = serde_json::Map::new();
                    for kind in ["pid", "net", "mnt"] {
                        let host = fs::read_link(format!("/proc/self/ns/{kind}"))?;
                        let sandbox = fs::read_link(directory.join(format!("ns/{kind}")))?;
                        if host == sandbox {
                            return Err("sandbox namespace was not isolated".into());
                        }
                        namespaces.insert(kind.to_owned(), json!({"host":host,"sandbox":sandbox}));
                    }
                    let devices = fs::read_to_string(directory.join("net/dev"))?;
                    let interfaces: Vec<_> = devices
                        .lines()
                        .skip(2)
                        .map(|line| line.split(':').next().unwrap_or("").trim())
                        .collect();
                    if interfaces != ["lo"] {
                        return Err("sandbox has external network interfaces".into());
                    }
                    return Ok(
                        json!({"namespaces":namespaces,"interfaces":interfaces,"host_pid":pid,"namespace_pid":namespace_pid,"namespace_ids":ids,
                        "owned_launcher_pid":process.0.id(),"executed_path":"/bundle with spaces/bin/standalone","sha256":expected_hash}),
                    );
                }
            }
            if let Ok(children) = fs::read_to_string(directory.join(format!("task/{pid}/children")))
            {
                for child in children.split_whitespace() {
                    queue.push(child.parse()?);
                }
            }
        }
        if deadline.expired() {
            return Err("could not bind owned sandbox process to delivered executable".into());
        }
        process.check_logs()?;
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn source_hidden_standalone_windows() -> Result<()> {
    if std::env::var("ORR_REQUIRE_STANDALONE_NATIVE").as_deref() != Ok("1") {
        return Err(
            "selected native test requires ORR_REQUIRE_STANDALONE_NATIVE=1, never skips".into(),
        );
    }
    let source_sha =
        option_env!("ORR_STATIC_STANDALONE_BUILD_SHA").ok_or("missing compile-bound source SHA")?;
    if source_sha.len() != 40
        || !source_sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || std::env::var("GITHUB_SHA")? != source_sha
    {
        return Err("compile/run source mismatch".into());
    }
    let out = PathBuf::from(
        std::env::var_os("ORR_STANDALONE_ARTIFACT_DIR").ok_or("missing evidence directory")?,
    );
    fs::create_dir_all(&out)?;
    write_json(
        &out.join("attempt.json"),
        &json!({"schema":SCHEMA,"status":"started","source_sha":source_sha}),
    )?;
    let mut display = x11::OwnedDisplay::spawn(&out.join("xvfb.stderr"))?;
    let observer = display.connect()?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let test_executable = std::env::current_exe()?.canonicalize()?;
    let mut receipts = Vec::new();
    use std::os::unix::fs::PermissionsExt;
    for (alias, title, binary) in [
        (
            "delivery",
            "StarterDelivery",
            env!("CARGO_BIN_EXE_delivery-player"),
        ),
        (
            "switchboard",
            "StarterSwitchboard",
            env!("CARGO_BIN_EXE_switchboard-player"),
        ),
    ] {
        let directory = out.join(alias);
        fs::create_dir(&directory)?;
        let scene = directory.join("authored.scene.yaml");
        let text = if alias == "delivery" {
            "schema: orr.scene/1\nsingletons:\n  Delivery: { goal: [2, 2], won: 0, action_held: 0 }\nentities:\n  e_00000001:\n    name: courier\n    Courier: { position: [2, 2] }\n"
        } else {
            "schema: orr.scene/1\nsingletons:\n  Board: { bits: 0, target: 1, solved: 0, moves: 0 }\nentities:\n  e_00000001:\n    name: cursor\n    Cursor: { position: [0, 0], selected: 0, held: 0 }\n"
        };
        fs::write(&scene, text)?;
        let runtime = directory.join("trusted runtime copy");
        fs::copy(binary, &runtime)?;
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755))?;
        let staged_bundle = directory.join("original bundle");
        let bundle = directory.join("relocated bundle with spaces");
        let mut packager = OwnedProcess(
            Command::new(env!("CARGO_BIN_EXE_bundle"))
                .arg(alias)
                .arg("--runtime")
                .arg(&runtime)
                .args(["--runtime-sha256", &hash(&runtime)?, "--scene"])
                .arg(&scene)
                .arg("--output")
                .arg(&staged_bundle)
                .args(["--source-sha", source_sha, "--trusted-runtime"])
                .stdin(Stdio::null())
                .stdout(fs::File::create(directory.join("package.stdout"))?)
                .stderr(fs::File::create(directory.join("package.stderr"))?)
                .spawn()?,
            vec![
                directory.join("package.stdout"),
                directory.join("package.stderr"),
                out.join("xvfb.stderr"),
            ],
        );
        packager.wait()?;
        fs::rename(&staged_bundle, &bundle)?;
        let evidence = directory.join("execution");
        fs::create_dir(&evidence)?;
        let mut hidden = vec![
            root.clone(),
            test_executable.clone(),
            PathBuf::from(binary).canonicalize()?,
            scene.canonicalize()?,
            bundle.canonicalize()?,
            runtime.canonicalize()?,
        ];
        if let Some(target) = std::env::var_os("CARGO_TARGET_DIR") {
            hidden.push(PathBuf::from(target).canonicalize()?);
        }
        for key in ["CARGO_HOME", "RUSTUP_HOME"] {
            if let Some(value) = std::env::var_os(key) {
                hidden.push(PathBuf::from(value).canonicalize()?);
            }
        }
        hidden.push(
            Path::new(binary)
                .parent()
                .and_then(Path::parent)
                .ok_or("target root")?
                .canonicalize()?,
        );
        for key in ["ORR_STANDALONE_TOOLCHAIN_ROOT", "ORR_STANDALONE_CARGO_HOME"] {
            hidden.push(
                PathBuf::from(
                    std::env::var_os(key).ok_or("missing explicit hidden toolchain root")?,
                )
                .canonicalize()?,
            );
        }
        hidden.sort();
        hidden.dedup();
        let mut process = isolated(&bundle, &evidence, &display.name, &hidden)?;
        let mut observations = Observations {
            path: evidence.join("observations.ndjson"),
            values: Vec::new(),
        };
        let initial = observations.until(&mut process, |v| v["paused"] == true)?;
        let pid = u32::try_from(number(&initial, "process_id")?)?;
        let binding = process_binding(&process, pid, &hash(&runtime)?)?;
        let window = observer.window(pid, title)?;
        observer.focus(window)?;
        let mut captures = vec![capture(
            &observer,
            window,
            &mut observations,
            &mut process,
            &evidence,
            "initial",
        )?];
        tap(&observer, ENTER)?;
        observations.until(&mut process, |v| {
            v["paused"] == false && v["resume_neutral_pending"] == false
        })?;
        observer.key(SPACE, true)?;
        observations.until(&mut process, |v| v["state"]["won"] == 1)?;
        observer.key(SPACE, false)?;
        tap(&observer, ESCAPE)?;
        captures.push(capture(
            &observer,
            window,
            &mut observations,
            &mut process,
            &evidence,
            "won",
        )?);
        tap(&observer, ENTER)?;
        observations.until(&mut process, |v| {
            v["paused"] == false && v["resume_neutral_pending"] == false && v["state"]["held"] == 0
        })?;
        observer.key(SPACE, true)?;
        observations.until(&mut process, |v| v["state"]["won"] == 0)?;
        observer.key(SPACE, false)?;
        tap(&observer, ESCAPE)?;
        captures.push(capture(
            &observer,
            window,
            &mut observations,
            &mut process,
            &evidence,
            "restarted",
        )?);
        let focus = clear_proof(
            alias,
            &observer,
            window,
            &mut process,
            &mut observations,
            true,
        )?;
        let escape = clear_proof(
            alias,
            &observer,
            window,
            &mut process,
            &mut observations,
            false,
        )?;
        observer.close(window)?;
        process.wait()?;
        observations.refresh()?;
        if fs::read_to_string(evidence.join("isolation.txt"))? != "source-hidden\n" {
            return Err("isolation preflight missing".into());
        }
        let replay = if alias == "delivery" {
            verify_replay::<DeliveryGame>(text, &evidence, &observations.values)?
        } else {
            verify_replay::<SwitchGame>(text, &evidence, &observations.values)?
        };
        let receipt = json!({"schema":SCHEMA,"alias":alias,"source_sha":source_sha,"status":"passed",
            "input_source":"XTEST","framebuffer_source":"XGetImage","physical_gpu":false,
            "source_hidden":true,"network_disabled":true,"pid_namespace":true,"hidden_paths":hidden,
            "process_id":pid,"window_id":window,"process_binding":binding,"mountinfo_sha256":hash(&evidence.join("mountinfo.txt"))?,"process_status_sha256":hash(&evidence.join("process-status.txt"))?,"test_executable_sha256":hash(&test_executable)?,
            "runtime_sha256":hash(Path::new(binary))?,"scene_sha256":hash(&scene)?,
            "bundle_sha256":hash(&bundle.join("bundle.json"))?,"captures":captures,"focus_clear":focus,"escape_clear":escape,
            "observations_sha256":hash(&evidence.join("observations.ndjson"))?,"replay":replay});
        write_json(&directory.join("receipt.json"), &receipt)?;
        receipts
            .push(json!({"alias":alias,"receipt_sha256":hash(&directory.join("receipt.json"))?}));
    }
    write_json(
        &out.join("result.json"),
        &json!({"schema":SCHEMA,"status":"passed","source_sha":source_sha,"games":receipts}),
    )?;
    Ok(())
}

fn verify_replay<G: orr_static_starter::presentation::StandaloneGame>(
    scene: &str,
    evidence: &Path,
    observations: &[Value],
) -> Result<Value> {
    use orr_static_starter::game_contract::player;
    let path = evidence.join("record.orrp");
    if fs::symlink_metadata(&path)?.file_type().is_symlink()
        || fs::metadata(&path)?.len() > 16 * 1024 * 1024
    {
        return Err("unsafe/oversize raw replay".into());
    }
    let bytes = fs::read(&path)?;
    if !bytes.starts_with(b"ORRP") {
        return Err("not unchanged raw ORRP".into());
    }
    let reader = orr_session::ReplayReader::<G>::parse(&bytes)?;
    let admitted = orr_static_starter::game_contract::admit::<G>(scene)?;
    // Canonical admission rejects debug records at tick zero or beyond the
    // input timeline, duplicate records, trailing bytes and forged keyframes.
    admitted.open_replay(&bytes)?;
    let mut source = player::<G>(scene)?;
    let first = observations.first().ok_or("empty observations")?;
    for observation in observations {
        if &observation["identity"] != admitted.identity()
            || observation["source_sha"]
                != option_env!("ORR_STATIC_STANDALONE_BUILD_SHA").unwrap_or("")
            || observation["process_id"] != first["process_id"]
        {
            return Err("admitted identity/source/process changed in observation".into());
        }
    }
    let expected = orr_session::ReplayReader::<G>::parse(&source.save_replay())?;
    if reader.header.game_id != expected.header.game_id
        || reader.header.build_hash != expected.header.build_hash
        || reader.header.seed != 42
        || reader.header.tick_rate != 60
        || reader.header.player_count != 1
        || reader.header.input_size != expected.header.input_size
        || reader.first_tick() != 1
        || reader.last_tick() == 0
        || reader.last_tick() > 3600
        || reader.tick_count() as u64 != reader.last_tick()
    {
        return Err("raw replay header/count mismatch".into());
    }
    let initial = orr_session::replay_seek_checked::<G>(&bytes, (), G::BUILD, 0)?;
    if initial.frame().checksum() != source.frame().checksum() {
        return Err("raw replay initial scene mismatch".into());
    }
    let mut consumed = Vec::new();
    for observation in observations {
        if observation["schema"] != "orr.static-standalone-observation/1" {
            return Err("observation schema mismatch".into());
        }
        for record in observation["consumed"]
            .as_array()
            .ok_or("missing consumed ticks")?
        {
            consumed.push(record);
        }
    }
    if consumed.len() != reader.tick_count() {
        return Err("actual presented inputs do not cover raw replay".into());
    }
    let mut independent = Vec::new();
    independent.push(
        json!({"tick":0,"checksum":format!("0x{:016x}",source.frame().checksum()),
        "state":G::state_value(orr_bridge::FrameView::of(source.frame()))}),
    );
    for tick in 1..=reader.last_tick() {
        let (inputs, commands) = reader.tick(tick).ok_or("raw replay tick gap")?;
        if inputs.len() != 1
            || !G::valid_input(&inputs[0])
            || !commands.is_empty()
            || !reader.debug_commands(tick).is_empty()
        {
            return Err("unexpected replay players/commands/debug writes".into());
        }
        source.set_input(orr_sim::PlayerSlot(0), inputs[0]);
        source.step_now().ok_or("source replay failed to advance")?;
        let expected = json!({"tick":tick,"input":G::input_value(&inputs[0]),
            "checksum":format!("0x{:016x}",source.frame().checksum()),"state":G::state_value(orr_bridge::FrameView::of(source.frame()))});
        if *consumed[usize::try_from(tick - 1)?] != expected {
            return Err(
                format!("presented input/state differs from fresh source at tick {tick}").into(),
            );
        }
        if reader
            .checksums
            .iter()
            .find(|(t, _)| *t == tick)
            .map(|(_, c)| *c)
            != source.checksum_at(tick)
        {
            return Err("raw recorded checksum differs from source".into());
        }
        independent.push(expected);
    }
    for observation in observations {
        let tick = usize::try_from(number(observation, "tick")?)?;
        let expected = independent.get(tick).ok_or("presentation beyond replay")?;
        if observation["state"] != expected["state"]
            || observation["checksum"] != expected["checksum"]
        {
            return Err("presentation state/checksum not reproduced by source".into());
        }
    }
    // This source artifact is independently regenerated from raw bytes, never a
    // reserialization of the native observation stream.
    write_json(
        &evidence.join("source-replay.json"),
        &json!({"ticks":independent}),
    )?;
    Ok(
        json!({"path":"record.orrp","sha256":hash(&path)?,"ticks":reader.tick_count(),
        "initial_checksum":format!("0x{:016x}",initial.frame().checksum()),
        "final_checksum":format!("0x{:016x}",source.frame().checksum()),
        "source_replay_sha256":hash(&evidence.join("source-replay.json"))?}),
    )
}
