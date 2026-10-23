//! Physical XTEST acceptance for the private four-action source fixture.
#[path = "support/multi_action_fixture.rs"]
mod fixture;
#[path = "support/standalone_x11.rs"]
mod x11;
use fixture::Fixture;
use orr_static_starter::game_contract::Starter;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use x11::Deadline;
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const LIMIT: Duration = Duration::from_secs(20);
const ENTER: u32 = 0xff0d;
const ESCAPE: u32 = 0xff1b;
const KEYS: [u32; 4] = [0x20, 0x66, 0x65, 0x71];
const SCHEMA: &str = "orr.static-game-actions-native/1";
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
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
        json!({"phase":phase,"observation":before,"before":before,"after":after,"png":format!("{phase}.png"),"sha256":hash(&path)?,
        "x11":{"visual":image.visual,"depth":image.spec.depth,"bits_per_pixel":image.spec.bits,
        "scanline_pad":image.spec.pad,"lsb_first":image.spec.lsb,"masks":image.spec.masks,
        "width":image.spec.width,"height":image.spec.height}}),
    )
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
fn neutral() -> Value {
    json!({"dx":0,"dy":0,"actions":0})
}
fn latest(obs: &Observations) -> Result<Value> {
    obs.values
        .last()
        .cloned()
        .ok_or_else(|| "empty observations".into())
}
fn dispatch(
    o: &x11::Observer,
    obs: &Observations,
    key: u32,
    down: bool,
    events: &mut Vec<Value>,
) -> Result<()> {
    let before = latest(obs)?;
    o.key(key, down)?;
    events.push(json!({"keysym":key,"pressed":down,"after_sequence":before["sequence"]}));
    Ok(())
}
fn wait_input(
    obs: &mut Observations,
    p: &mut OwnedProcess,
    before: &Value,
    input: &Value,
) -> Result<Value> {
    let tick = number(before, "tick")?;
    obs.until(p, |v| {
        v["tick"].as_u64().is_some_and(|n| n > tick)
            && v["consumed"]
                .as_array()
                .is_some_and(|a| a.iter().any(|t| t["input"] == *input))
    })
}
fn release_all(o: &x11::Observer) -> Result<()> {
    for k in KEYS.into_iter().chain([0x64, 0x77]) {
        o.key(k, false)?;
    }
    Ok(())
}
fn resume(o: &x11::Observer, obs: &mut Observations, p: &mut OwnedProcess) -> Result<Value> {
    resume_logged(o, obs, p, &mut Vec::new())
}
fn resume_logged(
    o: &x11::Observer,
    obs: &mut Observations,
    p: &mut OwnedProcess,
    events: &mut Vec<Value>,
) -> Result<Value> {
    let before = latest(obs)?;
    let start = events.len();
    dispatch(o, obs, ENTER, true, events)?;
    dispatch(o, obs, ENTER, false, events)?;
    let after = obs.until(p, |v| {
        v["paused"] == false
            && v["resume_neutral_pending"] == false
            && v["tick"]
                .as_u64()
                .is_some_and(|n| n > before["tick"].as_u64().unwrap_or(u64::MAX))
    })?;
    let tick = number(&before, "tick")? + 1;
    let first = obs
        .values
        .iter()
        .flat_map(|v| v["consumed"].as_array().into_iter().flatten())
        .find(|v| v["tick"] == tick)
        .ok_or("no naturally resumed first tick")?;
    if first["input"] != neutral() {
        return Err("first resumed tick is not neutral".into());
    }
    for event in &mut events[start..] {
        event["before_sequence"] = after["sequence"].clone();
    }
    Ok(after)
}
fn reset(
    o: &x11::Observer,
    w: u32,
    obs: &mut Observations,
    p: &mut OwnedProcess,
    blur: bool,
) -> Result<Value> {
    let before = latest(obs)?;
    let mut events = Vec::new();
    for k in KEYS.into_iter().chain([0x64, 0x77]) {
        dispatch(o, obs, k, true, &mut events)?;
    }
    let active = wait_input(obs, p, &before, &json!({"dx":1,"dy":1,"actions":15}))?;
    if blur {
        o.blur()?;
        events.push(json!({"kind":"focus_loss","after_sequence":active["sequence"]}));
    } else {
        dispatch(o, obs, ESCAPE, true, &mut events)?;
        dispatch(o, obs, ESCAPE, false, &mut events)?;
    }
    let paused = obs.until(p, |v| v["paused"] == true)?;
    for event in &mut events {
        event["before_sequence"] = paused["sequence"].clone();
    }
    let releases = events.len();
    for key in KEYS.into_iter().chain([0x64, 0x77]) {
        dispatch(o, obs, key, false, &mut events)?;
    }
    if blur {
        o.focus(w)?;
        let focused = obs.until(p, |v| v["focused"] == true)?;
        events.push(json!({"kind":"focus_regain","after_sequence":paused["sequence"],"before_sequence":focused["sequence"]}));
    }
    let no_auto = paused_proof(obs, p, &paused)?;
    for event in &mut events[releases..] {
        event["before_sequence"] = no_auto["sequence"].clone();
    }
    let resumed = resume_logged(o, obs, p, &mut events)?;
    let first_tick = number(&paused, "tick")? + 1;
    let first = obs
        .values
        .iter()
        .flat_map(|v| v["consumed"].as_array().into_iter().flatten())
        .find(|v| v["tick"] == first_tick)
        .ok_or("missing reset first input")?
        .clone();
    Ok(
        json!({"name":if blur{"focus_clear"}else{"escape_clear"},"before":active,"after":resumed,"events":events,"paused":paused,"no_auto_resume":no_auto,"resumed":resumed,"first_consumed":first}),
    )
}
#[test]
fn physical_four_action_standalone() -> Result<()> {
    if std::env::var("ORR_REQUIRE_GPU").as_deref() != Ok("1") {
        return Err("native selection requires ORR_REQUIRE_GPU=1; never skipped".into());
    }
    let source =
        option_env!("ORR_STATIC_STANDALONE_BUILD_SHA").ok_or("missing compile-time source SHA")?;
    if source.len() != 40
        || !source.bytes().all(|b| b.is_ascii_hexdigit())
        || std::env::var("GITHUB_SHA")? != source
    {
        return Err("source provenance mismatch".into());
    }
    let out = PathBuf::from(
        std::env::var_os("ORR_MULTI_ACTION_ARTIFACT_DIR").ok_or("missing artifact root")?,
    )
    .join("standalone");
    fs::create_dir_all(&out)?;
    let out = out.canonicalize()?;
    write_json(
        &out.join("attempt.json"),
        &json!({"schema":SCHEMA,"status":"started","mode":"standalone","source_sha":source}),
    )?;
    let scene = out.join("authored.scene.yaml");
    fs::write(&scene, Fixture::SCENE)?;
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_multi-action-fixture-player")).canonicalize()?;
    let mut display = x11::OwnedDisplay::spawn(&out.join("xvfb.stderr"))?;
    let observer = display.connect()?;
    let stdout = out.join("player.stdout");
    let stderr = out.join("player.stderr");
    let mut process = OwnedProcess(
        Command::new(&binary)
            .args(["--scene"])
            .arg(&scene)
            .arg("--record")
            .arg(out.join("record.orrp"))
            .arg("--observe")
            .arg(out.join("observations.ndjson"))
            .env("DISPLAY", &display.name)
            .env("WINIT_UNIX_BACKEND", "x11")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::null())
            .stdout(fs::File::create(&stdout)?)
            .stderr(fs::File::create(&stderr)?)
            .spawn()?,
        vec![stdout, stderr],
    );
    let mut obs = Observations {
        path: out.join("observations.ndjson"),
        values: Vec::new(),
    };
    let initial = obs.until(&mut process, |v| v["paused"] == true)?;
    if initial["adapter"]["device_type"] != "Cpu" {
        return Err("acceptance requires reported CPU/software adapter".into());
    }
    let pid = u32::try_from(number(&initial, "process_id")?)?;
    if pid != process.0.id() {
        return Err("observation process PID mismatch".into());
    }
    let window = observer.window(pid, Fixture::ID)?;
    observer.focus(window)?;
    let mut captures = vec![capture(
        &observer,
        window,
        &mut obs,
        &mut process,
        &out,
        "initial",
    )?];
    let mut scenarios = Vec::new();
    for (n, key) in KEYS.into_iter().enumerate() {
        release_all(&observer)?;
        let before = resume(&observer, &mut obs, &mut process)?;
        let mut events = Vec::new();
        dispatch(&observer, &obs, key, true, &mut events)?;
        let after = wait_input(
            &mut obs,
            &mut process,
            &before,
            &json!({"dx":0,"dy":0,"actions":1<<n}),
        )?;
        for event in &mut events {
            event["before_sequence"] = after["sequence"].clone();
        }
        scenarios.push(json!({"name":format!("independent_{}",1<<n),"before":before,"after":after,"events":events}));
        tap(&observer, ESCAPE)?;
        captures.push(capture(
            &observer,
            window,
            &mut obs,
            &mut process,
            &out,
            &format!("action_{}", 1 << n),
        )?);
    }
    release_all(&observer)?;
    let before = resume(&observer, &mut obs, &mut process)?;
    let mut events = Vec::new();
    for k in KEYS.into_iter().chain([0x64, 0x77]) {
        dispatch(&observer, &obs, k, true, &mut events)?;
    }
    let after = wait_input(
        &mut obs,
        &mut process,
        &before,
        &json!({"dx":1,"dy":1,"actions":15}),
    )?;
    for event in &mut events {
        event["before_sequence"] = after["sequence"].clone();
    }
    scenarios.push(json!({"name":"simultaneous","before":before,"after":after,"events":events}));
    tap(&observer, ESCAPE)?;
    captures.push(capture(
        &observer,
        window,
        &mut obs,
        &mut process,
        &out,
        "simultaneous",
    )?);
    release_all(&observer)?;
    let before = resume(&observer, &mut obs, &mut process)?;
    for k in KEYS.into_iter().chain([0x64, 0x77]) {
        observer.key(k, true)?;
    }
    let before = wait_input(
        &mut obs,
        &mut process,
        &before,
        &json!({"dx":1,"dy":1,"actions":15}),
    )?;
    let mut events = Vec::new();
    dispatch(&observer, &obs, KEYS[0], false, &mut events)?;
    let after = wait_input(
        &mut obs,
        &mut process,
        &before,
        &json!({"dx":1,"dy":1,"actions":14}),
    )?;
    for event in &mut events {
        event["before_sequence"] = after["sequence"].clone();
    }
    scenarios.push(json!({"name":"partial_release","before":before,"after":after,"events":events}));
    tap(&observer, ESCAPE)?;
    captures.push(capture(
        &observer,
        window,
        &mut obs,
        &mut process,
        &out,
        "partial_release",
    )?);
    release_all(&observer)?;
    let before = resume(&observer, &mut obs, &mut process)?;
    for k in KEYS {
        observer.key(k, true)?;
    }
    let before = wait_input(
        &mut obs,
        &mut process,
        &before,
        &json!({"dx":0,"dy":0,"actions":15}),
    )?;
    let mut events = Vec::new();
    for k in KEYS {
        dispatch(&observer, &obs, k, false, &mut events)?;
    }
    let after = wait_input(&mut obs, &mut process, &before, &neutral())?;
    for event in &mut events {
        event["before_sequence"] = after["sequence"].clone();
    }
    scenarios.push(json!({"name":"neutral","before":before,"after":after,"events":events}));
    tap(&observer, ESCAPE)?;
    captures.push(capture(
        &observer,
        window,
        &mut obs,
        &mut process,
        &out,
        "neutral",
    )?);
    resume(&observer, &mut obs, &mut process)?;
    scenarios.push(reset(&observer, window, &mut obs, &mut process, false)?);
    scenarios.push(reset(&observer, window, &mut obs, &mut process, true)?);
    tap(&observer, ESCAPE)?;
    let paused = obs.until(&mut process, |v| v["paused"] == true)?;
    paused_proof(&mut obs, &mut process, &paused)?;
    observer.close(window)?;
    process.wait()?;
    obs.refresh()?;
    let replay = verify_replay::<Fixture>(Fixture::SCENE, &out, &obs.values)?;
    let receipt = json!({"schema":SCHEMA,"mode":"standalone","status":"passed","packaged":false,"source_hidden":false,"boundaries":{"native_window":true,"server_keyboard_delivery":true,"physical_hardware_keyboard":false,"physical_gpu":false,"software_gpu":true,"packaged_export":false,"source_hidden_delivery":false},
      "provenance":{"source_sha":source,"run_id":std::env::var("GITHUB_RUN_ID")?,"run_attempt":std::env::var("GITHUB_RUN_ATTEMPT")?,"test_sha256":hash(&std::env::current_exe()?)?,"player_sha256":hash(&binary)?,"process_id":pid,"producer_sources":{"fixture_sha256":digest(include_bytes!("support/multi_action_fixture.rs")),"player_sha256":digest(include_bytes!("support/multi_action_fixture_player.rs")),"test_sha256":digest(include_bytes!("multi_action_standalone_x11.rs"))}},
      "profile":{"id":"private-native-actions/v1","actions":[{"name":"jump","key":"Space","mask":1},{"name":"fire","key":"F","mask":2},{"name":"interact","key":"E","mask":4},{"name":"shield","key":"Q","mask":8}]},
      "scene":{"path":"authored.scene.yaml","sha256":hash(&scene)?},"identity":initial["identity"],"artifacts":{"observations":{"path":"observations.ndjson","sha256":hash(&obs.path)?},"replay":{"path":"record.orrp","sha256":replay["sha256"]},"source_replay":{"path":"source-replay.json","sha256":replay["source_replay_sha256"]}},"replay":replay,"captures":captures,"scenarios":scenarios});
    write_json(&out.join("receipt.json"), &receipt)?;
    Ok(())
}
