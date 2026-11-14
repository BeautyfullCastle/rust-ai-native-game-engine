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

#[test]
#[ignore = "requires a working GPU adapter; run with --ignored --nocapture"]
fn each_mip_level_takes_an_upload_of_its_own_size() {
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("GPU upload evidence requires an available adapter");
    assert_eq!((orr_rhi::mip_level_count(4, 4), orr_rhi::mip_level_count(5, 2), orr_rhi::mip_level_count(1, 1)), (3, 3, 1));
    assert_eq!(orr_rhi::mip_level_size(5, 2, 2), (1, 1));
    let texture = gpu.create_texture_mipmapped(
        &TextureDesc { label: "mips", width: 4, height: 4, format: TextureFormat::Rgba8Unorm, usage: TextureUsage::TEXTURE_BINDING | TextureUsage::COPY_DST, sample_count: 1, view_formats: &[] },
        3,
    );
    let level = |width: u32, height: u32| vec![7u8; (width * height * 4) as usize];
    fn upload(width: u32, height: u32, data: &[u8]) -> TextureUpload<'_> {
        TextureUpload { origin: [0, 0], width, height, bytes_per_row: width * 4, data }
    }
    let (two, one) = (level(2, 2), level(1, 1));
    assert_eq!(gpu.write_texture_rgba8_mip(&texture, 1, &upload(2, 2, &two)), Ok(()));
    assert_eq!(gpu.write_texture_rgba8_mip(&texture, 2, &upload(1, 1, &one)), Ok(()));
    assert_eq!(gpu.write_texture_rgba8_mip(&texture, 1, &upload(4, 4, &level(4, 4))), Err(TextureUploadError::OutOfBounds), "level 1 is 2 x 2");
    assert_eq!(gpu.write_texture_rgba8_mip(&texture, 3, &upload(1, 1, &one)), Err(TextureUploadError::MipLevelOutOfRange));
}
