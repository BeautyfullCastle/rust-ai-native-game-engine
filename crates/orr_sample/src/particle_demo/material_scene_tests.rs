use super::*;

#[test]
fn explicit_material_cli_rejects_invalid_before_runtime() {
    for args in [
        vec![],
        vec!["--particles", "--materials", "legacy", "off"],
        vec!["--materials"],
        vec!["--materials", "auto"],
        vec!["--materials", "legacy", "--particles", "auto"],
        vec![
            "--materials",
            "legacy",
            "--materials",
            "showcase",
            "--particles",
            "off",
        ],
        vec!["--materials", "showcase", "--particles", "off", "--unknown"],
        vec!["--particles", "off"],
        vec!["--materials", "showcase"],
    ] {
        assert!(MaterialOptions::parse(args.into_iter().map(str::to_owned).collect()).is_err());
    }
    for name in ["legacy", "showcase"] {
        let parsed = MaterialOptions::parse(
            [
                "--particles",
                "sparks",
                "--materials",
                name,
                "--headless",
                "--showcase",
            ]
            .map(str::to_owned)
            .into(),
        )
        .unwrap();
        assert_eq!(
            parsed.materials,
            if name == "legacy" {
                MaterialMode::Legacy
            } else {
                MaterialMode::Showcase
            }
        );
        assert_eq!(parsed.particles.mode, Mode::Sparks);
        assert!(parsed.particles.headless && parsed.particles.showcase);
    }
    assert!(matches!(SceneRoute::default(), SceneRoute::Procedural));
    assert!(
        Options::parse(
            ["--materials", "legacy", "--particles", "off"]
                .map(str::to_owned)
                .into()
        )
        .is_err()
    );
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

#[test]
fn original_models_have_bounded_outward_geometry_and_cooked_provenance() {
    let models = models().unwrap();
    for (index, model) in models.iter().enumerate() {
        let source = model.source();
        assert_eq!(source.dependencies.len(), 1);
        assert_eq!(source.dependencies[0].uri, "$source");
        assert_eq!(
            source.dependencies[0].sha256,
            // Independently frozen SHA-256 vector of the authored UTF-8 recipe.
            "a7dc62b7401297a52fc626767df547daba6b3fe5f0a7af259a328ff27bddb99d"
        );
        assert!(RECIPE.contains("license=MIT OR Apache-2.0"));
        assert_eq!(
            source.images,
            [Image {
                width: 1,
                height: 1,
                rgba8: vec![255; 4]
            }]
        );
        assert_eq!(source.materials.len(), 1);
        assert_eq!(source.materials[0].base_color, COLORS[index]);
        assert_eq!(source.primitives.len(), 1);
        let primitive = &source.primitives[0];
        assert!(
            primitive
                .id
                .starts_with(&format!("{}#node=", source.asset_id))
        );
        assert_eq!(primitive.transform, IDENTITY);
        assert_eq!(primitive.vertices.len(), if index == 0 { 4 } else { 362 });
        assert_eq!(primitive.indices.len(), if index == 0 { 6 } else { 2160 });
        for vertex in &primitive.vertices {
            assert!(
                vertex
                    .position
                    .into_iter()
                    .all(|x| x.is_finite() && x.abs() <= 1.0)
            );
            assert!((dot(vertex.normal, vertex.normal) - 1.0).abs() < 2e-6);
            assert!(vertex.uv.into_iter().all(|x| (0.0..=1.0).contains(&x)));
            if index != 0 {
                assert_eq!(vertex.position, vertex.normal);
            }
        }
        for triangle in primitive.indices.chunks_exact(3) {
            let [a, b, c]: [u32; 3] = triangle.try_into().unwrap();
            let [a, b, c]: [Vertex; 3] = [a, b, c].map(|i: u32| primitive.vertices[i as usize]);
            let area = cross(sub(b.position, a.position), sub(c.position, a.position));
            assert!(dot(area, area) > 1e-7, "degenerate triangle");
            assert!(dot(area, a.normal) > 0.0, "inward triangle");
        }
        let bytes = model.to_bytes().unwrap();
        assert!(bytes.len() < 128 * 1024);
        assert_eq!(StaticModel::from_bytes(&bytes).unwrap().source(), source);
    }
    assert_eq!(
        models[1].source().primitives[0].vertices,
        models[2].source().primitives[0].vertices
    );
    assert_eq!(
        models[1].source().primitives[0].indices,
        models[2].source().primitives[0].indices
    );
    for (metal, roughness) in PRESETS {
        ScalarMaterial::new(metal, roughness).unwrap();
    }
}

fn assert_view_parity(demo: &Demo, placements: &Placements) {
    assert_eq!(placements.floor[0].translation, [0.0; 3]);
    assert_eq!(placements.floor[0].scale, [26.0, 1.0, 26.0]);
    // Procedural presentation stays unchanged; only actor placements are paired.
    assert_eq!(demo.list.planes[0].pos, [0.8, 0.0, 0.3]);
    assert_eq!(demo.list.planes[0].scale, [3.0, 1.0, 2.0]);
    let mut actual: Vec<_> = placements
        .player
        .iter()
        .chain(&placements.pickups)
        .map(|p| (p.translation, p.scale, p.rotation))
        .collect();
    let mut expected: Vec<_> = demo
        .list
        .spheres
        .iter()
        .map(|p| (p.pos, p.scale, p.rot))
        .collect();
    type PlacementTuple = ([f32; 3], [f32; 3], [f32; 4]);
    let order = |a: &PlacementTuple, b: &PlacementTuple| {
        a.0[0].total_cmp(&b.0[0]).then(a.0[2].total_cmp(&b.0[2]))
    };
    actual.sort_by(order);
    expected.sort_by(order);
    assert_eq!(actual, expected);
}

#[test]
fn immutable_extraction_tracks_real_pickup_restart_and_particle_parity() {
    for mode in [Mode::Off, Mode::Dust, Mode::Sparks] {
        let mut baseline = Demo::new(mode);
        let mut dressed = Demo::new(mode);
        let mut placements = Placements::new();
        let capacities = (placements.player.capacity(), placements.pickups.capacity());
        let initial_checksum = baseline.snapshot.as_ref().unwrap().predicted().checksum();
        let mut seen_pickup = false;
        for _ in 0..20 {
            placements.extract(&dressed).unwrap();
            assert_view_parity(&dressed, &placements);
            let before = dressed.snapshot.as_ref().unwrap().predicted().checksum();
            let particles = dressed.draws.clone();
            // Repeated extraction for both material selections is read-only.
            for _material in [MaterialMode::Legacy, MaterialMode::Showcase] {
                placements.extract(&dressed).unwrap();
            }
            assert_eq!(
                before,
                dressed.snapshot.as_ref().unwrap().predicted().checksum()
            );
            assert_eq!(particles, dressed.draws);
            assert_eq!(
                capacities,
                (placements.player.capacity(), placements.pickups.capacity())
            );
            for demo in [&mut baseline, &mut dressed] {
                demo.update(
                    Duration::from_millis(17),
                    CollectInput {
                        x: FP::ONE,
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            assert_eq!(
                baseline.snapshot.as_ref().unwrap().predicted().checksum(),
                dressed.snapshot.as_ref().unwrap().predicted().checksum()
            );
            assert_eq!(baseline.draws, dressed.draws);
            placements.extract(&dressed).unwrap();
            assert_view_parity(&dressed, &placements);
            if Showcase::score(&dressed) == 1 {
                seen_pickup = true;
                assert_eq!(placements.pickups.len(), 1);
                assert_eq!(placements.pickups[0].translation, [1.6, 0.2, 0.6]);
            }
        }
        assert!(seen_pickup);
        assert_eq!(Showcase::score(&baseline), Showcase::score(&dressed));
        dressed = Demo::new(mode);
        placements.extract(&dressed).unwrap();
        assert_eq!(placements.pickups.len(), 2);
        assert!(dressed.draws.is_empty());
        assert_eq!(
            dressed.snapshot.as_ref().unwrap().predicted().checksum(),
            initial_checksum
        );
        assert_view_parity(&dressed, &placements);
    }
}

#[test]
fn semantic_actor_tags_and_bounds_are_explicit() {
    let demo = Demo::new(Mode::Off);
    let mut actor = *demo
        .snapshot
        .as_ref()
        .unwrap()
        .predicted()
        .iter::<CollectActor>()
        .next()
        .unwrap()
        .1;
    let mut placements = Placements::new();
    actor.kind = u32::MAX;
    actor.active = 1;
    assert!(placements.push_actor(&actor).is_err());
    actor.active = 0;
    placements.push_actor(&actor).unwrap();
    assert!(placements.player.is_empty() && placements.pickups.is_empty());
    actor.active = 1;
    actor.kind = PLAYER;
    placements.push_actor(&actor).unwrap();
    assert!(placements.push_actor(&actor).is_err());
    actor.kind = COLLECTIBLE;
    placements.push_actor(&actor).unwrap();
    placements.push_actor(&actor).unwrap();
    assert!(placements.push_actor(&actor).is_err());
}

fn gpu() -> Wgpu {
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("required material sample GPU adapter");
    let info = gpu.adapter().get_info();
    eprintln!(
        "material sample adapter: name={:?} backend={:?} device_type={:?} vendor={} device={} driver={:?} driver_info={:?} software={}",
        info.name,
        info.backend,
        info.device_type,
        info.vendor,
        info.device,
        info.driver,
        info.driver_info,
        gpu.is_software()
    );
    gpu
}
fn changed(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

#[test]
#[ignore = "requires separately admitted actual GPU lane"]
fn actual_material_scene_preserves_pickup_and_expired_particle_baseline() {
    let gpu = gpu();
    let target = orr_render::OffscreenTarget::new(&gpu, 640, 480, TextureFormat::Rgba8UnormSrgb);
    let mut capture = SceneCapture::new(&gpu);
    let mut demo = Demo::new(Mode::Sparks);
    let mut renderers = [
        MaterialScene::new(&gpu, target.format(), Mode::Sparks, MaterialMode::Legacy).unwrap(),
        MaterialScene::new(&gpu, target.format(), Mode::Sparks, MaterialMode::Showcase).unwrap(),
    ];
    let mut without_particles =
        MaterialScene::new(&gpu, target.format(), Mode::Off, MaterialMode::Showcase).unwrap();
    let mut clock = Showcase::default();
    let initial = capture_pair(
        &mut renderers,
        &target,
        &demo,
        &clock,
        "initial",
        &mut capture,
    );
    assert!(changed(&initial[0], &initial[1]) > 100);
    assert!(
        initial[1]
            .chunks_exact(4)
            .filter(|p| p[..3].iter().any(|&c| c > 32))
            .count()
            > 1000
    );
    for _ in 0..20 {
        clock
            .update(&mut demo, Duration::from_millis(17), true)
            .unwrap();
        if !demo.draws.is_empty() {
            break;
        }
    }
    let pickup = capture_pair(
        &mut renderers,
        &target,
        &demo,
        &clock,
        "pickup",
        &mut capture,
    );
    assert_eq!(Showcase::score(&demo), 1);
    assert_eq!(demo.draws.len(), 16);
    assert_eq!(renderers[1].placements.pickups.len(), 1);
    without_particles
        .draw(target.render_view(), target.size(), &demo)
        .unwrap();
    assert!(changed(&target.read_rgba8(), &pickup[1]) > 0);
    let anchor = clock.pickup_observed_at.unwrap();
    clock
        .update(&mut demo, Duration::from_millis(100), true)
        .unwrap();
    let spread = capture_pair(
        &mut renderers,
        &target,
        &demo,
        &clock,
        "spread",
        &mut capture,
    );
    assert_eq!(demo.draws.len(), 16);
    assert_ne!(pickup[1], spread[1]);
    let destination = anchor + Duration::from_millis(700);
    while clock.elapsed < destination {
        clock
            .update(
                &mut demo,
                Duration::from_millis(17).min(destination - clock.elapsed),
                true,
            )
            .unwrap();
    }
    let expired = capture_pair(
        &mut renderers,
        &target,
        &demo,
        &clock,
        "expired",
        &mut capture,
    );
    assert!(demo.draws.is_empty());
    without_particles
        .draw(target.render_view(), target.size(), &demo)
        .unwrap();
    assert_eq!(expired[1], target.read_rgba8());
    clock.restart(&mut demo);
    let restart = capture_pair(
        &mut renderers,
        &target,
        &demo,
        &clock,
        "restart",
        &mut capture,
    );
    assert_eq!(initial, restart);
    capture.finish();
}

#[test]
#[ignore = "requires separately admitted actual GPU lane"]
fn actual_metallic_and_roughness_sensitivity_changes_only_one_parameter() {
    let gpu = gpu();
    let target = orr_render::OffscreenTarget::new(&gpu, 256, 256, TextureFormat::Rgba8UnormSrgb);
    let model = models().unwrap()[1].clone();
    let mut scene = ImportedSceneRenderer::new(gpu.clone(), target.format()).unwrap();
    let camera = Camera3D::perspective([0.0, 0.0, 3.0], [0.0; 3], 48.0);
    let light = orr_render::Lighting {
        direction: [-0.35, -0.25, -1.0],
        shadows: false,
        ..Default::default()
    };
    let mut render = |metal, rough| {
        let mut renderer = ModelRenderer::new_with_materials(
            gpu.clone(),
            scene.scene_format(),
            model.clone(),
            &[MaterialShading::MetallicRoughness(
                ScalarMaterial::new(metal, rough).unwrap(),
            )],
        )
        .unwrap();
        scene
            .draw(
                ImportedSceneTarget {
                    view: target.render_view(),
                    size: target.size(),
                    format: target.format(),
                    sample_count: 1,
                },
                &camera,
                &light,
                &PointLightSettings::default(),
                &mut [ImportedBatch::Static(&mut renderer)],
            )
            .unwrap();
        target.read_rgba8()
    };
    let base = render(0.0, 0.25);
    let identical = render(0.0, 0.25);
    assert_eq!(base, identical, "negative control must be byte-identical");
    for alternate in [render(1.0, 0.25), render(0.0, 0.75)] {
        let mut interior_changes = 0;
        // Central disk is strictly inside this unit sphere's projected silhouette.
        for y in 96..160 {
            for x in 96..160 {
                let offset = (y * 256 + x) * 4;
                let difference: u32 = base[offset..offset + 3]
                    .iter()
                    .zip(&alternate[offset..offset + 3])
                    .map(|(&a, &b)| u32::from(a.abs_diff(b)))
                    .sum();
                if difference > 8 {
                    interior_changes += 1;
                }
            }
        }
        assert!(
            interior_changes > 32,
            "scalar sensitivity must change shared interior pixels: {interior_changes}"
        );
    }
}

// Capture is an explicit test-only side channel. Partial files survive failed
// assertions; only finish() appends PASS. No runtime screenshot flag is added.
struct SceneCapture {
    directory: Option<std::path::PathBuf>,
    manifest: Option<std::fs::File>,
    records: usize,
    bytes: usize,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
impl SceneCapture {
    fn new(gpu: &Wgpu) -> Self {
        let directory = std::env::var_os("ORR_MATERIAL_CAPTURE_DIR").map(std::path::PathBuf::from);
        let manifest = directory.as_ref().map(|directory| {
            assert!(directory.is_absolute());
            assert_eq!(directory.canonicalize().unwrap(), *directory);
            assert!(directory.symlink_metadata().unwrap().is_dir());
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("manifest.txt"))
                .unwrap()
        });
        let info = gpu.adapter().get_info();
        let mut result = Self {
            directory,
            manifest,
            records: 0,
            bytes: 0,
        };
        result.line("STATE=RUNNING; partial captures are not a passed acceptance result");
        result.line(&format!("adapter name={:?} backend={:?} device_type={:?} vendor={} device={} driver={:?} driver_info={:?} software={}",info.name,info.backend,info.device_type,info.vendor,info.device,info.driver,info.driver_info,gpu.is_software()));
        result.line("size=640x480 format=Rgba8UnormSrgb encoding=PNG-RGBA8 camera=player_follow eye_offset=2.4,3.6,-1.8 fov_y_max_degrees=40 aspect_cap=2 floor_half_extent=26 msaa=1 shadows=true radius=6.5 hdr=false");
        result.line(&format!(
            "recipe_sha256={} colors={COLORS:?} presets={PRESETS:?}",
            hex(&Sha256::digest(RECIPE.as_bytes()))
        ));
        result
    }
    fn line(&mut self, line: &str) {
        use std::io::Write;
        assert!(!line.contains('\n') && line.len() <= 4096);
        self.bytes += line.len() + 1;
        assert!(self.bytes <= 64 * 1024);
        if let Some(file) = &mut self.manifest {
            writeln!(file, "{line}").unwrap();
            file.flush().unwrap();
        }
        eprintln!("material capture {line}");
    }
    fn frame(&mut self, mode: &str, stage: &str, pixels: &[u8], demo: &Demo, clock: &Showcase) {
        assert!(matches!(mode, "legacy" | "showcase"));
        assert!(matches!(
            stage,
            "initial" | "pickup" | "spread" | "expired" | "restart"
        ));
        assert_eq!(pixels.len(), 640 * 480 * 4);
        let name = format!("{mode}-{stage}.png");
        let encoded = if let Some(directory) = &self.directory {
            struct Limited {
                file: std::fs::File,
                bytes: usize,
            }
            impl std::io::Write for Limited {
                fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                    if bytes.len() > 2 * 1024 * 1024 - self.bytes {
                        return Err(std::io::Error::other("material PNG exceeds2MiB"));
                    }
                    let n = std::io::Write::write(&mut self.file, bytes)?;
                    self.bytes += n;
                    Ok(n)
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    std::io::Write::flush(&mut self.file)
                }
            }
            let path = directory.join(&name);
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            let mut encoder = png::Encoder::new(Limited { file, bytes: 0 }, 640, 480);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(pixels).unwrap();
            writer.finish().unwrap();
            let bytes = std::fs::read(path).unwrap();
            assert!(bytes.len() <= 2 * 1024 * 1024);
            hex(&Sha256::digest(bytes))
        } else {
            "not-written".into()
        };
        let snapshot = demo.snapshot.as_ref().unwrap();
        let actors = snapshot
            .predicted()
            .iter::<CollectActor>()
            .filter(|(_, a)| a.active != 0)
            .count();
        self.line(&format!("frame={name} raw_rgba_sha256={} png_sha256={encoded} tick={} checksum={:016x} elapsed_us={} pickup_us={:?} score={} particles={} actors={actors} lighting={:?}",hex(&Sha256::digest(pixels)),snapshot.predicted().tick(),snapshot.predicted().checksum(),clock.elapsed.as_micros(),clock.pickup_observed_at.map(|t|t.as_micros()),Showcase::score(demo),demo.draws.len(),demo.list.lighting));
        self.records += 1;
    }
    fn finish(mut self) {
        assert_eq!(self.records, 10);
        self.line("STATE=PASS frames=10");
    }
}
fn capture_pair(
    renderers: &mut [MaterialScene<Wgpu>; 2],
    target: &orr_render::OffscreenTarget<Wgpu>,
    demo: &Demo,
    clock: &Showcase,
    stage: &str,
    capture: &mut SceneCapture,
) -> [Vec<u8>; 2] {
    std::array::from_fn(|i| {
        renderers[i]
            .draw(target.render_view(), target.size(), demo)
            .unwrap();
        let pixels = target.read_rgba8();
        capture.frame(
            if i == 0 { "legacy" } else { "showcase" },
            stage,
            &pixels,
            demo,
            clock,
        );
        pixels
    })
}

// Independent f64 camera construction, never production inverse/view helpers.
fn dsub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn ddot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn dcross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: [f64; 3]) -> [f64; 3] {
    let length = ddot(a, a).sqrt();
    a.map(|x| x / length)
}
fn basis() -> [[f64; 3]; 3] {
    let forward = unit([-2.4, -3.6, 1.8]);
    let right = unit(dcross(forward, [0.0, 1.0, 0.0]));
    [right, dcross(right, forward), forward]
}
fn project(world: [f64; 3], size: (u32, u32)) -> [f64; 2] {
    project_at(world, size, [0.0, 0.2, 0.0])
}
fn project_at(world: [f64; 3], size: (u32, u32), player: [f64; 3]) -> [f64; 2] {
    let [right, up, forward] = basis();
    let delta = dsub(world, [player[0] + 2.4, player[1] + 3.6, player[2] - 1.8]);
    let z = ddot(delta, forward);
    assert!(z > 0.1 && z < 100.0);
    let tan =
        (20.0_f64.to_radians()).tan() * (2.0 / (f64::from(size.0) / f64::from(size.1))).min(1.0);
    let aspect = f64::from(size.0) / f64::from(size.1);
    [
        (1.0 + ddot(delta, right) / (z * tan * aspect)) * f64::from(size.0) * 0.5,
        (1.0 - ddot(delta, up) / (z * tan)) * f64::from(size.1) * 0.5,
    ]
}
fn turn(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn hull(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    let mut result = Vec::new();
    for point in &points {
        while result.len() >= 2
            && turn(result[result.len() - 2], result[result.len() - 1], *point) <= 0.0
        {
            result.pop();
        }
        result.push(*point);
    }
    let lower = result.len();
    for point in points.iter().rev().skip(1) {
        while result.len() > lower
            && turn(result[result.len() - 2], result[result.len() - 1], *point) <= 0.0
        {
            result.pop();
        }
        result.push(*point);
    }
    result.pop();
    assert!(result.len() >= 3);
    result
}
fn signed_margin(hull: &[[f64; 2]], point: [f64; 2]) -> f64 {
    hull.iter()
        .zip(hull.iter().cycle().skip(1))
        .take(hull.len())
        .map(|(&a, &b)| turn(a, b, point) / ((b[0] - a[0]).hypot(b[1] - a[1])))
        .fold(f64::INFINITY, f64::min)
}
fn object_hulls(
    model: &StaticModel,
    instances: &[StaticInstance],
    size: (u32, u32),
) -> Vec<Vec<[f64; 2]>> {
    assert_eq!(model.source().primitives.len(), 1);
    assert_eq!(model.source().primitives[0].transform, IDENTITY);
    instances
        .iter()
        .map(|instance| {
            assert_eq!(instance.rotation, [0.0, 0.0, 0.0, 1.0]);
            hull(
                model.source().primitives[0]
                    .vertices
                    .iter()
                    .map(|v| {
                        project(
                            std::array::from_fn(|i| {
                                f64::from(instance.translation[i])
                                    + f64::from(instance.scale[i]) * f64::from(v.position[i])
                            }),
                            size,
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}
fn isolated_pixels(
    state: &mut MaterialScene<Wgpu>,
    target: &orr_render::OffscreenTarget<Wgpu>,
    demo: &Demo,
    index: usize,
) -> Vec<u8> {
    state.placements.extract(demo).unwrap();
    state.scene.clear = [1.0, 0.0, 1.0, 1.0];
    let (renderer, instances) = match index {
        0 => (&mut state.floor, &state.placements.floor[..]),
        1 => (&mut state.player, &state.placements.player[..]),
        2 => (&mut state.pickups, &state.placements.pickups[..]),
        _ => panic!("bounded object index"),
    };
    state
        .scene
        .draw(
            ImportedSceneTarget {
                view: target.render_view(),
                size: target.size(),
                format: target.format(),
                sample_count: 1,
            },
            &grounded_view(player_position(demo), target.size(), demo.list.lighting)
                .unwrap()
                .0,
            &demo.list.lighting,
            &PointLightSettings::default(),
            &mut [ImportedBatch::StaticInstances {
                renderer,
                instances,
            }],
        )
        .unwrap();
    target.read_rgba8()
}

#[test]
#[ignore = "requires separately admitted actual GPU lane"]
fn actual_isolated_material_silhouettes_match_independent_projection() {
    let gpu = gpu();
    let target = orr_render::OffscreenTarget::new(&gpu, 320, 240, TextureFormat::Rgba8UnormSrgb);
    let initial_demo = Demo::new(Mode::Off);
    let corner_demo = moved_demo(Mode::Off, [1, 1]);
    let models = models().unwrap();
    let mut legacy =
        MaterialScene::new(&gpu, target.format(), Mode::Off, MaterialMode::Legacy).unwrap();
    let mut material =
        MaterialScene::new(&gpu, target.format(), Mode::Off, MaterialMode::Showcase).unwrap();
    let mut placements = Placements::new();
    for (index, model) in models.iter().enumerate() {
        let demo = if index == 0 {
            &corner_demo
        } else {
            &initial_demo
        };
        placements.extract(demo).unwrap();
        let instances = match index {
            0 => &placements.floor[..],
            1 => &placements.player[..],
            2 => &placements.pickups[..],
            _ => unreachable!(),
        };
        let hulls = if index == 0 {
            Vec::new()
        } else {
            object_hulls(model, instances, target.size())
        };
        let before = isolated_pixels(&mut legacy, &target, demo, index);
        let after = isolated_pixels(&mut material, &target, demo, index);
        let mask = |pixels: &[u8]| {
            pixels
                .chunks_exact(4)
                .map(|p| p != [255, 0, 255, 255])
                .collect::<Vec<_>>()
        };
        let legacy_mask = mask(&before);
        let material_mask = mask(&after);
        assert_eq!(
            legacy_mask, material_mask,
            "paired full silhouette index{index}"
        );
        let (mut interior, mut background) = (0, 0);
        for y in 0..240 {
            for x in 0..320 {
                let point = [f64::from(x) + 0.5, f64::from(y) + 0.5];
                let margins: Vec<_> = if index == 0 {
                    let hit = floor_hit(point, target.size(), player_position(demo).map(f64::from));
                    // A fixed world-space band avoids finite-plane edge raster uncertainty.
                    vec![(26.0 - hit[0].abs()).min(26.0 - hit[2].abs()) * 20.0]
                } else {
                    hulls.iter().map(|h| signed_margin(h, point)).collect()
                };
                let actual = material_mask[(y * 320 + x) as usize];
                // Predetermined 2px margin, derived only from authored geometry.
                if margins.iter().any(|m| *m > 2.0) {
                    interior += 1;
                    assert!(actual, "missing interior {index}/{x}/{y}");
                }
                if margins.iter().all(|m| *m < -2.0) {
                    background += 1;
                    assert!(!actual, "unexpected exterior {index}/{x}/{y}");
                }
            }
        }
        assert!(interior > 16 && background > 100);
        eprintln!(
            "material silhouette index={index} independent_interior={interior} independent_background={background} edge_margin_px=2"
        );
    }
}

fn fence_billboard(position: [f64; 3], size: f64, demo: &Demo, behind: bool) {
    let eye = [2.4, 3.8, -1.8];
    let [right, up, _] = basis();
    for x in [-1.0, 1.0] {
        for y in [-1.0, 1.0] {
            let corner =
                std::array::from_fn(|i| position[i] + (right[i] * x + up[i] * y) * size * 0.5);
            assert!(if behind {
                corner[1] < -0.6
            } else {
                corner[1] > 0.6
            });
            let ray = dsub(corner, eye);
            let t = -eye[1] / ray[1];
            assert!(if behind {
                (0.0..1.0).contains(&t)
            } else {
                t > 1.0
            });
            let hit: [f64; 3] = std::array::from_fn(|i| eye[i] + t * ray[i]);
            // Arena-sized floor, with a fixed 0.5 world-unit safety margin.
            assert!(hit[0].abs() < 25.5 && hit[2].abs() < 25.5);
            let screen = project(corner, (640, 480));
            assert!(screen[0] > 16.0 && screen[0] < 624.0 && screen[1] > 16.0 && screen[1] < 464.0);
            if !behind {
                for (_, actor) in demo
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .predicted()
                    .iter::<CollectActor>()
                {
                    if actor.active == 0 {
                        continue;
                    }
                    let center = [
                        f64::from(fp_to_f32(actor.position.x) * 0.1),
                        0.2,
                        f64::from(fp_to_f32(actor.position.y) * 0.1),
                    ];
                    let u = (ddot(dsub(center, eye), ray) / ddot(ray, ray)).clamp(0.0, 1.0);
                    let nearest = std::array::from_fn(|i| eye[i] + u * ray[i]);
                    let distance = dsub(nearest, center);
                    assert!(
                        ddot(distance, distance).sqrt() > 0.3,
                        "above control must avoid opaque spheres"
                    );
                }
            }
        }
    }
}
#[test]
#[ignore = "requires separately admitted actual GPU lane"]
fn actual_floor_occludes_behind_particle_but_keeps_above_control() {
    let gpu = gpu();
    let target = orr_render::OffscreenTarget::new(&gpu, 640, 480, TextureFormat::Rgba8UnormSrgb);
    let mut source = Demo::new(Mode::Sparks);
    for _ in 0..20 {
        source
            .update(
                Duration::from_millis(17),
                CollectInput {
                    x: FP::ONE,
                    ..Default::default()
                },
            )
            .unwrap();
        if !source.draws.is_empty() {
            break;
        }
    }
    let mut particle = source.draws[0];
    particle.size = 0.12;
    particle.color = [1.0; 4];
    let mut demo = Demo::new(Mode::Sparks);
    for mode in [MaterialMode::Legacy, MaterialMode::Showcase] {
        let mut renderer = MaterialScene::new(&gpu, target.format(), Mode::Sparks, mode).unwrap();
        demo.draws.clear();
        renderer
            .draw(target.render_view(), target.size(), &demo)
            .unwrap();
        let baseline = target.read_rgba8();
        for behind in [true, false] {
            let position = [-0.8, if behind { -0.7 } else { 0.7 }, 0.3];
            fence_billboard(position, 0.12, &demo, behind);
            particle.position =
                Vec3::new(position[0] as f32, position[1] as f32, position[2] as f32);
            demo.draws = vec![particle];
            renderer
                .draw(target.render_view(), target.size(), &demo)
                .unwrap();
            let pixels = target.read_rgba8();
            if behind {
                assert_eq!(
                    baseline, pixels,
                    "floor must occlude the complete behind billboard"
                );
            } else {
                assert!(
                    changed(&baseline, &pixels) > 4,
                    "above control must be visible"
                );
            }
        }
    }
}

#[test]
fn independent_projection_and_billboard_controls_have_fixed_margins() {
    let demo = Demo::new(Mode::Off);
    let center = project([0.0, 0.2, 0.0], (320, 240));
    assert!((center[0] - 160.0).abs() < 1e-10 && (center[1] - 120.0).abs() < 1e-10);
    let [right, up, _] = basis();
    let toward_right = project(
        std::array::from_fn(|i| [0.0, 0.2, 0.0][i] + right[i] * 0.2),
        (320, 240),
    );
    let toward_up = project(
        std::array::from_fn(|i| [0.0, 0.2, 0.0][i] + up[i] * 0.2),
        (320, 240),
    );
    assert!(toward_right[0] > center[0] + 5.0 && toward_up[1] < center[1] - 5.0);
    let square = hull(vec![
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 10.0],
        [0.0, 10.0],
        [5.0, 5.0],
    ]);
    assert_eq!(square.len(), 4);
    assert_eq!(signed_margin(&square, [5.0, 5.0]), 5.0);
    assert_eq!(signed_margin(&square, [-3.0, 5.0]), -3.0);
    assert_eq!(signed_margin(&square, [5.0, 0.0]), 0.0);
    for behind in [true, false] {
        fence_billboard(
            [-0.8, if behind { -0.7 } else { 0.7 }, 0.3],
            0.12,
            &demo,
            behind,
        );
    }
    // A displaced billboard is not accepted by the fixed floor-footprint gate.
    assert!(
        std::panic::catch_unwind(|| fence_billboard(
            [8.0, -0.7, 0.3],
            0.12,
            &Demo::new(Mode::Off),
            true
        ))
        .is_err()
    );
}

const EXTREMA: [[i64; 2]; 9] = [
    [0, 0],
    [-1, -1],
    [1, -1],
    [-1, 1],
    [1, 1],
    [-1, 0],
    [1, 0],
    [0, -1],
    [0, 1],
];
fn player_position(demo: &Demo) -> [f32; 3] {
    let mut placements = Placements::new();
    placements.extract(demo).unwrap();
    placements.player[0].translation
}
fn moved_demo(mode: Mode, direction: [i64; 2]) -> Demo {
    let mut demo = Demo::new(mode);
    if direction != [0, 0] {
        for _ in 0..254 {
            demo.update(
                Duration::from_millis(17),
                CollectInput {
                    x: FP(direction[0] << 16),
                    y: FP(direction[1] << 16),
                    ..Default::default()
                },
            )
            .unwrap();
        }
    }
    demo
}
fn plane_hit(point: [f64; 2], size: (u32, u32), player: [f64; 3], height: f64) -> [f64; 3] {
    let [right, up, forward] = basis();
    let aspect = f64::from(size.0) / f64::from(size.1);
    let tan = 20.0_f64.to_radians().tan() * (2.0 / aspect).min(1.0);
    let x = (2.0 * point[0] / f64::from(size.0) - 1.0) * tan * aspect;
    let y = (1.0 - 2.0 * point[1] / f64::from(size.1)) * tan;
    let ray: [f64; 3] = std::array::from_fn(|i| forward[i] + right[i] * x + up[i] * y);
    let eye = [player[0] + 2.4, player[1] + 3.6, player[2] - 1.8];
    assert!(ray[1] < -0.1);
    let t = (height - eye[1]) / ray[1];
    assert!(t > 0.0);
    std::array::from_fn(|i| eye[i] + ray[i] * t)
}
fn floor_hit(point: [f64; 2], size: (u32, u32), player: [f64; 3]) -> [f64; 3] {
    plane_hit(point, size, player, 0.0)
}
#[test]
fn grounded_camera_floor_and_shadow_bounds_cover_playable_extrema() {
    let light = orr_render::Lighting::default();
    let direction = unit(light.direction.map(f64::from));
    let side = unit(dcross(direction, [0.0, 1.0, 0.0]));
    let up = dcross(side, direction);
    let texel = 13.0 / 1024.0;
    let normal_bias = 19.5 / 1024.0;
    for dir in EXTREMA {
        let demo = moved_demo(Mode::Off, dir);
        let player = player_position(&demo);
        assert!((player[0] - dir[0] as f32 * 25.4).abs() < 1e-5);
        assert!((player[2] - dir[1] as f32 * 25.4).abs() < 1e-5);
        assert_eq!(player[1], 0.2);
        for size in [(640, 480), (1280, 720), (480, 800), (2560, 720)] {
            let (camera, actual_light) = grounded_view(player, size, demo.list.lighting).unwrap();
            assert!(!demo.list.lighting.shadows);
            assert!(actual_light.shadows);
            let clip = camera
                .view_proj(size.0 as f32 / size.1 as f32)
                .transform_point4(player);
            assert!((clip[0] / clip[3]).abs() < 1e-5 && (clip[1] / clip[3]).abs() < 1e-5);
            let center = player.map(f64::from);
            let snapped: [f64; 3] =
                [side, up, direction].map(|axis| (ddot(center, axis) / texel).round() * texel);
            let matrix = orr_render::light_view_proj(&actual_light, 1024);
            for x in [0.0, f64::from(size.0)] {
                for y in [0.0, f64::from(size.1)] {
                    for height in [0.0, 0.4] {
                        let hit = plane_hit([x, y], size, center, height);
                        // Ground and 0.4-high caster slice, plus full directional projection displacement.
                        for travel in [-0.4 / -direction[1], 0.0, 0.4 / -direction[1]] {
                            let p: [f64; 3] = std::array::from_fn(|i| {
                                hit[i]
                                    + direction[i] * travel
                                    + if i == 1 { normal_bias } else { 0.0 }
                            });
                            let independent = [
                                (ddot(p, side) - snapped[0]) / 6.5,
                                (ddot(p, up) - snapped[1]) / 6.5,
                            ];
                            let actual = matrix.transform_point4(p.map(|v| v as f32));
                            assert!((f64::from(actual[0]) - independent[0]).abs() < 2e-5);
                            assert!((f64::from(actual[1]) - independent[1]).abs() < 2e-5);
                            // Two texels include 3x3 PCF plus comparison-linear support.
                            assert!(
                                independent
                                    .iter()
                                    .all(|v| v.abs() + 2.0 * texel / 6.5 < 1.0)
                            );
                            assert!(actual[2] > 0.01 && actual[2] < 0.99);
                        }
                    }
                }
            }
        }
        if dir == [0, 0] || (dir[0] != 0 && dir[1] != 0) {
            let (shadow, lit) = grounding_regions(&demo);
            eprintln!(
                "geometry-only grounding pose={dir:?} eligible={} lit={} pixel_queries=307200",
                shadow.len(),
                lit.len()
            );
            assert!(
                shadow.len() >= 8 && lit.len() >= 500,
                "inconclusive authored camera/light: fixed receiver regions are empty or insufficient"
            );
        }
        // Sphere bounds and full 0.4-height shadow footprint stay on the finite floor.
        for axis in [0, 2] {
            let displacement = 0.4 * (direction[axis] / direction[1]).abs();
            assert!(f64::from(player[axis]).abs() + 0.2 + displacement < 26.0);
        }
    }
    for size in [(0, 480), (640, 0), (8193, 480), (640, 8193)] {
        assert_eq!(
            grounded_view([0.0, 0.2, 0.0], size, light).unwrap_err(),
            orr_render::ImportedSceneError::InvalidTarget.to_string()
        );
    }
}
#[test]
fn grounded_follow_preserves_authoritative_movement_events_and_restart() {
    for direction in EXTREMA {
        let mut traces: Vec<_> = [Mode::Off, Mode::Dust, Mode::Sparks]
            .into_iter()
            .flat_map(|mode| {
                [MaterialMode::Legacy, MaterialMode::Showcase]
                    .into_iter()
                    .map(move |material| (Demo::new(mode), material))
            })
            .collect();
        for _ in 0..254 {
            let mut expected = None;
            for (demo, _material) in &mut traces {
                demo.update(
                    Duration::from_millis(17),
                    CollectInput {
                        x: FP(direction[0] << 16),
                        y: FP(direction[1] << 16),
                        ..Default::default()
                    },
                )
                .unwrap();
                let before = demo.snapshot.as_ref().unwrap().predicted().checksum();
                let draws = demo.draws.clone();
                let player = player_position(demo);
                let _ = grounded_view(player, (640, 480), demo.list.lighting).unwrap();
                assert_eq!(
                    before,
                    demo.snapshot.as_ref().unwrap().predicted().checksum()
                );
                assert_eq!(draws, demo.draws);
                let state = (before, Showcase::score(demo));
                if let Some(expected) = expected {
                    assert_eq!(state, expected);
                } else {
                    expected = Some(state);
                }
            }
        }
        for (demo, _) in &mut traces {
            let mut clock = Showcase::default();
            clock.restart(demo);
            assert_eq!(player_position(demo), [0.0, 0.2, 0.0]);
            let before = demo.snapshot.as_ref().unwrap().predicted().checksum();
            demo.update(Duration::ZERO, CollectInput::default())
                .unwrap();
            assert_eq!(
                before,
                demo.snapshot.as_ref().unwrap().predicted().checksum()
            );
            assert_eq!(Showcase::score(demo), 0);
        }
    }
}

fn shadow_floor_hulls(demo: &Demo) -> Vec<Vec<[f64; 2]>> {
    let mut placements = Placements::new();
    placements.extract(demo).unwrap();
    let vertices = sphere().0;
    let d = demo.list.lighting.direction.map(f64::from);
    placements
        .player
        .iter()
        .chain(&placements.pickups)
        .map(|instance| {
            hull(
                vertices
                    .iter()
                    .map(|v| {
                        let world: [f64; 3] = std::array::from_fn(|i| {
                            f64::from(instance.translation[i])
                                + f64::from(instance.scale[i]) * f64::from(v.position[i])
                        });
                        [
                            world[0] - world[1] * d[0] / d[1],
                            world[2] - world[1] * d[2] / d[1],
                        ]
                    })
                    .collect(),
            )
        })
        .collect()
}
fn grounding_regions(demo: &Demo) -> (Vec<usize>, Vec<usize>) {
    let player = player_position(demo).map(f64::from);
    let shadows = shadow_floor_hulls(demo);
    let mut placements = Placements::new();
    placements.extract(demo).unwrap();
    let sphere_vertices = sphere().0;
    let eye = [player[0] + 2.4, player[1] + 3.6, player[2] - 1.8];
    let visible: Vec<_> = placements
        .player
        .iter()
        .chain(&placements.pickups)
        .filter(|instance| ddot(dsub(instance.translation.map(f64::from), eye), basis()[2]) > 0.4)
        .map(|instance| {
            hull(
                sphere_vertices
                    .iter()
                    .map(|v| {
                        project_at(
                            std::array::from_fn(|i| {
                                f64::from(instance.translation[i])
                                    + f64::from(instance.scale[i]) * f64::from(v.position[i])
                            }),
                            (640, 480),
                            player,
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    let (mut shadow, mut lit) = (Vec::new(), Vec::new());
    for y in 0..480 {
        for x in 0..640 {
            let screen = [f64::from(x) + 0.5, f64::from(y) + 0.5];
            let p = floor_hit(screen, (640, 480), player);
            if p[0].abs() > 25.8
                || p[2].abs() > 25.8
                || visible.iter().any(|h| signed_margin(h, screen) > -2.0)
            {
                continue;
            }
            let i = ((y * 640 + x) * 4) as usize;
            let margins: Vec<_> = shadows
                .iter()
                .map(|h| signed_margin(h, [p[0], p[2]]))
                .collect();
            if margins[0] > 0.06 {
                shadow.push(i);
            }
            if margins.iter().all(|m| *m < -0.12) {
                lit.push(i);
            }
        }
    }
    (shadow, lit)
}
fn assert_grounding_pixels(off: &[u8], on: &[u8], demo: &Demo) {
    let (shadow, lit) = grounding_regions(demo);
    assert!(
        shadow.len() >= 8 && lit.len() >= 500,
        "inconclusive fixed geometry regions"
    );
    let mut dark = 0;
    for i in &shadow {
        let a = &off[*i..*i + 4];
        let b = &on[*i..*i + 4];
        let before: u16 = a[..3].iter().map(|&v| u16::from(v)).sum();
        let after: u16 = b[..3].iter().map(|&v| u16::from(v)).sum();
        assert!(
            after <= before + 3,
            "shadow interior unexpectedly brightened"
        );
        if before >= after + 2 {
            dark += 1;
        }
    }
    for i in &lit {
        assert!(
            off[*i..*i + 4]
                .iter()
                .zip(&on[*i..*i + 4])
                .all(|(&a, &b)| a.abs_diff(b) <= 1),
            "lit floor control changed"
        );
    }
    assert!(dark >= 8, "fixed eligible shadow interiors did not darken");
    eprintln!(
        "grounding eligible={} dark={dark} lit={} shadow_inset_world=0.06 lit_outset_world=0.12 object_band_px=2",
        shadow.len(),
        lit.len()
    );
}

struct GroundCapture {
    root: std::path::PathBuf,
    manifest: std::fs::File,
    frames: usize,
    bytes: u64,
}
impl GroundCapture {
    fn new(gpu: &Wgpu) -> Self {
        use std::io::Write;
        let root = std::path::PathBuf::from(
            std::env::var_os("ORR_GROUNDING_CAPTURE_DIR")
                .expect("private grounding capture directory"),
        );
        assert!(root.is_absolute() && root.is_dir());
        assert_eq!(root.canonicalize().unwrap(), root);
        assert!(std::fs::read_dir(&root).unwrap().next().is_none());
        let mut manifest = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("manifest.txt"))
            .unwrap();
        let info = gpu.adapter().get_info();
        writeln!(manifest,"STATE=RUNNING partial is not acceptance; size=640x480 Rgba8UnormSrgb samples=1; adapter name={:?} backend={:?} device_type={:?} vendor={} device={} driver={:?} driver_info={:?} software={}",info.name,info.backend,info.device_type,info.vendor,info.device,info.driver,info.driver_info,gpu.is_software()).unwrap();
        manifest.flush().unwrap();
        Self {
            root,
            manifest,
            frames: 0,
            bytes: 0,
        }
    }
    fn frame(&mut self, name: &str, rgba: &[u8], demo: &Demo) {
        use std::io::Write;
        assert_eq!(rgba.len(), 640 * 480 * 4);
        assert!(name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
        let raw = self.root.join(format!("{name}.rgba"));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&raw)
            .unwrap()
            .write_all(rgba)
            .unwrap();
        // Persist raw data before encoding or pixel assertions.
        let png_path = self.root.join(format!("{name}.png"));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&png_path)
            .unwrap();
        let mut encoder = png::Encoder::new(file, 640, 480);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
        writer.finish().unwrap();
        let encoded = std::fs::read(&png_path).unwrap();
        assert!(encoded.len() <= 2 * 1024 * 1024);
        self.bytes += rgba.len() as u64 + encoded.len() as u64;
        assert!(self.bytes <= 48 * 1024 * 1024);
        self.frames += 1;
        writeln!(
            self.manifest,
            "frame={name} raw={} png={} player={:?} checksum={:016x} score={} particles={}",
            hex(&Sha256::digest(rgba)),
            hex(&Sha256::digest(encoded)),
            player_position(demo),
            demo.snapshot.as_ref().unwrap().predicted().checksum(),
            Showcase::score(demo),
            demo.draws.len()
        )
        .unwrap();
        self.manifest.flush().unwrap();
    }
    fn finish(mut self) {
        use std::io::Write;
        assert_eq!(self.frames, 18);
        writeln!(self.manifest, "STATE=PASS frames=18 visual_review_pending").unwrap();
        self.manifest.flush().unwrap();
    }
}
#[test]
#[ignore = "requires reviewed actual GPU grounding capture lease"]
fn actual_grounded_scene_tracks_player_and_preserves_shadow_controls() {
    let gpu = gpu();
    let target = orr_render::OffscreenTarget::new(&gpu, 640, 480, TextureFormat::Rgba8UnormSrgb);
    let mut capture = GroundCapture::new(&gpu);
    let mut scene =
        MaterialScene::new(&gpu, target.format(), Mode::Off, MaterialMode::Showcase).unwrap();
    let mut initial = Vec::new();
    for (name, direction) in [
        ("initial", [0, 0]),
        ("corner-nw", [-1, -1]),
        ("corner-ne", [1, -1]),
        ("corner-sw", [-1, 1]),
        ("corner-se", [1, 1]),
    ] {
        let demo = moved_demo(Mode::Off, direction);
        let checksum = demo.snapshot.as_ref().unwrap().predicted().checksum();
        scene
            .draw_grounded(target.render_view(), target.size(), &demo, false)
            .unwrap();
        let off = target.read_rgba8();
        capture.frame(&format!("{name}-off"), &off, &demo);
        scene
            .draw(target.render_view(), target.size(), &demo)
            .unwrap();
        let on = target.read_rgba8();
        capture.frame(&format!("{name}-on"), &on, &demo);
        assert_grounding_pixels(&off, &on, &demo);
        assert_eq!(
            checksum,
            demo.snapshot.as_ref().unwrap().predicted().checksum()
        );
        if name == "initial" {
            initial = on;
        }
    }
    let mut demo = Demo::new(Mode::Off);
    for name in ["initial-repeat-on", "initial-restart-on"] {
        if name == "initial-restart-on" {
            let mut clock = Showcase::default();
            demo = moved_demo(Mode::Off, [1, 0]);
            clock.restart(&mut demo);
        }
        scene
            .draw(target.render_view(), target.size(), &demo)
            .unwrap();
        let raw = target.read_rgba8();
        capture.frame(name, &raw, &demo);
        assert_eq!(raw, initial);
    }
    let mut demo = Demo::new(Mode::Sparks);
    let mut clock = Showcase::default();
    for _ in 0..20 {
        clock
            .update(&mut demo, Duration::from_millis(17), true)
            .unwrap();
        if clock.pickup_observed_at.is_some() {
            break;
        }
    }
    let anchor = clock.pickup_observed_at.expect("real pickup observed");
    assert_eq!(Showcase::score(&demo), 1);
    let mut particles =
        MaterialScene::new(&gpu, target.format(), Mode::Sparks, MaterialMode::Showcase).unwrap();
    for milliseconds in [100, 250, 700] {
        let destination = anchor + Duration::from_millis(milliseconds);
        while clock.elapsed < destination {
            clock
                .update(
                    &mut demo,
                    Duration::from_millis(17).min(destination - clock.elapsed),
                    true,
                )
                .unwrap();
        }
        scene
            .draw(target.render_view(), target.size(), &demo)
            .unwrap();
        let off = target.read_rgba8();
        capture.frame(
            &format!("pickup-{milliseconds}ms-particles-off"),
            &off,
            &demo,
        );
        particles
            .draw(target.render_view(), target.size(), &demo)
            .unwrap();
        let on = target.read_rgba8();
        capture.frame(&format!("pickup-{milliseconds}ms-particles-on"), &on, &demo);
        if milliseconds == 700 {
            assert!(demo.draws.is_empty());
            assert_eq!(off, on);
        } else {
            assert_eq!(demo.draws.len(), 16);
            assert!(changed(&off, &on) > 0);
        }
    }
    capture.finish();
}
