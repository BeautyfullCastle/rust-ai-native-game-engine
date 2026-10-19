use super::*;
use orr_view::particles::{EmitterDesc, ParticleBlend};

const STEP: Duration = Duration::from_millis(17);
fn authored() -> EmitterDesc {
    EmitterDesc {
        origin: Vec3::new(999.0, 999.0, 999.0),
        seed: 987654,
        velocity: Vec3::ZERO,
        velocity_jitter: Vec3::ZERO,
        burst: 24,
        rate: 0.0,
        duration: 0.0,
        lifetime: 0.4,
        size_start: 1.2,
        size_end: 0.4,
        color_start: [0.1, 1.0, 0.3, 1.0],
        color_end: [0.1, 0.3, 1.0, 0.0],
        blend: ParticleBlend::Alpha,
    }
}
fn checksum(demo: &Demo) -> u64 {
    demo.snapshot.as_ref().unwrap().predicted().checksum()
}
fn step(demos: &mut [Demo; 4], clocks: &mut [Showcase; 4], dt: Duration) {
    for (demo, clock) in demos.iter_mut().zip(clocks.iter_mut()) {
        clock.update(demo, dt, true).unwrap();
    }
    for demo in &demos[1..] {
        assert_eq!(checksum(&demos[0]), checksum(demo));
    }
    assert!(demos[0].draws.is_empty());
}
fn demos(custom: EmitterDesc) -> [Demo; 4] {
    [
        Demo::new(Mode::Off),
        Demo::new(Mode::Dust),
        Demo::new(Mode::Sparks),
        Demo::with_preset(custom),
    ]
}
#[test]
fn custom_pickup_checksums_origin_dedup_pause_expiry_and_restart() {
    let mut demos = demos(authored());
    let mut clocks = std::array::from_fn(|_| Showcase::default());
    let mut seen_cycles = [false; 3];
    for _ in 0..300 {
        step(&mut demos, &mut clocks, STEP);
        let custom = &mut demos[3];
        if !custom.draws.is_empty() {
            seen_cycles[clocks[3].completed_cycles as usize] = true;
            assert_eq!(custom.draws.len(), 24);
            assert!(custom
                .draws
                .iter()
                .all(|d| d.position == origin(0).unwrap()));
            assert!(custom.draws.iter().all(|d| d.blend == ParticleBlend::Alpha));
            let before = (checksum(custom), custom.draws.clone(), clocks[3].elapsed);
            clocks[3].update(custom, Duration::MAX, false).unwrap();
            for _ in 0..3 {
                clocks[3].update(custom, Duration::ZERO, true).unwrap();
            }
            assert_eq!(
                before,
                (checksum(custom), custom.draws.clone(), clocks[3].elapsed)
            );
        }
        if clocks[3].elapsed > Duration::from_secs(1) {
            assert!(custom.draws.is_empty());
        }
    }
    assert!(seen_cycles.into_iter().all(|seen| seen));
    let custom = &mut demos[3];
    custom.restart();
    assert_eq!(custom.custom, Some(authored()));
    assert!(custom.draws.is_empty());
    assert_eq!(checksum(custom), checksum(&Demo::new(Mode::Off)));
    // A fresh session repeats the same genuine pickup seed, regardless of a supplied template seed.
    let jittered = EmitterDesc {
        velocity_jitter: Vec3::new(0.7, 0.2, 0.7),
        ..authored()
    };
    *custom = Demo::with_preset(jittered);
    let mut control = Demo::with_preset(EmitterDesc {
        seed: 1,
        origin: Vec3::ZERO,
        ..jittered
    });
    for _ in 0..10 {
        let input = CollectInput {
            x: FP::ONE,
            ..Default::default()
        };
        custom.update(STEP, input).unwrap();
        control.update(STEP, input).unwrap();
    }
    assert!(!custom.draws.is_empty());
    assert_eq!(custom.draws, control.draws);
}

#[test]
fn preset_cli_rejects_invalid_inputs_before_constructing_runtime() {
    for args in [
        vec![],
        vec!["--preset"],
        vec!["--preset", "a", "--preset", "b"],
        vec!["--particles", "off", "--preset", "a"],
        vec!["--preset", "a", "--particles", "sparks"],
    ] {
        assert!(PresetOptions::parse(args.into_iter().map(str::to_string).collect()).is_err());
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("invalid.json");
    std::fs::write(&file, b"{}").unwrap();
    assert!(PresetOptions::parse(vec![
        "--preset".into(),
        file.display().to_string(),
        "--headless".into()
    ])
    .is_err());
    assert!(PresetOptions::parse(vec![
        "--preset".into(),
        dir.path().join("missing.json").display().to_string()
    ])
    .is_err());
    let legacy = PresetOptions::parse(vec!["--particles".into(), "off".into()]).unwrap();
    assert_eq!(legacy.options.mode, Mode::Off);
    assert!(legacy.preset.is_none());
}

fn save_frame(directory: &Option<std::path::PathBuf>, name: &str, pixels: &[u8]) {
    assert_eq!(pixels.len(), 320 * 240 * 4);
    if let Some(directory) = directory {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))
            .unwrap();
        let mut encoder = png::Encoder::new(file, 320, 240);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
        writer.finish().unwrap();
    }
}

