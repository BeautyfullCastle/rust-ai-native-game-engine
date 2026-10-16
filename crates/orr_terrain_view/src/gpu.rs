//! Existing immutable ModelRenderer integration. A model rebuild/reupload is
//! required after every terrain edit or marker change. Camera/light/size changes
//! reuse it. Each draw clears its own color/depth; this is not an overlay into
//! Renderer3D's depth buffer. No renderer APIs are added or modified.
use crate::{to_static_model, QueryMarker};
use orr_render::{
    orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu},
    Camera3D, Lighting, ModelRenderer,
};
use orr_terrain::Terrain;
use std::{fs::File, io::BufWriter, path::Path};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub const FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;
pub const CLEAR: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

pub fn renderer(
    gpu: &Wgpu,
    terrain: &Terrain,
    markers: &[QueryMarker],
) -> Result<ModelRenderer<Wgpu>> {
    let model = to_static_model(terrain, markers)?
        .ok_or("empty terrain has no model; clear the target without a model draw")?;
    let mut renderer = ModelRenderer::new(gpu.clone(), FORMAT, model)?;
    renderer.clear = CLEAR;
    Ok(renderer)
}
pub fn target(gpu: &Wgpu, size: (u32, u32)) -> <Wgpu as Rhi>::Texture {
    gpu.create_texture(&TextureDesc {
        label: "heightfield terrain readback",
        width: size.0,
        height: size.1,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    })
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
    let texture = target(&gpu, size);
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
    Camera3D::orthographic([8.0, 8.0, 11.0], [0.0, 0.5, 0.0], 6.0)
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
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels)?;
    Ok(())
}
