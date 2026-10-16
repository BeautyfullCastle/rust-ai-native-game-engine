//! Browser correctness proof for the 3D sphere LOD fixture.
//!
//! Needs Playwright/Chromium and the GPU wasm package from `tools/build_web.sh`.
//! Missing prerequisites skip unless the corresponding `ORR_REQUIRE_*` flag is set.
#![allow(clippy::disallowed_types)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn required(backend: &str) -> bool {
    std::env::var("ORR_REQUIRE_BROWSER").is_ok_and(|v| v == "1")
        || std::env::var("ORR_REQUIRE_GPU").is_ok_and(|v| v == "1")
        || (backend == "webgpu" && std::env::var("ORR_REQUIRE_WEBGPU").is_ok_and(|v| v == "1"))
}

fn fixture_dir(backend: &str) -> PathBuf {
    if let Some(root) = std::env::var_os("SPHERE_LOD_SCREENSHOTS") {
        let path = PathBuf::from(root).join(backend);
        std::fs::create_dir_all(&path).unwrap_or_else(|e| panic!("cannot create screenshot directory {}: {e}", path.display()));
        return path;
    }
    loop {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("orr_sphere_lod_{}_{}_{}", std::process::id(), backend, id));
        match std::fs::create_dir(&path) {
            Ok(()) => return path,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("cannot create browser fixture directory: {error}"),
        }
    }
}

#[derive(Debug)]
struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn read_png(path: &Path) -> Image {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().expect("read screenshot PNG");
    let mut bytes = vec![0; reader.output_buffer_size().expect("PNG output buffer size")];
    let info = reader.next_frame(&mut bytes).expect("decode screenshot PNG");
    let bytes = &bytes[..info.buffer_size()];
    let samples = info.color_type.samples();
    let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
    for pixel in bytes.chunks_exact(samples) {
        match info.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(pixel),
            png::ColorType::Rgb => rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
            png::ColorType::GrayscaleAlpha => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]),
            png::ColorType::Grayscale => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]),
            png::ColorType::Indexed => panic!("PNG palette was not expanded"),
        }
    }
    Image { width: info.width, height: info.height, rgba }
}

fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * image.width + x) * 4) as usize;
    image.rgba[i..i + 4].try_into().expect("RGBA pixel")
}

fn foreground(image: &Image, x: u32, y: u32) -> bool {
    let px = pixel(image, x, y);
    px[0].max(px[1]).max(px[2]) > 8
}

fn boundary(image: &Image) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for y in 0..image.height {
        for x in 0..image.width {
            if !foreground(image, x, y) {
                continue;
            }
            let edge = (-1i32..=1).any(|dy| {
                (-1i32..=1).any(|dx| {
                    if dx == 0 && dy == 0 {
                        false
                    } else {
                        let nx = x as i32 + dx;
                        let ny = y as i32 + dy;
                        nx < 0
                            || ny < 0
                            || nx >= image.width as i32
                            || ny >= image.height as i32
                            || !foreground(image, nx as u32, ny as u32)
                    }
                })
            });
            if edge {
                out.push((x, y));
            }
        }
    }
    out
}

fn assert_boundaries_within_one_pixel(reference: &Image, candidate: &Image) {
    assert_eq!((reference.width, reference.height), (candidate.width, candidate.height));
    assert_eq!((reference.width, reference.height), (256, 256), "fixture pixels are physical 256x256 canvas pixels");
    let a = boundary(reference);
    let b = boundary(candidate);
    assert!(!a.is_empty() && !b.is_empty(), "sphere silhouette is missing");
    let covered = |from: &[(u32, u32)], to: &[(u32, u32)]| {
        from.iter().all(|&(x, y)| {
            to.iter().any(|&(tx, ty)| {
                let dx = x as i32 - tx as i32;
                let dy = y as i32 - ty as i32;
                dx * dx + dy * dy <= 1
            })
        })
    };
    assert!(covered(&a, &b), "reference silhouette boundary moved more than one pixel");
    assert!(covered(&b, &a), "LOD silhouette boundary moved more than one pixel");
}

fn assert_center_within_four(reference: &Image, candidate: &Image) {
    assert_eq!((reference.width, reference.height), (candidate.width, candidate.height));
    assert_rgb_within_four(reference, candidate, reference.width / 2, reference.height / 2);
}

fn assert_rgb_within_four(reference: &Image, candidate: &Image, x: u32, y: u32) {
    let a = pixel(reference, x, y);
    let b = pixel(candidate, x, y);
    assert!(a[..3].iter().any(|&c| c > 8) && b[..3].iter().any(|&c| c > 8), "sphere center pixel is missing");
    for channel in 0..3 {
        assert!(a[channel].abs_diff(b[channel]) <= 4, "center channel {channel} differs: {a:?} vs {b:?}");
    }
}

