//! API-only acceptance of the actual EditorApp offscreen WGPU framebuffer.
//! No OS input/window automation and no synthetic Screenshot events are used.
//! Paused query/capture/query checks detect drift; they are not an atomic bundle.
use egui::{Pos2, Rect, ViewportId};
use egui_kittest::Harness;
use orr_editor::{registered_project::PreparedRegisteredProject, Editor, EditorApp};
use orr_remote::{Caps, ErpClient, RpcError};
use serde_json::{json, Value as J};
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    path::PathBuf,
    time::{Duration, Instant},
};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 720;

fn fixture(alias: &str) -> (tempfile::TempDir, PathBuf) {
    let (manifest, filename, scene) = match alias {
        "delivery" => (
            include_str!("../scenes/delivery.project.json"),
            "delivery.scene.yaml",
            crate::delivery::SCENE,
        ),
        "switchboard" => (
            include_str!("../scenes/switchboard.project.json"),
            "switchboard.scene.yaml",
            crate::switchboard::SCENE,
        ),
        _ => panic!("unknown project fixture"),
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path, manifest).unwrap();
    std::fs::write(dir.path().join(filename), scene).unwrap();
    (dir, path)
}

fn harness(editor: Editor, alias: &str, focused: bool) -> Harness<'static, EditorApp> {
    let alias = alias.to_owned();
    let mut harness = Harness::builder()
        .with_size([WIDTH as f32, HEIGHT as f32])
        .with_pixels_per_point(1.0)
        .wgpu()
        .build_eframe(move |cc| {
            let gpu = cc.wgpu_render_state.as_ref().expect("actual WGPU renderer required");
            let info = gpu.adapter.get_info();
            let kind = format!("{:?}", info.device_type);
            let classification = match kind.as_str() {
                "DiscreteGpu" | "IntegratedGpu" => "hardware_wgpu",
                "Cpu" => "software_wgpu",
                _ => "unclassified_wgpu",
            };
            eprintln!("API_GPU_ADAPTER {}", json!({"project":alias,"classification":classification,"adapter_type":kind,"surface":"offscreen EditorApp; not OS-window acceptance"}));
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        });
    harness.input_mut().focused = focused;
    harness
        .input_mut()
        .viewports
        .get_mut(&ViewportId::ROOT)
        .unwrap()
        .inner_rect = Some(Rect::from_min_size(
        Pos2::ZERO,
        egui::vec2(WIDTH as f32, HEIGHT as f32),
    ));
    harness.run_steps(3);
    // Warm the real renderer before beginning the protocol's bounded deadline.
    let image = harness
        .render()
        .expect("WGPU readback must succeed; no CPU fake fallback");
    assert_eq!((image.width(), image.height()), (WIDTH, HEIGHT));
    harness
}

fn rpc_result(
    harness: &mut Harness<'_, EditorApp>,
    agent: &mut ErpClient,
    method: &str,
    params: J,
) -> Result<J, RpcError> {
    rpc_before(
        harness,
        agent,
        method,
        params,
        Instant::now() + Duration::from_secs(10),
    )
}