#[test]
#[ignore = "requires actual GPU and exact editor UI-saved acceptance file"]
fn ui_saved_preset_renders_and_expires_in_real_sample() {
    let path = std::env::var("ORR_PARTICLE_PRESET_ACCEPTANCE_FILE")
        .expect("editor UI-saved file is mandatory; no synthetic fallback");
    let selected =
        PresetOptions::parse(vec!["--preset".into(), path.clone(), "--showcase".into()]).unwrap();
    let custom = selected.preset.expect("custom preset selected");
    let expected = authored();
    assert_eq!(custom.burst, expected.burst);
    assert_eq!(custom.velocity, expected.velocity);
    assert_eq!(custom.velocity_jitter, expected.velocity_jitter);
    assert_eq!(
        (custom.rate, custom.duration, custom.lifetime),
        (expected.rate, expected.duration, expected.lifetime)
    );
    assert_eq!(
        (custom.size_start, custom.size_end),
        (expected.size_start, expected.size_end)
    );
    assert_eq!(
        (custom.color_start, custom.color_end, custom.blend),
        (expected.color_start, expected.color_end, expected.blend)
    );
    let directory =
        std::env::var_os("ORR_PARTICLE_PRESET_CAPTURE_DIR").map(std::path::PathBuf::from);
    if let Some(directory) = &directory {
        assert!(directory.is_absolute());
        assert_eq!(directory.canonicalize().unwrap(), *directory);
        assert!(directory.symlink_metadata().unwrap().is_dir());
    }
    let gpu =
        Wgpu::headless(WgpuOptions::default()).expect("required sample GPU adapter unavailable");
    let target = orr_render::OffscreenTarget::new(&gpu, 320, 240, TextureFormat::Rgba8Unorm);
    let mut renderers = [Mode::Off, Mode::Dust, Mode::Sparks, Mode::Sparks]
        .map(|mode| DemoRenderer::new(&gpu, target.format(), mode).unwrap());
    let mut demos = demos(custom);
    let mut clocks = std::array::from_fn(|_| Showcase::default());
    for _ in 0..20 {
        step(&mut demos, &mut clocks, STEP);
        if clocks[0].pickup_observed_at.is_some() {
            break;
        }
    }
    let anchor = clocks[0]
        .pickup_observed_at
        .expect("genuine pickup required");
    let mut records = Vec::new();
    let mut previous = None;
    for ms in [0, 100, 250, 700] {
        let destination = anchor + Duration::from_millis(ms);
        while clocks[0].elapsed < destination {
            let dt = STEP.min(destination - clocks[0].elapsed);
            step(&mut demos, &mut clocks, dt);
        }
        let pixels: Vec<_> = renderers
            .iter_mut()
            .zip(&demos)
            .map(|(renderer, demo)| {
                renderer
                    .draw(target.render_view(), target.size(), demo)
                    .unwrap();
                target.read_rgba8()
            })
            .collect();
        let changed = |index: usize| {
            pixels[index]
                .chunks_exact(4)
                .zip(pixels[0].chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count()
        };
        if ms < 400 {
            assert_eq!(demos[3].draws.len(), 24);
            assert!(
                changed(3) > changed(1),
                "custom larger silhouette must exceed dust footprint"
            );
            assert!(
                changed(3) > changed(2),
                "custom larger silhouette must exceed sparks footprint"
            );
            assert_ne!(pixels[3], pixels[1]);
            assert_ne!(pixels[3], pixels[2]);
            if ms == 0 {
                assert!(
                    pixels[3]
                        .chunks_exact(4)
                        .zip(pixels[0].chunks_exact(4))
                        .any(|(a, b)| a != b && a[1] > a[0] && a[1] > a[2]),
                    "authored green color must be visible"
                );
            }
            if let Some(before) = &previous {
                assert_ne!(&pixels[3], before, "custom shape/color evolves over time");
            }
        } else {
            assert!(demos[3].draws.is_empty());
            assert_eq!(
                pixels[3], pixels[0],
                "natural expiry restores exact scene baseline"
            );
        }
        previous = Some(pixels[3].clone());
        save_frame(&directory, &format!("custom-{ms:03}ms.png"), &pixels[3]);
        if ms == 0 {
            for (i, name) in ["off", "dust", "sparks"].iter().enumerate() {
                save_frame(&directory, &format!("{name}-000ms.png"), &pixels[i]);
            }
        }
        records.push(format!(
            "after_pickup_ms={ms} changed_pixels={} particles={} checksum={:016x}",
            changed(3),
            demos[3].draws.len(),
            checksum(&demos[3])
        ));
    }
    let evidence = format!(
        "Production PresetOptions loader -> real Collect inputs/events -> existing DemoRenderer\nfile={path}\nadapter={} software={}\n{}\n",
        gpu.adapter_name(),
        gpu.is_software(),
        records.join("\n")
    );
    eprintln!("{evidence}");
    if let Some(directory) = directory {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("runtime-evidence.txt"))
            .unwrap()
            .write_all(evidence.as_bytes())
            .unwrap();
    }
}
