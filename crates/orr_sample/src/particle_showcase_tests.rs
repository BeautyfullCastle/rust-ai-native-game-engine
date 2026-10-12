use super::*;
const STEP: Duration = Duration::from_millis(17);
fn checksum(demo: &Demo) -> u64 {
    demo.snapshot.as_ref().unwrap().predicted().checksum()
}
fn modes() -> [Demo; 3] {
    [
        Demo::new(Mode::Off),
        Demo::new(Mode::Dust),
        Demo::new(Mode::Sparks),
    ]
}
fn advance(demos: &mut [Demo; 3], clocks: &mut [Showcase; 3], dt: Duration) {
    for (demo, clock) in demos.iter_mut().zip(clocks.iter_mut()) {
        clock.update(demo, dt, true).unwrap();
    }
    assert_eq!(checksum(&demos[0]), checksum(&demos[1]));
    assert_eq!(checksum(&demos[0]), checksum(&demos[2]));
    assert!(demos[0].draws.is_empty());
    for clock in &clocks[1..] {
        assert_eq!(clock.elapsed, clocks[0].elapsed);
        assert_eq!(clock.pickup_observed_at, clocks[0].pickup_observed_at);
        assert_eq!(clock.completed_cycles, clocks[0].completed_cycles);
    }
}
#[test]
fn genuine_pickup_cycles_keep_modes_authoritatively_identical() {
    let mut demos = modes();
    let mut clocks = std::array::from_fn(|_| Showcase::default());
    let mut seen = [false; 4];
    for _ in 0..400 {
        advance(&mut demos, &mut clocks, STEP);
        assert!(Showcase::score(&demos[0]) <= 1);
        if Showcase::score(&demos[0]) == 1 {
            seen[clocks[0].completed_cycles as usize] = true;
        }
        for demo in &demos {
            assert!(demo.draws.len() <= 1024);
        }
    }
    assert_eq!(clocks[0].completed_cycles, 3);
    assert!(seen.into_iter().all(|v| v));
}
#[test]
fn unchanged_presets_spread_fade_and_expire_after_real_pickup() {
    let mut demos = modes();
    let mut clocks = std::array::from_fn(|_| Showcase::default());
    for _ in 0..20 {
        advance(&mut demos, &mut clocks, STEP);
        if clocks[0].pickup_observed_at.is_some() {
            break;
        }
    }
    assert!(clocks[0].pickup_observed_at.is_some());
    let first = [demos[1].draws.clone(), demos[2].draws.clone()];
    assert_eq!(first[0].len(), 12);
    assert_eq!(first[1].len(), 16);
    advance(&mut demos, &mut clocks, Duration::from_millis(100));
    for (n, original) in first.iter().enumerate() {
        let current = &demos[n + 1].draws;
        assert_eq!(current.len(), original.len());
        assert!(current
            .iter()
            .zip(original)
            .any(|(a, b)| a.position != b.position));
        assert!(current
            .iter()
            .zip(original)
            .all(|(a, b)| a.color[3] < b.color[3]));
        assert!(current.iter().zip(original).all(|(a, b)| a.size != b.size));
    }
    for _ in 0..4 {
        advance(&mut demos, &mut clocks, Duration::from_millis(150));
    }
    assert!(demos.iter().all(|d| d.draws.is_empty()));
    assert_eq!(Showcase::score(&demos[0]), 1);
}
#[test]
fn focus_pause_and_zero_delta_preserve_time_state_and_particles() {
    let mut demo = Demo::new(Mode::Sparks);
    let mut clock = Showcase::default();
    for _ in 0..10 {
        clock.update(&mut demo, STEP, true).unwrap();
    }
    let before = (
        checksum(&demo),
        demo.draws.clone(),
        clock.elapsed,
        clock.pickup_observed_at,
    );
    clock.update(&mut demo, Duration::MAX, false).unwrap();
    for _ in 0..4 {
        clock.update(&mut demo, Duration::ZERO, true).unwrap();
    }
    assert_eq!(
        before,
        (
            checksum(&demo),
            demo.draws.clone(),
            clock.elapsed,
            clock.pickup_observed_at
        )
    );
    let mut control = Demo::new(Mode::Sparks);
    let mut control_clock = Showcase::default();
    for _ in 0..11 {
        control_clock.update(&mut control, STEP, true).unwrap();
    }
    clock.update(&mut demo, STEP, true).unwrap();
    assert_eq!(checksum(&demo), checksum(&control));
    assert_eq!(demo.draws, control.draws);
    assert_eq!(clock.elapsed, control_clock.elapsed);
}
#[test]
fn explicit_restart_clears_old_cycle_and_zero_time_does_not_restart_boundary() {
    let mut demo = Demo::new(Mode::Dust);
    let mut clock = Showcase::default();
    for _ in 0..8 {
        clock
            .update(&mut demo, Duration::from_millis(250), true)
            .unwrap();
    }
    assert_eq!(clock.elapsed, Showcase::CYCLE);
    let before = checksum(&demo);
    clock.update(&mut demo, Duration::ZERO, true).unwrap();
    assert_eq!(checksum(&demo), before);
    assert_eq!(clock.completed_cycles, 0);
    clock.restart(&mut demo);
    assert_eq!(checksum(&demo), checksum(&Demo::new(Mode::Dust)));
    assert_eq!(clock.elapsed, Duration::ZERO);
    assert!(clock.pickup_observed_at.is_none());
    assert!(demo.draws.is_empty());
    assert_eq!(Showcase::score(&demo), 0);
}

