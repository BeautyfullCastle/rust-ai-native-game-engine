//! Collect-only duplication acceptance; the existing six panel unit tests stay unchanged.
#![cfg(feature = "collect-ui")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::collect_ui_panel::Panel;
use orr_sample::{
    authored_ui::{Action, Binding, Document, Kind, Node, Screen, MAX_NODES},
    collect_project::PreparedCollectUi,
};
use std::{collections::BTreeSet, fs};

fn node(id: &str, parent: Option<&str>, kind: Kind) -> Node {
    Node {
        id: id.into(), parent: parent.map(str::to_owned), kind,
        screen: Screen::Playing, anchor: [0, 0], offset: [12, 12], size: [160, 36],
    }
}
fn label(text: &str) -> Kind { Kind::Label { text: text.into(), binding: Some(Binding::Score) } }
fn tree() -> Document {
    Document { schema: 1, nodes: vec![
        node("external", None, Kind::Container),
        node("abcdefghijklmnopqrstuvwxyz123456", Some("external"), Kind::Container),
        node("unrelated", None, label("Unrelated")),
        node("nested", Some("abcdefghijklmnopqrstuvwxyz123456"), Kind::Container),
        node("ui_copy_1", None, label("Collision")),
        node("leaf", Some("nested"), label("Score: ")),
        node("button", Some("abcdefghijklmnopqrstuvwxyz123456"), Kind::Button { text: "Restart".into(), action: Action::Restart }),
    ] }
}
struct State { panel: Panel, editable: bool }
fn harness(document: Document) -> (tempfile::TempDir, Harness<'static, State>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("collect.ui.json");
    // Also permits deliberately malformed inputs to check defensive atomicity.
    fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    let font = include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").to_vec();
    let mut h = Harness::builder().with_size([1100.0, 1400.0]).build_ui_state(
        |ui, s: &mut State| s.panel.show(ui, s.editable),
        State { panel: Panel::new(PreparedCollectUi { path, document, font }), editable: true },
    );
    h.run_steps(3);
    click(&mut h, "Collect UI document");
    (dir, h)
}
fn click(h: &mut Harness<'_, State>, label: &str) {
    h.get_by_label(label).click(); h.run_steps(3);
}
fn select(h: &mut Harness<'_, State>, id: &str) {
    h.get_by_role_and_label(Role::ComboBox, "UI node").click(); h.run_steps(3);
    click(h, id);
}
const DUPLICATE: &str = "Duplicate node and descendants";

#[test]
fn widgets_duplicate_noncontiguous_subtree_preserve_fields_and_single_undo() {
    let original = tree();
    original.validate().unwrap();
    let (dir, mut h) = harness(original.clone());
    let disk = fs::read(dir.path().join("collect.ui.json")).unwrap();
    select(&mut h, &original.nodes[1].id);
    click(&mut h, DUPLICATE);
    let copy = h.state().panel.document().clone();
    assert_eq!(&copy.nodes[..7], &original.nodes);
    assert_eq!(copy.nodes.len(), 11);
    let mapping = [(1, "ui_copy_2"), (3, "ui_copy_3"), (5, "ui_copy_4"), (6, "ui_copy_5")];
    for (offset, (source, id)) in mapping.iter().enumerate() {
        let mut expected = original.nodes[*source].clone();
        expected.id = (*id).into();
        expected.parent = match *source {
            1 => Some("external".into()),
            3 | 6 => Some("ui_copy_2".into()),
            5 => Some("ui_copy_3".into()),
            _ => unreachable!(),
        };
        assert_eq!(copy.nodes[7 + offset], expected);
    }
    h.get_by_label("Stable id: ui_copy_2");
    copy.to_bytes().unwrap();
    assert_eq!(fs::read(dir.path().join("collect.ui.json")).unwrap(), disk, "no autosave");
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().panel.document(), &original);
    assert!(!h.state_mut().panel.undo(), "one duplicate is exactly one undo");
}

