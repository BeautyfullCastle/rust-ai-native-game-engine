//! Offscreen color/sampling contract on both native alternate-view adapters and
//! downlevel GL (which needs the encoded-byte mirror). Uses the existing GPU
//! harness: mandatory ORR_REQUIRE_GPU=1 lanes fail if no adapter is available.
#![allow(clippy::float_arithmetic)]

use orr_render::orr_rhi::{Binding, Blend, ColorAttachment, Command, PipelineDesc, Rhi, TextureDesc, TextureFormat, TextureUpload, TextureUsage};
use orr_render::{Camera, Camera3D, OffscreenTarget, RenderList, RenderList3D, Renderer, Renderer3D, Settings3D};

// Sampling to a UNORM attachment models egui's gamma-space native-texture input.
// textureLoad at integer coordinates makes every channel, alpha and row testable.
const SAMPLE: &str = r#"
@group(0) @binding(0) var image: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1., -1.), vec2<f32>(3., -1.), vec2<f32>(-1., 3.));
    return vec4<f32>(p[i], 0., 1.);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(image, vec2<i32>(p.xy), 0);
}
"#;

fn assert_pixel(image: &[u8], size: (u32, u32), expected: [u8; 4]) {
    let offset = ((size.1 / 2 * size.0 + size.0 / 2) * 4) as usize;
    let pixel = &image[offset..offset + 4];
    for (actual, expected) in pixel.iter().zip(expected) {
        assert!(actual.abs_diff(expected) <= 2, "pixel {pixel:?}, expected {expected:?}");
    }
}

#[test]
fn offscreen_sampling_preserves_encoding_alpha_cached_views_and_resize() {
    let Some((_guard, gpu)) = super::gpu() else { return };
    eprintln!("offscreen adapter: {:?}; alternate views: {}", gpu.adapter().get_info(), gpu.texture_view_formats_supported());
    if let Ok(expected) = std::env::var("ORR_EXPECT_VIEW_FORMATS") {
        assert_eq!(gpu.texture_view_formats_supported(), expected == "1", "wrong adapter capability for this lane");
    }
    for format in [TextureFormat::Rgba8UnormSrgb, TextureFormat::Bgra8UnormSrgb,
                   TextureFormat::Rgba8Unorm, TextureFormat::Bgra8Unorm] {
        let mut target = OffscreenTarget::new(&gpu, 13, 7, format);
        assert_eq!(target.format(), format, "never replace the requested render encoding");
        let mut renderer = Renderer::new(gpu.clone(), format);
        let camera = Camera::new([0.0, 0.0], 4.0);
        let shader = gpu.create_shader("sample view regression", SAMPLE);
        let pipeline = gpu.create_pipeline(&PipelineDesc::color(
            "sample view regression", &shader, ("vs", "fs"), &[], TextureFormat::Rgba8Unorm, Blend::Opaque,
        ));
        for size in [(13, 7), (17, 5), (1, 1)] {
            let generation = target.generation();
            let changed = target.resize(size.0, size.1);
            assert_eq!(target.generation(), generation + u64::from(changed));
            assert!(!target.resize(size.0, size.1));
            let sampled = OffscreenTarget::new(&gpu, size.0, size.1, TextureFormat::Rgba8Unorm);
            // Register once, reuse after every render, exactly like an egui texture ID.
            let bind = gpu.create_bind_group(&pipeline, 0, &[Binding::Texture { binding: 0, view: target.sample_view() }]);
            let read_sample = || {
                let mut encoder = gpu.create_encoder("sample cached view");
                gpu.encode_render_pass(&mut encoder, "sample", &ColorAttachment {
                    view: sampled.render_view(), clear: Some([0.; 4]), resolve: None,
                }, &[Command::SetPipeline(&pipeline), Command::SetBindGroup(0, &bind),
                     Command::Draw { vertices: 0..3, instances: 0..1 }]);
                gpu.submit(encoder);
                sampled.read_rgba8()
            };
            // Newly allocated textures/mirrors agree, including alpha and odd row widths.
            assert_eq!(read_sample(), target.read_rgba8());
            for mode in 0..5 {
                renderer.clear = Some([0.25, 0.0, 0.0, 0.25]);
                let mut list = RenderList::new();
                list.quad([0., 0.], [20., 20.], 0., [0., 0.5, 0., 0.5]);
                match mode {
                    0 => target.render(&mut renderer, &list, &camera),
                    1 => { // Custom multi-pass path, including a returned error after submission.
                        let result = target.render_into(|_, view, size| {
                            renderer.draw(view, size, &list, &camera);
                            Err::<(), _>("after submission")
                        });
                        assert_eq!(result, Err("after submission"));
                    }
                    2 => { // Low-level clients have an explicit synchronization boundary.
                        renderer.draw(target.render_view(), target.size(), &list, &camera);
                        target.update_sample_view();
                    }
                    3 => { // Empty frame replaces prior contents, rather than a stale UI image.
                        renderer.clear = Some([0.5, 0.125, 0.0, 0.5]);
                        target.render(&mut renderer, &RenderList::new(), &camera);
                    }
                    _ => { // Asymmetric colors/alpha expose flipped rows, channel swaps and low-end gamma.
                        renderer.clear = Some([0.002, 0.04, 0.8, 0.2]);
                        list.clear();
                        list.quad([-3., 2.], [1., 1.], 0., [0.004, 0.18, 0.003, 0.75]);
                        target.render(&mut renderer, &list, &camera);
                    }
                }
                let stored = target.read_rgba8();
                assert_eq!(read_sample(), stored, "cached view stale or color-converted: {format:?}, {size:?}, mode {mode}");
                let expected = match (format.is_srgb(), mode) {
                    (true, 0..=2) => [99, 137, 0, 159], // linear blend: RGB=(.125,.25,0), A=.625
                    (false, 0..=2) => [32, 64, 0, 159],
                    (true, 3) => [188, 99, 0, 128],
                    (false, 3) => [128, 32, 0, 128],
                    (true, _) => [7, 56, 231, 51],
                    (false, _) => [1, 10, 204, 51],
                };
                assert_pixel(&stored, size, expected);
            }
        }
        assert!(!target.resize(0, 0)); // clamped size already 1x1
    }
}