const SIZE: (u32, u32) = (640, 480);
const TIMES: [u64; 5] = [0, 100, 250, 450, 700];
struct Capture {
    directory: Option<std::path::PathBuf>,
    records: Vec<String>,
}
impl Capture {
    fn new() -> Self {
        let directory =
            std::env::var_os("ORR_PARTICLE_SHOWCASE_CAPTURE_DIR").map(std::path::PathBuf::from);
        if let Some(path) = &directory {
            assert!(path.is_absolute());
            assert_eq!(path.canonicalize().unwrap(), *path);
            assert!(path.symlink_metadata().unwrap().is_dir());
            for mode in ["dust", "sparks"] {
                for time in TIMES {
                    assert!(!path.join(format!("{mode}-{time:03}ms.png")).exists());
                }
            }
            assert!(!path.join("sequence.txt").exists());
        }
        Self {
            directory,
            records: Vec::with_capacity(10),
        }
    }
    fn frame(&mut self, name: &str, pixels: &[u8], record: String) {
        assert!(self.records.len() < 10);
        assert_eq!(pixels.len(), SIZE.0 as usize * SIZE.1 as usize * 4);
        self.records.push(record);
        let Some(path) = &self.directory else { return };
        struct Limited {
            file: std::fs::File,
            bytes: usize,
        }
        impl std::io::Write for Limited {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > 1536 * 1024 - self.bytes {
                    return Err(std::io::Error::other("showcase PNG exceeds1.5MiB"));
                }
                let n = std::io::Write::write(&mut self.file, bytes)?;
                self.bytes += n;
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                std::io::Write::flush(&mut self.file)
            }
        }
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join(name))
            .unwrap();
        let mut encoder = png::Encoder::new(Limited { file, bytes: 0 }, SIZE.0, SIZE.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
        writer.finish().unwrap();
    }
    fn finish(self, adapter: &str) {
        assert_eq!(self.records.len(), 10);
        assert!(adapter.len() <= 256);
        let text=format!("Actual temporal particle readbacks\nadapter={adapter}\nsize=640x480\nTimes are elapsed since observed genuine pickup; presentation_elapsed includes its initial update.\nFresh-session cycles use identical inputs across off/dust/sparks.\n{}\n",self.records.join("\n"));
        assert!(text.len() <= 16 * 1024);
        if let Some(path) = self.directory {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path.join("sequence.txt"))
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        }
        eprintln!("{text}");
    }
}
#[test]
#[ignore = "requires separately admitted actual GPU and optional private sequence capture"]
fn actual_temporal_dust_and_sparks_spread_fade_and_expire() {
    let gpu =
        Wgpu::headless(WgpuOptions::default()).expect("required showcase GPU adapter unavailable");
    eprintln!(
        "temporal particle adapter={} software={}",
        gpu.adapter_name(),
        gpu.is_software()
    );
    let mut demos = modes();
    let mut clocks = std::array::from_fn(|_| Showcase::default());
    for _ in 0..20 {
        advance(&mut demos, &mut clocks, STEP);
        if clocks[0].pickup_observed_at.is_some() {
            break;
        }
    }
    let anchor = clocks[0]
        .pickup_observed_at
        .expect("genuine pickup must occur");
    let target = orr_render::OffscreenTarget::new(&gpu, SIZE.0, SIZE.1, TextureFormat::Rgba8Unorm);
    let mut renderers = [
        DemoRenderer::new(&gpu, target.format(), Mode::Off).unwrap(),
        DemoRenderer::new(&gpu, target.format(), Mode::Dust).unwrap(),
        DemoRenderer::new(&gpu, target.format(), Mode::Sparks).unwrap(),
    ];
    assert!(renderers.iter().all(|r| r.geometry.samples() == 1));
    let mut capture = Capture::new();
    let mut previous: [Option<Vec<i16>>; 2] = [None, None];
    for ms in TIMES {
        let destination = anchor + Duration::from_millis(ms);
        while clocks[0].elapsed < destination {
            let dt = STEP.min(destination - clocks[0].elapsed);
            advance(&mut demos, &mut clocks, dt);
        }
        renderers[0]
            .draw(target.render_view(), target.size(), &demos[0])
            .unwrap();
        let baseline = target.read_rgba8();
        for (i, name) in [(1, "dust"), (2, "sparks")] {
            renderers[i]
                .draw(target.render_view(), target.size(), &demos[i])
                .unwrap();
            let pixels = target.read_rgba8();
            let changed = pixels
                .chunks_exact(4)
                .zip(baseline.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            let delta: Vec<i16> = pixels
                .iter()
                .zip(&baseline)
                .map(|(a, b)| i16::from(*a) - i16::from(*b))
                .collect();
            if ms <= 250 {
                assert!(changed > 0, "{name} must remain visible at{ms}ms");
                if let Some(before) = &previous[i - 1] {
                    assert_ne!(&delta, before, "{name} must visually evolve");
                }
            }
            if ms == 700 {
                assert!(demos[i].draws.is_empty());
                assert_eq!(
                    pixels, baseline,
                    "expired{name} must restore exact baseline"
                );
            }
            previous[i - 1] = Some(delta);
            let record=format!("mode={name} after_pickup_ms={ms} presentation_elapsed_us={} pickup_observed_us={} tick={} particles={} changed_pixels={changed} authoritative_checksum={:016x}",clocks[i].elapsed.as_micros(),anchor.as_micros(),demos[i].snapshot.as_ref().unwrap().predicted().tick(),demos[i].draws.len(),checksum(&demos[i]));
            capture.frame(&format!("{name}-{ms:03}ms.png"), &pixels, record);
        }
    }
    capture.finish(&gpu.adapter_name());
}