fn assert_pixel_equal(reference: &Image, candidate: &Image, x: u32, y: u32) {
    assert_eq!((reference.width, reference.height), (candidate.width, candidate.height));
    assert_eq!(pixel(reference, x, y), pixel(candidate, x, y), "near instance pixel changed at ({x}, {y})");
}

fn run_backend(backend: &str) {
    let required = required(backend);
    let dir = fixture_dir(backend);
    let repo = repo_root();
    let runner = repo.join("tools").join("webtransport").join("sphere_lod.cjs");
    let out = Command::new("node")
        .arg(runner)
        .env("SPHERE_LOD_BACKEND", backend)
        .env("SPHERE_LOD_SCREENSHOTS", &dir)
        .output();
    let out = match out {
        Ok(out) => out,
        Err(error) if !required => {
            eprintln!("SKIP {backend}: cannot run Node ({error})");
            return;
        }
        Err(error) => panic!("required {backend} browser fixture cannot run Node: {error}"),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    eprintln!("---- {backend} sphere LOD browser fixture ----\n{stdout}{stderr}------------------------------------------");
    if out.status.code() == Some(2) {
        assert!(!required, "required {backend} browser fixture skipped:\n{stdout}{stderr}");
        eprintln!("SKIP {backend}: browser prerequisites or backend are unavailable");
        return;
    }
    assert!(out.status.success(), "{backend} sphere LOD browser fixture failed:\n{stdout}{stderr}");

    let expected_backend = if backend == "webgpu" { "BrowserWebGpu" } else { "Gl" };
    let expected = [
        "default_near_fixed",
        "default_near_lod",
        "default_far_fixed",
        "default_far_lod",
        "default_mixed_fixed",
        "default_mixed_lod",
        "low_near_fixed",
        "low_near_lod",
        "low_far_fixed",
        "low_far_lod",
        "low_mixed_fixed",
        "low_mixed_lod",
    ];
    for key in expected {
        let line = stdout
            .lines()
            .find(|line| line.starts_with("RESULT ") && line.contains(&format!("\"key\":\"{key}\"")))
            .unwrap_or_else(|| panic!("missing result for {key}:\n{stdout}"));
        assert!(line.contains(&format!("\"actual_backend\":\"{expected_backend}\"")), "wrong actual backend: {line}");
        assert!(line.contains("\"size\":[256,256]"), "unexpected fixture size: {line}");
        assert!(line.contains("\"format\":\""), "missing actual target format: {line}");
        assert!(line.contains("\"msaa_samples\":"), "missing actual MSAA sample count: {line}");
        assert!(!line.contains("\"msaa_samples\":0"), "invalid actual MSAA sample count: {line}");
        let (expected_near, expected_far) = if key.contains("_mixed_") {
            if key.ends_with("_lod") { (1, 1) } else { (2, 0) }
        } else if key.ends_with("_far_lod") {
            (0, 1)
        } else {
            (1, 0)
        };
        assert!(line.contains(&format!("\"near_spheres\":{expected_near}")), "unexpected near bucket: {line}");
        assert!(line.contains(&format!("\"far_spheres\":{expected_far}")), "unexpected far bucket: {line}");
    }

    for preset in ["default", "low"] {
        let fixed_near = read_png(&dir.join(format!("{preset}_near_fixed.png")));
        let lod_near = read_png(&dir.join(format!("{preset}_near_lod.png")));
        assert_eq!(fixed_near.rgba, lod_near.rgba, "{backend}/{preset}: disabled and all-near pixels differ");

        let fixed_far = read_png(&dir.join(format!("{preset}_far_fixed.png")));
        let lod_far = read_png(&dir.join(format!("{preset}_far_lod.png")));
        assert_boundaries_within_one_pixel(&fixed_far, &lod_far);
        assert_center_within_four(&fixed_far, &lod_far);

        let mixed_fixed = read_png(&dir.join(format!("{preset}_mixed_fixed.png")));
        let mixed_lod = read_png(&dir.join(format!("{preset}_mixed_lod.png")));
        // The near red sphere keeps its full mesh; the far blue sphere keeps its material
        // after it is packed at first_instance = 1 for the second WebGL indexed draw.
        assert_pixel_equal(&mixed_fixed, &mixed_lod, 113, 128);
        assert_rgb_within_four(&mixed_fixed, &mixed_lod, 167, 128);
    }
}

#[test]
fn explicit_browser_backends_preserve_sphere_lod_pixels() {
    run_backend("webgl");
    run_backend("webgpu");
}
