//! Explicit GPU acceptance: missing GPU always fails, no silent skip. Save the
//! original readbacks with ORR_RAW16_CAPTURE_DIR; CPU mesh tests are separate.
#![allow(clippy::float_arithmetic)]
mod common;
use common::*;
use orr_fp::FP;
use orr_package::Project;
use orr_render::{
    orr_rhi::{Rhi, Wgpu, WgpuOptions},
    Camera3D,
};
use orr_terrain::Terrain;
use orr_terrain_import::{import_raw16, Endian};
use orr_terrain_view::{
    gpu,
    package::{self, install_and_reload},
    QueryMarker,
};
use std::{fs, path::Path};
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}
fn save(root: &Path, name: &str, pixels: &[u8], size: (u32, u32)) {
    gpu::save_png(&root.join(format!("{name}.png")), pixels, size).unwrap();
    fs::write(root.join(format!("{name}.rgba")), pixels).unwrap();
}
fn orange_count(
    pixels: &[u8],
    anchor: [f32; 3],
    camera: &Camera3D,
    size: (u32, u32),
    radius: i32,
) -> usize {
    let point = camera.world_to_screen(anchor, size).unwrap();
    let x = point[0] as i32;
    let y = point[1] as i32;
    let mut orange = 0;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let (px, py) = (x + dx, y + dy);
            if px < 0 || py < 0 || px >= size.0 as i32 || py >= size.1 as i32 {
                continue;
            }
            let o = ((py as u32 * size.0 + px as u32) * 4) as usize;
            let c = &pixels[o..o + 4];
            if c[0] > 220 && c[1] < 120 && c[2] < 50 {
                orange += 1;
            }
        }
    }
    orange
}
#[test]
#[ignore = "requires explicit GPU lease and readback inspection"]
fn raw16_package_reload_oblique_side_and_orientation_controls_render() {
    let require_software = std::env::var_os("ORR_RAW16_REQUIRE_SOFTWARE").is_some();
    let device = Wgpu::headless(WgpuOptions {
        force_software: require_software,
        ..Default::default()
    })
    .expect("required RAW16 GPU unavailable");
    eprintln!(
        "RAW16 acceptance adapter: {} (software: {})",
        device.adapter_name(),
        device.is_software()
    );
    if require_software {
        assert!(
            device.is_software(),
            "software acceptance requires a software adapter"
        );
    }
    let captures =
        std::env::var_os("ORR_RAW16_CAPTURE_DIR").expect("capture directory is required");
    let captures = Path::new(&captures);
    assert!(captures.is_dir());
    assert_eq!(
        fs::read_dir(captures).unwrap().count(),
        0,
        "fresh capture directory required"
    );
    let samples = [0, 16384, 65535, 32768, 8192, 49152];
    let mut config = recipe();
    config.height_scale = FP::from_int(3);
    config.spacing = FP::from_int(2);
    config.origin = [FP::from_int(-2), FP::from_int(-1)];
    let baseline_recipe = config.clone();
    let original = import_raw16(&raw(&samples, Endian::Little), &config).unwrap();
    let root = temp();
    let project = root.path().join("project");
    let source = root.path().join("source");
    install_and_reload(&original, &project, &source, "1.0.0").unwrap();
    fs::write(source.join(package::ASSET_PATH), b"source changed").unwrap();
    let verified = Project::open(&project, package::runtime())
        .unwrap()
        .read_asset(package::PACKAGE_NAME, package::ASSET_PATH)
        .unwrap();
    let loaded = Terrain::load(&verified).unwrap();
    assert_eq!(loaded, original);
    config.flip_z = true;
    let flipped = import_raw16(&raw(&samples, Endian::Little), &config).unwrap();
    config.flip_z = false;
    config.width = 2;
    config.depth = 3;
    let transposed = import_raw16(
        &raw(
            &[
                samples[0], samples[3], samples[1], samples[4], samples[2], samples[5],
            ],
            Endian::Little,
        ),
        &config,
    )
    .unwrap();
    let size = (320, 256);
    let light = gpu::lighting();
    let cameras = [
        (
            "oblique",
            Camera3D::orthographic([7., 6., 9.], [0., 0.5, 0.], 4.5),
        ),
        (
            "side",
            Camera3D::orthographic([0., 0.5, 10.], [0., 0.5, 0.], 4.5),
        ),
    ];
    let mut base_view = gpu::renderer(&device, &loaded, &[]).unwrap();
    let mut flip_view = gpu::renderer(&device, &flipped, &[]).unwrap();
    let mut transpose_view = gpu::renderer(&device, &transposed, &[]).unwrap();
    for (label, camera) in cameras {
        let baseline = gpu::capture(&mut base_view, &camera, &light, size).unwrap();
        let flip = gpu::capture(&mut flip_view, &camera, &light, size).unwrap();
        let transpose = gpu::capture(&mut transpose_view, &camera, &light, size).unwrap();
        assert!(
            baseline
                .chunks_exact(4)
                .filter(|p| p[1] > p[0] && p[1] > p[2])
                .count()
                > 200,
            "visible terrain required"
        );
        assert!(
            differing(&baseline, &flip) > 100,
            "flip-Z control must visibly differ"
        );
        assert!(
            differing(&baseline, &transpose) > 100,
            "transpose control must visibly differ"
        );
        assert_eq!(
            baseline,
            gpu::capture(&mut base_view, &camera, &light, size).unwrap()
        );
        save(captures, &format!("{label}-baseline"), &baseline, size);
        save(captures, &format!("{label}-flip-z"), &flip, size);
        save(captures, &format!("{label}-transpose"), &transpose, size);
    }
    let vertex = QueryMarker::sample(&loaded, FP::ZERO, FP::from_int(1)).unwrap();
    let query = QueryMarker::sample(&loaded, FP::ONE, FP::ZERO).unwrap();
    assert_eq!(vertex.anchor(), loaded.vertex_position(4).unwrap());
    assert_eq!(
        query.anchor()[1],
        loaded.surface(FP::ONE, FP::ZERO).unwrap().height
    );
    let mut top = Camera3D::orthographic([0., 12., 0.], [0.; 3], 3.5);
    top.up = [0., 0., -1.];
    let top_baseline = gpu::capture(&mut base_view, &top, &light, size).unwrap();
    let mut marked = gpu::renderer(&device, &loaded, &[vertex, query]).unwrap();
    let image = gpu::capture(&mut marked, &top, &light, size).unwrap();
    assert!(orange_count(&image, vertex.anchor().map(FP::to_f32), &top, size, 6) > 0);
    assert!(orange_count(&image, query.anchor().map(FP::to_f32), &top, size, 6) > 0);
    assert!(differing(&top_baseline, &image) > 30);
    save(captures, "top-baseline", &top_baseline, size);
    save(captures, "top-markers", &image, size);
    // Independent integer oracle for this exact fixture, not a second call to sample.
    assert_eq!(vertex.anchor(), [FP::ZERO, FP::from_raw(-40960), FP::ONE]);
    assert_eq!(query.anchor(), [FP::ONE, FP::from_raw(32769), FP::ZERO]);
    // At spacing2, existing marker centre is1.5 * (2 *0.12) =0.36 above anchor.
    // Side projection makes Y visible. These masks are fixed before device output.
    let side = Camera3D::orthographic([0., 0.5, 10.], [0., 0.5, 0.], 4.5);
    let side_marked = gpu::capture(&mut marked, &side, &light, size).unwrap();
    let mut wrong_recipe = baseline_recipe;
    wrong_recipe.height_offset = FP::ONE; // Exactly +2 world Y, same X/Z and raw data.
    let wrong_height = import_raw16(&raw(&samples, Endian::Little), &wrong_recipe).unwrap();
    let wrong_vertex = QueryMarker::sample(&wrong_height, FP::ZERO, FP::ONE).unwrap();
    let wrong_query = QueryMarker::sample(&wrong_height, FP::ONE, FP::ZERO).unwrap();
    let mut wrong_view =
        gpu::renderer(&device, &wrong_height, &[wrong_vertex, wrong_query]).unwrap();
    let wrong_pixels = gpu::capture(&mut wrong_view, &side, &light, size).unwrap();
    for centre in [[0., -0.625 + 0.36, 1.], [1., 32769. / 65536. + 0.36, 0.]] {
        let wrong_centre = [centre[0], centre[1] + 2., centre[2]];
        assert!(
            orange_count(&side_marked, centre, &side, size, 2) >= 4,
            "correct height anchor missing"
        );
        assert_eq!(
            orange_count(&side_marked, wrong_centre, &side, size, 2),
            0,
            "correct marker appeared at wrong Y"
        );
        assert_eq!(
            orange_count(&wrong_pixels, centre, &side, size, 2),
            0,
            "height-shifted control matched original Y"
        );
        assert!(
            orange_count(&wrong_pixels, wrong_centre, &side, size, 2) >= 4,
            "shifted height control missing"
        );
    }
    save(captures, "side-markers", &side_marked, size);
    save(captures, "side-wrong-height", &wrong_pixels, size);
    assert_eq!(fs::read_dir(captures).unwrap().count(), 20);
}