#[test]
fn offscreen_sampling_tracks_msaa_3d_and_linear_2d_compositing() {
    let Some((_guard, gpu)) = super::gpu() else { return };
    let format = TextureFormat::Rgba8UnormSrgb;
    let target = OffscreenTarget::new(&gpu, 32, 16, format);
    let shader = gpu.create_shader("sample 3d regression", SAMPLE);
    let pipeline = gpu.create_pipeline(&PipelineDesc::color(
        "sample 3d regression", &shader, ("vs", "fs"), &[], TextureFormat::Rgba8Unorm, Blend::Opaque,
    ));
    let bind = gpu.create_bind_group(&pipeline, 0, &[Binding::Texture { binding: 0, view: target.sample_view() }]);
    let sampled = OffscreenTarget::new(&gpu, 32, 16, TextureFormat::Rgba8Unorm);
    let camera = Camera3D::perspective([0., 0., 5.], [0.; 3], 45.);
    let list = RenderList3D::new();
    let mut overlay_renderer = Renderer::new(gpu.clone(), format);
    overlay_renderer.clear = Some([0.; 4]);
    let mut overlay = RenderList::new();
    overlay.quad([16., 8.], [40., 40.], 0., [0., 0.5, 0., 0.5]);
    for msaa in [1, 4] {
        let mut renderer = Renderer3D::with_settings(gpu.clone(), format, Settings3D { msaa, ..Settings3D::LOW });
        renderer.clear = [0.25, 0., 0., 0.25];
        for composite in [false, true, false] {
            if composite {
                target.render3d_overlay(&mut renderer, &list, &camera, &mut overlay_renderer, &overlay);
                assert_eq!(overlay_renderer.clear, Some([0.; 4]));
            } else {
                target.render3d(&mut renderer, &list, &camera);
            }
            let mut encoder = gpu.create_encoder("sample 3d cached view");
            gpu.encode_render_pass(&mut encoder, "sample", &ColorAttachment {
                view: sampled.render_view(), clear: Some([0.; 4]), resolve: None,
            }, &[Command::SetPipeline(&pipeline), Command::SetBindGroup(0, &bind),
                 Command::Draw { vertices: 0..3, instances: 0..1 }]);
            gpu.submit(encoder);
            let stored = target.read_rgba8();
            assert_eq!(sampled.read_rgba8(), stored, "3D mirror stale: msaa={msaa}, overlay={composite}");
            assert_pixel(&stored, target.size(), if composite { [99, 137, 0, 159] } else { [137, 0, 0, 64] });
            assert_eq!(renderer.last_frame_stats().msaa_samples, gpu.supported_samples(format, msaa));
            assert_eq!(renderer.last_frame_stats().upload_calls, 2);
            assert_eq!(renderer.last_frame_stats().upload_bytes, 2 * 288);
        }
    }
}

#[test]
fn srgb_unorm_gpu_copy_preserves_every_channel_byte_in_both_directions() {
    let Some((_guard, gpu)) = super::gpu() else { return };
    eprintln!("copy adapter: {:?}", gpu.adapter().get_info());
    let bytes: Vec<u8> = (0..256u16)
        .flat_map(|v| [v as u8, (255 - v) as u8, (v ^ 0x55) as u8, (v ^ 0xaa) as u8])
        .collect();
    for (source_format, destination_format) in [
        (TextureFormat::Rgba8UnormSrgb, TextureFormat::Rgba8Unorm),
        (TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb),
    ] {
        let desc = TextureDesc {
            label: "copy test",
            width: 16,
            height: 16,
            format: source_format,
            usage: TextureUsage::COPY_DST | TextureUsage::COPY_SRC | TextureUsage::TEXTURE_BINDING,
            sample_count: 1,
            view_formats: &[],
        };
        let source = gpu.create_texture(&desc);
        let destination = gpu.create_texture(&TextureDesc { format: destination_format, ..desc });
        gpu.write_texture_rgba8(&source, &TextureUpload {
            origin: [0, 0], width: 16, height: 16, bytes_per_row: 64, data: &bytes,
        }).unwrap();
        gpu.copy_texture(&source, &destination);
        assert_eq!(gpu.read_texture(&destination), bytes);
    }
}
