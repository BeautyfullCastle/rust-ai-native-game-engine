//! Runnable, non-windowed edit -> cook -> package -> reload -> render/query lab.
#![allow(clippy::float_arithmetic)]
use orr_fp::FP;
use orr_render::orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_terrain::{Edit, TerrainDocument};
use orr_terrain_view::{fixture, gpu, package::install_and_reload, QueryMarker};
use std::{fs, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        return Err("usage: terrain_lab NEW_OUTPUT_DIRECTORY (writes PNGs and actual installed package; needs a GPU or software Vulkan adapter)".into());
    }
    let output = PathBuf::from(&args[0]);
    // Never overwrite arbitrary user artifacts.
    fs::create_dir(&output)?;
    let gpu = Wgpu::headless(WgpuOptions::default())?;
    eprintln!(
        "adapter={} software={}",
        gpu.adapter_name(),
        gpu.is_software()
    );
    let mut doc = TerrainDocument::new(fixture());
    let project = output.join("project");
    let before = install_and_reload(doc.terrain(), &project, &output.join("source-v1"), "1.0.0")?;
    let light = gpu::lighting();
    let camera = gpu::oblique_camera();
    let size = (1000, 750);
    let mut renderer = gpu::renderer(&gpu, &before.terrain, &[])?;
    gpu::save_png(
        &output.join("01-before.png"),
        &gpu::capture(&mut renderer, &camera, &light, size)?,
        size,
    )?;
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
    ])?;
    let revision = doc.terrain().revision();
    assert!(doc.undo());
    assert_eq!(doc.terrain(), &before.terrain);
    assert!(doc.redo());
    assert_eq!(doc.terrain().revision(), revision);
    let after = install_and_reload(doc.terrain(), &project, &output.join("source-v2"), "1.0.1")?;
    let marker =
        QueryMarker::sample(&after.terrain, FP::from_raw(32768), FP::from_raw(16384)).unwrap();
    renderer = gpu::renderer(&gpu, &after.terrain, &[marker])?;
    gpu::save_png(
        &output.join("02-edited-and-query.png"),
        &gpu::capture(&mut renderer, &camera, &light, size)?,
        size,
    )?;
    gpu::save_png(
        &output.join("03-top-hole-and-query.png"),
        &gpu::capture(&mut renderer, &gpu::top_camera(), &light, size)?,
        size,
    )?;
    let mut pan = gpu::top_camera();
    pan.eye[0] += 1.0;
    pan.target[0] += 1.0;
    let wide = (1200, 600);
    gpu::save_png(
        &output.join("04-camera-and-resize.png"),
        &gpu::capture(&mut renderer, &pan, &light, wide)?,
        wide,
    )?;
    let revision: String = revision.iter().map(|b| format!("{b:02x}")).collect();
    let surface = after
        .terrain
        .surface(marker.anchor()[0], marker.anchor()[2])
        .unwrap();
    let report = serde_json::json!({
        "adapter":gpu.adapter_name(),"software_adapter":gpu.is_software(),
        "asset_id":after.terrain.asset_id(),"terrain_revision":revision,
        "package_digest":after.package_digest,"package_version":"1.0.1",
        "loaded_from":"Project::read_asset verified installed immutable bytes",
        "query":{"x_raw":marker.anchor()[0].raw(),"height_raw":marker.anchor()[1].raw(),"z_raw":marker.anchor()[2].raw(),"normal_raw":surface.normal.map(FP::raw),"slope_raw":surface.slope.map(FP::raw)},
        "hole_query_is_none":after.terrain.sample(FP::from_raw(-163840),FP::from_raw(-163840)).is_none(),
        "center_height_raw":after.terrain.sample(FP::ZERO,FP::ZERO).unwrap().raw(),
        "renderer_limitations":["fresh independent color/depth pass","model rebuilt after edit or marker change","opaque diffuse material only","offscreen lab, not scene/physics/editor integration"]
    });
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
