//! Offscreen single-model integration; no modifications to existing render APIs.
use crate::{to_static_model, PathDisplay, Result};
use orr_fp::FP;
use orr_navigation::NavigationPath;
use orr_render::{
    orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu},
    Camera3D, Lighting, ModelRenderer,
};
use orr_terrain::Terrain;
use std::{fs::File, io::BufWriter, path::Path};
pub const FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;
pub fn renderer(
    gpu: &Wgpu,
    terrain: &Terrain,
    path: Option<&NavigationPath>,
    agent: [FP; 3],
    display: PathDisplay,
) -> Result<ModelRenderer<Wgpu>> {
    let mut renderer = ModelRenderer::new(
        gpu.clone(),
        FORMAT,
        to_static_model(terrain, path, agent, display)?,
    )?;
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    Ok(renderer)
}
pub fn capture(
    renderer: &mut ModelRenderer<Wgpu>,
    camera: &Camera3D,
    lighting: &Lighting,
    size: (u32, u32),
) -> Result<Vec<u8>> {
    if size.0 == 0 || size.1 == 0 || size.0 > 8192 || size.1 > 8192 {
        return Err("capture size must be 1..8192 per axis".into());
    }
    let gpu = renderer.rhi().clone();
    let texture = gpu.create_texture(&TextureDesc {
        label: "terrain navigation readback",
        width: size.0,
        height: size.1,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = gpu.create_texture_view(&texture, None);
    renderer.draw(&view, size, camera, lighting)?;
    Ok(gpu.read_texture(&texture))
}
pub fn top_camera() -> Camera3D {
    let mut camera = Camera3D::orthographic([0.0, 12.0, 0.0], [0.0; 3], 5.0);
    camera.up = [0.0, 0.0, -1.0];
    camera
}
pub fn oblique_camera() -> Camera3D {
    Camera3D::orthographic([8.0, 9.0, 11.0], [0.0, 0.5, 0.0], 6.0)
}
pub fn lighting() -> Lighting {
    Lighting {
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient: 1.0,
        intensity: 0.0,
        shadows: false,
        tonemap: false,
        exposure: 1.0,
        ..Default::default()
    }
}
pub fn save_png(path: &Path, pixels: &[u8], size: (u32, u32)) -> Result<()> {
    if pixels.len()
        != (size.0 as usize)
            .saturating_mul(size.1 as usize)
            .saturating_mul(4)
    {
        return Err("PNG RGBA byte count mismatch".into());
    }
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels)?;
    Ok(())
}
