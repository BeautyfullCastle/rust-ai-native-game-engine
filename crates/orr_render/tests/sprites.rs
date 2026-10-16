//! Actual headless readback acceptance; ORR_REQUIRE_GPU=1 forbids a silent skip.
#![cfg(feature = "sprites")]
#![allow(clippy::float_arithmetic)]
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera, OffscreenTarget, SpriteDrawList, SpriteInstance, SpriteRenderError, SpriteRenderer,
};
use orr_sprite::SpriteDocument;

fn document() -> SpriteDocument {
    SpriteDocument::from_json(
        r#"{
        "format":"orr_sprite","version":1,"atlas":{"image":"test.rgba","width":4,"height":2},
        "regions":[{"id":0,"x":0,"y":0,"width":2,"height":2},
                   {"id":1,"x":2,"y":0,"width":1,"height":1},
                   {"id":2,"x":3,"y":0,"width":1,"height":1},
                   {"id":3,"x":2,"y":1,"width":1,"height":1},
                   {"id":4,"x":3,"y":1,"width":1,"height":1}],"clips":[]
    }"#,
    )
    .unwrap()
}
fn pixel(bytes: &[u8], x: usize, y: usize) -> [u8; 4] {
    bytes[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4]
        .try_into()
        .unwrap()
}
fn near(actual: [u8; 4], expected: [u8; 4]) {
    assert!(
        actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2),
        "actual {actual:?}, expected {expected:?}"
    );
}
#[test]
fn atlas_tint_flip_alpha_layer_rotation_camera_and_rejection_readback() {
    let gpu = match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => gpu,
        Err(e) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required GPU unavailable: {e}"
            );
            eprintln!("SKIP: {e}");
            return;
        }
    };
    eprintln!(
        "sprite acceptance adapter: {} (software: {})",
        gpu.adapter_name(),
        gpu.is_software()
    );
    // Region 0 is RG/BW, region 1 opaque white, 2 transparent magenta,
    // 3 encoded mid-gray, 4 half-alpha red. Distinct neighbors catch UV bleed.
    let atlas = [
        255, 0, 0, 255, 0, 255, 0, 255, 255, 255, 255, 255, 255, 0, 255, 0, 0, 0, 255, 255, 255,
        255, 255, 255, 128, 128, 128, 255, 255, 0, 0, 128,
    ];
    assert!(matches!(
        SpriteRenderer::new(
            gpu.clone(),
            TextureFormat::Rgba8Unorm,
            document(),
            &atlas[..31]
        ),
        Err(SpriteRenderError::Upload(_))
    ));
    let mut renderer =
        SpriteRenderer::new(gpu.clone(), TextureFormat::Rgba8Unorm, document(), &atlas).unwrap();
    renderer.clear = Some([0.0, 0.0, 0.0, 1.0]);
    let target = OffscreenTarget::new(&gpu, 64, 64, TextureFormat::Rgba8Unorm);
    let camera = Camera::new([0.0, 0.0], 2.0);
    let sprite = SpriteInstance {
        size: [2.0, 2.0],
        ..SpriteInstance::default()
    };
    let mut list = SpriteDrawList {
        sprites: vec![sprite],
    };
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    let pixels = target.read_rgba8();
    near(pixel(&pixels, 20, 20), [255, 0, 0, 255]);
    near(pixel(&pixels, 44, 20), [0, 255, 0, 255]);
    near(pixel(&pixels, 20, 44), [0, 0, 255, 255]);
    near(pixel(&pixels, 44, 44), [255, 255, 255, 255]);
    near(pixel(&pixels, 8, 8), [0, 0, 0, 255]);
    list.sprites[0].flip_x = true;
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    let pixels = target.read_rgba8();
    near(pixel(&pixels, 20, 20), [0, 255, 0, 255]);
    near(pixel(&pixels, 44, 44), [0, 0, 255, 255]);
    list.sprites[0].flip_x = false;
    list.sprites[0].flip_y = true;
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    let pixels = target.read_rgba8();
    near(pixel(&pixels, 20, 20), [0, 0, 255, 255]);
    near(pixel(&pixels, 44, 44), [0, 255, 0, 255]);
    list.sprites[0].flip_x = true;
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    let pixels = target.read_rgba8();
    near(pixel(&pixels, 20, 20), [255, 255, 255, 255]);
    near(pixel(&pixels, 44, 44), [255, 0, 0, 255]);
    list.sprites[0] = SpriteInstance {
        rotation: std::f32::consts::FRAC_PI_2,
        ..sprite
    };
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    let pixels = target.read_rgba8();
    near(pixel(&pixels, 20, 20), [0, 255, 0, 255]);
    near(pixel(&pixels, 44, 20), [255, 255, 255, 255]);
    // Moved/zoomed camera and tint.
    list.sprites[0] = SpriteInstance {
        region: 1,
        position: [1.0, 0.0],
        size: [0.5, 0.5],
        tint: [0.25, 0.5, 1.0, 1.0],
        ..sprite
    };
    renderer
        .draw(
            target.render_view(),
            target.size(),
            &list,
            &Camera::new([1.0, 0.0], 1.0),
        )
        .unwrap();
    let pixels = target.read_rgba8();
    near(pixel(&pixels, 32, 32), [64, 128, 255, 255]);
    near(pixel(&pixels, 20, 32), [0, 0, 0, 255]);
    let wide = OffscreenTarget::new(&gpu, 128, 64, TextureFormat::Rgba8Unorm);
    renderer
        .draw(
            wide.render_view(),
            wide.size(),
            &list,
            &Camera::new([1.0, 0.0], 1.0),
        )
        .unwrap();
    let wide_pixels = wide.read_rgba8();
    let at = |x: usize, y: usize| -> [u8; 4] {
        wide_pixels[(y * 128 + x) * 4..(y * 128 + x) * 4 + 4]
            .try_into()
            .unwrap()
    };
    near(at(64, 32), [64, 128, 255, 255]);
    near(at(52, 32), [0, 0, 0, 255]);
    // Ascending layers override insertion order, equal orders preserve it.
    list.sprites = vec![
        SpriteInstance {
            region: 1,
            tint: [1.0, 0.0, 0.0, 0.5],
            order: 2,
            ..sprite
        },
        SpriteInstance {
            region: 1,
            tint: [0.0, 0.0, 1.0, 1.0],
            order: -2,
            ..sprite
        },
        SpriteInstance {
            region: 2,
            order: 3,
            ..sprite
        },
    ];
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    near(pixel(&target.read_rgba8(), 32, 32), [128, 0, 128, 255]);
    list.sprites[1].order = 2;
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    near(pixel(&target.read_rgba8(), 32, 32), [0, 0, 255, 255]);
    // Uploaded alpha and instance alpha multiply; sRGB bytes decode to linear.
    list.sprites = vec![SpriteInstance {
        region: 4,
        tint: [1.0, 1.0, 1.0, 0.5],
        ..sprite
    }];
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    near(pixel(&target.read_rgba8(), 32, 32), [64, 0, 0, 255]);
    list.sprites[0].region = 3;
    list.sprites[0].tint = [1.0; 4];
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    near(pixel(&target.read_rgba8(), 32, 32), [55, 55, 55, 255]);
    // Only the sRGB render view is needed here. OffscreenTarget also creates
    // a linear sampling view for egui, which requires VIEW_FORMATS on GLES.
    let srgb_target = gpu.create_texture(&TextureDesc {
        label: "sprite sRGB readback",
        width: 64,
        height: 64,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let srgb_view = gpu.create_texture_view(&srgb_target, None);
    let mut srgb_renderer = SpriteRenderer::new(
        gpu.clone(),
        TextureFormat::Rgba8UnormSrgb,
        document(),
        &atlas,
    )
    .unwrap();
    srgb_renderer
        .draw(&srgb_view, (64, 64), &list, &camera)
        .unwrap();
    near(
        pixel(&gpu.read_texture(&srgb_target), 32, 32),
        [128, 128, 128, 255],
    );
    // Rejection never clears/submits a partially valid draw.
    let before = target.read_rgba8();
    list.sprites.push(SpriteInstance {
        region: 999,
        ..sprite
    });
    assert_eq!(
        renderer.draw(target.render_view(), target.size(), &list, &camera),
        Err(SpriteRenderError::MissingRegion(999))
    );
    assert_eq!(target.read_rgba8(), before);
    list.sprites.pop();
    list.sprites[0].position[0] = f32::NAN;
    assert_eq!(
        renderer.draw(target.render_view(), target.size(), &list, &camera),
        Err(SpriteRenderError::InvalidInstance(0))
    );
    assert_eq!(target.read_rgba8(), before);
    assert_eq!(
        renderer.draw(
            target.render_view(),
            target.size(),
            &SpriteDrawList::default(),
            &Camera {
                center: [0.0; 2],
                half_extent: 0.0
            }
        ),
        Err(SpriteRenderError::InvalidCamera)
    );
    assert_eq!(target.read_rgba8(), before);
    assert_eq!(
        renderer.draw(
            target.render_view(),
            target.size(),
            &SpriteDrawList::default(),
            &Camera {
                center: [0.0; 2],
                half_extent: f32::from_bits(1)
            }
        ),
        Err(SpriteRenderError::InvalidCamera)
    );
    assert_eq!(target.read_rgba8(), before);
    list.clear();
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    near(pixel(&target.read_rgba8(), 32, 32), [0, 0, 0, 255]);
    // Grow beyond the initial instance buffer capacity, then reuse it.
    list.sprites = vec![
        SpriteInstance {
            region: 1,
            ..sprite
        };
        1025
    ];
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    near(pixel(&target.read_rgba8(), 32, 32), [255, 255, 255, 255]);
    list.clear();
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    // Overlay retains the previous clear where no sprite is drawn.
    renderer.clear = None;
    list.push(SpriteInstance {
        region: 1,
        ..sprite
    });
    renderer
        .draw(target.render_view(), target.size(), &list, &camera)
        .unwrap();
    near(pixel(&target.read_rgba8(), 32, 32), [255, 255, 255, 255]);
    near(pixel(&target.read_rgba8(), 8, 8), [0, 0, 0, 255]);
}