#[test]
fn widgets_repeated_duplicates_are_independent_and_reopen_exactly() {
    let original = Document { schema: 1, nodes: vec![node("ui_copy_1", None, label("Original"))] };
    let (dir, mut h) = harness(original.clone());
    select(&mut h, "ui_copy_1");
    click(&mut h, DUPLICATE);
    h.get_by_label("Stable id: ui_copy_2");
    h.get_by_role(Role::TextInput).focus(); h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A); h.run_steps(2);
    h.get_by_role(Role::TextInput).type_text("Independent"); h.run_steps(3);
    assert_eq!(h.state().panel.document().nodes[0], original.nodes[0]);
    assert!(matches!(&h.state().panel.document().nodes[1].kind, Kind::Label { text, .. } if text == "Independent"));
    click(&mut h, DUPLICATE);
    h.get_by_label("Stable id: ui_copy_3");
    let final_doc = h.state().panel.document().clone();
    assert_eq!(final_doc.nodes.iter().map(|n| &n.id).collect::<BTreeSet<_>>().len(), 3);
    click(&mut h, "Save UI document");
    let reopened = Document::parse(&fs::read(dir.path().join("collect.ui.json")).unwrap()).unwrap();
    assert_eq!(reopened, final_doc);
    let (_again, h2) = harness(reopened);
    assert_eq!(h2.state().panel.document(), &final_doc);
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().panel.document().nodes.len(), 2);
    assert_eq!(fs::read(dir.path().join("collect.ui.json")).unwrap(), final_doc.to_bytes().unwrap());
}

#[test]
fn widgets_copy_root_selection_parents_new_child_to_copy() {
    let original = tree();
    let (_dir, mut h) = harness(original.clone());
    select(&mut h, &original.nodes[1].id);
    click(&mut h, DUPLICATE);
    click(&mut h, "Add label");
    assert_eq!(h.state().panel.document().nodes.last().unwrap().parent.as_deref(), Some("ui_copy_2"));
    click(&mut h, "Undo UI edit");
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().panel.document(), &original);
}

#[test]
fn widgets_no_selection_and_play_are_readonly() {
    let original = tree();
    let (dir, mut h) = harness(original.clone());
    click(&mut h, DUPLICATE);
    assert_eq!(h.state().panel.document(), &original);
    assert!(!h.state_mut().panel.undo());
    select(&mut h, &original.nodes[1].id);
    h.state_mut().editable = false; h.run_steps(3);
    click(&mut h, DUPLICATE);
    assert_eq!(h.state().panel.document(), &original);
    assert!(!h.state_mut().panel.undo());
    assert_eq!(Document::parse(&fs::read(dir.path().join("collect.ui.json")).unwrap()).unwrap(), original);
}

#[test]
fn widgets_node_limit_failure_preserves_document_selection_and_history() {
    let mut original = Document { schema: 1, nodes: vec![node("root", None, Kind::Container)] };
    for i in 1..MAX_NODES - 1 { original.nodes.push(node(&format!("child_{i}"), Some("root"), label("Child"))); }
    let (dir, mut h) = harness(original.clone());
    select(&mut h, "child_1"); click(&mut h, DUPLICATE);
    let full = h.state().panel.document().clone();
    assert_eq!(full.nodes.len(), MAX_NODES);
    select(&mut h, "root"); click(&mut h, DUPLICATE);
    assert_eq!(h.state().panel.document(), &full);
    h.get_by_label("Stable id: root");
    h.get_by_label("Duplicating this subtree would exceed 32 UI nodes");
    assert_eq!(Document::parse(&fs::read(dir.path().join("collect.ui.json")).unwrap()).unwrap(), original);
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().panel.document(), &original, "failed copy did not push history");
    assert!(!h.state_mut().panel.undo());
}

#[test]
fn widgets_invalid_depth_geometry_font_and_byte_candidates_fail_atomically() {
    let mut depth = tree();
    depth.nodes.push(node("too_deep", Some("nested"), Kind::Container));
    depth.nodes.push(node("depth_five", Some("too_deep"), label("Deep")));
    let mut geometry = tree(); geometry.nodes[0].size[0] = 0;
    let mut font = tree(); font.nodes[2].kind = label("🦄");
    let mut bytes = tree(); bytes.nodes[2].kind = label(&"A".repeat(70_000));
    assert!(serde_json::to_vec(&bytes).unwrap().len() > 64 * 1024);
    for original in [depth, geometry, font, bytes] {
        let (dir, mut h) = harness(original.clone());
        let before = fs::read(dir.path().join("collect.ui.json")).unwrap();
        select(&mut h, &original.nodes[1].id);
        click(&mut h, DUPLICATE);
        assert_eq!(h.state().panel.document(), &original);
        h.get_by_label(&format!("Stable id: {}", original.nodes[1].id));
        assert!(!h.state_mut().panel.undo());
        assert_eq!(fs::read(dir.path().join("collect.ui.json")).unwrap(), before);
    }
}

