use super::*;

#[test]
#[ignore = "explicit CPU editor/export acceptance; requires ORR_MATERIAL_ROOM_RUNTIME"]
fn color_widgets_bake_assign_save_and_source_hidden_export_use_authored_materials() {
    let fixture = Fixture::new();
    let prepared = PreparedProject::open(&fixture.root).unwrap();
    let (_, path, scene, models) = prepared.into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    editor.select(Some(Target::Guid(fixture.guids[&PLAYER].clone())));
    let mut h = Harness::builder()
        .with_size([1700.0, 1500.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| {
            let mut app = EditorApp::new(editor, None);
            app.models.install_room(models).unwrap();
            app
        });
    settle(&mut h);
    let checksum = h.state().editor.checksum();
    click(&mut h, "Box blockout");
    click(&mut h, "New box recipe");
    h.state_mut().models.blockout.color = [0.8, 0.2, 0.1];
    click(&mut h, "Apply material color");
    assert_eq!(recipe(&h).version, 2);
    click(&mut h, "Undo blockout");
    assert_eq!(recipe(&h).version, 1);
    click(&mut h, "Redo blockout");
    assert_eq!(recipe(&h).colors()[0], [0.8, 0.2, 0.1]);
    click(&mut h, "Material slot 1");
    assert_eq!(h.state().models.blockout.color, [0.3, 0.65, 1.0]);
    h.state_mut().models.blockout.color = [0.1, 0.8, 0.3];
    click(&mut h, "Apply material color");
    click(&mut h, "Apply face material");
    let lock = fs::read(fixture.root.join("orr.packages.lock.json")).unwrap();
    click(&mut h, "Bake immutable box");
    assert!(h.state().models.blockout.error().is_some());
    assert_eq!(
        fs::read(fixture.root.join("orr.packages.lock.json")).unwrap(),
        lock
    );
    click(&mut h, "Save box recipe");
    click(&mut h, "Bake immutable box");
    assert!(
        h.state().models.blockout.error().is_none(),
        "{:?}",
        h.state().models.blockout.error()
    );
    click(&mut h, "Static model binding");
    click(&mut h, "Load model asset");
    click(&mut h, "Assign static model");
    click(&mut h, "Save model bindings");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    let placements = h.state().models.placements(&h.state().editor);
    assert_eq!(
        placements[0].model.source().materials[0].base_color,
        [0.8, 0.2, 0.1, 1.0]
    );
    assert_eq!(
        placements[0].model.source().materials[1].base_color,
        [0.1, 0.8, 0.3, 1.0]
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    drop(h);
    let built = PathBuf::from(
        std::env::var_os("ORR_MATERIAL_ROOM_RUNTIME")
            .expect("focused test requires built room runtime"),
    )
    .canonicalize()
    .unwrap();
    let runtime = fixture.temp.path().join("trusted material runtime");
    fs::copy(built, &runtime).unwrap();
    let bundle = fixture.temp.path().join("deployed material game");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
        project: fixture.root.clone(),
        runtime: runtime.clone(),
        runtime_sha256: file_hash(&runtime),
        output: bundle.clone(),
        trusted_runtime: true,
        source_revision: None,
    })
    .unwrap();
    fs::rename(&fixture.root, fixture.root.with_extension("hidden")).unwrap();
    fs::rename(&fixture.source, fixture.source.with_extension("hidden")).unwrap();
    fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
    let prepared = PreparedProject::open(&bundle.join("project")).unwrap();
    let asset = prepared
        .models()
        .assets
        .values()
        .next()
        .unwrap()
        .static_model()
        .unwrap();
    assert_eq!(asset.source().materials[0].base_color, [0.8, 0.2, 0.1, 1.0]);
    assert_eq!(asset.source().materials[1].base_color, [0.1, 0.8, 0.3, 1.0]);
    let output = Command::new(bundle.join("run-room-escape"))
        .current_dir(fixture.temp.path())
        .args(["--headless", "--ticks", "1", "--check-model-poses"])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("room model poses: 2 frames checked; final static: 1 animated: 0"));
}
