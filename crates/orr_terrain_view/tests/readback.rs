//! The release acceptance path is ORR_REQUIRE_GPU=1: missing adapters fail.
//! Software Vulkan is accepted and is identified in the evidence output.
#![cfg(feature = "gpu")]
#![allow(clippy::float_arithmetic)]
use orr_fp::FP;
use orr_package::Project;
use orr_render::{
    orr_rhi::{Rhi, Wgpu, WgpuOptions},
    Camera3D,
};
use orr_terrain::Edit;
use orr_terrain::TerrainDocument;
use orr_terrain_view::{
    fixture, gpu,
    package::{self, install_and_reload},
    QueryMarker,
};

fn adapter() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => {
            eprintln!(
                "terrain acceptance adapter: {} (software: {})",
                gpu.adapter_name(),
                gpu.is_software()
            );
            Some(gpu)
        }
        Err(e) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required terrain GPU unavailable: {e}"
            );
            eprintln!("SKIP terrain GPU (set ORR_REQUIRE_GPU=1 to require): {e}");
            None
        }
    }
}
fn pixel(pixels: &[u8], world: [f32; 3], camera: &Camera3D, size: (u32, u32)) -> [u8; 4] {
    let xy = camera.world_to_screen(world, size).unwrap();
    assert!(xy[0] >= 0.0 && xy[1] >= 0.0 && xy[0] < (size.0 as f32) && xy[1] < (size.1 as f32));
    let offset = ((xy[1] as u32 * size.0 + xy[0] as u32) * 4) as usize;
    pixels[offset..offset + 4].try_into().unwrap()
}
fn black(c: [u8; 4]) {
    assert_eq!(c, [0, 0, 0, 255]);
}
fn green(c: [u8; 4]) {
    assert!(
        c[1] > 110 && c[1] > c[0] + 30 && c[1] > c[2] + 20,
        "expected terrain green, got {c:?}"
    );
}
fn orange(c: [u8; 4]) {
    assert!(
        c[0] > 220 && c[1] < 120 && c[2] < 50,
        "expected query marker orange, got {c:?}"
    );
}

