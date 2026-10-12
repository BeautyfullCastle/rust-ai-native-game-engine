use super::*;
#[test]
fn composed_geometry_settings_request_one_sample() {
    let settings = geometry_settings();
    let defaults = orr_render::Settings3D::default();
    assert_eq!(settings.msaa, 1);
    assert_eq!(settings.shadow_map_size, defaults.shadow_map_size);
    assert_eq!(settings.mesh_segments, defaults.mesh_segments);
}
#[test]
fn options_require_explicit_mode_and_reject_duplicate_or_unknown_flags() {
    for args in [
        vec![],
        vec!["--particles", "dust", "--showcase", "--showcase"],
        vec!["--particles"],
        vec!["--particles", "auto"],
        vec!["--particles", "off", "--particles", "sparks"],
        vec!["--particles", "dust", "--headless", "--headless"],
    ] {
        assert!(Options::parse(args.into_iter().map(str::to_string).collect()).is_err());
    }
    let options = Options::parse(vec![
        "--particles".into(),
        "sparks".into(),
        "--headless".into(),
    ])
    .unwrap();
    assert_eq!(options.mode, Mode::Sparks);
    assert!(options.headless);
    assert!(!options.showcase);
    assert!(
        Options::parse(vec![
            "--particles".into(),
            "dust".into(),
            "--showcase".into()
        ])
        .unwrap()
        .showcase
    );
}
#[test]
fn real_pickups_drive_presets_without_changing_authoritative_checksums() {
    let mut off = Demo::new(Mode::Off);
    let mut dust = Demo::new(Mode::Dust);
    let mut sparks = Demo::new(Mode::Sparks);
    let (mut dust_seen, mut sparks_seen) = (false, false);
    for _ in 0..20 {
        let input = CollectInput {
            x: FP::ONE,
            ..CollectInput::default()
        };
        for d in [&mut off, &mut dust, &mut sparks] {
            d.update(Duration::from_millis(17), input).unwrap();
        }
        let checksum = off.snapshot.as_ref().unwrap().predicted().checksum();
        assert_eq!(
            checksum,
            dust.snapshot.as_ref().unwrap().predicted().checksum()
        );
        assert_eq!(
            checksum,
            sparks.snapshot.as_ref().unwrap().predicted().checksum()
        );
        assert!(off.draws.is_empty());
        dust_seen |= !dust.draws.is_empty();
        sparks_seen |= !sparks.draws.is_empty();
        assert!(dust
            .draws
            .iter()
            .all(|d| d.blend == orr_view::particles::ParticleBlend::Alpha));
        assert!(sparks
            .draws
            .iter()
            .all(|d| d.blend == orr_view::particles::ParticleBlend::Additive));
    }
    assert!(dust_seen && sparks_seen);
    assert_eq!(
        off.snapshot
            .as_ref()
            .unwrap()
            .predicted()
            .singleton::<CollectRun>()
            .score,
        1
    );
}
#[test]
fn checked_pickup_origins_and_fresh_session_restart_discard_effects() {
    assert!(origin(2).is_none() && origin(u32::MAX).is_none());
    let mut d = Demo::new(Mode::Sparks);
    for _ in 0..10 {
        d.update(
            Duration::from_millis(17),
            CollectInput {
                x: FP::ONE,
                ..CollectInput::default()
            },
        )
        .unwrap();
    }
    assert!(!d.draws.is_empty());
    d = Demo::new(d.mode);
    assert!(d.draws.is_empty());
    assert_eq!(d.particles.particle_count(), 0);
    assert_eq!(
        d.snapshot
            .as_ref()
            .unwrap()
            .predicted()
            .singleton::<CollectRun>()
            .score,
        0
    );
}
#[test]
fn repeated_zero_delta_poll_does_not_replay_bursts() {
    let mut d = Demo::new(Mode::Sparks);
    for _ in 0..10 {
        d.update(
            Duration::from_millis(17),
            CollectInput {
                x: FP::ONE,
                ..CollectInput::default()
            },
        )
        .unwrap();
    }
    let before = d.draws.clone();
    for _ in 0..10 {
        d.update(Duration::ZERO, CollectInput::default()).unwrap();
    }
    assert_eq!(before, d.draws);
}
// Test-only capture is opt-in by an admitted output directory, never a runtime CLI flag.
// The directory must already exist as an ordinary canonical absolute path. Fixed
// exclusive names prevent replacement; all bytes come from asserted GPU readbacks.
fn capture(baseline: &[u8], visible: &[u8], checksum: u64, changed: usize, adapter: &str) {
    let Some(directory) = std::env::var_os("ORR_PARTICLE_CAPTURE_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    assert!(directory.is_absolute());
    assert_eq!(directory.canonicalize().unwrap(), directory);
    assert!(directory.symlink_metadata().unwrap().is_dir());
    assert_eq!(baseline.len(), 320 * 240 * 4);
    assert_eq!(visible.len(), baseline.len());
    struct Bounded {
        file: std::fs::File,
        written: usize,
    }
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > 400 * 1024 - self.written {
                return Err(std::io::Error::other("PNG capture exceeds400KiB"));
            }
            let n = std::io::Write::write(&mut self.file, bytes)?;
            self.written += n;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            std::io::Write::flush(&mut self.file)
        }
    }
    for (name, bytes) in [
        ("particles-off.png", baseline),
        ("pickup-sparks.png", visible),
    ] {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))
            .unwrap();
        let mut encoder = png::Encoder::new(Bounded { file, written: 0 }, 320, 240);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(bytes).unwrap();
        writer.finish().unwrap();
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("capture.txt"))
        .unwrap();
    assert!(adapter.len() <= 256);
    writeln!(file,"Actual sample GPU readback\nsize=320x240\nchanged_pixels={changed}\nauthoritative_checksum={checksum:016x}\nadapter={adapter}\nOff/on checksums equal; empty-particle route restored baseline exactly.").unwrap();
}
#[test]
#[ignore = "requires separately admitted actual GPU lane"]
fn actual_sample_pixels_preserve_frame_and_optionally_capture_png() {
    let gpu =
        Wgpu::headless(WgpuOptions::default()).expect("required sample GPU adapter unavailable");
    eprintln!(
        "sample particle adapter={} software={}",
        gpu.adapter_name(),
        gpu.is_software()
    );
    let mut off = Demo::new(Mode::Off);
    let mut on = Demo::new(Mode::Sparks);
    for _ in 0..20 {
        let input = CollectInput {
            x: FP::ONE,
            ..CollectInput::default()
        };
        off.update(Duration::from_millis(17), input).unwrap();
        on.update(Duration::from_millis(17), input).unwrap();
        if !on.draws.is_empty() {
            break;
        }
    }
    assert!(!on.draws.is_empty());
    let checksum = off.snapshot.as_ref().unwrap().predicted().checksum();
    assert_eq!(
        checksum,
        on.snapshot.as_ref().unwrap().predicted().checksum()
    );
    let target = orr_render::OffscreenTarget::new(&gpu, 320, 240, TextureFormat::Rgba8Unorm);
    let mut r_off = DemoRenderer::new(&gpu, target.format(), Mode::Off).unwrap();
    let mut r_on = DemoRenderer::new(&gpu, target.format(), Mode::Sparks).unwrap();
    // This fixture also catches accidental reuse of Renderer3D::new on MSAA-capable adapters.
    eprintln!(
        "sample adapter supported samples={}",
        gpu.supported_samples(target.format(), 4)
    );
    assert_eq!(r_off.geometry.samples(), 1);
    assert_eq!(r_on.geometry.samples(), 1);
    r_off
        .draw(target.render_view(), target.size(), &off)
        .unwrap();
    let baseline = target.read_rgba8();
    r_on.draw(target.render_view(), target.size(), &on).unwrap();
    let visible = target.read_rgba8();
    let changed = baseline
        .chunks_exact(4)
        .zip(visible.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed >= 4,
        "actual pickup particles must change at least4pixels, got{changed}"
    );
    on.draws.clear();
    r_on.draw(target.render_view(), target.size(), &on).unwrap();
    assert_eq!(baseline, target.read_rgba8());
    capture(&baseline, &visible, checksum, changed, &gpu.adapter_name());
}
