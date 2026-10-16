//! Mandatory production viewport acceptance. GPU absence fails, never skips.
#![cfg(feature = "terrain")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui_kittest::Harness;
use orr_editor::{game::EditorGame, terrain_document::NewTerrain, Editor, EditorApp};
use orr_fp::FP;
use orr_package::{Project, Runtime};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_terrain::Edit;
use std::path::{Path, PathBuf};

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(h.state().editor.yard_rows_coherent());
}
fn pixels(h: &mut Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
    settle(h);
    let gpu = h
        .state()
        .viewport3d_gpu()
        .expect("actual EditorApp viewport")
        .gpu();
    let bytes = gpu.read_rgba8();
    if let Some(root) = std::env::var_os("TERRAIN_CAPTURE_DIR") {
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        let file = std::fs::File::create(root.join(format!("{name}.png"))).unwrap();
        let size = gpu.target().size();
        let mut encoder = png::Encoder::new(file, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&bytes)
            .unwrap();
    }
    bytes
}
fn config() -> NewTerrain {
    NewTerrain {
        asset_id: "yard/terrain".into(),
        width: 3,
        depth: 3,
        origin: [FP::from_int(-4); 2],
        spacing: FP::from_int(4),
        height: FP::ONE,
    }
}
#[test]
fn actual_main_viewport_raised_hole_shared_models_camera_resize_and_saved_restore() {
    let rhi = Wgpu::headless(WgpuOptions::default()).expect("mandatory terrain GPU adapter");
    eprintln!(
        "terrain GPU {} software={}",
        rhi.adapter_name(),
        rhi.is_software()
    );
    drop(rhi);
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let scene = root.join("terrain.scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    let asset = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[asset])
        .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    editor.camera3d = orr_render::OrbitCamera::new([0.0, 1.5, 0.0], 0.45, 0.75, 16.0);
    let mut h = Harness::builder()
        .with_size([1280.0, 1000.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    settle(&mut h);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let base = pixels(&mut h, "01-host");
    {
        let app = h.state_mut();
        app.terrain.document.set_scene(Some(&scene)).unwrap();
        app.terrain
            .document
            .new_local("ground.orrt", config())
            .unwrap();
    }
    let flat = pixels(&mut h, "02-flat-terrain");
    assert_ne!(flat, base);
    assert_eq!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .model_cache_counts()
            .0,
        1
    );
    h.state_mut()
        .terrain
        .document
        .apply(&[Edit::SetHeight {
            x: 1,
            z: 1,
            height: FP::from_int(5),
        }])
        .unwrap();
    let raised = pixels(&mut h, "03-raised");
    assert_ne!(raised, flat);
    h.state_mut()
        .terrain
        .document
        .apply(&[Edit::SetHole {
            x: 0,
            z: 0,
            hole: true,
        }])
        .unwrap();
    let holes = pixels(&mut h, "04-hole");
    assert_ne!(holes, raised);
    assert_eq!(
        h.state()
            .terrain
            .document
            .terrain()
            .unwrap()
            .triangles()
            .len(),
        6
    );
    h.state_mut()
        .terrain
        .document
        .set_query(Some([FP::ONE, FP::ONE]))
        .unwrap();
    let queried = pixels(&mut h, "04b-query-marker");
    assert_ne!(queried, holes);
    let surface = h.state().terrain.document.surface();
    assert!(surface.is_some());
    {
        let app = h.state_mut();
        assert!(app.editor.select_named("box_right"));
        app.editor.sync();
        app.models.open_for_editor(&app.editor, true).unwrap();
        app.models.package = "sample-imported-scene".into();
        app.models.asset = "foreground.glb".into();
        app.models.assign(&app.editor).unwrap();
        app.editor.select(None);
    }
    let mixed = pixels(&mut h, "05-shared-model-depth");
    assert_ne!(mixed, holes);
    assert_eq!(
        h.state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .model_cache_counts()
            .0,
        2
    );
    let camera = h.state().editor.camera3d;
    h.state_mut().editor.camera3d.yaw += 0.6;
    let moved = pixels(&mut h, "06-camera");
    assert_ne!(moved, mixed);
    h.state_mut().editor.camera3d = camera;
    h.set_size(egui::vec2(1500.0, 1100.0));
    let resized = pixels(&mut h, "07-resize");
    assert_ne!(resized.len(), mixed.len());
    h.set_size(egui::vec2(1280.0, 1000.0));
    let saved = pixels(&mut h, "08-before-save");
    let bytes = h.state().terrain.document.bytes().unwrap().to_vec();
    let revision = h.state().terrain.document.revision();
    h.state_mut().terrain.document.save().unwrap();
    h.state_mut().terrain.document.close().unwrap();
    let detached = pixels(&mut h, "09-detached");
    assert_ne!(detached, saved);
    h.state_mut()
        .terrain
        .document
        .open_local("ground.orrt")
        .unwrap();
    h.state_mut()
        .terrain
        .document
        .set_query(Some([FP::ONE, FP::ONE]))
        .unwrap();
    assert_eq!(h.state().terrain.document.surface(), surface);
    let reopened = pixels(&mut h, "10-reopened");
    assert_eq!(reopened, saved);
    assert_eq!(h.state().terrain.document.bytes().unwrap(), bytes);
    assert_eq!(h.state().terrain.document.revision(), revision);
    let model = h.state().terrain.document.model().unwrap().clone();
    assert!(h
        .state_mut()
        .terrain
        .document
        .apply(&[Edit::SetHeight {
            x: 1,
            z: 1,
            height: FP::from_raw(65536000001)
        }])
        .is_err());
    assert!(std::sync::Arc::ptr_eq(
        h.state().terrain.document.model().unwrap(),
        &model
    ));
    assert_eq!(pixels(&mut h, "11-rejected-edit"), saved);
    let all: Vec<_> = (0..2)
        .flat_map(|z| (0..2).map(move |x| Edit::SetHole { x, z, hole: true }))
        .collect();
    h.state_mut().terrain.document.apply(&all).unwrap();
    assert!(h.state().terrain.document.model().is_none());
    assert_eq!(pixels(&mut h, "12-all-holes"), detached);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
}

#[test]
fn scene_terrain_hole_reveals_entity_model_and_filled_neighbor_occludes_it() {
    use orr_editor::viewport3d::{ModelPlacement, SceneModel, Viewport3dGpu};
    use std::sync::Arc;
    let rhi = Wgpu::headless(WgpuOptions::default()).expect("mandatory composed terrain GPU");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let scene = root.join("depth.scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    let rear = orr_terrain::Terrain::new(
        "rear-model".into(),
        3,
        3,
        [FP::from_int(-2); 2],
        FP::from_int(2),
        vec![FP::ZERO; 9],
        vec![false; 4],
    )
    .unwrap();
    let mut source = orr_terrain_view::to_static_model(&rear, &[])
        .unwrap()
        .unwrap()
        .source()
        .clone();
    for rgba in source.images[0].rgba8.as_chunks_mut::<4>().0 {
        rgba.copy_from_slice(&[220, 30, 30, 255]);
    }
    let rear = Arc::new(orr_model::StaticModel::new(source).unwrap());
    let terrain = orr_terrain::Terrain::new(
        "front-terrain".into(),
        3,
        3,
        [FP::from_int(-2); 2],
        FP::from_int(2),
        vec![FP::from_int(2); 9],
        vec![true, false, false, false],
    )
    .unwrap();
    let front = Arc::new(
        orr_terrain_view::to_static_model(&terrain, &[])
            .unwrap()
            .unwrap(),
    );
    let placements = [ModelPlacement {
        entity: editor.rows()[0].entity,
        model: rear,
        instance: Default::default(),
    }];
    let scene_models = [SceneModel {
        model: front,
        instance: Default::default(),
    }];
    let mut list = orr_editor::viewport3d::YardFrame::default().list(&[], None);
    list.lighting.shadows = false;
    let mut camera = orr_render::Camera3D::orthographic([0.0, 10.0, 0.0], [0.0, 0.0, 0.0], 3.0);
    camera.up = [0.0, 0.0, -1.0];
    let size = (240, 240);
    let mut gpu = Viewport3dGpu::new(&rhi, size);
    let render = |gpu: &mut Viewport3dGpu, front: &[SceneModel], rear: &[ModelPlacement]| {
        gpu.render_scene(
            size,
            &list,
            &camera,
            rear,
            front,
            #[cfg(feature = "animated-models")]
            &[],
            Default::default(),
            #[cfg(feature = "irradiance-probes")]
            None,
        )
        .unwrap();
        gpu.read_rgba8()
    };
    let rear_only = render(&mut gpu, &[], &placements);
    let front_only = render(&mut gpu, &scene_models, &[]);
    let mixed = render(&mut gpu, &scene_models, &placements);
    let pixel = |bytes: &[u8], world| {
        let p = camera.world_to_screen(world, size).unwrap();
        let offset = (p[1] as usize * size.0 as usize + p[0] as usize) * 4;
        bytes[offset..offset + 4].to_vec()
    };
    for world in [[-1.0, 2.0, -1.0], [-0.5, 2.0, -1.5]] {
        assert_eq!(
            pixel(&mixed, world),
            pixel(&rear_only, world),
            "hole reveals entity model"
        );
        assert_ne!(pixel(&mixed, world), pixel(&front_only, world));
    }
    for world in [[1.0, 2.0, -1.0], [-1.0, 2.0, 1.0], [1.0, 2.0, 1.0]] {
        assert_eq!(
            pixel(&mixed, world),
            pixel(&front_only, world),
            "filled neighboring terrain owns depth"
        );
        assert_ne!(pixel(&mixed, world), pixel(&rear_only, world));
    }
    list.lighting.shadows = true;
    gpu.render_scene(
        size,
        &list,
        &camera,
        &placements,
        &scene_models,
        #[cfg(feature = "animated-models")]
        &[],
        Default::default(),
        #[cfg(feature = "irradiance-probes")]
        None,
    )
    .unwrap();
    let shadowed = gpu.read_rgba8();
    assert_ne!(
        shadowed, mixed,
        "raised scene terrain casts onto the shared rear model"
    );
    let before = shadowed;
    let cache = gpu.model_cache_counts();
    let overflow: Vec<_> = (0..257)
        .map(|_| SceneModel {
            model: scene_models[0].model.clone(),
            instance: Default::default(),
        })
        .collect();
    assert!(gpu
        .render_scene(
            size,
            &list,
            &camera,
            &[],
            &overflow,
            #[cfg(feature = "animated-models")]
            &[],
            Default::default(),
            #[cfg(feature = "irradiance-probes")]
            None
        )
        .is_err());
    assert_eq!(gpu.read_rgba8(), before);
    assert_eq!(gpu.model_cache_counts(), cache);
}
