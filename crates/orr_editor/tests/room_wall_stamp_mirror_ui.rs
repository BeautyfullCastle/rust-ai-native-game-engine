//! Linux production EditorApp acceptance for mirror-before-quarter-turn wall groups.
//! Uses actual authoring widgets, input routing and composed GPU frames.
//! This does not establish the separately mandatory Windows hardware-UI proof.
#![cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    feature = "room-authoring"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Event, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    Editor, EditorApp, HostSpec,
};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
fn fixture(root: &Path) {
    fs::create_dir(root).unwrap();
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = orr_reflect::Scene::unbake(&room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    fs::write(root.join("room.scene.yaml"), &text).unwrap();
    fs::write(
        root.join("room.camera.json"),
        orr_sample::room_camera::Document::readable_default()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    orr_package::Project::open_for_install(
        root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[source])
    .unwrap();
    let loaded =
        model_bindings::load_asset(root, "sample-imported-scene", "foreground.glb").unwrap();
    let binding = Binding::from_asset(
        "sample-imported-scene".into(),
        "foreground.glb".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let scene = PreparedScene::parse(&text).unwrap();
    let bindings = scene
        .frame()
        .entities()
        .filter(|entity| {
            scene
                .frame()
                .get::<RoomActor>(*entity)
                .is_some_and(|actor| [PLAYER, KEY].contains(&actor.kind))
        })
        .map(|entity| {
            (
                scene.index().guid(entity).unwrap().to_string(),
                binding.clone(),
            )
        })
        .collect();
    fs::write(
        root.join("room.models.json"),
        serde_json::to_vec(&Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings,
        })
        .unwrap(),
    )
    .unwrap();
}
fn harness(root: &Path) -> Harness<'static, EditorApp> {
    let mut prepared = PreparedProject::open(root).unwrap();
    let camera = prepared.take_camera().unwrap();
    let (_, path, scene, models) = prepared.into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    editor.install_room_camera(camera.document.clone()).unwrap();
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                adapter.get_info().backend == wgpu::Backend::Vulkan
                    && adapter.get_info().device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan required".into())
    }));
    Harness::builder()
        .with_size([1600.0, 1150.0])
        .with_step_dt(1.0 / 60.0)
        // Native eframe renderer defaults; predictable test filtering must not
        // change the production viewport texture composition.
        .with_render_options(egui_wgpu::RendererOptions::default())
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.models.install_room(models).unwrap();
            app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
            app
        })
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    h.run_steps(20);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "disabled {label}"
    );
    h.get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(h);
}
// Capture the complete real EditorApp framebuffer, then compare a fixed interior
// of its displayed viewport. The full frame is retained alongside the ROI; no
// primitive-only GpuViewport::read_rgba8 evidence participates in this test.
fn pixels(
    h: &mut Harness<'_, EditorApp>,
    dir: &Path,
    prefix: &str,
    regions: &mut Vec<serde_json::Value>,
) -> Vec<u8> {
    // Native texture sampling depends on the full framebuffer geometry too.
    // Restore identical public egui remembered panel widths in both app runs;
    // otherwise history text can widen the inspector before save only.
    h.set_size(egui::vec2(1730.0, 1009.0));
    settle(h);
    for (name, width) in [("inspector", 600.0), ("hierarchy", 230.0)] {
        let id = egui::Id::new(name);
        let mut panel = egui::containers::panel::PanelState::load(&h.ctx, id)
            .expect("production panel has rendered");
        if name == "inspector" {
            panel.outer_rect.min.x = panel.outer_rect.max.x - width;
        } else {
            panel.outer_rect.max.x = panel.outer_rect.min.x + width;
        }
        h.ctx.data_mut(|data| data.insert_persisted(id, panel));
    }
    h.run_steps(20);
    let target = (900, 850);
    for _ in 0..6 {
        settle(h);
        let actual = h.state().viewport3d_gpu().unwrap().gpu().target().size();
        if actual == target {
            break;
        }
        let window = h.ctx.viewport_rect().size();
        let ratio = h.ctx.pixels_per_point();
        h.set_size(egui::vec2(
            window.x + (target.0 as f32 - actual.0 as f32) / ratio,
            window.y + (target.1 as f32 - actual.1 as f32) / ratio,
        ));
    }
    settle(h);
    assert_eq!(h.ctx.pixels_per_point(), 1.0);
    assert_eq!(
        h.ctx.viewport_rect().size(),
        egui::vec2(1730.0, 1009.0),
        "identical complete framebuffer geometry is required for exact native composition"
    );
    assert_eq!(
        h.state().viewport3d_gpu().unwrap().gpu().target().size(),
        target
    );
    assert!(
        h.query_by_label("Instantiate Room wall stamp").is_none(),
        "close the authoring window before measuring displayed scene pixels"
    );
    for (name, width) in [("inspector", 600.0), ("hierarchy", 230.0)] {
        assert_eq!(
            egui::containers::panel::PanelState::load(&h.ctx, egui::Id::new(name))
                .unwrap()
                .outer_rect
                .width(),
            width,
            "fixed {name} panel geometry"
        );
    }
    let image = h
        .render()
        .expect("actual composed main EditorApp framebuffer");
    let rect = h.state().ui.viewport_rect.unwrap();
    // egui logical coordinates can retain subpixel f32 rounding after panel
    // layout; compare their physical pixel dimensions with the actual target.
    assert_eq!(
        (rect.width().round() as u32, rect.height().round() as u32),
        target
    );
    orr_editor::app::write_png(
        &dir.join(format!("{prefix}-composed.png")),
        &egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
    )
    .unwrap();
    // Exclude the top viewport HUD and border. Every retained pixel comes from
    // the composed native texture in the actual application framebuffer.
    let origin = [
        rect.min.x.round() as u32 + 5,
        rect.min.y.round() as u32 + 48,
    ];
    let size = [890usize, 797usize];
    let logical_rect = serde_json::json!([rect.min.x, rect.min.y, rect.max.x, rect.max.y]);
    if let Some(first) = regions.first() {
        assert_eq!(
            first["viewport_rect"], logical_rect,
            "all captures must have the exact same logical viewport rectangle"
        );
    }
    regions.push(serde_json::json!({
        "name":format!("{prefix}-viewport.png"),
        "composed_name":format!("{prefix}-composed.png"),
        "x":origin[0],"y":origin[1],"width":size[0],"height":size[1],
        "viewport_rect":logical_rect,
        "framebuffer_width":image.width(),"framebuffer_height":image.height(),
        "panel_widths":{"hierarchy":230,"inspector":600},
    }));
    let mut rgba = Vec::with_capacity(size[0] * size[1] * 4);
    for y in 0..size[1] as u32 {
        for x in 0..size[0] as u32 {
            rgba.extend_from_slice(&image.get_pixel(origin[0] + x, origin[1] + y).0);
        }
    }
    assert!(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| *pixel != &rgba[..4])
            .count()
            > 1000,
        "displayed viewport must contain rendered scene detail"
    );
    orr_editor::app::write_png(
        &dir.join(format!("{prefix}-viewport.png")),
        &egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
    )
    .unwrap();
    rgba
}
fn shot(h: &mut Harness<'_, EditorApp>, dir: &Path, name: &str) {
    let image = h.render().unwrap();
    orr_editor::app::write_png(
        &dir.join(name),
        &egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
    )
    .unwrap();
}
fn menu_click(h: &mut Harness<'_, EditorApp>, menu: &str, item: &str) {
    if h.query_by_label(item).is_none() {
        h.get_by_label(menu).click();
        h.run_steps(2);
    }
    h.get_by_label(item).click();
    settle(h);
}
fn player_position(h: &Harness<'_, EditorApp>) -> [f32; 3] {
    let frame = h.state().editor.snapshot().unwrap().predicted();
    let entity = h
        .state()
        .editor
        .rows()
        .iter()
        .map(|row| row.entity)
        .find(|&e| frame.get::<RoomActor>(e).is_some_and(|a| a.kind == PLAYER))
        .unwrap();
    h.state()
        .models
        .placements(&h.state().editor)
        .into_iter()
        .find(|p| p.entity == entity)
        .unwrap()
        .instance
        .translation
}
fn field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(10);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    settle(h);
}
fn component(
    h: &mut Harness<'_, EditorApp>,
    guid: &orr_reflect::Guid,
    name: &str,
) -> orr_reflect::Value {
    h.state_mut()
        .editor
        .field_of(&orr_editor::Target::Guid(guid.clone()), name, "")
        .unwrap()
}

