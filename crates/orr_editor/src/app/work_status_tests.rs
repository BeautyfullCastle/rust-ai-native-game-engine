//! Actual EditorApp observations through the paired ERP host. No native window
//! or GPU is claimed by this CPU harness. The explicit subprocess acceptance
//! requires a prebuilt MCP binary and is mandatory in the focused CI gate.
use super::*;
use egui_kittest::{kittest::Queryable, Harness};
use orr_remote::work_status::{DomainId, EditorSnapshot, Operations};
use serde_json::{json, Value as J};
use std::{fs, path::Path, time::Duration};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "orr-work-status-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn scene(&self) -> PathBuf {
        let path = self.0.join("SECRET-scene-title.yaml");
        fs::write(
            &path,
            include_bytes!("../../../../scenes/physics_demo.scene.yaml"),
        )
        .unwrap();
        path
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn harness(editor: Editor) -> Harness<'static, EditorApp> {
    let mut h = Harness::builder()
        .with_size([1500.0, 1000.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| EditorApp::new(editor, None));
    h.state_mut().editor.sync();
    h.run_steps(3);
    h
}
fn status(h: &mut Harness<'_, EditorApp>) -> J {
    h.state_mut()
        .editor
        .host_call("editor.work_status", json!({"max_age_ms":5000}))
        .unwrap()
}
fn row<'a>(status: &'a J, id: &str) -> &'a J {
    status["editor_local"]["domains"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["domain_id"] == id)
        .unwrap()
}
fn clean_assessment() -> &'static str {
    if cfg!(feature = "mesh-navigation") {
        "indeterminate"
    } else {
        "none_observed"
    }
}
fn assert_no_window_actions(h: &Harness<'_, EditorApp>) {
    assert!(!h
        .output()
        .viewport_output
        .values()
        .flat_map(|v| &v.commands)
        .any(|v| matches!(
            v,
            egui::ViewportCommand::Close | egui::ViewportCommand::CancelClose
        )));
}

#[test]
fn actual_app_work_status_requires_first_frame_and_has_fixed_inventory() {
    let dir = Directory::new();
    let mut editor = Editor::open(&dir.scene()).unwrap();
    let before = editor.host_call("editor.work_status", json!({})).unwrap();
    assert_eq!(before["availability"], "not_ready");
    assert_eq!(before["assessment"]["state"], "indeterminate");
    let mut h = harness(editor);
    let ready = status(&mut h);
    assert_eq!(ready["schema_version"], 1);
    assert_eq!(ready["assessment"]["state"], clean_assessment());
    assert_eq!(
        ready["editor_local"]["domains"].as_array().unwrap().len(),
        16
    );
    for id in DomainId::ALL {
        let r = row(&ready, id.as_str());
        assert!(r["domain_revision"].is_null());
    }
    assert!(!ready.to_string().contains("SECRET-scene-title"));
    assert!(!h.state().ui.close_review.blocked);
    assert_no_window_actions(&h);
}

#[test]
fn actual_app_work_status_poll_is_passive_and_does_not_refresh_snapshot() {
    let dir = Directory::new();
    let scene = dir.scene();
    let mut h = harness(Editor::open(&scene).unwrap());
    h.get_by_label("Timeline").focus();
    h.run_steps(2);
    let focus = h.ctx.memory(|m| m.focused());
    let rect = h.state().ui.viewport_rect;
    let tab = h.state().ui.bottom_tab;
    let bytes = fs::read(scene).unwrap();
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let first = status(&mut h);
    for _ in 0..12 {
        let next = status(&mut h);
        assert_eq!(
            next["observation"]["snapshot_seq"],
            first["observation"]["snapshot_seq"]
        );
        assert_eq!(
            next["observation"]["ui_frame"],
            first["observation"]["ui_frame"]
        );
        assert!(
            next["observation"]["age_ms"].as_u64().unwrap()
                >= first["observation"]["age_ms"].as_u64().unwrap()
        );
        assert_eq!(next["host_scene"], first["host_scene"]);
    }
    assert_eq!(h.ctx.memory(|m| m.focused()), focus);
    assert_eq!(h.state().ui.viewport_rect, rect);
    assert!(h.state().ui.bottom_tab == tab);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(fs::read(h.state().editor.path().unwrap()).unwrap(), bytes);
    assert_no_window_actions(&h);
}

