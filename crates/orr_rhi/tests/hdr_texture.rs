//! Explicit software-adapter gate: failure to obtain or use HDR is a failure.
#![allow(clippy::float_arithmetic)] // View-layer pixel assertions use floats.

use orr_rhi::{
    decode_rgba16f_texel, Binding, Blend, ColorAttachment, Command, PipelineDesc, Rhi, SamplerDesc, TextureDesc,
    TextureFormat, TextureFormatCapabilities, TextureUpload, TextureUploadError, TextureUsage, Wgpu, WgpuOptions,
};

const SHADER: &str = r#"
@vertex fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(positions[index], 0.0, 1.0);
}
@fragment fn source(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4(2.0 + floor(pixel.x), -1.0 - floor(pixel.y), 0.5, 1.0);
}
@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@fragment fn sampled(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(source_texture, source_sampler, pixel.xy / vec2(35.0, 3.0));
    return vec4(color.rgb, 0.5);
}
"#;

#[test]
fn rgba16f_render_filter_blend_and_padded_readback_preserve_hdr() {
    let gpu = Wgpu::headless(WgpuOptions { force_software: true, ..WgpuOptions::default() })
        .expect("HDR RHI evidence requires an available software adapter");
    eprintln!("HDR RHI adapter: {} (software: {})", gpu.adapter_name(), gpu.is_software());
    assert!(gpu.is_software());
    let features_before = gpu.device().features();
    let caps = gpu.texture_format_capabilities(TextureFormat::Rgba16Float);
    let usage = TextureUsage::RENDER_ATTACHMENT | TextureUsage::TEXTURE_BINDING | TextureUsage::COPY_SRC;
    assert!(caps.usages.contains(usage), "software adapter must support HDR usages: {caps:?}");
    assert!(caps.filterable && caps.blendable, "software adapter must filter and blend HDR: {caps:?}");
    assert!(caps.supports_hdr_postprocessing);
    assert!(caps.max_dimension_2d >= 35);
    assert!(caps.max_readback_buffer_size >= 512 * 3);
    assert!(gpu.sample_count_supported(TextureFormat::Rgba16Float, 1));
    assert!(!gpu.sample_count_supported(TextureFormat::Rgba16Float, 0));
    // A restriction and its clone must fail closed, without mutating the shared
    // device or a previously cloned unrestricted handle.
    let restricted = gpu.clone().without_hdr_support();
    assert_eq!(
        restricted.texture_format_capabilities(TextureFormat::Rgba16Float),
        TextureFormatCapabilities::default()
    );
    assert_eq!(
        restricted.clone().texture_format_capabilities(TextureFormat::Rgba16Float),
        TextureFormatCapabilities::default()
    );
    assert!(!restricted.sample_count_supported(TextureFormat::Rgba16Float, 1));
    assert_eq!(gpu.texture_format_capabilities(TextureFormat::Rgba16Float), caps);
    assert_eq!(restricted.device().features(), features_before);
    assert_eq!(
        restricted.texture_format_capabilities(TextureFormat::Rgba8Unorm),
        gpu.texture_format_capabilities(TextureFormat::Rgba8Unorm),
    );

    let desc = TextureDesc {
        label: "HDR readback test",
        width: 35,
        height: 3,
        format: TextureFormat::Rgba16Float,
        usage,
        sample_count: 1,
        view_formats: &[],
    };
    let source = gpu.create_texture(&desc);
    let source_view = gpu.create_texture_view(&source, None);
    let destination = gpu.create_texture(&desc);
    let destination_view = gpu.create_texture_view(&destination, None);
    let shader = gpu.create_shader("HDR test", SHADER);
    let source_pipeline = gpu.create_pipeline(&PipelineDesc::color(
        "HDR source",
        &shader,
        ("vs", "source"),
        &[],
        TextureFormat::Rgba16Float,
        Blend::Opaque,
    ));
    let sample_pipeline = gpu.create_pipeline(&PipelineDesc::color(
        "HDR sample",
        &shader,
        ("vs", "sampled"),
        &[],
        TextureFormat::Rgba16Float,
        Blend::Alpha,
    ));
    let sampler = gpu.create_sampler(&SamplerDesc { linear: true, compare: false });
    let bindings = gpu.create_bind_group(
        &sample_pipeline,
        0,
        &[Binding::Texture { binding: 0, view: &source_view }, Binding::Sampler { binding: 1, sampler: &sampler }],
    );
    let mut encoder = gpu.create_encoder("HDR test");
    gpu.encode_render_pass(
        &mut encoder,
        "HDR source",
        &ColorAttachment { view: &source_view, clear: Some([0.0; 4]), resolve: None },
        &[Command::SetPipeline(&source_pipeline), Command::Draw { vertices: 0..3, instances: 0..1 }],
    );
    gpu.encode_render_pass(
        &mut encoder,
        "HDR filtered alpha blend",
        &ColorAttachment { view: &destination_view, clear: Some([0.5, 0.25, 0.75, 1.0]), resolve: None },
        &[
            Command::SetPipeline(&sample_pipeline),
            Command::SetBindGroup(0, &bindings),
            Command::Draw { vertices: 0..3, instances: 0..1 },
        ],
    );
    gpu.submit(encoder);
    let source_bytes = gpu.read_texture(&source);
    let destination_bytes = gpu.read_texture(&destination);
    // 280-byte texel rows require 512-byte GPU copy rows. Verify that no row
    // padding leaks and every row/column remains in its original position.
    assert_eq!(source_bytes.len(), 35 * 3 * 8);
    assert_eq!(destination_bytes.len(), 35 * 3 * 8);
    for (index, (source, destination)) in
        source_bytes.chunks_exact(8).zip(destination_bytes.chunks_exact(8)).enumerate()
    {
        let x = (index % 35) as f32;
        let y = (index / 35) as f32;
        assert_eq!(decode_rgba16f_texel(source.try_into().unwrap()), [2.0 + x, -1.0 - y, 0.5, 1.0]);
        let expected = [1.25 + x * 0.5, -0.375 - y * 0.5, 0.625, 1.0];
        let actual = decode_rgba16f_texel(destination.try_into().unwrap());
        for channel in 0..4 {
            assert!(
                (actual[channel] - expected[channel]).abs() <= 0.016,
                "pixel {index}, actual {actual:?}, expected {expected:?}"
            );
        }
    }
    assert_eq!(gpu.device().features(), features_before);
    // The byte upload API remains RGBA8-only and cannot reinterpret FP16 data.
    assert_eq!(
        gpu.write_texture_rgba8(
            &source,
            &TextureUpload { origin: [0, 0], width: 1, height: 1, bytes_per_row: 4, data: &[0; 4] }
        ),
        Err(TextureUploadError::UnsupportedFormat),
    );
    assert_eq!(gpu.read_texture(&source), source_bytes);
}