fn named(h: &Harness<'_, EditorApp>, name: &str) -> BTreeSet<orr_reflect::Guid> {
    h.state()
        .editor
        .rows()
        .iter()
        .filter(|row| row.name.as_deref() == Some(name))
        .filter_map(|row| row.guid.clone())
        .collect()
}

fn disabled(h: &Harness<'_, EditorApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

fn select_sources(h: &mut Harness<'_, EditorApp>, sources: &[(String, orr_reflect::Guid)]) {
    click(h, "Read Room walls");
    for (name, guid) in sources {
        h.get_by_role_and_label(Role::CheckBox, &format!("{name} ({guid})"))
            .scroll_to_me();
        h.run_steps(20);
        h.get_by_role_and_label(Role::CheckBox, &format!("{name} ({guid})"))
            .click_accesskit();
        settle(h);
    }
}

// Real production input routing, rather than mutating physics or player position.
fn drive_right(h: &mut Harness<'_, EditorApp>) -> ([f32; 3], [f32; 3]) {
    click(h, LBL_PLAY);
    click(h, "Take control");
    let start = player_position(h);
    h.event(Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        h.run_steps(2);
        std::thread::sleep(Duration::from_millis(10));
    }
    h.event(Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    settle(h);
    (start, player_position(h))
}

// Independent literal oracle, never calculated with production placement code.
// Source centers: (2,4), (4,5.5), (1,7); pivot (-1,1), offset (6,2).
// Columns are source walls; rows are mirror-X then 0/90/180/270 degrees.
// For 90 degrees, wrong-axis (R90 Mz) and wrong-order (Mx R90) are necessarily
// identical to R270 Mx. The 270-degree row is an actual UI negative control for
// both errors, independently of the unchecked 90-degree control below.
const MIRRORED_CENTERS: [[[&str; 2]; 3]; 4] = [
    [["2", "6"], ["0", "7.5"], ["3", "9"]],
    [["2", "0"], ["0.5", "-2"], ["-1", "1"]],
    [["8", "0"], ["10", "-1.5"], ["7", "-3"]],
    [["8", "6"], ["9.5", "8"], ["11", "5"]],
];
const UNMIRRORED_90: [[&str; 2]; 3] = [["2", "6"], ["0.5", "8"], ["-1", "5"]];

fn member_mut<'a>(value: &'a mut orr_reflect::Value, name: &str) -> &'a mut orr_reflect::Value {
    use orr_reflect::Value;
    match value {
        Value::Struct(fields) | Value::Variant(_, fields) => {
            &mut fields.iter_mut().find(|(key, _)| key == name).unwrap().1
        }
        _ => panic!("expected struct or variant"),
    }
}
fn exact_geometry(
    h: &mut Harness<'_, EditorApp>,
    guid: &orr_reflect::Guid,
    original: &[orr_reflect::Value; 3],
    turns: usize,
    mirrored: bool,
    wall: usize,
) {
    use orr_reflect::Value;
    let mut expected = original.clone();
    let Value::Vec3(position) = member_mut(&mut expected[1], "pos") else {
        panic!("position")
    };
    let centers = if mirrored {
        &MIRRORED_CENTERS[turns]
    } else {
        assert_eq!(turns, 1, "unchecked compatibility control is 90 degrees");
        &UNMIRRORED_90
    };
    position.x = centers[wall][0].parse().unwrap();
    position.z = centers[wall][1].parse().unwrap();
    if turns % 2 == 1 {
        let shape = member_mut(&mut expected[2], "shape");
        let Value::Vec3(half) = member_mut(shape, "half_extents") else {
            panic!("extents")
        };
        std::mem::swap(&mut half.x, &mut half.z);
    }
    for (name, value) in [
        "RoomEscapeV1::Actor",
        "orr_physics3d::Body",
        "orr_physics3d::Collider",
    ]
    .into_iter()
    .zip(expected)
    {
        assert_eq!(
            component(h, guid, name),
            value,
            "mirror_x={mirrored}, turns={turns}, wall={wall}, component={name}"
        );
    }
}
fn rotation(h: &mut Harness<'_, EditorApp>, degrees: &str) {
    h.get_by_label(degrees).scroll_to_me();
    h.run_steps(20);
    h.get_by_label(degrees).click();
    settle(h);
}