fn rpc_before(
    harness: &mut Harness<'_, EditorApp>,
    agent: &mut ErpClient,
    method: &str,
    params: J,
    deadline: Instant,
) -> Result<J, RpcError> {
    assert!(
        Instant::now() < deadline,
        "request deadline before {method}"
    );
    let id = agent
        .post(method, params)
        .expect("post bounded ERP request");
    loop {
        assert!(
            Instant::now() < deadline,
            "request deadline before {method} pump"
        );
        harness.run_steps(1);
        assert!(
            Instant::now() < deadline,
            "request deadline after {method} pump"
        );
        agent
            .poll()
            .expect("ERP remains connected while EditorApp pumps");
        if let Some(result) = agent.take_response(id) {
            assert!(
                Instant::now() < deadline,
                "late terminal response for {method}"
            );
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "no terminal response for {method}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn rpc(harness: &mut Harness<'_, EditorApp>, agent: &mut ErpClient, method: &str, params: J) -> J {
    rpc_result(harness, agent, method, params).unwrap_or_else(|error| panic!("{method}: {error:?}"))
}

fn assert_paused_anchor(expected: &J, current: &J, label: &str) {
    assert_eq!(current["playing"], false);
    for key in ["mode", "playing", "head_tick", "epoch", "checksum"] {
        assert_eq!(expected[key], current[key], "paused {label} drift: {key}");
    }
}

fn capture_image(
    harness: &mut Harness<'_, EditorApp>,
    agent: &mut ErpClient,
    anchor: &J,
    world_anchor: &J,
    label: &str,
) -> J {
    // Only terminal, read-only stale/busy captures may be attempted again.
    // Never retry mutations, transport uncertainty, timeout, unavailable or GPU failure.
    // The ten-second polling deadline is checked around every UI pump. A
    // synchronous WGPU readback can overrun a check until its own bounded GPU
    // wait returns (kittest WAIT_TIMEOUT is ten seconds); late results fail.
    let deadline = Instant::now() + Duration::from_secs(10);
    for attempt in 1..=4 {
        let state = rpc_before(harness, agent, "sim.state", J::Null, deadline).unwrap();
        assert_paused_anchor(anchor, &state, label);
        let world = rpc_before(
            harness,
            agent,
            "world.query",
            json!({"values":true,"limit":32}),
            deadline,
        )
        .unwrap();
        assert_eq!(
            &world, world_anchor,
            "paused world anchor changed before capture"
        );
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(5000);
        assert!(remaining >= 50, "capture attempt deadline exhausted");
        let result = rpc_before(
            harness,
            agent,
            "view.screenshot",
            json!({"target":"app_framebuffer","timeout_ms":remaining as u64,"max_width":WIDTH,"max_height":HEIGHT}),
            deadline,
        );
        if let Err(error) = &result {
            match error.kind() {
                Some(kind @ ("view_stale" | "view_busy")) => {
                    eprintln!(
                        "API_GPU_CAPTURE_RETRY {}",
                        json!({"label":label,"attempt":attempt,"kind":kind,"remaining_attempts":4-attempt})
                    );
                }
                _ => panic!("view.screenshot is not retryable: {error:?}"),
            }
        }
        let world = rpc_before(
            harness,
            agent,
            "world.query",
            json!({"values":true,"limit":32}),
            deadline,
        )
        .unwrap();
        let state = rpc_before(harness, agent, "sim.state", J::Null, deadline).unwrap();
        assert_paused_anchor(anchor, &state, label);
        assert_eq!(
            &world, world_anchor,
            "paused world anchor changed during capture"
        );
        match result {
            Ok(image) => {
                assert!(Instant::now() < deadline, "late screenshot result");
                return image;
            }
            Err(error) => match error.kind() {
                Some(kind @ ("view_stale" | "view_busy")) => {
                    assert!(attempt < 4, "read-only capture attempts exhausted: {kind}");
                    assert!(
                        Instant::now() < deadline,
                        "capture retry deadline exhausted"
                    );
                    harness.run_steps(2);
                    assert!(
                        Instant::now() < deadline,
                        "capture retry pump exceeded deadline"
                    );
                }
                _ => panic!("view.screenshot is not retryable: {error:?}"),
            },
        }
    }
    unreachable!("bounded attempts return an image or fail")
}

fn number(value: &J) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .expect("numeric protocol field")
}

fn describe(harness: &mut Harness<'_, EditorApp>, agent: &mut ErpClient) -> J {
    rpc(harness, agent, "editor.gameplay.describe", json!({}))
}

fn wait_description(
    harness: &mut Harness<'_, EditorApp>,
    agent: &mut ErpClient,
    mut done: impl FnMut(&J) -> bool,
) -> J {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let description = describe(harness, agent);
        assert_eq!(
            description["error"], false,
            "semantic owner reports an error"
        );
        if done(&description) {
            return description;
        }
        assert!(
            Instant::now() < deadline,
            "semantic API readiness did not settle"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn mutation(
    harness: &mut Harness<'_, EditorApp>,
    agent: &mut ErpClient,
    key: &mut u64,
    method: &str,
    mut params: J,
) -> J {
    let description = describe(harness, agent);
    for field in ["binding", "connection", "scope"] {
        assert!(description[field]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        params[field] = description[field].clone();
    }
    *key = key.checked_add(1).expect("bounded test request keys");
    params["request_key"] = json!(key.to_string());
    let response = rpc(harness, agent, method, params);
    assert_eq!(response["binding"], description["binding"]);
    assert_eq!(response["connection"], description["connection"]);
    response
}

fn control(
    harness: &mut Harness<'_, EditorApp>,
    agent: &mut ErpClient,
    key: &mut u64,
    action: &str,
) {
    let result = mutation(
        harness,
        agent,
        key,
        "editor.gameplay.control",
        json!({"action":action}),
    );
    assert_eq!(result["status"], "queued");
}

fn wait_completed(harness: &mut Harness<'_, EditorApp>, agent: &mut ErpClient, command: &J) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = rpc(
            harness,
            agent,
            "editor.gameplay.status",
            json!({"binding":command["binding"],"connection":command["connection"],"command_id":command["command_id"]}),
        );
        match status["status"].as_str().expect("semantic command status") {
            "completed" => {
                assert!(number(&status["neutral"]["tick"]) > number(&status["consumed"]["tick"]));
                return;
            }
            "pending" | "consumed" => {}
            other => panic!("semantic command did not complete: {other}"),
        }
        assert!(
            Instant::now() < deadline,
            "semantic command completion deadline"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

struct Capture {
    metadata: J,
    rgba: Vec<u8>,
    viewport: Rect,
}

fn capture_paused(
    harness: &mut Harness<'_, EditorApp>,
    agent: &mut ErpClient,
    identity: &J,
    label: &str,
) -> Capture {
    wait_description(harness, agent, |state| {
        state["playing"] == false && state["cleanup_pending"] == false
    });
    let before = rpc(harness, agent, "sim.state", J::Null);
    assert_eq!(before["playing"], false);
    let world_before = rpc(
        harness,
        agent,
        "world.query",
        json!({"values":true,"limit":32}),
    );
    assert_eq!(world_before["checksum"], before["checksum"]);
    let image = capture_image(harness, agent, &before, &world_before, label);
    let world_after = rpc(
        harness,
        agent,
        "world.query",
        json!({"values":true,"limit":32}),
    );
    let after = rpc(harness, agent, "sim.state", J::Null);
    // This sequential check is deliberately narrower than atomic query+capture.
    for key in ["mode", "playing", "head_tick", "epoch", "checksum"] {
        assert_eq!(before[key], after[key], "paused {label} drift: {key}");
    }
    assert_eq!(
        world_before, world_after,
        "paused world query changed during capture"
    );
    assert_eq!(image["status"], "captured");
    assert_eq!(image["source"], "editor.app_framebuffer");
    assert_eq!(image["game"], identity["engine"]["game"]);
    assert_eq!(image["build_id"], identity["engine"]["build_id"]);
    assert_eq!(image["mode"], before["mode"]);
    assert_eq!(image["paused"], true);
    assert_eq!(number(&image["tick"]), number(&before["head_tick"]));
    // Screenshot epoch is PlaySession timeline epoch, not FrameScope.play_epoch.
    assert_eq!(number(&image["epoch"]), number(&before["epoch"]));
    assert_eq!(image["checksum"], before["checksum"]);
    assert!(number(&image["frame_seq"]) > 0 && number(&image["ui_frame"]) > 0);
    assert_eq!(number(&image["width"]), u64::from(WIDTH));
    assert_eq!(number(&image["height"]), u64::from(HEIGHT));
    assert_eq!(image["mime_type"], "image/png");
    let png = orr_remote::codec::b64_decode(image["png_base64"].as_str().unwrap())
        .expect("valid PNG base64");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let mut reader = png::Decoder::new(Cursor::new(&png))
        .read_info()
        .expect("decode actual framebuffer PNG");
    assert_eq!((reader.info().width, reader.info().height), (WIDTH, HEIGHT));
    assert_eq!(reader.info().color_type, png::ColorType::Rgba);
    let mut rgba = vec![0; reader.output_buffer_size().expect("bounded output size")];
    let decoded = reader
        .next_frame(&mut rgba)
        .expect("read actual rendered image");
    rgba.truncate(decoded.buffer_size());
    assert_eq!(rgba.len(), (WIDTH * HEIGHT * 4) as usize);
    assert!(
        rgba.chunks_exact(4).any(|pixel| pixel[..3] != [0, 0, 0]),
        "blank fake image rejected"
    );
    let viewport = harness
        .state()
        .ui
        .viewport_rect
        .expect("actual app game viewport");
    assert!(viewport.is_positive());
    let mut metadata = image;
    metadata.as_object_mut().unwrap().remove("png_base64");
    let hash = Sha256::digest(&png)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if let Some(directory) = std::env::var_os("ORR_GAMEPLAY_API_CAPTURE_DIR") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).expect("prepare explicit local capture directory");
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(format!("{label}.png")))
            .expect("preserve existing captures; use a fresh output directory");
        file.write_all(&png)
            .expect("preserve actual ERP PNG locally");
    }
    eprintln!(
        "API_GPU_CAPTURE {}",
        json!({"label":label,"width":WIDTH,"height":HEIGHT,"sha256":hash,"game":metadata["game"],"tick":metadata["tick"],"epoch":metadata["epoch"],"checksum":metadata["checksum"],"consistency":"paused query/capture/query; not atomic"})
    );
    Capture {
        metadata,
        rgba,
        viewport,
    }
}

#[allow(clippy::float_arithmetic)] // Actual raster coordinates only, never game simulation.
fn color_centroid(capture: &Capture, linear: [f32; 3]) -> (usize, f64) {
    let color: egui::Color32 =
        egui::Rgba::from_rgba_unmultiplied(linear[0], linear[1], linear[2], 1.0).into();
    let want = color.to_array();
    let mut count = 0;
    let mut x_sum = 0u64;
    for (index, pixel) in capture.rgba.chunks_exact(4).enumerate() {
        let x = (index % WIDTH as usize) as u32;
        let y = (index / WIDTH as usize) as u32;
        if capture.viewport.contains(egui::pos2(x as f32, y as f32))
            && (0..3).all(|channel| pixel[channel].abs_diff(want[channel]) <= 4)
        {
            count += 1;
            x_sum += u64::from(x);
        }
    }
    (
        count,
        if count == 0 {
            0.0
        } else {
            x_sum as f64 / count as f64
        },
    )
}

fn project_pixels(alias: &str, capture: &Capture) {
    let marker = match alias {
        "delivery" => [1.0, 0.8, 0.2],
        "switchboard" => [0.3, 0.6, 1.0],
        _ => unreachable!(),
    };
    assert!(
        color_centroid(capture, marker).0 > 20,
        "the actual project marker must be present, not only editor chrome"
    );
}

#[test]
#[allow(clippy::float_arithmetic)] // Actual raster coordinates only, never game simulation.
fn two_projects_api_edit_play_and_capture_actual_wgpu_png_without_cua() {
    for alias in ["delivery", "switchboard"] {
        let (_dir, path) = fixture(alias);
        let source = std::fs::read(&path).unwrap();
        let prepared = PreparedRegisteredProject::open(&path, crate::gameplay::registry()).unwrap();
        let scene_path = prepared.scene_path().to_owned();
        let scene_file = std::fs::read(&scene_path).unwrap();
        let editor = Editor::open_registered_project(prepared).unwrap();
        let mut agent = editor
            .gameplay_api_client(
                "api-gpu-acceptance",
                Caps::parse("read,scene_edit,sim_control").unwrap(),
            )
            .unwrap();
        let mut harness = harness(editor, alias, alias == "delivery");
        let identity = rpc(&mut harness, &mut agent, "rpc.discover", J::Null);
        let original = rpc(&mut harness, &mut agent, "scene.save", J::Null);
        let original_capture = capture_paused(
            &mut harness,
            &mut agent,
            &identity,
            &format!("{alias}-original-edit"),
        );
        project_pixels(alias, &original_capture);
        let patch = match alias {
            "delivery" => json!({"name":"Delivery","path":"goal","value":[0,0]}),
            "switchboard" => json!({"name":"Board","path":"target","value":1}),
            _ => unreachable!(),
        };
        let edited = rpc(&mut harness, &mut agent, "world.singleton.patch", patch);
        assert_eq!(edited["changed"], true);
        let edited_document = rpc(&mut harness, &mut agent, "scene.save", J::Null);
        let edited_history = rpc(&mut harness, &mut agent, "history.list", J::Null);
        let edited_capture = capture_paused(
            &mut harness,
            &mut agent,
            &identity,
            &format!("{alias}-edited"),
        );
        project_pixels(alias, &edited_capture);
        assert!(
            original_capture.rgba != edited_capture.rgba,
            "actual framebuffer must change after the API edit"
        );
        if alias == "delivery" {
            let before = color_centroid(&original_capture, [1.0, 0.8, 0.2]);
            let after = color_centroid(&edited_capture, [1.0, 0.8, 0.2]);
            assert!(
                before.0 > 20 && after.0 > 20,
                "actual goal marker must be visible"
            );
            assert!(
                after.1 < before.1 - 10.0,
                "API goal edit must move the visible goal left"
            );
        }
        let mut request_key = 0;
        let admitted = describe(&mut harness, &mut agent);
        assert!(admitted["intents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|intent| intent == "act"));
        control(&mut harness, &mut agent, &mut request_key, "play");
        wait_description(&mut harness, &mut agent, |state| state["can_claim"] == true);
        control(&mut harness, &mut agent, &mut request_key, "claim");
        wait_description(&mut harness, &mut agent, |state| {
            state["can_submit"] == true
        });
        if alias == "delivery" {
            let active = describe(&mut harness, &mut agent);
            for focused in [false, true, false] {
                harness.input_mut().focused = focused;
                harness.run_steps(2);
                let current = describe(&mut harness, &mut agent);
                assert_eq!(current["phase"], "active");
                assert_eq!(current["can_submit"], true);
                for key in ["binding", "connection", "scope"] {
                    assert_eq!(
                        active[key], current[key],
                        "UI focus must preserve API ownership"
                    );
                }
            }
        }
        let command = mutation(
            &mut harness,
            &mut agent,
            &mut request_key,
            "editor.gameplay.submit",
            json!({"intent":"act","payload":{"press":1}}),
        );
        assert!(command["command_id"].as_str().is_some());
        wait_completed(&mut harness, &mut agent, &command);
        control(&mut harness, &mut agent, &mut request_key, "pause");
        wait_description(&mut harness, &mut agent, |state| {
            state["playing"] == false
                && state["phase"] == "off"
                && state["cleanup_pending"] == false
        });
        let won = rpc(
            &mut harness,
            &mut agent,
            "world.singleton.get",
            json!({"name":if alias == "delivery" {"Delivery"} else {"Board"},"path":if alias == "delivery" {"won"} else {"solved"}}),
        );
        assert_eq!(won["value"], 1);
        let paused = capture_paused(
            &mut harness,
            &mut agent,
            &identity,
            &format!("{alias}-won-paused"),
        );
        project_pixels(alias, &paused);
        assert_eq!(paused.metadata["mode"], "play");
        if alias == "switchboard" {
            assert!(
                color_centroid(&paused, [0.2, 1.0, 0.2]).0 > 20,
                "actual switched-on marker must be green"
            );
        }
        let tick = number(&rpc(&mut harness, &mut agent, "sim.state", J::Null)["head_tick"]);
        control(&mut harness, &mut agent, &mut request_key, "step");
        wait_description(&mut harness, &mut agent, |state| {
            number(&state["tick"]) == tick + 1 && state["playing"] == false
        });
        let stepped = capture_paused(
            &mut harness,
            &mut agent,
            &identity,
            &format!("{alias}-stepped"),
        );
        project_pixels(alias, &stepped);
        assert_eq!(number(&stepped.metadata["tick"]), tick + 1);
        control(&mut harness, &mut agent, &mut request_key, "stop");
        wait_description(&mut harness, &mut agent, |state| {
            state["mode"] == "edit" && state["cleanup_pending"] == false
        });
        assert_eq!(
            rpc(&mut harness, &mut agent, "scene.save", J::Null),
            edited_document
        );
        assert_eq!(
            rpc(&mut harness, &mut agent, "history.list", J::Null),
            edited_history
        );
        rpc(&mut harness, &mut agent, "history.undo", J::Null);
        assert_eq!(
            rpc(&mut harness, &mut agent, "scene.save", J::Null),
            original
        );
        // Direct UI claim and egui fault injection only for human/force-release regressions.
        if alias == "delivery" {
            harness.input_mut().focused = true;
            // This is the human UI baseline, not an AI lifecycle acceptance:
            // Undo can still deliver a late owner fence after a fresh describe.
            // Use the same direct Play path as the human toolbar and claim below;
            // do not retry semantic mutations or weaken their stale-scope guard.
            harness.state_mut().editor.play();
            wait_description(&mut harness, &mut agent, |state| state["can_claim"] == true);
            harness.state_mut().editor.claim_gameplay(0).unwrap();
            wait_description(&mut harness, &mut agent, |state| state["phase"] == "active");
            harness.input_mut().focused = false;
            wait_description(&mut harness, &mut agent, |state| {
                state["phase"] == "off" && state["cleanup_pending"] == false
            });
            control(&mut harness, &mut agent, &mut request_key, "claim");
            wait_description(&mut harness, &mut agent, |state| {
                state["can_submit"] == true
            });
            harness.input_mut().events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            wait_description(&mut harness, &mut agent, |state| {
                state["phase"] == "off" && state["cleanup_pending"] == false
            });
            control(&mut harness, &mut agent, &mut request_key, "stop");
            wait_description(&mut harness, &mut agent, |state| state["mode"] == "edit");
        }
        assert_eq!(std::fs::read(&path).unwrap(), source);
        assert_eq!(std::fs::read(&scene_path).unwrap(), scene_file);
    }
}

/// Explicit binary-dependent acceptance, distinct from the in-process retrying helper.
#[test]
#[ignore = "requires same-candidate ORR_GAMEPLAY_CLI_BIN; run explicitly"]
fn gameplay_cli_two_projects_capture_actual_wgpu_png() {
    use orr_remote::{Auth, ServerConfig, TokenEntry};
    use std::process::{Command, Stdio};
    let binary = PathBuf::from(
        std::env::var_os("ORR_GAMEPLAY_CLI_BIN").expect("explicit fresh CLI binary required"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    for alias in ["delivery", "switchboard"] {
        let (dir, path) = fixture(alias);
        let source = std::fs::read(&path).unwrap();
        let prepared = PreparedRegisteredProject::open(&path, crate::gameplay::registry()).unwrap();
        let scene = prepared.scene_path().to_owned();
        let scene_source = std::fs::read(&scene).unwrap();
        let token = format!(
            "test-only-capture-{}",
            dir.path().file_name().unwrap().to_string_lossy()
        );
        let mut config = ServerConfig::new(Auth::Tokens(vec![TokenEntry {
            token: token.clone(),
            client: "capture-fixture".into(),
            caps: Caps::parse("read").unwrap(),
        }]));
        config.bind = "127.0.0.1:0".parse().unwrap();
        let spec = prepared.host_spec().with_listener(config);
        let mut editor = Editor::start(&spec).unwrap();
        editor.sync();
        let url = editor.erp_status().unwrap().0;
        let address: std::net::SocketAddr = url
            .strip_prefix("ws://")
            .unwrap()
            .trim_end_matches('/')
            .parse()
            .unwrap();
        assert!(address.ip().is_loopback() && address.port() != 0);
        let mut agent = editor
            .gameplay_api_client("capture-observer", Caps::parse("read").unwrap())
            .unwrap();
        let mut harness = harness(editor, alias, false);
        let before = rpc(&mut harness, &mut agent, "scene.save", J::Null);
        let history = rpc(&mut harness, &mut agent, "history.list", J::Null);
        let identity = rpc(
            &mut harness,
            &mut agent,
            "editor.gameplay.describe",
            json!({}),
        );
        let output_path = dir.path().join("capture.png");
        let mut child = Command::new(&binary)
            .args([
                "--erp",
                &url,
                "--json",
                "gameplay",
                "--timeout-ms",
                "10000",
                "capture",
                "--out",
            ])
            .arg(&output_path)
            .env("ORR_ERP_TOKEN", &token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("CLI capture deadline; no retry");
            }
            harness.run_steps(1);
            std::thread::sleep(Duration::from_millis(2));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "capture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata: J = serde_json::from_slice(&output.stdout).unwrap();
        assert!(metadata.get("png_base64").is_none());
        assert!(!String::from_utf8_lossy(&output.stdout).contains(&token));
        assert_eq!(metadata["game"], identity["game"]);
        assert_eq!(metadata["build_id"], identity["build_id"]);
        assert_eq!(metadata["status"], "captured");
        assert_eq!(metadata["local_png_path"], json!(output_path));
        let bytes = std::fs::read(&output_path).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (WIDTH, HEIGHT));
        assert_eq!(reader.info().color_type, png::ColorType::Rgba);
        let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut rgba).unwrap();
        reader.finish().unwrap();
        let capture = Capture {
            metadata,
            rgba,
            viewport: harness.state().ui.viewport_rect.unwrap(),
        };
        project_pixels(alias, &capture);
        assert_eq!(rpc(&mut harness, &mut agent, "scene.save", J::Null), before);
        assert_eq!(
            rpc(&mut harness, &mut agent, "history.list", J::Null),
            history
        );
        assert_eq!(std::fs::read(&path).unwrap(), source);
        assert_eq!(std::fs::read(&scene).unwrap(), scene_source);
        eprintln!(
            "CLI_GPU_CAPTURE {}",
            json!({"project":alias,"sha256":Sha256::digest(&bytes).iter().map(|byte|format!("{byte:02x}")).collect::<String>(),"source":"actual EditorApp offscreen WGPU; no retry; not hardware certification"})
        );
    }
}
/// One real game session, driven only through the immutable CLI and existing ERP.
/// Runtime frame reads are observation oracles, never a state mutation shortcut.
#[cfg(feature = "stormglass")]
#[test]
#[ignore = "requires verified ORR_GAMEPLAY_CLI_BIN and fresh ORR_GAMEPLAY_API_CAPTURE_DIR"]
fn gameplay_cli_stormglass_play_loop_actual_wgpu_png() {
    use orr_games::coop_defense_game::{
        DefenseEnemy, DefenseRun, DefenseUnit, Scenario, SUMMON_COST,
    };
    use orr_remote::{Auth, ServerConfig, TokenEntry};
    use std::{
        io::Write,
        process::{Child, Command, Stdio},
    };

    // Kill/reap our own CLI even if the renderer/assertion unwinds.
    struct OwnedChild(Option<Child>);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            if let Some(child) = self.0.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    fn save(path: &std::path::Path, value: &J) {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        file.write_all(&serde_json::to_vec_pretty(value).unwrap())
            .unwrap();
    }
    fn observed(h: &Harness<'_, EditorApp>) -> J {
        let evidence = h.state().editor.gameplay_frame_evidence().unwrap();
        let frame = evidence.snapshot().predicted();
        let r = frame.singleton::<DefenseRun>();
        let units: Vec<_> = frame
            .dense::<DefenseUnit>()
            .1
            .iter()
            .filter(|u| u.active != 0)
            .map(|u| json!({"board":u.board,"cell":u.cell,"role":u.role,"tier":u.tier}))
            .collect();
        let enemies: Vec<_> = frame
            .dense::<DefenseEnemy>()
            .1
            .iter()
            .filter(|e| e.active != 0)
            .map(|e| json!({"id":e.id,"hp":e.hp,"max_hp":e.max_hp,"progress":e.progress}))
            .collect();
        json!({"tick":r.tick,"generation":r.generation,"wave":r.wave,"reactor_hp":r.reactor_hp,
            "gold":r.gold,"kills":r.kills,"leaks":r.leaks,"units":units,"enemies":enemies})
    }
    fn cli(
        h: &mut Harness<'_, EditorApp>,
        binary: &std::path::Path,
        url: &str,
        token: &str,
        args: &[&str],
    ) -> J {
        let mut child = OwnedChild(Some(
            Command::new(binary)
                .args(["--erp", url, "--json", "gameplay", "--timeout-ms", "10000"])
                .args(args)
                .env("ORR_ERP_TOKEN", token)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ));
        let deadline = Instant::now() + Duration::from_secs(15);
        while child.0.as_mut().unwrap().try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "CLI deadline; no retry");
            h.run_steps(1);
            std::thread::sleep(Duration::from_millis(2));
        }
        let output = child.0.take().unwrap().wait_with_output().unwrap();
        assert!(!String::from_utf8_lossy(&output.stdout).contains(token));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(token));
        assert!(
            output.status.success(),
            "CLI operation failed (raw output withheld)"
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn capture(
        h: &mut Harness<'_, EditorApp>,
        binary: &std::path::Path,
        url: &str,
        token: &str,
        directory: &std::path::Path,
        label: &str,
        identity: &J,
    ) -> J {
        let before = observed(h);
        let path = directory.join(format!("{label}.png"));
        let metadata = cli(
            h,
            binary,
            url,
            token,
            &["capture", "--out", path.to_str().unwrap()],
        );
        assert_eq!(metadata["status"], "captured");
        assert_eq!(metadata["game"], identity["game"]);
        assert_eq!(metadata["build_id"], identity["build_id"]);
        assert_eq!(metadata["paused"], true);
        assert_eq!(observed(h), before, "paused state drift during capture");
        let bytes = std::fs::read(&path).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (WIDTH, HEIGHT));
        assert_eq!(reader.info().color_type, png::ColorType::Rgba);
        let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut rgba).unwrap();
        reader.finish().unwrap();
        assert!(rgba.chunks_exact(4).any(|p| p[..3] != [0, 0, 0]));
        let hash = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        save(
            &directory.join(format!("{label}.json")),
            &json!({"capture":metadata,"observed":before,
            "sha256":hash,"source":"actual Stormglass presenter / EditorApp offscreen WGPU; manually inspect PNG"}),
        );
        before
    }
    let binary = PathBuf::from(std::env::var_os("ORR_GAMEPLAY_CLI_BIN").unwrap());
    assert!(binary.is_absolute() && binary.is_file());
    let directory = PathBuf::from(std::env::var_os("ORR_GAMEPLAY_API_CAPTURE_DIR").unwrap());
    assert!(directory.is_absolute());
    std::fs::create_dir(&directory).expect("fresh external artifact directory required");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project.json");
    let scene = dir.path().join("stormglass.scene.yaml");
    let manifest = include_bytes!("../scenes/stormglass.project.json");
    let source = include_bytes!("../scenes/stormglass.scene.yaml");
    std::fs::write(&path, manifest).unwrap();
    std::fs::write(&scene, source).unwrap();
    let prepared = PreparedRegisteredProject::open(&path, crate::stormglass::registry()).unwrap();
    let token = format!(
        "test-only-loop-{}",
        dir.path().file_name().unwrap().to_string_lossy()
    );
    let mut config = ServerConfig::new(Auth::Tokens(vec![TokenEntry {
        token: token.clone(),
        client: "loop-fixture".into(),
        caps: Caps::parse("read,sim_control").unwrap(),
    }]));
    config.bind = "127.0.0.1:0".parse().unwrap();
    let mut editor = Editor::start(&prepared.host_spec().with_listener(config)).unwrap();
    editor.sync();
    let url = editor.erp_status().unwrap().0;
    let address: std::net::SocketAddr = url
        .strip_prefix("ws://")
        .unwrap()
        .trim_end_matches('/')
        .parse()
        .unwrap();
    assert!(address.ip().is_loopback() && address.port() != 0);
    let mut agent = editor
        .gameplay_api_client("loop-observer", Caps::parse("read").unwrap())
        .unwrap();
    let mut h = harness(editor, "stormglass", false);
    let identity = cli(&mut h, &binary, &url, &token, &["describe"]);
    let history = rpc(&mut h, &mut agent, "history.list", J::Null);
    let saved = rpc(&mut h, &mut agent, "scene.save", J::Null);
    cli(&mut h, &binary, &url, &token, &["control", "play"]);
    let summon = cli(
        &mut h,
        &binary,
        &url,
        &token,
        &[
            "act",
            "act",
            "--payload",
            r#"{"action":1,"source_cell":0,"target_cell":0}"#,
        ],
    );
    save(&directory.join("summon-receipt.json"), &summon);
    assert_eq!(summon["status"], "completed");
    cli(&mut h, &binary, &url, &token, &["control", "pause"]);
    let purchased = capture(
        &mut h,
        &binary,
        &url,
        &token,
        &directory,
        "01-summon",
        &identity,
    );
    let scenario = Scenario::default();
    assert_eq!(
        purchased["units"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|u| u["board"] == 0)
            .count(),
        1
    );
    // This bounded initial observation must precede passive income; otherwise fail,
    // rather than claiming a net balance proves the purchase.
    assert!(purchased["tick"].as_u64().unwrap() < 60);
    assert_eq!(purchased["gold"][0], scenario.starting_gold - SUMMON_COST);
    cli(&mut h, &binary, &url, &token, &["control", "play"]);
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        h.run_steps(1);
        let state = observed(&h);
        if state["kills"].as_u64().unwrap() > 0 && !state["enemies"].as_array().unwrap().is_empty()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no actual combat progress within 45 seconds"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    cli(&mut h, &binary, &url, &token, &["control", "pause"]);
    let combat = capture(
        &mut h,
        &binary,
        &url,
        &token,
        &directory,
        "02-combat",
        &identity,
    );
    assert!(combat["kills"].as_u64().unwrap() > 0);
    assert!(
        !combat["enemies"].as_array().unwrap().is_empty(),
        "combat capture requires visible enemy HP"
    );
    assert!(combat["tick"].as_u64().unwrap() > purchased["tick"].as_u64().unwrap());
    cli(&mut h, &binary, &url, &token, &["control", "play"]);
    let restart = cli(
        &mut h,
        &binary,
        &url,
        &token,
        &[
            "act",
            "act",
            "--payload",
            r#"{"action":5,"source_cell":0,"target_cell":0}"#,
        ],
    );
    save(&directory.join("restart-receipt.json"), &restart);
    assert_eq!(restart["status"], "completed");
    cli(&mut h, &binary, &url, &token, &["control", "pause"]);
    let reset = capture(
        &mut h,
        &binary,
        &url,
        &token,
        &directory,
        "03-restart",
        &identity,
    );
    assert_eq!(
        reset["generation"].as_u64().unwrap(),
        combat["generation"].as_u64().unwrap() + 1
    );
    assert!(
        reset["tick"].as_u64().unwrap() < 45,
        "reset observation missed pre-AI initial interval"
    );
    assert_eq!(
        reset["gold"],
        json!([scenario.starting_gold, scenario.starting_gold])
    );
    assert_eq!(reset["reactor_hp"], scenario.reactor_hp);
    assert_eq!(reset["wave"], 1);
    assert_eq!(reset["kills"], 0);
    assert_eq!(reset["leaks"], 0);
    assert!(reset["units"].as_array().unwrap().is_empty());
    assert!(reset["enemies"].as_array().unwrap().is_empty());
    cli(&mut h, &binary, &url, &token, &["control", "stop"]);
    assert_eq!(rpc(&mut h, &mut agent, "history.list", J::Null), history);
    assert_eq!(rpc(&mut h, &mut agent, "scene.save", J::Null), saved);
    assert_eq!(std::fs::read(&path).unwrap(), manifest);
    assert_eq!(std::fs::read(&scene).unwrap(), source);
    save(
        &directory.join("receipts.json"),
        &json!({"summon":summon,"restart":restart,"limitations":"software/offscreen evidence only; no win/lose, audio, native window, physical GPU or real-model claim"}),
    );
}

/// Layout-only acceptance: no summon, combat or simulation command is replayed.
#[cfg(feature = "stormglass")]
#[test]
#[ignore = "requires actual WGPU and a fresh ORR_GAMEPLAY_API_CAPTURE_DIR"]
#[allow(clippy::float_arithmetic)] // Screen rectangles only.
fn viewport_focus_enlarges_stormglass_and_preserves_registered_projects() {
    use egui_kittest::kittest::Queryable;
    let directory = PathBuf::from(std::env::var_os("ORR_GAMEPLAY_API_CAPTURE_DIR").unwrap());
    assert!(directory.is_absolute());
    std::fs::create_dir(&directory).expect("fresh output directory required");
    for alias in ["stormglass", "delivery"] {
        let (_dir, path) = if alias == "stormglass" {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("project.json");
            std::fs::write(&path, include_bytes!("../scenes/stormglass.project.json")).unwrap();
            std::fs::write(
                dir.path().join("stormglass.scene.yaml"),
                include_bytes!("../scenes/stormglass.scene.yaml"),
            )
            .unwrap();
            (dir, path)
        } else {
            fixture(alias)
        };
        let registry = if alias == "stormglass" {
            crate::stormglass::registry()
        } else {
            crate::gameplay::registry()
        };
        let prepared = PreparedRegisteredProject::open(&path, registry).unwrap();
        let editor = Editor::open_registered_project(prepared).unwrap();
        let mut agent = editor
            .gameplay_api_client("focus-observer", Caps::parse("read").unwrap())
            .unwrap();
        let mut h = harness(editor, alias, true);
        let identity = rpc(&mut h, &mut agent, "rpc.discover", J::Null);
        let document = rpc(&mut h, &mut agent, "scene.save", J::Null);
        let history = rpc(&mut h, &mut agent, "history.list", J::Null);
        let state = rpc(&mut h, &mut agent, "sim.state", J::Null);
        let normal = capture_paused(&mut h, &mut agent, &identity, &format!("{alias}-normal"));
        h.get_by_label("Focus viewport").click();
        h.run_steps(3);
        assert!(h.state().ui.viewport_focus);
        for label in [
            orr_editor::app::LBL_PLAY,
            orr_editor::app::LBL_PAUSE,
            orr_editor::app::LBL_STEP,
            orr_editor::app::LBL_STOP,
        ] {
            assert!(h.query_by_label(label).is_some());
        }
        let focused = capture_paused(&mut h, &mut agent, &identity, &format!("{alias}-focused"));
        assert!(focused.viewport.width() > normal.viewport.width() + 400.0);
        assert!(focused.viewport.height() > normal.viewport.height() + 1.0,
            "hidden bottom panel must return space: {:?} -> {:?}", normal.viewport, focused.viewport);
        if alias == "stormglass" {
            // These two exact sRGB fills are the board tiles, not the HUD. Their
            // rendered area must grow: merely adding a letterbox cannot pass.
            let tile_pixels = |capture: &Capture| {
                capture
                    .rgba
                    .chunks_exact(4)
                    .filter(|p| {
                        [[92u8, 111, 91], [66, 88, 78]]
                            .iter()
                            .any(|color| p[..3].iter().zip(color).all(|(a, b)| a.abs_diff(*b) <= 2))
                    })
                    .count()
            };
            let before = tile_pixels(&normal);
            let after = tile_pixels(&focused);
            assert!(before > 1000, "missing visible board tiles");
            assert!(
                after > before + before / 2,
                "tile pixel area did not grow: {before} -> {after}"
            );
            eprintln!("FOCUS_BOARD_PIXELS {before} -> {after}");
        } else {
            project_pixels(alias, &normal);
            project_pixels(alias, &focused);
        }
        h.get_by_label("Restore panels").click();
        h.run_steps(3);
        assert!(!h.state().ui.viewport_focus);
        assert_eq!(h.state().ui.viewport_rect.unwrap(), normal.viewport);
        assert_paused_anchor(
            &state,
            &rpc(&mut h, &mut agent, "sim.state", J::Null),
            alias,
        );
        assert_eq!(rpc(&mut h, &mut agent, "scene.save", J::Null), document);
        assert_eq!(rpc(&mut h, &mut agent, "history.list", J::Null), history);
    }
}

/// Project HUD only; no gameplay commands or combat replay.
#[cfg(feature = "stormglass")]
#[test]
#[ignore = "requires WGPU, verified ORR_STORMGLASS_HUD_BASELINE PNG, and fresh capture directory"]
fn compact_stormglass_hud_preserves_controls_and_enlarges_board() {
    use egui_kittest::kittest::Queryable;
    let directory = PathBuf::from(std::env::var_os("ORR_GAMEPLAY_API_CAPTURE_DIR").unwrap());
    assert!(directory.is_absolute());
    std::fs::create_dir(&directory).expect("fresh output directory required");
    let baseline_path = PathBuf::from(std::env::var_os("ORR_STORMGLASS_HUD_BASELINE").unwrap());
    assert!(baseline_path.is_absolute());
    let bytes = std::fs::read(&baseline_path).unwrap();
    assert!(bytes.len() <= 4 * 1024 * 1024, "bounded baseline image");
    let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    assert_eq!((reader.info().width, reader.info().height), (WIDTH, HEIGHT));
    assert_eq!(reader.info().color_type, png::ColorType::Rgba);
    let mut baseline = vec![0; reader.output_buffer_size().unwrap()];
    let decoded = reader.next_frame(&mut baseline).unwrap();
    baseline.truncate(decoded.buffer_size());
    let tile_pixels = |rgba: &[u8]| {
        rgba.chunks_exact(4)
            .filter(|p| {
                [[92u8, 111, 91], [66, 88, 78]]
                    .iter()
                    .any(|color| p[..3].iter().zip(color).all(|(a, b)| a.abs_diff(*b) <= 2))
            })
            .count()
    };
    let before = tile_pixels(&baseline);
    assert!(
        before > 1000,
        "baseline must contain rendered Stormglass boards"
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path, include_bytes!("../scenes/stormglass.project.json")).unwrap();
    std::fs::write(
        dir.path().join("stormglass.scene.yaml"),
        include_bytes!("../scenes/stormglass.scene.yaml"),
    )
    .unwrap();
    let prepared = PreparedRegisteredProject::open(&path, crate::stormglass::registry()).unwrap();
    let editor = Editor::open_registered_project(prepared).unwrap();
    let mut agent = editor
        .gameplay_api_client("compact-hud-observer", Caps::parse("read").unwrap())
        .unwrap();
    let mut h = harness(editor, "stormglass", true);
    let identity = rpc(&mut h, &mut agent, "rpc.discover", J::Null);
    let document = rpc(&mut h, &mut agent, "scene.save", J::Null);
    let history = rpc(&mut h, &mut agent, "history.list", J::Null);
    let state = rpc(&mut h, &mut agent, "sim.state", J::Null);
    h.get_by_label("Focus viewport").click();
    h.run_steps(3);
    let compact = capture_paused(&mut h, &mut agent, &identity, "stormglass-compact");
    let after = tile_pixels(&compact.rgba);
    assert!(
        after > before + before / 10,
        "compact board area did not improve: {before} -> {after}"
    );
    for _ in 0..2 {
        h.get_by_label("Details").click();
        h.run_steps(3);
        for label in [
            "Source cell",
            "Target cell",
            "Summon",
            "Move / merge",
            "Upgrade",
            "Invest",
            "Restart",
            "Restore panels",
        ] {
            assert!(h.query_by_label(label).is_some(), "missing control {label}");
        }
        h.get_by_label("Details").click();
        h.run_steps(3);
    }
    let restored = capture_paused(&mut h, &mut agent, &identity, "stormglass-compact-restored");
    assert_eq!(
        tile_pixels(&restored.rgba),
        after,
        "closing details must restore board area"
    );
    h.get_by_label("Details").click();
    h.run_steps(3);
    let expanded = capture_paused(&mut h, &mut agent, &identity, "stormglass-details");
    assert!(
        tile_pixels(&expanded.rgba) < after,
        "bounded help must occupy visible space"
    );
    assert_paused_anchor(
        &state,
        &rpc(&mut h, &mut agent, "sim.state", J::Null),
        "stormglass",
    );
    assert_eq!(rpc(&mut h, &mut agent, "scene.save", J::Null), document);
    assert_eq!(rpc(&mut h, &mut agent, "history.list", J::Null), history);
    eprintln!("COMPACT_HUD_BOARD_PIXELS {before} -> {after}");
    let comparison = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("hud-comparison.json"))
        .unwrap();
    serde_json::to_writer_pretty(comparison, &json!({
            "baseline_sha256": Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
            "baseline_tile_pixels": before, "compact_tile_pixels": after,
            "limitations": "verified prior focused edit-state framebuffer comparison; software/offscreen only; no gameplay command replay"
        }),
    ).unwrap();
}
