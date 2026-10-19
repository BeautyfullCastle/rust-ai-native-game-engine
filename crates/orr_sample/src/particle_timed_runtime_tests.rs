//! Schema-2 acceptance through the unchanged production loader and Collect runtime.
use super::*;
use orr_view::particles::{EmitterDesc, ParticleBlend};

// Exactly representable: counts test floor(elapsed * rate), not frame-rate independence.
const STEP: Duration = Duration::from_micros(31_250);
fn authored() -> EmitterDesc {
    EmitterDesc {
        origin: Vec3::new(999.0, 999.0, 999.0),
        seed: 987654,
        velocity: Vec3::ZERO,
        velocity_jitter: Vec3::ZERO,
        burst: 0,
        rate: 4.0,
        duration: 1.0,
        lifetime: 0.125,
        size_start: 2.0,
        size_end: 1.0,
        color_start: [1.0, 0.0, 0.0, 1.0],
        color_end: [1.0, 0.0, 0.0, 1.0],
        blend: ParticleBlend::Alpha,
    }
}
fn checksum(demo: &Demo) -> u64 {
    demo.snapshot.as_ref().unwrap().predicted().checksum()
}
fn demos(custom: EmitterDesc) -> [Demo; 4] {
    [
        Demo::new(Mode::Off),
        Demo::new(Mode::Dust),
        Demo::new(Mode::Sparks),
        Demo::with_preset(custom),
    ]
}
fn step(demos: &mut [Demo; 4], clocks: &mut [Showcase; 4]) {
    for (demo, clock) in demos.iter_mut().zip(clocks.iter_mut()) {
        clock.update(demo, STEP, true).unwrap();
    }
    for i in 1..4 {
        assert_eq!(checksum(&demos[0]), checksum(&demos[i]));
        assert_eq!(clocks[0].pickup_observed_at, clocks[i].pickup_observed_at);
        assert_eq!(clocks[0].completed_cycles, clocks[i].completed_cycles);
    }
    assert!(demos[0].draws.is_empty());
}
fn pickup(demos: &mut [Demo; 4], clocks: &mut [Showcase; 4]) {
    for _ in 0..20 {
        assert!(demos[3].draws.is_empty(), "no pre-event emission");
        assert_eq!(demos[3].particles.emitter_count(), 0);
        step(demos, clocks);
        if clocks[3].pickup_observed_at.is_some() {
            assert_eq!(Showcase::score(&demos[3]), 1);
            assert!(
                demos[3].draws.is_empty(),
                "zero burst stays initially empty"
            );
            assert_eq!(demos[3].particles.emitter_count(), 1);
            return;
        }
    }
    panic!("genuine Collect pickup required");
}
fn load(path: &std::path::Path) -> EmitterDesc {
    let selected = PresetOptions::parse(vec![
        "--preset".into(),
        path.display().to_string(),
        "--showcase".into(),
    ])
    .unwrap();
    assert!(selected.options.showcase);
    let custom = selected
        .preset
        .expect("production loader must select custom preset");
    assert_eq!(
        custom,
        EmitterDesc {
            origin: Vec3::ZERO,
            seed: 0,
            ..authored()
        }
    );
    custom
}

#[test]
fn timed_file_uses_production_loader_and_real_pickup_lifecycle() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("timed.json");
    std::fs::write(&path, br#"{"schema":2,"burst":0,"rate":4.0,"duration":1.0,"lifetime":0.125,"velocity":[0,0,0],"velocity_jitter":[0,0,0],"size_start":2,"size_end":1,"color_start":[1,0,0,1],"color_end":[1,0,0,1],"blend":"alpha"}"#).unwrap();
    let custom = load(&path);
    let mut demos = demos(custom);
    let mut clocks = std::array::from_fn(|_| Showcase::default());
    // A stationary live session never creates effects merely because a timed file was loaded.
    for _ in 0..8 {
        for demo in &mut demos {
            demo.update(STEP, CollectInput::default()).unwrap();
        }
        assert!(demos.iter().all(|d| d.draws.is_empty()));
        assert_eq!(demos[3].particles.emitter_count(), 0);
        assert!(demos.iter().all(|d| checksum(d) == checksum(&demos[0])));
    }
    for demo in &mut demos {
        demo.restart();
    }
    for cycle in 0..3 {
        pickup(&mut demos, &mut clocks);
        let mut births = 0;
        for age_step in 1..=36 {
            if age_step > 1 {
                step(&mut demos, &mut clocks);
            }
            let expected = usize::from((8..36).contains(&age_step) && age_step % 8 < 4);
            assert_eq!(demos[3].draws.len(), expected, "age_step={age_step}");
            assert_eq!(
                demos[3].particles.emitter_count(),
                usize::from(age_step < 32)
            );
            if age_step % 8 == 0 && age_step <= 32 {
                births += demos[3].draws.len();
            }
            assert!(demos[3]
                .draws
                .iter()
                .all(|d| d.position == origin(0).unwrap()));
            // Repeated genuine bridge polls/confirmations cannot restart or duplicate emission.
            let before = (
                checksum(&demos[3]),
                demos[3].draws.clone(),
                clocks[3].elapsed,
            );
            clocks[3]
                .update(&mut demos[3], Duration::MAX, false)
                .unwrap();
            for _ in 0..3 {
                clocks[3]
                    .update(&mut demos[3], Duration::ZERO, true)
                    .unwrap();
            }
            assert_eq!(
                before,
                (
                    checksum(&demos[3]),
                    demos[3].draws.clone(),
                    clocks[3].elapsed
                )
            );
        }
        assert_eq!(births, 4, "exactly one emission sequence per pickup");
        assert_eq!(demos[3].custom, Some(custom));
        if cycle < 2 {
            while clocks[3].elapsed < Showcase::CYCLE {
                step(&mut demos, &mut clocks);
            }
            step(&mut demos, &mut clocks);
            assert_eq!(clocks[3].completed_cycles, cycle + 1);
            assert!(demos[3].draws.is_empty());
            assert_eq!(demos[3].particles.emitter_count(), 0);
        }
    }
    clocks[3].restart(&mut demos[3]);
    assert_eq!(demos[3].custom, Some(custom));
    assert_eq!(demos[3].particles.emitter_count(), 0);
    assert!(demos[3].draws.is_empty());
    assert_eq!(checksum(&demos[3]), checksum(&Demo::new(Mode::Off)));
}