fn mirror_checked(h: &Harness<'_, EditorApp>) -> bool {
    h.get_by_role_and_label(Role::CheckBox, "Mirror X around pivot")
        .accesskit_node()
        .toggled()
        == Some(egui::accesskit::Toggled::True)
}
fn mirror(h: &mut Harness<'_, EditorApp>, checked: bool) {
    if mirror_checked(h) != checked {
        h.get_by_role_and_label(Role::CheckBox, "Mirror X around pivot")
            .scroll_to_me();
        h.run_steps(20);
        h.get_by_role_and_label(Role::CheckBox, "Mirror X around pivot")
            .click_accesskit();
        settle(h);
    }
    assert_eq!(mirror_checked(h), checked);
}
fn project_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else if path != root.join("room.scene.yaml") {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

#[test]
#[ignore = "requires software Vulkan and fresh ORR_ROOM_WALL_MIRROR_LINUX_DIR"]
fn linux_room_wall_mirror_quarterturns_history_reopen_and_collision() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_ROOM_WALL_MIRROR_LINUX_DIR").expect("fresh evidence directory"),
    );
    assert!(evidence.is_absolute() && !evidence.exists());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("Mirrored Room fixture");
    fixture(&root);
    let preserved = project_bytes(&root);
    let stamp = evidence.join("asymmetric-three-walls.room-stamp.json");
    let hidden = evidence.join("source-hidden.room-stamp.json");
    let mut h = harness(&root);
    settle(&mut h);
    assert!(h.state().has_gpu());
    let initial_rows = h.state().editor.rows().len();
    menu_click(&mut h, "File", "Room level authoring…");
    let mut sources = Vec::new();
    for (name, x, z, hx, hz) in [
        ("mirror_source_a", "2", "4", "1", "0.15"),
        ("mirror_source_b", "4", "5.5", "0.375", "0.625"),
        ("mirror_source_c", "1", "7", "0.25", "0.75"),
    ] {
        for (label, value) in [
            ("Room wall name", name),
            ("Wall center X", x),
            ("Wall center Z", z),
            ("Wall half-width X", hx),
            ("Wall half-depth Z", hz),
        ] {
            field(&mut h, label, value);
        }
        click(&mut h, "Create Room wall");
        let guids = named(&h, name);
        assert_eq!(guids.len(), 1);
        sources.push((name.to_owned(), guids.into_iter().next().unwrap()));
    }
    click(&mut h, "Close window");
    menu_click(&mut h, "File", "Save Ctrl+S");
    let source_checksum = h.state().editor.checksum();
    let source_scene = fs::read(root.join("room.scene.yaml")).unwrap();
    let original: Vec<_> = sources
        .iter()
        .map(|(_, guid)| {
            [
                component(&mut h, guid, "RoomEscapeV1::Actor"),
                component(&mut h, guid, "orr_physics3d::Body"),
                component(&mut h, guid, "orr_physics3d::Collider"),
            ]
        })
        .collect();
    h.state_mut().editor.select(None);
    let mut viewport_regions = Vec::new();
    let baseline_pixels = pixels(&mut h, &evidence, "01-source", &mut viewport_regions);
    menu_click(&mut h, "File", "Room wall groups…");
    assert!(!mirror_checked(&h), "mirror defaults to false");
    select_sources(&mut h, &sources);
    click(&mut h, "Capture selected walls");
    field(&mut h, "Room wall stamp file", &stamp.display().to_string());
    click(&mut h, "Save Room wall stamp");
    let stamp_bytes = fs::read(&stamp).unwrap();
    click(&mut h, "Load Room wall stamp");
    for (label, value) in [
        ("Stamp pivot X", "-1"),
        ("Stamp pivot Z", "1"),
        ("Stamp offset X", "6"),
        ("Stamp offset Z", "2"),
    ] {
        field(&mut h, label, value);
    }
    let base_entries = h.state().editor.history().entries.len();
    let mut routes = Vec::new();
    let mut route_origin = None;
    let mut final_guids = BTreeSet::new();
    // Last placement is the mirrored 90-degree barrier. Every other placement
    // uses the same source/pivot/offset and must permit the actual player route.
    for (mirrored, turns) in [(false, 1usize), (true, 0), (true, 2), (true, 3), (true, 1)] {
        mirror(&mut h, mirrored);
        rotation(&mut h, ["0°", "90°", "180°", "270°"][turns]);
        assert_eq!(h.state().editor.checksum(), source_checksum);
        click(&mut h, "Instantiate Room wall stamp");
        assert_eq!(h.state().editor.rows().len(), initial_rows + 6);
        assert_eq!(h.state().editor.history().entries.len(), base_entries + 1);
        let checksum = h.state().editor.checksum();
        assert_ne!(checksum, source_checksum);
        let mut instance_guids = BTreeSet::new();
        for (wall, ((name, source), geometry)) in sources.iter().zip(&original).enumerate() {
            let group = named(&h, name);
            assert_eq!(group.len(), 2);
            let guid = group.iter().find(|guid| *guid != source).unwrap();
            assert!(instance_guids.insert(guid.clone()));
            exact_geometry(&mut h, guid, geometry, turns, mirrored, wall);
            for (component_name, expected) in [
                "RoomEscapeV1::Actor",
                "orr_physics3d::Body",
                "orr_physics3d::Collider",
            ]
            .into_iter()
            .zip(geometry)
            {
                assert_eq!(&component(&mut h, source, component_name), expected);
            }
        }
        click(&mut h, "Save Room wall stamp");
        assert_eq!(
            fs::read(&stamp).unwrap(),
            stamp_bytes,
            "placement never changes loaded template"
        );
        shot(
            &mut h,
            &evidence,
            &format!("02-controls-mirror-{mirrored}-{}-degrees.png", turns * 90),
        );
        let (start, end) = drive_right(&mut h);
        if let Some(origin) = route_origin {
            assert_eq!(
                [start[0], start[2]],
                origin,
                "all controls start on the same X/Z route"
            );
        } else {
            route_origin = Some([start[0], start[2]]);
        }
        let blocked = mirrored && turns == 1;
        if blocked {
            assert!(
                end[0] > start[0] + 0.05 && end[0] < 1.7,
                "mirror-before-90 collider must block: {start:?} -> {end:?}"
            );
        } else {
            assert!(
                end[0] > 2.5,
                "negative control must pass barrier position: mirror_x={mirrored}, turns={turns}, {end:?}"
            );
        }
        routes.push(
            serde_json::json!({"mirror_x":mirrored,"degrees":turns*90,"start":start,"end":end,"blocked":blocked,
                "wrong_axis_and_wrong_order_control":mirrored && turns==3,
                "unchecked_compatibility_control":!mirrored}),
        );
        shot(
            &mut h,
            &evidence,
            &format!("03-gameplay-mirror-{mirrored}-{}-degrees.png", turns * 90),
        );
        click(&mut h, LBL_STOP);
        assert_eq!(h.state().editor.checksum(), checksum);
        menu_click(&mut h, "Edit", "Undo Ctrl+Z");
        assert_eq!(h.state().editor.checksum(), source_checksum);
        assert_eq!(h.state().editor.rows().len(), initial_rows + 3);
        // Invalid staging and a validly parsed but out-of-bounds transform must
        // preserve the entire history, including this pending redo entry.
        let undone = h.state().editor.history().clone();
        mirror(&mut h, !mirrored);
        assert_eq!(h.state().editor.checksum(), source_checksum);
        assert_eq!(h.state().editor.history(), &undone);
        mirror(&mut h, mirrored);
        assert_eq!(h.state().editor.history(), &undone);
        field(&mut h, "Stamp pivot X", "bad-pivot");
        assert!(disabled(&h, "Instantiate Room wall stamp"));
        assert_eq!(h.state().editor.history(), &undone);
        field(&mut h, "Stamp pivot X", "-1");
        field(&mut h, "Stamp offset X", "30");
        click(&mut h, "Instantiate Room wall stamp");
        assert_eq!(h.state().editor.checksum(), source_checksum);
        assert_eq!(h.state().editor.history(), &undone);
        assert_eq!(
            fs::read(root.join("room.scene.yaml")).unwrap(),
            source_scene
        );
        field(&mut h, "Stamp offset X", "6");
        menu_click(&mut h, "Edit", "Redo Ctrl+Y");
        assert_eq!(h.state().editor.checksum(), checksum);
        for (wall, ((name, source), geometry)) in sources.iter().zip(&original).enumerate() {
            let group = named(&h, name);
            assert_eq!(group.len(), 2);
            let guid = group.iter().find(|guid| *guid != source).unwrap();
            assert!(instance_guids.contains(guid), "redo preserves GUIDs");
            exact_geometry(&mut h, guid, geometry, turns, mirrored, wall);
        }
        if blocked {
            final_guids = instance_guids;
        } else {
            menu_click(&mut h, "Edit", "Undo Ctrl+Z");
            assert_eq!(h.state().editor.checksum(), source_checksum);
        }
    }
    click(&mut h, "Close window");
    fs::rename(&stamp, &hidden).unwrap();
    assert_eq!(fs::read(&hidden).unwrap(), stamp_bytes);
    menu_click(&mut h, "File", "Save Ctrl+S");
    assert!(!h.state().editor.is_dirty());
    let final_checksum = h.state().editor.checksum();
    let saved_scene = fs::read(root.join("room.scene.yaml")).unwrap();
    h.state_mut().editor.select(None);
    let saved_pixels = pixels(
        &mut h,
        &evidence,
        "04-mirrored-saved",
        &mut viewport_regions,
    );
    let changed_viewport_pixels = saved_pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(baseline_pixels.as_chunks::<4>().0)
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed_viewport_pixels > 1000,
        "mirrored geometry must visibly change the composed viewport: {changed_viewport_pixels}"
    );
    drop(h);
    let mut h = harness(&root);
    settle(&mut h);
    assert!(!stamp.exists());
    assert_eq!(h.state().editor.checksum(), final_checksum);
    assert_eq!(h.state().editor.rows().len(), initial_rows + 6);
    assert_eq!(fs::read(root.join("room.scene.yaml")).unwrap(), saved_scene);
    for (wall, ((name, source), geometry)) in sources.iter().zip(&original).enumerate() {
        let group = named(&h, name);
        assert_eq!(group.len(), 2);
        let guid = group.iter().find(|guid| *guid != source).unwrap();
        assert!(final_guids.contains(guid));
        exact_geometry(&mut h, guid, geometry, 1, true, wall);
        for (component_name, expected) in [
            "RoomEscapeV1::Actor",
            "orr_physics3d::Body",
            "orr_physics3d::Collider",
        ]
        .into_iter()
        .zip(geometry)
        {
            assert_eq!(&component(&mut h, source, component_name), expected);
        }
    }
    let reopened_pixels = pixels(
        &mut h,
        &evidence,
        "05-fresh-source-hidden",
        &mut viewport_regions,
    );
    assert!(
        saved_pixels == reopened_pixels,
        "fresh reopen preserves exact composed viewport pixels"
    );
    let (start, end) = drive_right(&mut h);
    assert_eq!(Some([start[0], start[2]]), route_origin);
    assert!(
        end[0] > start[0] + 0.05 && end[0] < 1.7,
        "fresh ordinary collider must still block: {end:?}"
    );
    shot(&mut h, &evidence, "06-fresh-source-hidden-gameplay.png");
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.checksum(), final_checksum);
    assert_eq!(
        project_bytes(&root),
        preserved,
        "all project sidecars and installed source assets remain byte-identical"
    );
    assert_eq!(fs::read(&hidden).unwrap(), stamp_bytes);
    let adapter = h
        .state()
        .viewport3d_gpu()
        .unwrap()
        .gpu()
        .adapter_name()
        .to_owned();
    assert!(
        adapter.to_lowercase().contains("llvmpipe"),
        "software Vulkan: {adapter}"
    );
    use sha2::{Digest, Sha256};
    fn manifest(root: &Path, at: &Path, entries: &mut Vec<serde_json::Value>) {
        if at.is_dir() {
            for entry in fs::read_dir(at).unwrap() {
                manifest(root, &entry.unwrap().path(), entries);
            }
        } else {
            let bytes = fs::read(at).unwrap();
            entries.push(serde_json::json!({
                "name":at.strip_prefix(root).unwrap().to_str().unwrap().replace('\\', "/"),
                "bytes":bytes.len(),
                "sha256":Sha256::digest(&bytes).iter().map(|byte|format!("{byte:02x}")).collect::<String>(),
            }));
        }
    }
    let mut retained_files = Vec::new();
    manifest(&evidence, &root, &mut retained_files);
    manifest(&evidence, &hidden, &mut retained_files);
    retained_files.sort_by_key(|value| value["name"].as_str().unwrap().to_owned());
    let mut captures = Vec::new();
    for entry in fs::read_dir(&evidence).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) == Some("png") {
            let bytes = fs::read(&path).unwrap();
            captures.push(serde_json::json!({"name":path.file_name().unwrap().to_str().unwrap(),"bytes":bytes.len(),"sha256":Sha256::digest(&bytes).iter().map(|byte|format!("{byte:02x}")).collect::<String>()}));
        }
    }
    captures.sort_by_key(|value| value["name"].as_str().unwrap().to_owned());
    fs::write(evidence.join("acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1,"platform":"linux-x86_64","production_editor_app":true,"adapter":adapter,
        "test_source_sha256":Sha256::digest(include_bytes!("room_wall_stamp_mirror_ui.rs"))
            .iter().map(|byte|format!("{byte:02x}")).collect::<String>(),
        "physical_os_input":false,"ui_input":"egui pointer/accessibility/keyboard events",
        "fixture":"three asymmetric walls, non-origin pivot (-1,1), offset (6,2)",
        "renderer_options":"native egui_wgpu::RendererOptions::default",
        "mirror_default_false":true,"actual_mirror_control":true,
        "mirror_before_quarterturn_exact_geometry":true,"capture_save_load_check_instantiate":true,
        "mirrored_centers":MIRRORED_CENTERS,"unchecked_90_centers":UNMIRRORED_90,
        "fresh_guids_and_redo_identity":true,"one_atomic_history_entry":true,
        "invalid_placement_preserves_redo":true,"loaded_template_bytes_unchanged":true,
        "all_routes_same_xz_start":true,"mirror_toggle_preserves_redo":true,
        "all_project_sidecars_and_installed_assets_unchanged":true,"source_geometry_unchanged":true,
        "source_hidden_during_fresh_reopen":true,"reopen_pixels_equal":true,
        "pixel_source":"full composed EditorApp framebuffer; exact interior viewport ROI 890x797",
        "changed_viewport_pixels":changed_viewport_pixels,"viewport_regions":viewport_regions,
        "retained_files":retained_files,
        "gameplay_controls":routes,"reopened_gameplay":{"start":start,"end":end},
        "wrong_axis_and_order_equivalence":"R90 Mz = Mx R90 = R270 Mx about the same pivot, offset last",
        "windows_hardware_ui_tested":false,"windows_hardware_ui_required_before_merge":true,
        "linux_socket_fixture_limit":"separate Linux filesystem Unix-socket fixture can be blocked by sandbox socket restrictions; this UI proof does not replace it",
        "scene_checksum":format!("0x{final_checksum:016x}"),"captures":captures
    })).unwrap()).unwrap();
}
