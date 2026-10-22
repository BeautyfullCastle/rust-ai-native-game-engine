//! Actual native eframe framebuffer acceptance, selected only by native-acceptance.
//! Injected egui events prove the App path, not physical OS keyboard delivery.
use eframe::App;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY},
    editor::input::Phase,
    Editor, EditorApp, Owner,
};
use orr_static_starter::{
    delivery::DeliveryGame,
    integration::{registry, Starter},
    switchboard::SwitchGame,
};
use serde_json::{json, Value as J};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const SIZE: [u32; 2] = [1400, 1000];
const SCHEMA: &str = "orr.static-game-native/1";
fn write_json(path: &Path, value: &J) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn hash(path: &Path) -> String {
    let result = std::process::Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("hosted coreutils sha256sum");
    assert!(
        result.status.success(),
        "sha256sum failed for {}",
        path.display()
    );
    let text = String::from_utf8(result.stdout).unwrap();
    let digest = text.split_whitespace().next().unwrap();
    assert!(digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()));
    digest.to_owned()
}
fn run_identity() -> J {
    json!({"compiled_source_sha":option_env!("ORR_STATIC_NATIVE_BUILD_SHA"),"executable_sha256":hash(&std::env::current_exe().unwrap()),"git_sha":std::env::var("GITHUB_SHA").ok(),"run_id":std::env::var("GITHUB_RUN_ID").ok(),"run_attempt":std::env::var("GITHUB_RUN_ATTEMPT").ok()})
}
#[test]
fn registered_games_native_window() {
    let out = std::env::var_os("ORR_NATIVE_STATIC_GAME_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("orr-static-native-{}", std::process::id()))
        });
    std::fs::create_dir_all(&out).unwrap();
    write_json(
        &out.join("attempt.json"),
        &json!({"schema":SCHEMA,"status":"started","provenance":run_identity(),"required_games":["delivery","switchboard"]}),
    );
    let outcome = std::panic::catch_unwind(|| {
        assert_eq!(
            std::env::var("ORR_REQUIRE_NATIVE_EDITOR").as_deref(),
            Ok("1"),
            "native selected: ORR_REQUIRE_NATIVE_EDITOR=1 required, never skipped"
        );
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert!(
            std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()),
            "native selected: DISPLAY required"
        );
        for key in ["GITHUB_SHA", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT"] {
            assert!(
                std::env::var(key).is_ok_and(|v| !v.is_empty()),
                "hosted provenance {key} required"
            );
        }
        let Some(compiled) = option_env!("ORR_STATIC_NATIVE_BUILD_SHA") else {
            panic!("native binary must be source-SHA bound at compilation");
        };
        assert!(
            compiled.len() == 40 && compiled.bytes().all(|c| c.is_ascii_hexdigit()),
            "compiled source SHA must be 40 hex digits"
        );
        assert_eq!(
            compiled,
            std::env::var("GITHUB_SHA").unwrap(),
            "stale native binary must not relabel itself with runtime SHA"
        );
        let mut contracts = serde_json::Map::new();
        for alias in ["delivery", "switchboard"] {
            let handle = registry().handle(alias).unwrap();
            let id = handle.expected_identity();
            let input = handle.expected_input();
            contracts.insert(alias.into(),json!({"identity":{"game":id.game,"build_id":id.build_id,"schema":id.schema},
            "input_contract":{"schema":input.schema(),"value_format":input.value_format(),"max_players":input.max_players()},
            "neutral":handle.neutral_value(),"managed_held":{"version":1,"lease_ms":2000,"heartbeat_ms":500}}));
        }
        write_json(
            &out.join("expected-contracts.json"),
            &json!({"schema":SCHEMA,"games":contracts}),
        );
        let contracts_hash = hash(&out.join("expected-contracts.json"));
        write_json(
            &out.join("attempt.json"),
            &json!({"schema":SCHEMA,"status":"started","provenance":run_identity(),"required_games":["delivery","switchboard"],"expected_contracts_sha256":contracts_hash}),
        );
        run_games(&out, contracts_hash.clone());
        write_json(
            &out.join("attempt.json"),
            &json!({"schema":SCHEMA,"status":"passed","provenance":run_identity(),"required_games":["delivery","switchboard"],"expected_contracts_sha256":contracts_hash}),
        );
    });
    if let Err(error) = outcome {
        let reason = error
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| error.downcast_ref::<&str>().copied())
            .unwrap_or("non-string panic");
        let mut failed: J =
            serde_json::from_slice(&std::fs::read(out.join("attempt.json")).unwrap()).unwrap();
        failed["status"] = json!("failed");
        failed["reason"] = json!(reason);
        write_json(&out.join("attempt.json"), &failed);
        std::panic::resume_unwind(error);
    }
}
struct Prepared {
    alias: &'static str,
    directory: PathBuf,
    scene: PathBuf,
    editor: Editor,
    play: egui::Pos2,
    pause: egui::Pos2,
    take: egui::Pos2,
}
type Completion = Arc<Mutex<Option<Result<(), String>>>>;
fn prepare_game(alias: &'static str, out: &Path) -> Prepared {
    let directory = out.join(alias);
    std::fs::create_dir_all(&directory).unwrap();
    let scene = directory.join("authored.scene.yaml");
    let text = if alias == "delivery" {
        DeliveryGame::SCENE
            .replace("[0, 0]", "[2, 2]")
            .replace("[4, 0]", "[3, 3]")
    } else {
        SwitchGame::SCENE
            .replace("target: 5", "target: 4")
            .replace("position: [0, 0]", "position: [2, 0]")
            .replace("selected: 0", "selected: 1")
    };
    std::fs::write(&scene, text).unwrap();
    let handle = registry().handle(alias).unwrap();
    let mut editor = Editor::open_registered(&scene, handle).unwrap();
    let actual_input = editor.host_call("registry.input", json!({})).unwrap();
    handle.expected_input().validate(&actual_input).unwrap();
    assert_eq!(
        actual_input["managed_held"],
        json!({"version":1,"lease_ms":2000,"heartbeat_ms":500})
    );
    // This non-rendering harness locates widgets only. None of its pixels are evidence.
    let mut locator = Harness::builder()
        .with_size([SIZE[0] as f32, SIZE[1] as f32])
        .build_eframe(move |_| EditorApp::new(editor, None));
    locator.run_steps(5);
    let play = locator.get_by_label(LBL_PLAY).rect().center();
    let pause = locator.get_by_label(LBL_PAUSE).rect().center();
    let take = locator.get_by_label("Take control").rect().center();
    let editor = locator.into_state().editor;
    Prepared {
        alias,
        directory,
        scene,
        editor,
        play,
        pause,
        take,
    }
}
fn run_games(out: &Path, contracts_hash: String) {
    let prepared: Vec<_> = ["delivery", "switchboard"]
        .into_iter()
        .map(|alias| prepare_game(alias, out))
        .collect();
    let result: Completion = Arc::new(Mutex::new(None));
    let finished = result.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([SIZE[0] as f32, SIZE[1] as f32]),
        event_loop_builder: Some(Box::new(|builder| {
            #[cfg(target_os = "linux")]
            {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_any_thread(true);
            }
            #[cfg(target_os = "windows")]
            {
                use winit::platform::windows::EventLoopBuilderExtWindows;
                builder.with_any_thread(true);
            }
        })),
        ..Default::default()
    };
    eframe::run_native("Registered game native acceptance",options,Box::new(move |cc| {
        cc.egui_ctx.set_pixels_per_point(1.0);
        let info=cc.wgpu_render_state.as_ref().expect("required native GPU renderer").adapter.get_info();
        let backend=format!("{:?}",info.backend); let device_type=format!("{:?}",info.device_type);
        let description=format!("{} {} {}",info.name,info.driver,info.driver_info).to_lowercase();
        assert_eq!(backend,"Vulkan"); assert_eq!(device_type,"Cpu","hosted acceptance must use software Mesa Vulkan");
        assert!(!description.contains("swiftshader") && ["mesa","llvmpipe","lavapipe"].iter().any(|s|description.contains(s)),"expected Mesa software Vulkan, got {description}");
        let adapter=json!({"name":info.name,"driver":info.driver,"driver_info":info.driver_info,"backend":backend,"device_type":device_type});
        let mut proofs=std::collections::VecDeque::new();
        for Prepared {alias,directory,scene,editor,play,pause,take} in prepared {
        let handle=registry().handle(alias).unwrap();
        let identity=handle.expected_identity(); let input=handle.expected_input();
        let receipt=json!({"schema":SCHEMA,"game":alias,"source":"editor.app_framebuffer","input_source":"queued_egui_raw_input",
            "identity":{"game":identity.game,"build_id":identity.build_id,"schema":identity.schema},
            "input_contract":{"schema":input.schema(),"value_format":input.value_format(),"max_players":input.max_players()},
            "expected_contracts_sha256":contracts_hash,"adapter":adapter,"scene":{"path":"authored.scene.yaml","sha256":hash(&scene)},"captures":[],"checks":{},"provenance":run_identity(),
            "boundaries":{"native_window":true,"physical_keyboard":false,"physical_gpu":false,"packaged_export":false}});
        write_json(&directory.join("native-receipt.json"),&receipt);
        proofs.push_back(Proof { app:EditorApp::new(editor,cc.wgpu_render_state.clone()),alias,directory,play,pause,take,
            stage:0,stage_at:0,queued:vec![],focused:true,pending:None,receipt,baseline:J::Null,finished:finished.clone(),last:alias=="switchboard" });
        }
        let current=proofs.pop_front().unwrap();
        Ok(Box::new(NativeSuite {current,remaining:proofs}))
    })).expect("actual native eframe window/event loop");
    result
        .lock()
        .unwrap()
        .take()
        .expect("native window exited before proof completed")
        .unwrap();
}
// One winit event loop hosts both genuinely distinct registered games. Recreating
// EventLoop after run_native returns is unsupported on the required X11 runner.
struct NativeSuite {
    current: Proof,
    remaining: std::collections::VecDeque<Proof>,
}
impl App for NativeSuite {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        self.current.raw_input_hook(ctx, input);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.current.ui(ui, frame);
        if self.current.stage == 20 && !self.remaining.is_empty() {
            self.current
                .finished
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .unwrap();
            self.current = self.remaining.pop_front().unwrap();
            ui.ctx().request_repaint();
        }
    }
}
struct Proof {
    last: bool,
    app: EditorApp,
    alias: &'static str,
    directory: PathBuf,
    play: egui::Pos2,
    pause: egui::Pos2,
    take: egui::Pos2,
    stage: u8,
    stage_at: u64,
    queued: Vec<egui::Event>,
    focused: bool,
    pending: Option<(&'static str, J)>,
    receipt: J,
    baseline: J,
    finished: Completion,
}
impl Proof {
    fn stage(&mut self, n: u8) {
        self.stage = n;
        self.stage_at = self.app.frame_count();
    }
    fn check(&mut self, name: &str) {
        self.receipt["checks"][name] = json!(true);
    }
    fn click(&mut self, p: egui::Pos2) {
        self.queued.push(egui::Event::PointerMoved(p));
        for pressed in [true, false] {
            self.queued.push(egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn key(&mut self, key: egui::Key, pressed: bool) {
        self.queued.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
    fn clear_keys(&mut self) {
        for k in [
            egui::Key::W,
            egui::Key::A,
            egui::Key::S,
            egui::Key::D,
            egui::Key::Space,
        ] {
            self.key(k, false);
        }
    }
    fn escape(&mut self) {
        self.key(egui::Key::Escape, true);
        self.key(egui::Key::Escape, false);
    }
    fn released(&self) -> bool {
        self.app.editor.input_phase() == Phase::Off && !self.app.editor.input_cleanup_pending()
    }
    fn typed(&mut self) -> J {
        let (component, singleton) = if self.alias == "delivery" {
            ("Courier", "Delivery")
        } else {
            ("Cursor", "Board")
        };
        let actor = self
            .app
            .editor
            .host_call(
                "world.get",
                json!({"entity":"e_00000001","component":component}),
            )
            .unwrap()["value"]
            .clone();
        let state = self
            .app
            .editor
            .host_call("world.singleton.get", json!({"name":singleton}))
            .unwrap()["value"]
            .clone();
        json!({"actor":actor,"state":state})
    }
    fn won(&mut self) -> bool {
        let t = self.typed();
        t["state"][if self.alias == "delivery" {
            "won"
        } else {
            "solved"
        }] == 1
    }
    #[allow(clippy::float_arithmetic)] // Convert view camera coordinates to framebuffer pixels only.
    fn screen(&self, world: [f32; 2]) -> egui::Pos2 {
        let p = self
            .app
            .editor
            .camera
            .world_to_screen(world, self.app.ui.viewport_px);
        self.app.ui.viewport_rect.unwrap().min + egui::vec2(p[0], p[1])
    }
    fn capture(&mut self, ctx: &egui::Context, phase: &'static str) {
        let state = self.app.editor.host_call("sim.state", J::Null).unwrap();
        assert_eq!(
            state["playing"], false,
            "capture must freeze authoritative state"
        );
        let metadata = json!({"phase":phase,"ui_frame":self.app.frame_count(),"state":state,"typed":self.typed(),
            "selected_guids":self.app.editor.selected_guids().iter().map(ToString::to_string).collect::<Vec<_>>(),
            "camera":{"center":self.app.editor.camera.center,"half_extent":self.app.editor.camera.half_extent},
            "viewport":{"min":[self.app.ui.viewport_rect.unwrap().min.x,self.app.ui.viewport_rect.unwrap().min.y],"px":self.app.ui.viewport_px}});
        self.pending = Some((phase, metadata));
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
            phase,
        )));
    }
    fn neutral(&mut self, label: &str) {
        assert!(self.released());
        assert!(!self.app.editor.sim().playing);
        let before = self.typed();
        self.app.editor.step(1);
        self.app.editor.sync();
        let after = self.typed();
        assert_eq!(
            after["actor"]["position"], before["actor"]["position"],
            "one neutral tick must not move"
        );
        if self.alias == "switchboard" {
            assert_eq!(after["state"]["moves"], before["state"]["moves"]);
            assert_eq!(after["actor"]["held"], 0);
        } else {
            assert_eq!(after["state"]["action_held"], 0);
        }
        let before_release = self.receipt["release_proofs"][label]["before_release"].clone();
        assert!(!before_release.is_null());
        self.receipt["release_proofs"][label] = json!({"before_release":before_release,"before":before,"after":after,"neutral_ticks":1,"phase":"Off","cleanup_pending":false});
        self.check(label);
    }
    #[allow(clippy::float_arithmetic)] // Independent framebuffer pixel geometry, never simulation state.
    fn pixels(&self, image: &egui::ColorImage, phase: &str) -> J {
        assert_eq!(image.size, [SIZE[0] as usize, SIZE[1] as usize]);
        let count = |world: [f32; 2], kind: &str| -> usize {
            let p = self.screen(world);
            let mut total = 0;
            for y in (p.y as i32 - 8).max(0)..=(p.y as i32 + 8).min(SIZE[1] as i32 - 1) {
                for x in (p.x as i32 - 8).max(0)..=(p.x as i32 + 8).min(SIZE[0] as i32 - 1) {
                    let [r, g, b, _] =
                        image.pixels[y as usize * SIZE[0] as usize + x as usize].to_array();
                    let yes = match kind {
                        "blue" => b.saturating_sub(r) > 25 && b.saturating_sub(g) > 10,
                        "green" => g.saturating_sub(r) > 40 && g.saturating_sub(b) > 40,
                        "gold" => r > 200 && g > 150 && b < 180,
                        _ => false,
                    };
                    if yes {
                        total += 1;
                    }
                }
            }
            total
        };
        let selection_outline = if phase == "edit" {
            let world = if self.alias == "delivery" {
                [2.0, 2.0]
            } else {
                [2.0, 0.0]
            };
            let center = self.screen(world);
            let edge = self.screen([world[0] + 0.4, world[1]]);
            let radius = (edge.x - center.x).abs();
            let bound = radius.ceil() as i32 + 5;
            let mut count = 0;
            for y in
                (center.y as i32 - bound).max(0)..=(center.y as i32 + bound).min(SIZE[1] as i32 - 1)
            {
                for x in (center.x as i32 - bound).max(0)
                    ..=(center.x as i32 + bound).min(SIZE[0] as i32 - 1)
                {
                    let distance =
                        ((x as f32 - center.x).powi(2) + (y as f32 - center.y).powi(2)).sqrt();
                    let [r, g, b, _] =
                        image.pixels[y as usize * SIZE[0] as usize + x as usize].to_array();
                    if (distance - radius).abs() < 5.0 && r > 220 && g > 180 && b < 180 {
                        count += 1;
                    }
                }
            }
            assert!(
                count > 10,
                "selected actor outline absent from native framebuffer: {count}"
            );
            count
        } else {
            0
        };
        // Oracles use authored known world positions/colors, never draw() output.
        let mut counts = if self.alias == "delivery" {
            let blue = count([2.0, 2.0], "blue");
            let gold = count([3.0, 3.0], "gold");
            if phase == "edit" || phase == "reopened" {
                assert!(blue > 20, "delivery courier blue pixels absent: {blue}");
            }
            if phase == "won" {
                assert_eq!(blue, 0, "courier must leave its authored origin");
            }
            assert!(gold > 20, "delivery goal pixels absent: {gold}");
            json!({"courier_origin_blue":blue,"goal_gold":gold})
        } else {
            let blue = count([2.0, 0.0], "blue");
            let green = count([4.0, 2.0], "green");
            if phase == "edit" || phase == "reopened" {
                assert!(blue > 20, "switch cursor pixels absent: {blue}");
                assert_eq!(green, 0);
            }
            if phase == "won" {
                assert!(green > 20, "switch target lamp must be green: {green}");
            }
            json!({"cursor_origin_blue":blue,"target_lamp_green":green})
        };
        counts["selection_outline"] = json!(selection_outline);
        counts
    }
    fn save_capture(&mut self, phase: &str, mut metadata: J, image: &egui::ColorImage) {
        let pixels = self.pixels(image, phase);
        let name = format!("{phase}.png");
        let path = self.directory.join(&name);
        orr_editor::app::write_png(&path, image).unwrap();
        metadata["png"] = json!(name);
        metadata["sha256"] = json!(hash(&path));
        metadata["pixel_counts"] = pixels;
        self.receipt["captures"]
            .as_array_mut()
            .unwrap()
            .push(metadata);
        write_json(&self.directory.join("native-receipt.json"), &self.receipt);
    }
}
impl App for Proof {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        input.focused = self.focused;
        input
            .events
            .push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        input.events.append(&mut self.queued);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.app.frame_count() > 1800 {
            write_json(
                &self.directory.join("failure.json"),
                &json!({"stage":self.stage,"receipt":self.receipt}),
            );
            *self.finished.lock().unwrap() = Some(Err(format!(
                "native stage {} exhausted frame budget",
                self.stage
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // Drive the same asynchronous pump as production UI. Do not insert
        // Editor::sync here: it refreshes caches the actual App must maintain.
        self.app.ui(ui, frame);
        if let Some((phase, metadata)) = self.pending.take() {
            let image = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot {
                        viewport_id,
                        user_data,
                        image,
                    } if *viewport_id == egui::ViewportId::ROOT
                        && user_data
                            .data
                            .as_ref()
                            .and_then(|d| d.downcast_ref::<&'static str>())
                            == Some(&phase) =>
                    {
                        Some(image.clone())
                    }
                    _ => None,
                })
            });
            if let Some(image) = image {
                self.save_capture(phase, metadata, &image);
                match phase {
                    "edit" => {
                        self.click(self.play);
                        self.stage(2);
                    }
                    "won" => {
                        self.click(self.play);
                        self.stage(7);
                    }
                    "neutral" => {
                        self.app.editor.stop();
                        self.app.editor.sync();
                        assert!(self.app.editor.restart());
                        self.app.editor.sync();
                        self.stage(19);
                    }
                    "reopened" => {
                        self.receipt["status"] = json!("passed");
                        write_json(&self.directory.join("native-receipt.json"), &self.receipt);
                        *self.finished.lock().unwrap() = Some(Ok(()));
                        if self.last {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        self.stage(20);
                    }
                    _ => unreachable!(),
                }
            } else {
                self.pending = Some((phase, metadata));
            }
        } else {
            let stage = self.stage;
            match stage {
                0 if self.app.frame_count() > 5 => {
                    let p = self.screen(if self.alias == "delivery" {
                        [2.0, 2.0]
                    } else {
                        [2.0, 0.0]
                    });
                    self.click(p);
                    self.stage(1);
                }
                1 if self.app.editor.selected_guids().len() == 1
                    && self.app.frame_count() > self.stage_at + 3 =>
                {
                    assert_eq!(
                        self.app.editor.selected_guids()[0].to_string(),
                        "e_00000001"
                    );
                    self.check("viewport_pick");
                    self.capture(&ctx, "edit");
                }
                2 if self.app.editor.sim().playing => {
                    self.click(self.take);
                    self.stage(3);
                }
                3 if self.app.editor.input_phase() == Phase::Active => {
                    self.check("take_control_ui");
                    self.key(egui::Key::D, true);
                    if self.alias == "delivery" {
                        self.key(egui::Key::W, true);
                    }
                    self.key(egui::Key::Space, true);
                    self.stage(4);
                }
                4 if self.won() => {
                    let typed = self.typed();
                    if self.alias == "delivery" {
                        assert_eq!(typed["actor"]["position"], json!([3, 3]));
                        assert_eq!(typed["state"]["goal"], json!([3, 3]));
                    } else {
                        assert_eq!(typed["state"]["bits"], 4);
                        assert_eq!(typed["state"]["target"], 4);
                    }
                    self.check("keyboard_win");
                    self.clear_keys();
                    self.escape();
                    self.stage(5);
                }
                5 if self.released() => {
                    self.click(self.pause);
                    self.stage(6);
                }
                6 if !self.app.editor.sim().playing
                    && self.app.frame_count() > self.stage_at + 3 =>
                {
                    // Flush one acknowledged neutral tick before a fresh Space edge.
                    self.app.editor.step(1);
                    self.app.editor.sync();
                    self.capture(&ctx, "won");
                }
                7 if self.app.editor.sim().playing => {
                    self.click(self.take);
                    self.stage(8);
                }
                8 if self.app.editor.input_phase() == Phase::Active => {
                    self.key(egui::Key::Space, true);
                    self.stage(9);
                }
                9 if !self.won() => {
                    let reset = self.typed();
                    if self.alias == "delivery" {
                        assert_eq!(reset["actor"]["position"], json!([0, 0]));
                    } else {
                        assert_eq!(reset["state"]["bits"], 0);
                        assert_eq!(reset["state"]["moves"], 0);
                    }
                    self.receipt["restart_proof"] = reset;
                    self.check("space_restart");
                    self.clear_keys();
                    // Separate nonterminal authoring fixture for release proofs. Two
                    // action edges cannot solve target 1111 and mask held movement.
                    self.app.editor.stop();
                    self.app.editor.sync();
                    if self.alias == "switchboard" {
                        assert!(self.app.editor.set_field(
                            &Owner::Singleton("Board".into()),
                            "target",
                            orr_reflect::Value::Int(15)
                        ));
                        self.app.editor.sync();
                    }
                    self.receipt["release_fixture"] = self.typed();
                    self.click(self.play);
                    self.stage(21);
                }
                10 if self.typed()["actor"]["position"] != self.baseline => {
                    assert!(!self.won(), "release proof must remain nonterminal");
                    let before = self.typed();
                    assert_eq!(
                        before[if self.alias == "delivery" {
                            "state"
                        } else {
                            "actor"
                        }][if self.alias == "delivery" {
                            "action_held"
                        } else {
                            "held"
                        }],
                        1
                    );
                    self.receipt["release_proofs"]["escape_neutral"]["before_release"] = before;
                    self.escape();
                    self.stage(11);
                }
                11 if self.released() => {
                    self.clear_keys();
                    self.click(self.pause);
                    self.stage(12);
                }
                12 if !self.app.editor.sim().playing => {
                    self.neutral("escape_neutral");
                    self.receipt["no_reclaim_proofs"]["escape"]["before"] = self.typed();
                    self.baseline = self.typed()["actor"]["position"].clone();
                    self.click(self.play);
                    self.stage(13);
                }
                13 if self.app.editor.sim().playing
                    && self.app.frame_count() > self.stage_at + 5 =>
                {
                    assert!(self.released());
                    assert_eq!(self.typed()["actor"]["position"], self.baseline);
                    self.receipt["no_reclaim_proofs"]["escape"]["after"] = self.typed();
                    self.receipt["no_reclaim_proofs"]["escape"]["phase"] = json!("Off");
                    self.receipt["no_reclaim_proofs"]["escape"]["cleanup_pending"] = json!(false);
                    self.receipt["no_reclaim_proofs"]["escape"]["observed_ui_frames"] =
                        json!(self.app.frame_count() - self.stage_at);
                    self.check("no_auto_reclaim");
                    self.click(self.take);
                    self.stage(14);
                }
                14 if self.app.editor.input_phase() == Phase::Active => {
                    self.baseline = self.typed()["actor"]["position"].clone();
                    self.key(egui::Key::Space, true);
                    self.key(egui::Key::A, true);
                    if self.alias == "delivery" {
                        self.key(egui::Key::S, true);
                    }
                    self.stage(15);
                }
                15 if self.typed()["actor"]["position"] != self.baseline => {
                    assert!(!self.won(), "focus release proof must remain nonterminal");
                    let before = self.typed();
                    assert_eq!(
                        before[if self.alias == "delivery" {
                            "state"
                        } else {
                            "actor"
                        }][if self.alias == "delivery" {
                            "action_held"
                        } else {
                            "held"
                        }],
                        1
                    );
                    self.receipt["release_proofs"]["focus_neutral"]["before_release"] = before;
                    self.focused = false;
                    self.stage(16);
                }
                16 if self.released() => {
                    self.clear_keys();
                    self.focused = true;
                    self.click(self.pause);
                    self.stage(17);
                }
                17 if !self.app.editor.sim().playing => {
                    self.neutral("focus_neutral");
                    self.receipt["no_reclaim_proofs"]["focus"]["before"] = self.typed();
                    self.baseline = self.typed()["actor"]["position"].clone();
                    self.click(self.play);
                    self.stage(24);
                }
                18 if self.app.frame_count() > self.stage_at + 3 => {
                    self.capture(&ctx, "neutral");
                }
                19 if self.app.frame_count() > self.stage_at + 3 => {
                    assert_eq!(
                        self.app.editor.game().registered(),
                        Some(registry().handle(self.alias).unwrap())
                    );
                    assert!(self.released());
                    self.check("reopen_identity");
                    self.capture(&ctx, "reopened");
                }
                21 if self.app.editor.sim().playing => {
                    self.click(self.take);
                    self.stage(22);
                }
                22 if self.app.editor.input_phase() == Phase::Active => {
                    self.baseline = self.typed()["actor"]["position"].clone();
                    self.key(egui::Key::D, true);
                    self.key(egui::Key::Space, true);
                    self.stage(10);
                }
                24 if self.app.editor.sim().playing
                    && self.app.frame_count() > self.stage_at + 5 =>
                {
                    assert!(self.released());
                    assert_eq!(self.typed()["actor"]["position"], self.baseline);
                    self.receipt["no_reclaim_proofs"]["focus"]["after"] = self.typed();
                    self.receipt["no_reclaim_proofs"]["focus"]["phase"] = json!("Off");
                    self.receipt["no_reclaim_proofs"]["focus"]["cleanup_pending"] = json!(false);
                    self.receipt["no_reclaim_proofs"]["focus"]["observed_ui_frames"] =
                        json!(self.app.frame_count() - self.stage_at);
                    self.check("focus_regain_no_auto_reclaim");
                    self.click(self.pause);
                    self.stage(25);
                }
                25 if !self.app.editor.sim().playing => {
                    self.app.editor.camera.center = [0.0, 0.0];
                    self.app.editor.camera.half_extent = 12.0;
                    self.stage(18);
                }
                _ => {}
            }
        }
        ctx.request_repaint();
    }
}