#[test]
fn timed_pickup_overrides_template_origin_and_seed_with_nonzero_jitter() {
    let desc = EmitterDesc {
        velocity_jitter: Vec3::new(0.7, 0.2, 0.7),
        ..authored()
    };
    let mut demos = demos(desc);
    let mut clocks = std::array::from_fn(|_| Showcase::default());
    pickup(&mut demos, &mut clocks);
    for _ in 0..8 {
        step(&mut demos, &mut clocks);
    }
    assert_eq!(demos[3].draws.len(), 1);
    let draw = demos[3].draws[0];
    assert_ne!(
        draw.position,
        origin(0).unwrap(),
        "jitter must affect the comparison"
    );
    let mut expected = ParticlePool::new();
    expected
        .start(
            draw.key,
            Some(EmitterDesc {
                origin: origin(0).unwrap(),
                seed: 42,
                ..desc
            }),
        )
        .unwrap();
    expected.advance(0.25).unwrap();
    expected.advance(STEP.as_secs_f32()).unwrap();
    assert_eq!(demos[3].draws, expected.draws().collect::<Vec<_>>());
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
#[ignore = "requires actual GPU and exact editor UI-saved schema-2 acceptance file"]
fn ui_saved_timed_preset_renders_delayed_gap_and_final_expiry() {
    let path = std::path::PathBuf::from(
        std::env::var("ORR_PARTICLE_TIMED_ACCEPTANCE_FILE")
            .expect("actual UI-saved file required; no synthetic fallback"),
    );
    let before = std::fs::read(&path).unwrap();
    let custom = load(&path);
    let digest = std::process::Command::new("sha256sum")
        .arg("--")
        .arg(&path)
        .output()
        .expect("sha256sum required for exact-file evidence");
    assert!(digest.status.success());
    let digest = String::from_utf8(digest.stdout).unwrap();
    let directory = std::env::var_os("ORR_PARTICLE_TIMED_EVIDENCE").map(std::path::PathBuf::from);
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
    pickup(&mut demos, &mut clocks);
    let mut records = Vec::new();
    let mut age_step = 1;
    for (destination, label, count) in [
        (1, "initial-empty", 0),
        (8, "delayed-emission", 1),
        (12, "empty-gap", 0),
        (16, "second-emission", 1),
        (32, "final-emission", 1),
        (36, "final-expiry", 0),
    ] {
        while age_step < destination {
            step(&mut demos, &mut clocks);
            age_step += 1;
        }
        assert_eq!(demos[3].draws.len(), count);
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
        let changed = pixels[3]
            .chunks_exact(4)
            .zip(pixels[0].chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        if count == 0 {
            assert_eq!(pixels[3], pixels[0], "{label} restores exact baseline");
        } else {
            assert!(changed > 0, "{label} must produce visible particles");
            assert!(
                pixels[3]
                    .chunks_exact(4)
                    .zip(pixels[0].chunks_exact(4))
                    .any(|(a, b)| a != b && a[0] > a[1] && a[0] > a[2]),
                "authored red must be visible"
            );
            assert_ne!(pixels[3], pixels[1]);
            assert_ne!(pixels[3], pixels[2]);
        }
        save_frame(&directory, &format!("timed-{label}.png"), &pixels[3]);
        save_frame(&directory, &format!("timed-{label}-off.png"), &pixels[0]);
        records.push(format!("stage={label} emitter_age_steps={age_step} changed_pixels={changed} particles={count} checksum={:016x}", checksum(&demos[3])));
    }
    assert_eq!(demos[3].particles.emitter_count(), 0);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "same UI-saved bytes throughout acceptance"
    );
    let evidence = format!("Production PresetOptions -> genuine Collect pickup -> existing DemoRenderer\nfile={}\nsha256={}\nadapter={} software={}\n{}\n", path.display(), digest.trim(), gpu.adapter_name(), gpu.is_software(), records.join("\n"));
    eprintln!("{evidence}");
    if let Some(directory) = directory {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("timed-runtime-evidence.txt"))
            .unwrap()
            .write_all(evidence.as_bytes())
            .unwrap();
    }
}