#[test]
fn actual_app_work_status_current_host_is_not_replaced_by_older_editor_frame() {
    let dir = Directory::new();
    let scene = dir.scene();
    let mut h = harness(Editor::open(&scene).unwrap());
    let before = status(&mut h);
    assert!(h.state_mut().editor.select_named("body_05"));
    assert!(h.state_mut().editor.set_field(
        &Owner::Component("orr_physics::Body".into()),
        "pos.x",
        Value::Fixed(FP::from_int(2))
    ));
    // Deliberately no App callback/publication after the host mutation.
    let dirty = status(&mut h);
    assert_eq!(dirty["host_scene"]["dirty"], true);
    assert_eq!(dirty["assessment"]["state"], "known_blockers");
    assert_eq!(
        dirty["observation"]["snapshot_seq"],
        before["observation"]["snapshot_seq"]
    );
    assert_eq!(
        dirty["consistency"],
        "host_at_request_local_at_published_frame"
    );
    let revision = dirty["host_scene"]["preview_revision"].clone();
    assert!(h.state_mut().editor.save());
    let saved = status(&mut h);
    assert_eq!(saved["host_scene"]["dirty"], false);
    assert_eq!(
        saved["host_scene"]["preview_revision"], revision,
        "save is not a monotonic content revision"
    );
    assert_eq!(saved["assessment"]["state"], clean_assessment());
    assert!(!h.state().ui.close_review.blocked);
}

#[test]
fn actual_app_work_status_one_event_callback_and_early_return_publish() {
    let dir = Directory::new();
    let mut h = harness(Editor::open(&dir.scene()).unwrap());
    let before = status(&mut h);
    h.get_by_label("+ Body").click_accesskit();
    h.run_steps(1);
    let after = status(&mut h);
    assert_ne!(
        after["observation"]["snapshot_seq"],
        before["observation"]["snapshot_seq"]
    );
    assert_eq!(after["host_scene"]["dirty"], true);
    assert_eq!(after["assessment"]["state"], "known_blockers");
    // The same production final wrapper runs even when its draw callback exits.
    let app = h.state_mut();
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.with_close_review(ui, |_, _| {});
    });
    let final_status = app
        .editor
        .host_call("editor.work_status", json!({"max_age_ms":5000}))
        .unwrap();
    assert_ne!(
        final_status["observation"]["snapshot_seq"],
        after["observation"]["snapshot_seq"]
    );
}

#[test]
fn actual_app_work_status_restart_changes_binding_and_requires_new_publication() {
    let dir = Directory::new();
    let mut h = harness(Editor::open(&dir.scene()).unwrap());
    let before = status(&mut h);
    assert!(h.state_mut().editor.restart());
    let waiting = status(&mut h);
    assert_eq!(waiting["availability"], "not_ready");
    h.run_steps(1);
    let after = status(&mut h);
    assert_ne!(
        before["observation"]["owner_binding_id"],
        after["observation"]["owner_binding_id"]
    );
    assert_eq!(after["observation"]["snapshot_seq"], "1");
}

#[test]
fn actual_app_work_status_unknown_features_are_not_silently_clean() {
    let dir = Directory::new();
    let mut h = harness(Editor::open(&dir.scene()).unwrap());
    let report = status(&mut h);
    if cfg!(feature = "mesh-navigation") {
        assert_eq!(report["availability"], "incomplete");
        assert_eq!(row(&report, "mesh_geometry_art")["presence"], "unknown");
        assert_eq!(report["assessment"]["state"], "indeterminate");
    } else {
        assert_eq!(
            row(&report, "mesh_geometry_art")["presence"],
            "feature_disabled"
        );
        assert_eq!(
            row(&report, "mesh_geometry_art")["work_state"],
            "not_applicable"
        );
    }
}

