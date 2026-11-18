//! Screenshot files.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use orr_render::orr_rhi::Wgpu;
use orr_render::{Camera, Camera3D, OffscreenTarget, RenderList, RenderList3D, Renderer, Renderer3D};

/// Draws `list` into an offscreen picture of exactly `size` pixels and reads it back
/// as RGBA8 (sRGB encoded, top row first). It does not depend on the window size.
pub(crate) fn render_rgba(renderer: &mut Renderer<Wgpu>, list: &RenderList, camera: &Camera, size: (u32, u32)) -> Vec<u8> {
    let rhi = renderer.rhi().clone();
    let target = OffscreenTarget::new(&rhi, size.0, size.1, renderer.format());
    target.render(renderer, list, camera);
    target.read_rgba8()
}

/// The same as [`render_rgba`] for a 3D `list`.
pub(crate) fn render_rgba3(renderer: &mut Renderer3D<Wgpu>, list: &RenderList3D, camera: &Camera3D, size: (u32, u32)) -> Vec<u8> {
    let rhi = renderer.rhi().clone();
    let target = OffscreenTarget::new(&rhi, size.0, size.1, renderer.format());
    target.render3d(renderer, list, camera);
    target.read_rgba8()
}

/// Writes tightly packed RGBA8 pixels as a PNG file.
pub(crate) fn write_png(path: &Path, size: (u32, u32), rgba: &[u8]) -> Result<(), String> {
    let expected = size.0 as usize * size.1 as usize * 4;
    if rgba.len() != expected {
        return Err(format!("picture has {} bytes, a {}x{} RGBA picture needs {expected}", rgba.len(), size.0, size.1));
    }
    let file = File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| format!("write {}: {e}", path.display()))?;
    writer.write_image_data(rgba).map_err(|e| format!("write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_written_and_reads_back_the_same() {
        let dir = std::env::temp_dir().join(format!("orr_app_png_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.png");
        let pixels: Vec<u8> = (0..4 * 3 * 4).map(|i| (i * 7) as u8).collect();

        write_png(&path, (4, 3), &pixels).unwrap();

        let decoder = png::Decoder::new(std::io::BufReader::new(File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut back = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut back).unwrap();
        assert_eq!((info.width, info.height), (4, 3));
        assert_eq!(&back[..info.buffer_size()], pixels.as_slice());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_wrong_buffer_size_is_refused_before_a_file_is_made() {
        let path = std::env::temp_dir().join("orr_app_never_written.png");
        assert!(write_png(&path, (4, 3), &[0; 10]).is_err());
        assert!(!path.exists());
    }
}
