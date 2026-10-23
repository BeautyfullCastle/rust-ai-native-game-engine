//! Private real-X11 acceptance. No raw-input injection or direct input-router calls.
//! UI lifecycle operations are staged by the wrapper; every gameplay key is XTEST.
#[path = "support/multi_action_fixture.rs"]
mod fixture;
#[path = "support/standalone_x11.rs"]
mod x11;
use eframe::App;
use egui_kittest::{Harness, kittest::Queryable};
use fixture::Fixture;
use orr_bridge::{Bridge, BridgeEvent, FrameView, ViewUpdate};
use orr_editor::{Editor, EditorApp, static_adapter::*};
use orr_remote::{
    ExpectedInputDescriptor, GameHooks, LocalHost, RemoteBridge, RemoteConfig, RemoteViewDelivery,
    RpcError, Transport,
};
use orr_static_starter::{
    game_contract::{Starter, document, inputs, player},
    presentation::StandaloneGame,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const TITLE: &str = "Private multi-action editor XTEST";
const SIZE: [u32; 2] = [1024, 768];
const SCHEMA: &str = "orr.static-game-actions-native/1";
const KEYS: [u32; 4] = [0x20, 0x66, 0x65, 0x71];
const WAIT: Duration = Duration::from_secs(25);
fn require(ok: bool, reason: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(reason.into()) }
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn hash(path: &Path) -> Result<String> {
    Ok(Sha256::digest(fs::read(path)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn write(path: &Path, value: &Value) -> Result<()> {
    // Publish complete JSON atomically: the UI and driver poll these paths.
    let mut f = tempfile::NamedTempFile::new_in(path.parent().ok_or("JSON parent")?)?;
    f.write_all(&serde_json::to_vec(value)?)?;
    f.flush()?;
    f.persist_noclobber(path)?;
    Ok(())
}
fn read(path: &Path) -> Result<Value> {
    require(
        fs::symlink_metadata(path)?.file_type().is_file(),
        "not regular JSON",
    )?;
    require(
        fs::metadata(path)?.len() < 32 * 1024 * 1024,
        "JSON size limit",
    )?;
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn host(r: StaticHostRequest) -> std::result::Result<LocalHost, String> {
    let schema = serde_json::from_str(
        &inputs::<Fixture>()
            .type_json_schema(Fixture::INPUT)
            .ok_or("missing fixture input schema")?,
    )
    .map_err(|e| format!("{e}"))?;
    if r.expected_input
        != ExpectedInputDescriptor::new(schema, orr_remote::methods::VALUE_FORMAT, 1)?
    {
        return Err("typed fixture input contract mismatch".into());
    }
    let mut cfg = r.config;
    LocalHost::spawn_with_input::<Fixture>(
        move || {
            let doc = document::<Fixture>(&r.scene_text)?;
            cfg.limits.game = GameHooks::new(Fixture::ID);
            cfg.limits.build_id = Fixture::BUILD;
            cfg.limits.player_count = 1;
            cfg.limits.tick_rate = 60;
            cfg.limits.scene_path = Some(r.scene_path);
            cfg.limits.view_stream = None;
            Ok((doc, cfg))
        },
        Fixture::INPUT,
        1,
        |_, _| vec![],
        r.neutral_value,
        Some(r.work_status),
    )
}
struct Stream(RemoteBridge<Fixture>);
impl StaticFrameStream for Stream {
    fn poll_view(&mut self) -> ViewUpdate<()> {
        let u = self.0.poll_view();
        ViewUpdate {
            snapshot: u.snapshot,
            resync: u.resync,
            events: u
                .events
                .into_iter()
                .filter_map(|e| match e {
                    BridgeEvent::Lifecycle(l) => Some(BridgeEvent::Lifecycle(l)),
                    _ => None,
                })
                .collect(),
        }
    }
    fn take_errors(&self) -> Vec<RpcError> {
        self.0.take_errors()
    }
    fn is_alive(&self) -> bool {
        self.0.is_alive()
    }
    fn view_delivery(&self) -> RemoteViewDelivery {
        self.0.view_delivery()
    }
}
fn stream(
    t: Box<dyn Transport>,
    cfg: RemoteConfig,
    input: ExpectedInputDescriptor,
) -> std::result::Result<Box<dyn StaticFrameStream>, String> {
    Ok(Box::new(Stream(
        RemoteBridge::<Fixture>::connect_transport_with_input(t, cfg, input)?,
    )))
}
fn draw(frame: FrameView<'_>) -> Vec<orr_editor::game::Drawable> {
    let mut items = vec![];
    Fixture::extract(frame, &mut items);
    items
        .into_iter()
        .map(|i| orr_editor::game::Drawable {
            entity: i.entity,
            pos: [i.transform.pos.x, i.transform.pos.y],
            angle: i.transform.rot,
            turn: 0.0,
            style: i.style,
            outline: orr_sample::editor_view::Outline::Circle {
                radius: i.style.size,
            },
        })
        .collect()
}
fn registry() -> &'static StaticGameRegistry {
    static REGISTRY: std::sync::OnceLock<StaticGameRegistry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        StaticGameRegistry::with_action_profiles(
            vec![StaticGameDescriptor {
                game_id: Fixture::ID,
                local_alias: Fixture::ALIAS,
                build_id: Fixture::BUILD,
                types: Fixture::types,
                host,
                stream,
                draw,
                position: StaticPosition {
                    component: "Actor",
                    field: "position",
                },
                input: StaticInputDescriptor {
                    type_name: Fixture::INPUT,
                    types: inputs::<Fixture>,
                    max_players: 1,
                    hint: "Private XTEST acceptance",
                    held: |k| {
                        Fixture::input_value(&Fixture::held_input(
                            orr_static_starter::game_contract::HeldKeys {
                                x: k.x,
                                y: k.y,
                                action: k.action,
                            },
                        ))
                    },
                },
            }],
            vec![orr_static_starter::integration::action_descriptor::<Fixture>().unwrap()],
        )
        .unwrap()
    })
}

// Commands only stage supported lifecycle/UI operations. They never carry keys.
struct Wrapper {
    app: EditorApp,
    out: PathBuf,
    scene: PathBuf,
    generation: u64,
    sequence: u64,
    command: u64,
    raw: Vec<Value>,
    text: Option<String>,
    park: bool,
    finished: bool,
}
impl Wrapper {
    fn save(&mut self) {
        let stopped = self
            .app
            .editor
            .stop()
            .expect("current host stop must succeed; never reuse an earlier recording");
        fs::write(
            self.out.join(format!("epoch-{}.orrp", self.generation)),
            &stopped.replay,
        )
        .unwrap();
    }
    fn command(&mut self) {
        let path = self.out.join(format!("command-{}.json", self.command + 1));
        if !path.exists() {
            return;
        }
        let v = read(&path).unwrap();
        self.command += 1;
        match v["operation"].as_str().unwrap() {
            "park" => self.park = true,
            "text" => self.text = Some(String::new()),
            "dismiss_text" => self.text = None,
            "reopen" => {
                self.save();
                assert!(self.app.editor.open_path(&self.scene));
                self.generation += 1;
                self.app.editor.play();
            }
            "finish" => {
                self.save();
                self.finished = true;
                write(
                    &self.out.join("finish-ready.json"),
                    &json!({"scene_generation":self.generation,"recording_saved":true}),
                )
                .unwrap();
            }
            _ => panic!("unknown UI operation"),
        }
    }
    fn observation(&self, ctx: &egui::Context) -> Value {
        let snapshot = self.app.editor.snapshot().expect("editor snapshot");
        let frame = snapshot.predicted();
        let rect = self.app.ui.viewport_rect.expect("viewport");
        json!({"sequence":self.sequence,"command":self.command,"scene_generation":self.generation,"tick":frame.tick(),"state":Fixture::state_value(frame),"checksum":format!("0x{:016x}",frame.checksum()),"phase":format!("{:?}",self.app.editor.input_phase()),"cleanup_pending":self.app.editor.input_cleanup_pending(),"focused":ctx.input(|i|i.focused),"raw_keys":self.raw,"text":self.text,"viewport":{"min":[rect.min.x,rect.min.y],"px":[self.app.ui.viewport_px.0,self.app.ui.viewport_px.1]},"camera":{"center":self.app.editor.camera.center,"half_extent":self.app.editor.camera.half_extent},"parked":self.park})
    }
}
impl App for Wrapper {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        // Read-only: the exact winit/eframe events are passed on untouched.
        self.raw=input.events.iter().filter_map(|e|if let egui::Event::Key{key,physical_key,pressed,repeat,modifiers}=e {Some(json!({"key":format!("{key:?}"),"physical_key":physical_key.map(|k|format!("{k:?}")),"pressed":pressed,"repeat":repeat,"ctrl":modifiers.ctrl,"alt":modifiers.alt,"command":modifiers.command}))}else{None}).collect();
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.command();
        if self.finished {
            return;
        }
        if let Some(text) = self.text.as_mut() {
            egui::Window::new("Acceptance text focus")
                .fixed_pos([700.0, 40.0])
                .show(&ctx, |ui| {
                    let response = ui.text_edit_singleline(text);
                    response.request_focus();
                });
        }
        // Fixed camera is a presentation setup, never a game/input-state write.
        self.app.editor.camera.center = [0.0, 0.0];
        self.app.editor.camera.half_extent = 6.0;
        self.app.ui(ui, frame);
        // EditorApp publishes its document title during ui(). Keep this private
        // acceptance window identifiable after that production update, without
        // relaxing the observer's PID, mapped-window, or title checks.
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(TITLE.into()));
        self.sequence += 1;
        let observation = self.observation(&ctx);
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(self.out.join("observations.ndjson"))
            .unwrap();
        writeln!(f, "{}", observation).unwrap();
        f.flush().unwrap();
        if self.park {
            let release = self.out.join(format!("resume-{}", self.command));
            let until = x11::Deadline::after(WAIT);
            while !release.exists() {
                assert!(!until.expired(), "parked UI driver timeout");
                std::thread::sleep(Duration::from_millis(2));
            }
            self.park = false;
        }
        ctx.request_repaint_after(Duration::from_millis(10));
    }
}
fn editor_native_child() {
    let Some(out) = std::env::var_os("ORR_MULTI_EDITOR_CHILD") else {
        return;
    };
    let out = PathBuf::from(out);
    let scene = out.join("fixture.scene.yaml");
    let game = registry().handle(Fixture::ALIAS).unwrap();
    let mut editor = Editor::open_registered(&scene, game).unwrap();
    editor.play();
    // CPU layout discovery only. Neither its events nor pixels are proof.
    let mut locator = Harness::builder()
        .with_size([SIZE[0] as f32, SIZE[1] as f32])
        .build_eframe(move |_| EditorApp::new(editor, None));
    for _ in 0..20 {
        locator.run_steps(1);
        std::thread::sleep(Duration::from_millis(5));
    }
    let take = locator.get_by_label("Take control").rect().center();
    write(&out.join("layout.json"), &json!({"take":[take.x,take.y]})).unwrap();
    let editor = locator.into_state().editor;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([SIZE[0] as f32, SIZE[1] as f32]),
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_any_thread(true);
        })),
        ..Default::default()
    };
    eframe::run_native(TITLE,options,Box::new(move |cc|{
        cc.egui_ctx.set_pixels_per_point(1.0);let info=cc.wgpu_render_state.as_ref().expect("native GPU renderer").adapter.get_info();
        assert_eq!(format!("{:?}",info.backend),"Vulkan");assert_eq!(format!("{:?}",info.device_type),"Cpu");let description=format!("{} {} {}",info.name,info.driver,info.driver_info).to_lowercase();assert!(!description.contains("swiftshader")&&["mesa","llvmpipe","lavapipe"].iter().any(|s|description.contains(s)));
        write(&out.join("adapter.json"),&json!({"name":info.name,"driver":info.driver,"driver_info":info.driver_info,"backend":"Vulkan","device_type":"Cpu"})).unwrap();
        Ok(Box::new(Wrapper{app:EditorApp::new(editor,cc.wgpu_render_state.clone()),out,scene,generation:0,sequence:0,command:0,raw:vec![],text:None,park:false,finished:false}))
    })).unwrap();
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Driver {
    out: PathBuf,
    child: OwnedChild,
    x: x11::Observer,
    window: u32,
    command: u64,
    events: Vec<Value>,
    observations: Vec<Value>,
    scenarios: Vec<Value>,
    captures: Vec<Value>,
}
impl Driver {
    fn refresh(&mut self) -> Result<()> {
        require(
            self.child.0.try_wait()?.is_none(),
            "editor exited before driver finished",
        )?;
        let path = self.out.join("observations.ndjson");
        let bytes = fs::read(path)?;
        require(bytes.len() < 32 * 1024 * 1024, "observation size bound")?;
        let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let values: Vec<Value> = bytes[..end]
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .map(serde_json::from_slice)
            .collect::<std::result::Result<_, _>>()?;
        require(
            values.len() >= self.observations.len()
                && values[..self.observations.len()] == self.observations,
            "observation prefix changed",
        )?;
        self.observations = values;
        Ok(())
    }
    fn until(&mut self, p: impl Fn(&Value) -> bool) -> Result<Value> {
        let until = x11::Deadline::after(WAIT);
        loop {
            self.refresh()?;
            if let Some(v) = self.observations.last().filter(|v| p(v)) {
                return Ok(v.clone());
            }
            require(!until.expired(), "editor state deadline")?;
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn latest(&mut self) -> Result<Value> {
        self.until(|_| true)
    }
    fn advance(&mut self, from: &Value, n: u64) -> Result<Value> {
        let tick = from["tick"].as_u64().ok_or("tick")?;
        let generation = from["scene_generation"].clone();
        self.until(|v| {
            v["scene_generation"] == generation && v["tick"].as_u64().is_some_and(|t| t >= tick + n)
        })
    }
    fn command(&mut self, operation: &str) -> Result<Value> {
        self.command += 1;
        write(
            &self.out.join(format!("command-{}.json", self.command)),
            &json!({"operation":operation}),
        )?;
        let id = self.command;
        self.until(|v| v["command"] == id)
    }
    fn key(&mut self, key: u32, pressed: bool) -> Result<()> {
        self.x.key(key, pressed)?;
        self.events.push(json!({"sequence":self.events.len(),"after_observation":self.observations.last().map(|v|v["sequence"].clone()),"operation":"key","keysym":key,"pressed":pressed}));
        Ok(())
    }
    fn focus(&mut self, on: bool) -> Result<()> {
        if on {
            self.x.focus(self.window)?;
        } else {
            self.x.blur()?;
        }
        self.events
            .push(json!({"sequence":self.events.len(),"after_observation":self.observations.last().map(|v|v["sequence"].clone()),"operation":if on{"focus"}else{"blur"}}));
        Ok(())
    }
    fn claim(&mut self, with_space: bool) -> Result<Value> {
        self.focus(true)?;
        let parked = self.command("park")?;
        require(parked["phase"] == "Off", "claim requires actual Off")?;
        let layout = read(&self.out.join("layout.json"))?;
        let p = layout["take"].as_array().ok_or("take layout")?;
        self.x.click(
            self.window,
            p[0].as_f64().ok_or("take x")? as i16,
            p[1].as_f64().ok_or("take y")? as i16,
        )?;
        self.events.push(json!({"sequence":self.events.len(),"after_observation":self.observations.last().map(|v|v["sequence"].clone()),"operation":"take_control_click","x":p[0],"y":p[1]}));
        if with_space {
            self.key(KEYS[0], true)?;
        }
        fs::write(self.out.join(format!("resume-{}", self.command)), b"resume")?;
        self.until(|v| v["phase"] == "Active" && !v["cleanup_pending"].as_bool().unwrap_or(true))
    }
    fn capture(&mut self, name: &str, observation: &Value) -> Result<Value> {
        let before = self.advance(observation, 3)?;
        let image = self.x.capture(self.window)?;
        let file = format!("{name}.png");
        image.png(&self.out.join(&file))?;
        let sequence = before["sequence"].as_u64().ok_or("capture sequence")?;
        let after = self.until(|v| v["sequence"].as_u64().is_some_and(|s| s > sequence))?;
        require(
            before["scene_generation"] == after["scene_generation"]
                && before["state"]["held"] == after["state"]["held"]
                && before["camera"] == after["camera"]
                && before["viewport"] == after["viewport"],
            "capture held-marker geometry was not stable",
        )?;
        let c = json!({"phase":name,"png":file,"sha256":hash(&self.out.join(&file))?,"observation":before,"before":before,"after":after,"viewport":before["viewport"],"camera":before["camera"],"x11":{"width":image.spec.width,"height":image.spec.height,"depth":image.spec.depth,"bits_per_pixel":image.spec.bits,"scanline_pad":image.spec.pad,"lsb_first":image.spec.lsb,"masks":image.spec.masks,"visual":image.visual}});
        self.captures.push(c.clone());
        Ok(c)
    }
    fn scenario(&mut self, name: &str, before: Value, after: Value, capture: bool) -> Result<()> {
        if capture {
            self.capture(name, &after)?;
        }
        self.scenarios
            .push(json!({"name":name,"before":before,"after":after,"event_end":self.events.len()}));
        Ok(())
    }
    fn neutral(&mut self) -> Result<Value> {
        for k in KEYS.into_iter().chain([0x64, 0x77]) {
            self.key(k, false)?;
        }
        let start = self.until(|v| v["state"]["held"] == 0)?;
        self.advance(&start, 3)
    }
}
fn replay(out: &Path, generation: u64) -> Result<Value> {
    let name = format!("epoch-{generation}.orrp");
    let path = out.join(&name);
    let bytes = fs::read(&path)?;
    require(bytes.len() < 16 * 1024 * 1024, "replay size bound")?;
    let admitted = orr_static_starter::game_contract::admit::<Fixture>(Fixture::SCENE)?;
    admitted.open_replay(&bytes)?;
    let reader = orr_session::ReplayReader::<Fixture>::parse(&bytes)?;
    require(
        reader.first_tick() == 1
            && reader.last_tick() > 0
            && reader.last_tick() <= 3600
            && reader.tick_count() as u64 == reader.last_tick(),
        "replay tick coverage",
    )?;
    let mut source = player::<Fixture>(Fixture::SCENE)?;
    let mut ticks = vec![
        json!({"tick":0,"state":Fixture::state_value(FrameView::of(source.frame())),"checksum":format!("0x{:016x}",source.frame().checksum())}),
    ];
    for tick in 1..=reader.last_tick() {
        let (inputs, commands) = reader.tick(tick).ok_or("replay tick hole")?;
        require(
            inputs.len() == 1
                && Fixture::valid_input(&inputs[0])
                && commands.is_empty()
                && reader.debug_commands(tick).is_empty(),
            "invalid replay input or commands",
        )?;
        source.set_input(orr_sim::PlayerSlot(0), inputs[0]);
        source.step_now().ok_or("replay step")?;
        let checksum = source.frame().checksum();
        require(
            reader
                .checksums
                .iter()
                .find(|(t, _)| *t == tick)
                .map(|(_, c)| *c)
                == source.checksum_at(tick),
            "recorded checksum differs from independent source",
        )?;
        ticks.push(json!({"tick":tick,"input":Fixture::input_value(&inputs[0]),"state":Fixture::state_value(FrameView::of(source.frame())),"checksum":format!("0x{checksum:016x}")}));
    }
    Ok(
        json!({"scene_generation":generation,"file":name,"sha256":hash(&path)?,"record_source":"actual_editor_local_host_ORRP","state_source":"independent_replay_resimulation","ticks":ticks}),
    )
}
#[test]
fn multi_action_editor_x11() -> Result<()> {
    if std::env::var_os("ORR_MULTI_EDITOR_CHILD").is_some() {
        editor_native_child();
        return Ok(());
    }
    require(
        std::env::var("ORR_REQUIRE_NATIVE_EDITOR").as_deref() == Ok("1"),
        "selected editor native test requires ORR_REQUIRE_NATIVE_EDITOR=1",
    )?;
    let out = PathBuf::from(
        std::env::var_os("ORR_MULTI_ACTION_ARTIFACT_DIR")
            .ok_or("ORR_MULTI_ACTION_ARTIFACT_DIR required")?,
    )
    .join("editor");
    fs::create_dir(&out)?;
    let compiled =
        option_env!("ORR_STATIC_NATIVE_BUILD_SHA").ok_or("compile-time source SHA required")?;
    require(
        compiled.len() == 40 && compiled.bytes().all(|b| b.is_ascii_hexdigit()),
        "compile source SHA",
    )?;
    write(
        &out.join("attempt.json"),
        &json!({"schema":SCHEMA,"mode":"editor","status":"started","compiled_source_sha":compiled}),
    )?;
    fs::write(out.join("fixture.scene.yaml"), Fixture::SCENE)?;
    fs::File::create(out.join("observations.ndjson"))?;
    let mut display = x11::OwnedDisplay::spawn(&out.join("xvfb.log"))?;
    let observer = display.connect()?;
    let stdout = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(out.join("editor.stdout"))?;
    let stderr = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(out.join("editor.stderr"))?;
    let child = OwnedChild(
        Command::new(std::env::current_exe()?)
            .args(["--exact", "multi_action_editor_x11", "--nocapture"])
            .env("ORR_MULTI_EDITOR_CHILD", &out)
            .env("DISPLAY", &display.name)
            .env("WGPU_BACKEND", "vulkan")
            .env("WINIT_UNIX_BACKEND", "x11")
            .env("LIBGL_ALWAYS_SOFTWARE", "1")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()?,
    );
    let process_id = child.0.id();
    let window = observer.window(process_id, TITLE)?;
    let mut d = Driver {
        out: out.clone(),
        child,
        x: observer,
        window,
        command: 0,
        events: vec![],
        observations: vec![],
        scenarios: vec![],
        captures: vec![],
    };
    let initial = d.until(|v| v["phase"] == "Off" && v["tick"].as_u64().is_some_and(|t| t > 0))?;
    d.focus(true)?;
    d.claim(false)?;
    let start = d.neutral()?;
    d.advance(&start, 3)?;
    for (index, key) in KEYS.into_iter().enumerate() {
        let before = d.latest()?;
        d.key(key, true)?;
        let mask = 1u64 << index;
        let held = d.until(|v| v["phase"] == "Active" && v["state"]["held"] == mask)?;
        let after = d.advance(&held, 3)?;
        d.scenario(&format!("independent_{mask}"), before, after, true)?;
        d.key(key, false)?;
        d.neutral()?;
    }
    let before = d.latest()?;
    for k in KEYS.into_iter().chain([0x64, 0x77]) {
        d.key(k, true)?;
    }
    let all = d.until(|v| v["state"]["held"] == 15)?;
    let all = d.advance(&all, 4)?;
    d.scenario("simultaneous", before, all.clone(), true)?;
    d.key(KEYS[1], false)?;
    let partial = d.until(|v| v["state"]["held"] == 13)?;
    let partial = d.advance(&partial, 4)?;
    d.scenario("partial_release", all, partial, true)?;
    let before = d.latest()?;
    let neutral = d.neutral()?;
    d.scenario("neutral", before, neutral, true)?;
    for (name, key) in [("escape_clear", 0xff1b), ("tab_clear", 0xff09)] {
        for k in KEYS.into_iter().chain([0x64, 0x77]) {
            d.key(k, true)?;
        }
        let held_before = d.until(|v| v["state"]["held"] == 15)?;
        let held_before = d.advance(&held_before, 3)?;
        d.key(key, true)?;
        d.key(key, false)?;
        let off = d.until(|v| {
            v["phase"] == "Off"
                && v["state"]["held"] == 0
                && !v["cleanup_pending"].as_bool().unwrap_or(true)
        })?;
        let no_auto_reclaim = d.advance(&off, 5)?;
        require(no_auto_reclaim["phase"] == "Off", "release auto reclaimed")?;
        d.capture(name, &no_auto_reclaim)?;
        d.neutral()?;
        let reclaimed = d.claim(false)?;
        let neutral_after_reclaim = d.advance(&reclaimed, 5)?;
        d.scenarios.push(json!({"name":name,"before":held_before,"held_before":held_before,"off":off,"no_auto_reclaim":no_auto_reclaim,"reclaimed":reclaimed,"neutral_after_reclaim":neutral_after_reclaim,"after":neutral_after_reclaim,"event_end":d.events.len()}));
    }
    for k in KEYS.into_iter().chain([0x64, 0x77]) {
        d.key(k, true)?;
    }
    let held_before = d.until(|v| v["state"]["held"] == 15)?;
    let held_before = d.advance(&held_before, 3)?;
    d.focus(false)?;
    let off = d.until(|v| {
        v["phase"] == "Off"
            && v["state"]["held"] == 0
            && !v["cleanup_pending"].as_bool().unwrap_or(true)
    })?;
    d.focus(true)?;
    let no_auto_reclaim = d.advance(&off, 5)?;
    require(no_auto_reclaim["phase"] == "Off", "focus auto reclaimed")?;
    d.neutral()?;
    let reclaimed = d.claim(false)?;
    let neutral_after_reclaim = d.advance(&reclaimed, 5)?;
    d.scenarios.push(json!({"name":"focus_clear","before":held_before,"held_before":held_before,"off":off,"no_auto_reclaim":no_auto_reclaim,"reclaimed":reclaimed,"neutral_after_reclaim":neutral_after_reclaim,"after":neutral_after_reclaim,"event_end":d.events.len()}));
    d.key(0xff1b, true)?;
    d.key(0xff1b, false)?;
    d.until(|v| v["phase"] == "Off" && !v["cleanup_pending"].as_bool().unwrap_or(true))?;
    // The app is parked only to batch OS events. Claiming itself is never assigned.
    let before = d.latest()?;
    let active = d.claim(true)?;
    let claiming = d
        .observations
        .iter()
        .find(|v| {
            v["sequence"].as_u64() > before["sequence"].as_u64()
                && v["phase"] == "Claiming"
                && v["raw_keys"].as_array().is_some_and(|events| {
                    events
                        .iter()
                        .any(|e| e["physical_key"] == "Space" && e["pressed"] == true)
                })
        })
        .cloned()
        .ok_or("no same-frame real Space/Claiming observation; cannot claim this coverage")?;
    let after = d.advance(&active, 6)?;
    require(
        after["state"]["held"] == 0 && after["state"]["counts"] == before["state"]["counts"],
        "Claiming press resurfaced",
    )?;
    d.scenarios.push(json!({"name":"claiming_clear","before":before,"claiming":claiming,"active":active,"after":after,"event_end":d.events.len()}));
    d.neutral()?;
    let before = d.latest()?;
    d.key(0xffe3, true)?;
    d.key(KEYS[1], true)?;
    let after = d.advance(&before, 5)?;
    require(
        after["state"]["counts"] == before["state"]["counts"] && after["state"]["held"] == 0,
        "modified key reached game",
    )?;
    d.scenario("modified_key", before, after, false)?;
    d.key(KEYS[1], false)?;
    d.key(0xffe3, false)?;
    let before = d.latest()?;
    d.command("text")?;
    let focused = d.until(|v| v["phase"] == "Off" && v["state"]["held"] == 0)?;
    for k in KEYS {
        d.key(k, true)?;
        d.key(k, false)?;
    }
    let after = d.advance(&focused, 5)?;
    require(
        after["state"]["counts"] == before["state"]["counts"]
            && after["text"].as_str().is_some_and(|s| !s.is_empty()),
        "text focus not established or input leaked",
    )?;
    d.scenario("text_focus", before, after, false)?;
    d.command("dismiss_text")?;
    d.claim(false)?;
    d.neutral()?;
    for k in KEYS {
        d.key(k, true)?;
    }
    let before = d.until(|v| v["state"]["held"] == 15)?;
    d.command("reopen")?;
    let reset = d.until(|v| {
        v["scene_generation"] == 1
            && v["phase"] == "Off"
            && v["tick"].as_u64().is_some_and(|t| t > 0)
            && v["state"]["counts"] == json!([0, 0, 0, 0])
    })?;
    let reset = d.advance(&reset, 4)?;
    d.claim(false)?;
    let after = d.advance(&reset, 8)?;
    require(
        after["state"]["counts"] == json!([0, 0, 0, 0]) && after["state"]["held"] == 0,
        "scene reopen resurrected held actions",
    )?;
    d.scenarios.push(json!({"name":"scene_reset","before":before,"reset":reset,"after":after,"boundary":"supported_stop_save_replay_open_path_play","scene_generation_is_wrapper_label":true,"event_end":d.events.len()}));
    d.neutral()?;
    // Fresh post-reset physical edges must still function; all held pre-reset keys
    // were released before this deliberate new press.
    let before = d.latest()?;
    d.key(KEYS[3], true)?;
    let held = d.until(|v| v["state"]["held"] == 8)?;
    let after = d.advance(&held, 3)?;
    d.scenario("post_reset_fresh_press", before, after, true)?;
    d.neutral()?;
    d.command += 1;
    write(
        &out.join(format!("command-{}.json", d.command)),
        &json!({"operation":"finish"}),
    )?;
    let ready = out.join("finish-ready.json");
    let until = x11::Deadline::after(WAIT);
    while !ready.exists() {
        require(
            d.child.0.try_wait()?.is_none(),
            "editor exited before recording acknowledgment",
        )?;
        require(!until.expired(), "editor recording acknowledgment timeout")?;
        std::thread::sleep(Duration::from_millis(5));
    }
    require(
        read(&ready)? == json!({"scene_generation":1,"recording_saved":true}),
        "final recording acknowledgment mismatch",
    )?;
    d.x.close(d.window)?;
    let until = x11::Deadline::after(WAIT);
    loop {
        if let Some(status) = d.child.0.try_wait()? {
            require(status.success(), "editor child failed")?;
            break;
        }
        require(!until.expired(), "editor shutdown timeout")?;
        std::thread::sleep(Duration::from_millis(10));
    }
    let bytes = fs::read(out.join("observations.ndjson"))?;
    let observations: Vec<Value> = bytes
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(serde_json::from_slice)
        .collect::<std::result::Result<_, _>>()?;
    let replays = vec![replay(&out, 0)?, replay(&out, 1)?];
    // Bind each genuine live snapshot to the independently re-simulated recording.
    for observation in &observations {
        let generation = observation["scene_generation"]
            .as_u64()
            .ok_or("generation")? as usize;
        let tick = observation["tick"].as_u64().ok_or("tick")? as usize;
        if let Some(record) = replays[generation]["ticks"]
            .as_array()
            .and_then(|a| a.get(tick))
        {
            require(
                record["state"] == observation["state"]
                    && record["checksum"] == observation["checksum"],
                "live snapshot differs from actual host replay",
            )?;
        } else {
            return Err("live snapshot lies outside recorded epoch".into());
        }
    }
    write(
        &out.join("receipt.json"),
        &json!({"schema":SCHEMA,"mode":"editor","status":"passed","game":Fixture::ALIAS,"initial":initial,"provenance":{"compiled_source_sha":compiled,"executable_sha256":hash(&std::env::current_exe()?)?,"fixture_source_sha256":digest(include_bytes!("support/multi_action_fixture.rs")),"test_source_sha256":digest(include_bytes!("multi_action_editor_x11.rs")),"git_sha":std::env::var("GITHUB_SHA").ok(),"run_id":std::env::var("GITHUB_RUN_ID").ok(),"run_attempt":std::env::var("GITHUB_RUN_ATTEMPT").ok()},"adapter":read(&out.join("adapter.json"))?,"scene":{"file":"fixture.scene.yaml","sha256":hash(&out.join("fixture.scene.yaml"))?},"process_id":process_id,"window_id":window,"input_source":"XTEST_server_events","capture_source":"XGetImage_pid_bound_window","observations_file":"observations.ndjson","observations_sha256":hash(&out.join("observations.ndjson"))?,"events":d.events,"scenarios":d.scenarios,"captures":d.captures,"replays":replays,"boundaries":{"native_window":true,"xtest_server_events":true,"physical_keyboard":false,"physical_hardware_keyboard":false,"physical_gpu":false,"software_gpu":true,"packaged_export":false,"source_hidden":false,"production_editor_app":true,"raw_input_injection":false,"wrapper_lifecycle_api":true,"scene_generation_is_wrapper_label":true,"full_tick_state_is_replay_derived":true}}),
    )?;
    write(
        &out.join("completed.json"),
        &json!({"schema":SCHEMA,"mode":"editor","status":"passed","receipt_sha256":hash(&out.join("receipt.json"))?}),
    )?;
    Ok(())
}