#[cfg(feature = "sprites")]
#[test]
fn actual_app_work_status_sprite_sidecar_survives_scene_save_and_blocked_closes() {
    let dir = Directory::new();
    let scene = dir.scene();
    let mut h = harness(Editor::open(&scene).unwrap());
    let guid = h.state().editor.rows()[0].guid.clone().unwrap();
    let path = dir.0.join("SECRET-bindings.json");
    h.state_mut()
        .sprites
        .create(
            path.clone(),
            scene.file_name().unwrap().to_str().unwrap().into(),
            ".".into(),
        )
        .unwrap();
    let bindings = h.state_mut().sprites.bindings.as_mut().unwrap();
    bindings.save().unwrap();
    bindings.set_camera_follow(Some(guid)).unwrap();
    let bytes = fs::read(&path).unwrap();
    h.run_steps(1);
    let dirty = status(&mut h);
    assert_eq!(dirty["host_scene"]["dirty"], false);
    assert_eq!(row(&dirty, "sprite_bindings")["work_state"], "unsaved");
    assert_eq!(dirty["assessment"]["state"], "known_blockers");
    assert!(h
        .state()
        .close_review_items()
        .iter()
        .any(|item| item.name == "Sprite bindings · unsaved"));
    assert!(h.state_mut().editor.save());
    for _ in 0..3 {
        assert!(super::unsaved_close_tests::request_root_close(&mut h));
        let report = status(&mut h);
        assert_eq!(row(&report, "sprite_bindings")["work_state"], "unsaved");
        assert!(
            h.state().ui.close_review.blocked,
            "poll does not clear close status"
        );
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(!dirty.to_string().contains("SECRET"));
    h.state_mut().sprites.bindings.as_mut().unwrap().undo();
    h.run_steps(1);
    assert_eq!(
        row(&status(&mut h), "sprite_bindings")["work_state"],
        "unknown"
    );
    assert_eq!(status(&mut h)["availability"], "incomplete");
}

#[cfg(feature = "collect-ui")]
#[test]
fn actual_app_work_status_collect_document_and_preview_are_independent() {
    use orr_sample::{
        authored_ui::Document,
        collect_project::{PreparedProject, ProgressSupport, SpriteSupport},
    };

    let root = tempfile::tempdir().unwrap();
    let scene = root.path().join("scene.yaml");
    let ui_path = root.path().join("ui.json");
    fs::write(
        &scene,
        include_bytes!("../../../../scenes/collect_dodge_v1.scene.yaml"),
    )
    .unwrap();
    fs::write(&ui_path, Document::default_collect().to_bytes().unwrap()).unwrap();
    fs::write(
        root.path().join("orr.project.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": 2, "engine": "*",
            "entry": {
                "game": "collect-dodge-v1", "scene": "scene.yaml",
                "ui": {
                    "profile": "collect-authored-v1", "document": "ui.json",
                    "font": {"package": "korean-game-ui", "asset": "OrreryKoreanUI.otf"}
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    orr_package::Project::open(root.path(), orr_package::Runtime::content_only())
        .unwrap()
        .install(&[PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/game_ui_font")
            .canonicalize()
            .unwrap()])
        .unwrap();
    let project = PreparedProject::open_with_ui(
        root.path(),
        ProgressSupport::Unsupported,
        SpriteSupport::Unsupported,
        true,
    )
    .unwrap();
    let mut editor = Editor::start(&crate::HostSpec::PreparedCollect {
        scene: project.path().to_owned(),
        text: project.scene().text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    let mut h = harness(editor);
    let context = h.ctx.clone();
    crate::project::install_collect_presentation(h.state_mut(), project, &context);
    let mut document = h.state().collect_ui.as_ref().unwrap().document().clone();
    document.nodes[0].offset[0] += 1;
    h.state_mut()
        .collect_ui
        .as_mut()
        .unwrap()
        .apply_document(document)
        .unwrap();
    let before = fs::read(&ui_path).unwrap();
    h.state_mut().stage_close_test_position();
    h.run_steps(1);
    let report = status(&mut h);
    assert_eq!(report["host_scene"]["dirty"], false);
    assert_eq!(row(&report, "collect_ui_document")["work_state"], "unsaved");
    assert_eq!(
        row(&report, "collect_ui_preview")["work_state"],
        "staged_or_unfinished"
    );
    assert_eq!(report["assessment"]["state"], "known_blockers");
    assert!(h.state_mut().editor.save());
    assert_eq!(
        row(&status(&mut h), "collect_ui_document")["work_state"],
        "unsaved"
    );
    assert_eq!(fs::read(&ui_path).unwrap(), before);
    assert!(!h.state().ui.close_review.blocked);
    assert_no_window_actions(&h);
}

#[test]
fn actual_app_work_status_observation_never_invokes_close_only_predicates() {
    use super::work_status::Inspection;
    use orr_remote::work_status::{Presence, WorkState};
    for memory in [false, true] {
        let observed = Inspection::Observation.sidecar(
            DomainId::SpriteBindings,
            || memory,
            || panic!("passive observation must never probe the filesystem"),
        );
        assert_eq!(observed.presence, Presence::Loaded);
        assert_eq!(
            observed.work_state,
            if memory {
                WorkState::Unsaved
            } else {
                WorkState::Unknown
            }
        );
    }
    for blocked in [false, true] {
        let closing = Inspection::CloseRequest.sidecar(
            DomainId::SpriteBindings,
            || panic!("explicit close preserves the original complete predicate"),
            || blocked,
        );
        assert_eq!(closing.is_blocker(), blocked);
    }
}

#[cfg(feature = "sprites")]
#[test]
fn actual_app_work_status_missing_file_close_observations_remain_historical() {
    let dir = Directory::new();
    let scene = dir.scene();
    let mut h = harness(Editor::open(&scene).unwrap());
    let path = dir.0.join("SECRET-retained-bindings.json");
    h.state_mut()
        .sprites
        .create(
            path.clone(),
            scene.file_name().unwrap().to_str().unwrap().into(),
            ".".into(),
        )
        .unwrap();
    h.run_steps(1);
    // Existing constructors use equal current/saved memory even before Save.
    assert_eq!(
        row(&status(&mut h), "sprite_bindings")["work_state"],
        "unknown"
    );
    assert!(super::unsaved_close_tests::request_root_close(&mut h));
    assert!(h
        .state()
        .close_review_items()
        .iter()
        .any(|item| item.last_close));
    h.state_mut()
        .sprites
        .bindings
        .as_mut()
        .unwrap()
        .save()
        .unwrap();
    let bytes = fs::read(&path).unwrap();
    h.run_steps(1);
    assert_eq!(status(&mut h)["availability"], "incomplete");
    assert!(!super::unsaved_close_tests::request_root_close(&mut h));
    assert!(h.state().close_review_items().is_empty());
    fs::remove_file(&path).unwrap();
    h.state_mut().ui.bottom_tab = BottomTab::UnsavedWork;
    h.run_steps(2);
    assert_no_window_actions(&h);
    let unknown = status(&mut h);
    assert_eq!(row(&unknown, "sprite_bindings")["presence"], "loaded");
    assert_eq!(row(&unknown, "sprite_bindings")["work_state"], "unknown");
    assert_eq!(unknown["assessment"]["state"], "indeterminate");
    assert!(super::unsaved_close_tests::request_root_close(&mut h));
    assert!(h
        .state()
        .close_review_items()
        .iter()
        .all(|item| item.last_close));
    // A close result is not published as a new positive machine observation.
    assert_eq!(status(&mut h)["assessment"]["state"], "indeterminate");
    h.run_steps(2);
    h.get_by_label("Clear close status").click();
    h.run_steps(2);
    assert!(h.state().close_review_items().is_empty());
    assert!(!h.state().ui.close_review.blocked);
    assert_eq!(status(&mut h)["assessment"]["state"], "indeterminate");
    assert!(super::unsaved_close_tests::request_root_close(&mut h));
    fs::write(&path, bytes).unwrap();
    assert!(!super::unsaved_close_tests::request_root_close(&mut h));
    assert!(h.state().close_review_items().is_empty());
}

#[cfg(all(feature = "tilemap", feature = "project-create", target_os = "linux"))]
#[test]
fn actual_app_work_status_loaded_tilemap_is_unknown_without_frame_serialization() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tiles");
    orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "arena-tilemap-2d-v1".into(),
        seed: "work-status".into(),
    })
    .unwrap();
    let project = crate::project::PreparedProject::open(&root).unwrap();
    let mut h = Harness::builder()
        .with_size([1500.0, 1300.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |cc| {
            let mut editor = Editor::start(&project.host_spec()).unwrap();
            editor.sync();
            project.into_app(editor, None, &cc.egui_ctx).unwrap()
        });
    h.run_steps(3);
    assert!(!h.state().tilemap.as_ref().unwrap().dirty());
    let before = h.state().tilemap.as_ref().unwrap().history_snapshot();
    let report = status(&mut h);
    assert_eq!(row(&report, "tilemap")["presence"], "loaded");
    assert_eq!(row(&report, "tilemap")["work_state"], "unknown");
    assert_eq!(report["availability"], "incomplete");
    assert_eq!(report["assessment"]["state"], "indeterminate");
    assert!(!super::unsaved_close_tests::request_root_close(&mut h));
    assert_eq!(
        h.state().tilemap.as_ref().unwrap().history_snapshot(),
        before
    );
    assert_eq!(row(&status(&mut h), "tilemap")["work_state"], "unknown");
}

struct McpProcess {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    responses: std::sync::mpsc::Receiver<J>,
    serial: u64,
}
impl McpProcess {
    fn start(binary: &Path, url: &str) -> Self {
        use std::io::BufRead;
        let mut child = std::process::Command::new(binary)
            .args(["--erp", url, "--tools", "scene", "--timeout", "3"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, responses) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str(&line) else {
                    break;
                };
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            responses,
            serial: 0,
        }
    }
    fn request(&mut self, method: &str, params: J) -> J {
        self.serial += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":self.serial,"method":method,"params":params})
        )
        .unwrap();
        self.stdin.flush().unwrap();
        let reply = self
            .responses
            .recv_timeout(Duration::from_secs(5))
            .expect("bounded MCP response");
        assert_eq!(reply["id"], self.serial);
        assert!(reply.get("error").is_none(), "{reply}");
        reply["result"].clone()
    }
    fn status(&mut self, age: u64) -> J {
        let reply = self.request(
            "tools/call",
            json!({"name":"editor_work_status","arguments":{"max_age_ms":age}}),
        );
        assert_eq!(reply["isError"], false, "{reply}");
        assert!(reply["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not permission to close"));
        reply["structuredContent"].clone()
    }
}
impl Drop for McpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "required focused gate supplies ORR_WORK_STATUS_MCP_BIN; no silent fallback"]
fn actual_app_work_status_through_real_erp_and_mcp_all_versions() {
    let binary = PathBuf::from(
        std::env::var_os("ORR_WORK_STATUS_MCP_BIN")
            .expect("build orr_mcp first and set ORR_WORK_STATUS_MCP_BIN"),
    );
    assert!(binary.is_file(), "the exact built MCP binary must exist");
    for version in ["2024-11-05", "2025-03-26", "2025-06-18"] {
        let dir = Directory::new();
        let scene = dir.scene();
        let spec = crate::HostSpec::Local {
            scene: scene.clone(),
            listen: Some(orr_remote::ServerConfig::new(orr_remote::Auth::DevNoAuth)),
            debug_hooks: false,
        };
        let editor = Editor::start(&spec).unwrap();
        let url = editor.erp_status().unwrap().0;
        let mut mcp = McpProcess::start(&binary, &url);
        assert_eq!(mcp.request("initialize",json!({"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"work-status-test","version":"1"}}))["protocolVersion"],version);
        assert_eq!(mcp.status(5000)["availability"], "not_ready");
        let mut h = harness(editor);
        assert_eq!(mcp.status(5000)["assessment"]["state"], clean_assessment());
        h.get_by_label("+ Body").click_accesskit();
        h.run_steps(1);
        assert_eq!(mcp.status(5000)["assessment"]["state"], "known_blockers");
        assert!(h.state_mut().editor.undo());
        h.run_steps(1);
        assert_eq!(mcp.status(5000)["assessment"]["state"], clean_assessment());
        std::thread::sleep(Duration::from_millis(5));
        let stale = mcp.status(1);
        assert_eq!(stale["availability"], "stale");
        assert_eq!(stale["assessment"]["state"], "indeterminate");
        h.state_mut().editor.publish_work_status(EditorSnapshot {
            domains: Vec::new(),
            operations: Operations::Idle,
        });
        assert_eq!(mcp.status(5000)["availability"], "incomplete");
        h.state_mut().editor.retire_work_status_owner_for_test();
        let absent = mcp.status(5000);
        assert_eq!(absent["availability"], "owner_absent");
        assert_eq!(absent["assessment"]["state"], "indeterminate");
        assert!(!absent.to_string().contains("SECRET"));
    }
}
