//! Explicit hardware gate: an unavailable adapter fails, never silently passes.
use orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUpload, TextureUploadError, TextureUsage, Wgpu, WgpuOptions};

#[test]
#[ignore = "requires a working GPU adapter; run with --ignored --nocapture"]
fn rgba8_upload_readback_and_rejection_preserve_pixels() {
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("GPU upload evidence requires an available adapter");
    eprintln!("texture upload adapter: {}", gpu.adapter_name());
    for format in [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb] {
        let desc = TextureDesc {
            label: "upload test",
            width: 3,
            height: 2,
            format,
            usage: TextureUsage::COPY_DST | TextureUsage::COPY_SRC | TextureUsage::TEXTURE_BINDING,
            sample_count: 1,
            view_formats: &[],
        };
        let texture = gpu.create_texture(&desc);
        let initial = [10, 20, 30, 255].repeat(6);
        gpu.write_texture_rgba8(
            &texture,
            &TextureUpload { origin: [0, 0], width: 3, height: 2, bytes_per_row: 12, data: &initial },
        )
        .unwrap();
        assert_eq!(gpu.read_texture(&texture), initial);
        // Partial region, padded source rows, and no last-row padding.
        let patch = [255, 0, 0, 255, 0, 255, 0, 128, 99, 99, 99, 99, 0, 0, 255, 64, 255, 255, 0, 0];
        let mut upload = TextureUpload { origin: [1, 0], width: 2, height: 2, bytes_per_row: 12, data: &patch };
        gpu.write_texture_rgba8(&texture, &upload).unwrap();
        let expected =
            [10, 20, 30, 255, 255, 0, 0, 255, 0, 255, 0, 128, 10, 20, 30, 255, 0, 0, 255, 64, 255, 255, 0, 0];
        assert_eq!(gpu.read_texture(&texture), expected);
        upload.origin = [2, 0];
        assert_eq!(gpu.write_texture_rgba8(&texture, &upload), Err(TextureUploadError::OutOfBounds));
        assert_eq!(gpu.read_texture(&texture), expected);
        upload.origin = [1, 0];
        upload.data = &patch[..19];
        assert_eq!(
            gpu.write_texture_rgba8(&texture, &upload),
            Err(TextureUploadError::InvalidDataLength { expected: 20, actual: 19 })
        );
        assert_eq!(gpu.read_texture(&texture), expected);
        let no_copy = gpu.create_texture(&TextureDesc { usage: TextureUsage::TEXTURE_BINDING, ..desc });
        assert_eq!(gpu.write_texture_rgba8(&no_copy, &upload), Err(TextureUploadError::MissingCopyDst));
    }
}