#[test]
fn widgets_leaf_at_depth_four_and_every_screen_preserve_external_parent() {
    for screen in [Screen::Title, Screen::Playing, Screen::Menu, Screen::Terminal] {
        let mut original = tree();
        for node in &mut original.nodes { node.screen = screen; }
        let (_dir, mut h) = harness(original.clone());
        select(&mut h, "leaf"); click(&mut h, DUPLICATE);
        let mut expected = original.nodes[5].clone(); expected.id = "ui_copy_2".into();
        assert_eq!(h.state().panel.document().nodes.last(), Some(&expected));
        h.state().panel.document().validate().unwrap();
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
#[ignore = "mandatory explicit software GPU, trusted binaries and source-hidden read-only export"]
fn gpu_saved_duplicate_source_hidden_readonly_export_preserves_frames() {
    use orr_sample::collect_project::{PreparedProject, ProgressSupport, SpriteSupport};
    use std::{collections::BTreeMap, path::{Path, PathBuf}, process::{Command, Output}};
    fn good(out: Output) -> String {
        assert!(out.status.success(), "{}\n{}\n{}", out.status, String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }
    fn image(path: &Path) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(fs::File::open(path).unwrap()).read_info().unwrap();
        let mut bytes = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut bytes).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        bytes.truncate(info.buffer_size());
        (info.width, info.height, bytes)
    }
    fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, p: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(p).unwrap() {
                let entry = entry.unwrap(); let kind = entry.file_type().unwrap();
                assert!(!kind.is_symlink());
                if kind.is_dir() { visit(root, &entry.path(), out); }
                else { assert!(kind.is_file()); out.insert(entry.path().strip_prefix(root).unwrap().to_owned(), fs::read(entry.path()).unwrap()); }
            }
        }
        let mut out = BTreeMap::new(); visit(root, root, &mut out); out
    }
    fn open(root: &Path) -> PreparedProject {
        PreparedProject::open_with_ui(root, ProgressSupport::MetadataOnly, SpriteSupport::Supported, true).unwrap()
    }
    fn frame(root: &Path, ticks: u32, right: bool) -> (Vec<u8>, u64) {
        let p = open(root);
        let mut simulation = p.scene().simulation().unwrap();
        for tick in 1..=u64::from(ticks) {
            let mut inputs = orr_sim::TickInputs::new(tick, 1);
            inputs.set_input(orr_bridge::PlayerSlot(0), orr_sample::collect_game::CollectInput {
                x: if right { orr_fp::FP::ONE } else { orr_fp::FP::ZERO }, ..Default::default()
            });
            simulation.step(&inputs);
        }
        (simulation.frame().to_bytes(), simulation.checksum())
    }
    let work = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let base = work.path();
    let evidence = PathBuf::from(std::env::var_os("ORR_UI_DUPLICATE_CAPTURES").expect("mandatory retained evidence directory"));
    fs::create_dir_all(&evidence).unwrap();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap();
    assert!(!base.starts_with(&workspace));
    let tools = base.join("tools"); let captures = base.join("captures"); let empty = base.join("empty cwd");
    for path in [&tools, &captures, &empty] { fs::create_dir(path).unwrap(); }
    fs::create_dir(base.join("user-home")).unwrap();
    fs::create_dir(base.join("user-data")).unwrap();
    let mut bins = Vec::new();
    for (key, name) in [("ORR_NEW_ARENA_BIN", "orr_new_arena"), ("ORR_COLLECT_RUNTIME", "collect_dodge"), ("ORR_COLLECT_EXPORTER", "orr_export_collect")] {
        let original = PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("mandatory {key}"))).canonicalize().unwrap();
        assert!(original.is_file() && original.starts_with(&workspace));
        let copy = tools.join(name); fs::copy(&original, &copy).unwrap(); bins.push(copy);
    }
    let project = base.join("authored project");
    let isolated = |executable: &Path, bundle: Option<&Path>| {
        let mut hidden = vec![workspace.clone()];
        if bundle.is_some() { hidden.extend([project.clone(), tools.clone()]); }
        let mut cmd = Command::new("bwrap");
        cmd.args(["--die-with-parent", "--ro-bind", "/", "/", "--dev-bind", "/dev/null", "/dev/null", "--bind"]).arg(base).arg(base);
        for p in &hidden { cmd.arg("--tmpfs").arg(p); }
        if let Some(bundle) = bundle { cmd.arg("--ro-bind").arg(bundle).arg(bundle); }
        // Verify masks before execution; a source-hidden claim is not inferred
        // from merely constructing namespace flags.
        cmd.args(["--", "/bin/sh", "-c", "n=$1; shift; while [ \"$n\" -gt 0 ]; do test -z \"$(ls -A -- \"$1\")\" || exit 91; shift; n=$((n-1)); done; exec \"$@\"", "sh"]).arg(hidden.len().to_string());
        for p in hidden { cmd.arg(p); }
        cmd.arg(executable).current_dir(&empty).env("HOME", base.join("user-home")).env("XDG_DATA_HOME", base.join("user-data")).env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY"); cmd
    };
    good(isolated(&bins[0], None).args(["--output"]).arg(&project).args(["--template", "collect-dodge-ui-2d-v1", "--seed", "subtree-copy", "--game-id", "12345678-1234-4234-8234-123456789abc"]).output().unwrap());
    let prepared = open(&project);
    let ui_path = prepared.ui().unwrap().path.clone();
    // Initial authored group is a visible HUD root. All changes under test
    // below use actual Duplicate/position/Undo/Save controls.
    let mut root = node("hud", None, Kind::Container); root.offset = [24, 60]; root.size = [180, 48];
    let mut child = node("score", Some("hud"), label("Score: ")); child.offset = [8, 8];
    let original = Document { schema: 1, nodes: vec![root, child] };
    fs::write(&ui_path, original.to_bytes().unwrap()).unwrap();
    let scene_bytes = fs::read(project.join("level.scene.yaml")).unwrap();
    let before = frame(&project, 0, false);
    let baseline_frames = [(0, false), (8, true), (16, true), (40, false)].map(|(t, r)| frame(&project, t, r));
    let run = |bundle: Option<&Path>, ticks: u32, right: bool, capture: &Path| {
        let executable = bundle.map(|p| p.join("run-collect-dodge")).unwrap_or_else(|| bins[1].clone());
        let mut cmd = isolated(&executable, bundle);
        if bundle.is_none() { cmd.arg("--project").arg(&project); }
        cmd.args(["--headless", "--ticks", &ticks.to_string()]);
        if right { cmd.args(["--hold", "right"]); }
        let output = cmd.arg("--capture").arg(capture).output().unwrap();
        // Retain the original producer output and pixels even when a later
        // assertion fails; temporary test roots are deleted during unwinding.
        let name = capture.file_stem().unwrap().to_str().unwrap();
        fs::write(evidence.join(format!("{name}.stdout.txt")), &output.stdout).unwrap();
        fs::write(evidence.join(format!("{name}.stderr.txt")), &output.stderr).unwrap();
        if capture.is_file() { fs::copy(capture, evidence.join(capture.file_name().unwrap())).unwrap(); }
        let out = good(output);
        assert!(out.contains("software: true"), "mandatory software renderer: {out}");
        out
    };
    let original_capture = captures.join("original.png");
    run(None, 0, false, &original_capture);
    let source_ui = open(&project).take_ui().unwrap();
    let mut h = Harness::builder().with_size([1100.0, 1400.0]).build_ui_state(
        |ui, s: &mut State| s.panel.show(ui, s.editable), State { panel: Panel::new(source_ui), editable: true });
    h.run_steps(3); click(&mut h, "Collect UI document"); select(&mut h, "hud"); click(&mut h, DUPLICATE);
    h.get_by_label("Stable id: ui_copy_1");
    // Anchor x/y, size x/y, offset x/y: focus the copied root's offset Y.
    h.get_all_by_role(Role::SpinButton).nth(5).unwrap().focus(); h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A); h.run_steps(2);
    assert!(h.get_all_by_role(Role::SpinButton).nth(5).unwrap().is_focused());
    h.get_all_by_role(Role::SpinButton).nth(5).unwrap().type_text("180"); h.run_steps(3);
    h.key_press(egui::Key::Enter); h.run_steps(3);
    assert_eq!(h.state().panel.document().nodes[2].offset, [24, 180]);
    assert_eq!(h.state().panel.document().nodes[0].offset, [24, 60]);
    click(&mut h, "Save UI document");
    let copied = h.state().panel.document().clone();
    assert_eq!(open(&project).ui().unwrap().document, copied);
    assert_eq!(frame(&project, 0, false), before);
    assert_eq!(fs::read(project.join("level.scene.yaml")).unwrap(), scene_bytes);
    let edited_capture = captures.join("duplicated.png");
    run(None, 0, false, &edited_capture);
    let original_image = image(&original_capture); let copied_image = image(&edited_capture);
    assert_eq!((original_image.0, original_image.1), (copied_image.0, copied_image.1));
    assert_ne!(original_image.2, copied_image.2, "copy must produce real new HUD pixels");
    let changed: Vec<_> = original_image.2.chunks_exact(4).zip(copied_image.2.chunks_exact(4)).enumerate().filter(|(_, (a,b))| a != b).map(|(i,_)| i).collect();
    assert!(changed.len() > 30, "visible duplicate is more than an incidental pixel");
    assert!(changed.iter().all(|i| { let x = *i as u32 % copied_image.0; let y = *i as u32 / copied_image.0; (24..204).contains(&x) && (180..228).contains(&y) }), "only copied HUD bounds may change");
    let hash = good(Command::new("sha256sum").arg(&bins[1]).output().unwrap());
    let source_before = files(&project);
    let built = base.join("export original");
    good(isolated(&bins[2], None).arg("--project").arg(&project).arg("--runtime").arg(&bins[1]).args(["--runtime-sha256", hash.split_whitespace().next().unwrap(), "--trusted-runtime", "--output"]).arg(&built).output().unwrap());
    let bundle = base.join("relocated readonly bundle"); fs::rename(&built, &bundle).unwrap();
    let bundle_before = files(&bundle);
    assert_eq!(files(&project), source_before, "exporter cannot mutate source");
    let exported_ui_path = bundle.join("project").join(ui_path.strip_prefix(&project).unwrap());
    assert_eq!(fs::read(exported_ui_path).unwrap(), copied.to_bytes().unwrap());
    assert_eq!(open(&bundle.join("project")).ui().unwrap().document, copied);
    for (index, (ticks, right)) in [(0, false), (8, true), (16, true), (40, false)].into_iter().enumerate() {
        let after_frame = frame(&project, ticks, right);
        assert_eq!(baseline_frames[index], after_frame, "UI copy cannot change gameplay Frame bytes or checksum");
        assert_eq!(frame(&bundle.join("project"), ticks, right), after_frame);
        let source_capture = captures.join(format!("tick-{ticks}-source.png")); let export_capture = captures.join(format!("tick-{ticks}-export.png"));
        let source = run(None, ticks, right, &source_capture); let exported = run(Some(&bundle), ticks, right, &export_capture);
        fs::write(captures.join(format!("tick-{ticks}-source.txt")), &source).unwrap();
        fs::write(captures.join(format!("tick-{ticks}-export.txt")), &exported).unwrap();
        assert_eq!(source, exported);
        assert!(source.contains(&format!("collect tick: {ticks} checksum: 0x{:016x}", after_frame.1)));
        assert_eq!(image(&source_capture), image(&export_capture), "source-hidden exported pixels");
    }
    assert_eq!(files(&bundle), bundle_before); assert_eq!(files(&project), source_before);
    for entry in fs::read_dir(&captures).unwrap() { let entry = entry.unwrap(); fs::copy(entry.path(), evidence.join(entry.file_name())).unwrap(); }
    fs::write(evidence.join("saved-ui.json"), copied.to_bytes().unwrap()).unwrap();
    eprintln!("Verified duplicated HUD pixels, four exact Frame probes and four source-hidden readonly export pixel pairs");
}