#[test]
fn edit_cook_package_read_asset_reload_gpu_silhouette_hole_camera_resize_query_marker() {
    let Some(device) = adapter() else { return };
    // Resolve only the trusted system-temp root before creating fixture content.
    // macOS may use /var -> /private/var; package inputs must still reject links.
    let temp_root = std::env::temp_dir();
    #[cfg(unix)]
    let temp_root = std::fs::canonicalize(temp_root).unwrap();
    let tmp = tempfile::tempdir_in(temp_root).unwrap();
    let root = tmp.path().join("project");
    let mut doc = TerrainDocument::new(fixture());
    let before = install_and_reload(
        doc.terrain(),
        &root,
        &tmp.path().join("source-before"),
        "1.0.0",
    )
    .unwrap();
    let mut view = gpu::renderer(&device, &before.terrain, &[]).unwrap();
    let size = (256, 256);
    let top = gpu::top_camera();
    let light = gpu::lighting();
    let baseline = gpu::capture(&mut view, &top, &light, size).unwrap();
    green(pixel(&baseline, [-2.5, 0.0, -2.5], &top, size));
    green(pixel(&baseline, [3.5, 0.0, 3.5], &top, size));
    black(pixel(&baseline, [4.5, 0.0, 0.0], &top, size));
    let side = Camera3D::orthographic([0.0, 0.0, 12.0], [0.0; 3], 5.0);
    let original_silhouette = gpu::capture(&mut view, &side, &light, size).unwrap();
    green(pixel(&original_silhouette, [0.0, 1.0, 0.0], &side, size));
    black(pixel(&original_silhouette, [0.0, 3.0, 0.0], &side, size));

    doc.apply(&[
        Edit::SetHeight {
            x: 4,
            z: 4,
            height: FP::from_int(4),
        },
        Edit::SetHole {
            x: 1,
            z: 1,
            hole: true,
        },
    ])
    .unwrap();
    let edited_revision = doc.terrain().revision();
    assert!(doc.undo());
    assert_eq!(doc.terrain().revision(), before.terrain.revision());
    assert!(doc.redo());
    assert_eq!(doc.terrain().revision(), edited_revision);
    let after = install_and_reload(
        doc.terrain(),
        &root,
        &tmp.path().join("source-after"),
        "1.0.1",
    )
    .unwrap();
    assert_ne!(before.package_digest, after.package_digest);
    assert_eq!(after.terrain.revision(), edited_revision);
    assert_eq!(
        after.terrain.sample(FP::ZERO, FP::ZERO),
        Some(FP::from_int(4))
    );
    assert!(after
        .terrain
        .sample(FP::from_raw(-163840), FP::from_raw(-163840))
        .is_none());
    // Source changes after installation cannot become rendered/query state.
    std::fs::write(
        tmp.path().join("source-after").join(package::ASSET_PATH),
        b"changed source",
    )
    .unwrap();
    let reloaded = orr_terrain::Terrain::load(
        &Project::open(&root, package::runtime())
            .unwrap()
            .read_asset(package::PACKAGE_NAME, package::ASSET_PATH)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reloaded, after.terrain);
    // ModelRenderer is immutable: this is an explicit rebuild after edit/reload.
    view = gpu::renderer(&device, &reloaded, &[]).unwrap();
    let edited = gpu::capture(&mut view, &top, &light, size).unwrap();
    black(pixel(&edited, [-2.5, 0.0, -2.5], &top, size));
    green(pixel(&edited, [-1.5, 0.0, -2.5], &top, size));
    let raised = gpu::capture(&mut view, &side, &light, size).unwrap();
    green(pixel(&raised, [0.0, 3.0, 0.0], &side, size));
    assert_ne!(raised, original_silhouette);

    // Camera uniforms change on the same renderer. Resize recreates its depth.
    let mut moved = top;
    moved.eye[0] += 1.0;
    moved.target[0] += 1.0;
    let moved_pixels = gpu::capture(&mut view, &moved, &light, size).unwrap();
    assert_ne!(edited, moved_pixels);
    black(pixel(&moved_pixels, [-2.5, 0.0, -2.5], &moved, size));
    green(pixel(&moved_pixels, [2.5, 0.0, 2.5], &moved, size));
    let wide = (384, 192);
    let wide_pixels = gpu::capture(&mut view, &moved, &light, wide).unwrap();
    assert_eq!(wide_pixels.len(), (wide.0 * wide.1 * 4) as usize);
    black(pixel(&wide_pixels, [-2.5, 0.0, -2.5], &moved, wide));
    green(pixel(&wide_pixels, [2.5, 0.0, 2.5], &moved, wide));
    // Returning to the original viewport proves depth/size rebuild is reversible.
    assert_eq!(edited, gpu::capture(&mut view, &top, &light, size).unwrap());

    let marker =
        QueryMarker::sample(&reloaded, FP::from_raw(163840), FP::from_raw(163840)).unwrap();
    assert_eq!(
        marker.anchor(),
        [
            FP::from_raw(163840),
            FP::from_raw(8192),
            FP::from_raw(163840)
        ]
    );
    assert!(QueryMarker::sample(&reloaded, FP::from_raw(-163840), FP::from_raw(-163840)).is_none());
    view = gpu::renderer(&device, &reloaded, &[marker]).unwrap();
    let marked = gpu::capture(&mut view, &top, &light, size).unwrap();
    orange(pixel(&marked, marker.anchor().map(FP::to_f32), &top, size));
    black(pixel(&marked, [-2.5, 0.0, -2.5], &top, size));
    // A nonzero sloped query and side view detect wrong-Y marker placement;
    // top-down agreement alone cannot establish the sampled height visually.
    let sloped = QueryMarker::sample(&reloaded, FP::from_raw(32768), FP::from_raw(16384)).unwrap();
    assert_eq!(
        sloped.anchor(),
        [
            FP::from_raw(32768),
            FP::from_raw(172032),
            FP::from_raw(16384)
        ]
    );
    view = gpu::renderer(&device, &reloaded, &[marker, sloped]).unwrap();
    let side_marked = gpu::capture(&mut view, &side, &light, size).unwrap();
    orange(pixel(&side_marked, [0.5, 2.625 + 0.18, 0.25], &side, size));
    let wrong_height = pixel(&side_marked, [0.5, 0.18, 0.25], &side, size);
    assert!(
        !(wrong_height[0] > 220 && wrong_height[1] < 120 && wrong_height[2] < 50),
        "marker incorrectly rendered at zero height: {wrong_height:?}"
    );
    if let Some(path) = std::env::var_os("ORR_TERRAIN_SCREENSHOT") {
        let pixels = gpu::capture(&mut view, &gpu::oblique_camera(), &light, (800, 600)).unwrap();
        gpu::save_png(std::path::Path::new(&path), &pixels, (800, 600)).unwrap();
    }
    eprintln!("terrain acceptance PASS: edit/undo/redo -> cook -> actual package install/read_asset -> reload -> silhouette/hole/camera/resize/query marker; revision={:02x?}; package={}",reloaded.revision(),after.package_digest);
}
