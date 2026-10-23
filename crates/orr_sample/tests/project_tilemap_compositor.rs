//! Bounded background composition contracts. Recording RHI is CPU evidence,
//! never GPU evidence. ORR_REQUIRE_GPU=1 makes absent GPU acceptance fatal.
#![cfg(feature = "project")]
#![allow(clippy::float_arithmetic)]

use orr_render::orr_rhi::*;
use orr_render::{Camera, OffscreenTarget, RenderList, SpriteInstance};
use orr_sample::project_compositor::{MAX_PROJECT_SPRITES, ProjectCompositor, SpriteDraw};
use orr_sample::project_sprites::{Asset, AssetKey};
use orr_sprite::SpriteDocument;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

#[derive(Clone, Debug, PartialEq)]
struct Pass {
    label: String,
    clear: Option<[f64; 4]>,
    atlas: Option<u32>,
    instances: u32,
}
#[derive(Clone, Debug, Default, PartialEq)]
struct Effects {
    allocations: usize,
    writes: usize,
    submits: usize,
    passes: Vec<Pass>,
    instance_uploads: Vec<Vec<u8>>,
}
#[derive(Clone, Default)]
struct Recording(Rc<RefCell<Effects>>);
impl Rhi for Recording {
    type Buffer = String;
    type Texture = u32;
    type TextureView = u32;
    type Shader = ();
    type Pipeline = ();
    type BindGroup = Option<u32>;
    type Sampler = ();
    type Encoder = ();
    type Surface = ();
    type Frame = u32;
    fn adapter_name(&self) -> String {
        "recording CPU backend, not GPU evidence".into()
    }
    fn create_buffer(&self, d: &BufferDesc) -> String {
        self.0.borrow_mut().allocations += 1;
        d.label.into()
    }
    fn write_buffer(&self, b: &String, _: u64, bytes: &[u8]) {
        let mut e = self.0.borrow_mut();
        e.writes += 1;
        if b == "sprite instances" {
            e.instance_uploads.push(bytes.to_vec());
        }
    }
    fn texture_view_formats_supported(&self) -> bool {
        true
    }
    fn create_texture(&self, _: &TextureDesc) -> u32 {
        let mut e = self.0.borrow_mut();
        e.allocations += 1;
        e.allocations as u32
    }
    fn copy_texture(&self, _: &u32, _: &u32) {
        panic!("not an offscreen target")
    }
    fn write_texture_rgba8(
        &self,
        _: &u32,
        _: &TextureUpload<'_>,
    ) -> Result<(), TextureUploadError> {
        Ok(())
    }
    fn create_texture_view(&self, t: &u32, _: Option<TextureFormat>) -> u32 {
        *t
    }
    fn create_sampler(&self, _: &SamplerDesc) {}
    fn sample_count_supported(&self, _: TextureFormat, n: u32) -> bool {
        n == 1
    }
    fn create_shader(&self, _: &str, _: &str) {}
    fn create_pipeline(&self, _: &PipelineDesc<Self>) {}
    fn create_bind_group(&self, _: &(), _: u32, bindings: &[Binding<Self>]) -> Option<u32> {
        bindings.iter().find_map(|b| match b {
            Binding::Texture { view, .. } => Some(**view),
            _ => None,
        })
    }
    fn create_encoder(&self, _: &str) {}
    fn encode_pass(
        &self,
        _: &mut (),
        label: &str,
        color: Option<&ColorAttachment<Self>>,
        depth: Option<&DepthAttachment<Self>>,
        commands: &[Command<Self>],
    ) {
        assert!(depth.is_none());
        let mut atlas = None;
        let mut instances = 0;
        for command in commands {
            match command {
                Command::SetBindGroup(_, group) => atlas = **group,
                Command::Draw {
                    instances: range, ..
                } => instances += range.end - range.start,
                _ => {}
            }
        }
        self.0.borrow_mut().passes.push(Pass {
            label: label.into(),
            clear: color.unwrap().clear,
            atlas,
            instances,
        });
    }
    fn submit(&self, _: ()) {
        self.0.borrow_mut().submits += 1;
    }
    fn wait_idle(&self) {}
    fn read_texture(&self, _: &u32) -> Vec<u8> {
        panic!("recording is not GPU evidence")
    }
    fn surface_format(&self, _: &()) -> TextureFormat {
        TextureFormat::Rgba8Unorm
    }
    fn resize_surface(&self, _: &mut (), _: u32, _: u32) {}
    fn acquire_frame(&self, _: &mut ()) -> Acquire<u32> {
        panic!("no window")
    }
    fn read_frame(&self, _: &u32) -> Option<Vec<u8>> {
        None
    }
    fn frame_view<'a>(&self, frame: &'a u32) -> &'a u32 {
        frame
    }
    fn present(&self, _: u32) {
        panic!("no window")
    }
}

fn key(name: &str) -> AssetKey {
    (name.into(), "sprites.json".into())
}
fn solid(rgba: [u8; 4]) -> Asset {
    Asset {
        document: SpriteDocument::from_json(r#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":1,"height":1},"regions":[{"id":0,"x":0,"y":0,"width":1,"height":1}],"clips":[]}"#).unwrap(),
        rgba: rgba.to_vec(),
    }
}
fn sprite(name: &str, x: f32, order: i32) -> SpriteDraw {
    SpriteDraw {
        asset: key(name),
        instance: SpriteInstance {
            position: [x, 0.0],
            order,
            ..Default::default()
        },
    }
}
fn camera() -> Camera {
    Camera::new([0.0, 0.0], 1.0)
}

#[test]
fn recording_batch_proves_pass_order_atomic_admission_and_clear_restoration() {
    let rhi = Recording::default();
    let assets = BTreeMap::from([(key("a"), solid([255; 4])), (key("b"), solid([255; 4]))]);
    let mut compositor =
        ProjectCompositor::new(rhi.clone(), TextureFormat::Rgba8Unorm, &assets).unwrap();
    let mut shapes = RenderList::new();
    shapes.quad([0.0; 2], [0.5; 2], 0.0, [1.0; 4]);
    let actors = [
        sprite("a", 1.0, 99),
        sprite("b", 2.0, -99),
        sprite("a", 3.0, -999),
    ];
    let background = [sprite("a", -2.0, 999), sprite("a", -1.0, -999)];
    *rhi.0.borrow_mut() = Effects::default();
    compositor
        .draw_with_background(&0, (96, 64), &shapes, &background, &actors, &camera())
        .unwrap();
    let composed = rhi.0.borrow().clone();
    assert_eq!(composed.submits, 5); // One background + shapes + A/B/A.
    assert_eq!(
        composed
            .passes
            .iter()
            .map(|p| p.label.as_str())
            .collect::<Vec<_>>(),
        ["sprites", "2d", "sprites", "sprites", "sprites"]
    );
    assert_eq!(
        composed.passes.iter().filter(|p| p.clear.is_some()).count(),
        1
    );
    assert_eq!(composed.passes[0].clear, Some(orr_render::DEFAULT_CLEAR));
    assert!(composed.passes[1..].iter().all(|p| p.clear.is_none()));
    assert_eq!(composed.passes[0].atlas, composed.passes[2].atlas);
    assert_ne!(composed.passes[2].atlas, composed.passes[3].atlas);
    assert_eq!(composed.passes[2].atlas, composed.passes[4].atlas);
    assert_eq!(
        composed
            .passes
            .iter()
            .map(|p| p.instances)
            .collect::<Vec<_>>(),
        [2, 1, 1, 1, 1]
    );
    // GPU upload records use 52-byte sprite instances. Caller order wins over
    // the intentionally reversed order field within the first atlas run.
    let bytes = &composed.instance_uploads[0];
    assert_eq!(bytes.len(), 104);
    assert_eq!(f32::from_ne_bytes(bytes[0..4].try_into().unwrap()), -2.0);
    assert_eq!(f32::from_ne_bytes(bytes[52..56].try_into().unwrap()), -1.0);

    let mut bad_inputs = Vec::new();
    let mut bad = sprite("a", 0.0, 0);
    bad.instance.region = 9;
    bad_inputs.push(bad);
    let mut bad = sprite("a", 0.0, 0);
    bad.instance.tint[0] = f32::NAN;
    bad_inputs.push(bad);
    bad_inputs.push(sprite("missing", 0.0, 0));
    for bad in bad_inputs {
        *rhi.0.borrow_mut() = Effects::default();
        assert!(
            compositor
                .draw_with_background(
                    &0,
                    (96, 64),
                    &shapes,
                    &background,
                    &[actors[0].clone(), bad],
                    &camera()
                )
                .is_err()
        );
        assert_eq!(*rhi.0.borrow(), Effects::default());
    }
    for field in 0..3 {
        let mut bad = sprite("a", 0.0, 0);
        match field {
            0 => bad.instance.flip_x = true,
            1 => bad.instance.flip_y = true,
            _ => bad.instance.rotation = 0.1,
        }
        assert!(
            compositor
                .draw_with_background(&0, (96, 64), &shapes, &[bad], &actors, &camera())
                .is_err()
        );
        assert_eq!(*rhi.0.borrow(), Effects::default());
    }
    let oversized = vec![sprite("a", 0.0, 0); MAX_PROJECT_SPRITES];
    assert!(
        compositor
            .draw_with_background(&0, (96, 64), &shapes, &background, &oversized, &camera())
            .is_err()
    );
    let mut bad_camera = camera();
    bad_camera.center[1] = f32::INFINITY;
    assert!(
        compositor
            .draw_with_background(&0, (96, 64), &shapes, &background, &actors, &bad_camera)
            .is_err()
    );
    assert_eq!(*rhi.0.borrow(), Effects::default());

    let mut bad_shapes = shapes.clone();
    bad_shapes.shapes[0].center[0] = f32::NAN;
    assert!(
        compositor
            .draw_with_background(&0, (96, 64), &bad_shapes, &background, &actors, &camera())
            .is_err()
    );
    let mut bad_lines = shapes.clone();
    bad_lines.line([0.0; 2], [1.0; 2], f32::INFINITY, [1.0; 4]);
    assert!(
        compositor
            .draw_with_background(&0, (96, 64), &bad_lines, &background, &actors, &camera())
            .is_err()
    );
    assert_eq!(*rhi.0.borrow(), Effects::default());

    // Failed admission must not call an overlay or produce GPU effects.
    assert!(
        compositor
            .draw_with_background_and_overlay(
                &0,
                (96, 64),
                &bad_shapes,
                &background,
                &actors,
                &camera(),
                |_, _, _| panic!("overlay ran after failed admission")
            )
            .is_err()
    );
    assert_eq!(*rhi.0.borrow(), Effects::default());

    // A failed overlay occurs after valid drawing. It must not poison the next
    // ordinary draw's clear state.
    assert!(
        compositor
            .draw_with_background_and_overlay(
                &0,
                (96, 64),
                &shapes,
                &background,
                &actors,
                &camera(),
                |_, _, _| Err("overlay failure".into())
            )
            .is_err()
    );
    *rhi.0.borrow_mut() = Effects::default();
    compositor
        .draw(&0, (96, 64), &shapes, &actors, &camera())
        .unwrap();
    let original = rhi.0.borrow().clone();
    assert_eq!(original.submits + 1, composed.submits);
    assert_eq!(original.passes[0].clear, Some(orr_render::DEFAULT_CLEAR));
    assert!(original.passes[1..].iter().all(|p| p.clear.is_none()));
    *rhi.0.borrow_mut() = Effects::default();
    compositor
        .draw_with_background(&0, (96, 64), &shapes, &[], &actors, &camera())
        .unwrap();
    assert_eq!(*rhi.0.borrow(), original);

    // Exactly the combined maximum is legal; one extra background is not.
    *rhi.0.borrow_mut() = Effects::default();
    compositor
        .draw_with_background(
            &0,
            (96, 64),
            &shapes,
            &background[..1],
            &oversized[..MAX_PROJECT_SPRITES - 1],
            &camera(),
        )
        .unwrap();
    let mut too_many_assets = assets;
    for i in 0..7 {
        too_many_assets.insert(key(&format!("extra{i}")), solid([255; 4]));
    }
    let mut compositor =
        ProjectCompositor::new(rhi.clone(), TextureFormat::Rgba8Unorm, &too_many_assets).unwrap();
    *rhi.0.borrow_mut() = Effects::default();
    assert!(
        compositor
            .draw_with_background(&0, (96, 64), &shapes, &background, &actors, &camera())
            .is_err()
    );
    assert_eq!(*rhi.0.borrow(), Effects::default());
}

fn asymmetric_atlas() -> Asset {
    // Four 2x2 regions laid out horizontally. Literal texels are the oracle,
    // independent of any tilemap coordinate/region expansion implementation.
    let colors = [
        [
            [255, 0, 0, 255],
            [255, 255, 0, 255],
            [0, 255, 255, 255],
            [255, 255, 255, 255],
        ],
        [
            [0, 0, 255, 255],
            [255, 255, 255, 255],
            [255, 0, 0, 255],
            [255, 255, 0, 255],
        ],
        [
            [0, 255, 0, 255],
            [0, 255, 255, 255],
            [255, 0, 255, 255],
            [0, 0, 255, 255],
        ],
        [
            [255, 255, 0, 255],
            [255, 0, 255, 255],
            [0, 0, 255, 255],
            [255, 0, 0, 255],
        ],
    ];
    let mut rgba = Vec::new();
    for row in 0..2 {
        for region in &colors {
            for col in 0..2 {
                rgba.extend(region[row * 2 + col]);
            }
        }
    }
    Asset {
        document: SpriteDocument::from_json(r#"{"format":"orr_sprite","version":1,"atlas":{"image":"atlas.png","width":8,"height":2},"regions":[{"id":0,"x":0,"y":0,"width":2,"height":2},{"id":1,"x":2,"y":0,"width":2,"height":2},{"id":2,"x":4,"y":0,"width":2,"height":2},{"id":3,"x":6,"y":0,"width":2,"height":2}],"clips":[]}"#).unwrap(),
        rgba,
    }
}
fn pixel(bytes: &[u8], x: usize, y: usize) -> [u8; 4] {
    bytes[(y * 96 + x) * 4..(y * 96 + x) * 4 + 4]
        .try_into()
        .unwrap()
}

#[test]
fn gpu_asymmetric_three_by_two_blank_shape_actor_and_wrong_region() {
    let rhi = match Wgpu::headless(WgpuOptions::default()) {
        Ok(rhi) => rhi,
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "required GPU unavailable: {error}"
            );
            eprintln!("SKIP GPU tile background acceptance: {error}");
            return;
        }
    };
    eprintln!("tile background GPU: {}", rhi.adapter_name());
    if std::env::var("ORR_REQUIRE_GPU").as_deref() == Ok("1") {
        let info = rhi.adapter().get_info();
        assert_eq!(format!("{:?}", info.backend), "Vulkan");
        assert_eq!(format!("{:?}", info.device_type), "Cpu");
    }
    let assets = BTreeMap::from([
        (key("tiles"), asymmetric_atlas()),
        (key("actor"), solid([255, 0, 255, 255])),
    ]);
    let mut compositor =
        ProjectCompositor::new(rhi.clone(), TextureFormat::Rgba8Unorm, &assets).unwrap();
    let target = OffscreenTarget::new(&rhi, 96, 64, TextureFormat::Rgba8Unorm);
    // Row-major grid [0,2,empty;3,1,0], world y points up.
    let background: Vec<_> = [
        (0, [-1.0, 0.5]),
        (2, [0.0, 0.5]),
        (3, [-1.0, -0.5]),
        (1, [0.0, -0.5]),
        (0, [1.0, -0.5]),
    ]
    .into_iter()
    .map(|(region, position)| SpriteDraw {
        asset: key("tiles"),
        instance: SpriteInstance {
            region,
            position,
            ..Default::default()
        },
    })
    .collect();
    let mut shapes = RenderList::new();
    shapes.quad([-1.0, 0.5], [0.25, 0.25], 0.0, [0.0, 1.0, 0.0, 1.0]);
    let actors = [SpriteDraw {
        asset: key("actor"),
        instance: SpriteInstance {
            position: [-1.0, 0.5],
            size: [0.25, 0.25],
            ..Default::default()
        },
    }];
    target
        .render_into(|_, view, size| compositor.draw(view, size, &shapes, &actors, &camera()))
        .unwrap();
    let original = target.read_rgba8();
    target
        .render_into(|_, view, size| {
            compositor.draw_with_background(view, size, &shapes, &[], &actors, &camera())
        })
        .unwrap();
    assert_eq!(target.read_rgba8(), original, "no-tile path byte parity");
    target
        .render_into(|_, view, size| {
            compositor.draw_with_background(view, size, &shapes, &background, &actors, &camera())
        })
        .unwrap();
    let composed = target.read_rgba8();
    for (x, y, expected) in [
        (4, 4, [255, 0, 0, 255]),
        (28, 4, [255, 255, 0, 255]),
        (36, 4, [0, 255, 0, 255]),
        (4, 36, [255, 255, 0, 255]),
        (36, 36, [0, 0, 255, 255]),
        (68, 36, [255, 0, 0, 255]),
        (10, 16, [0, 255, 0, 255]),   // Shape above the background.
        (16, 16, [255, 0, 255, 255]), // Actor sprite above the shape.
    ] {
        assert_eq!(pixel(&composed, x, y), expected, "at ({x},{y})");
    }
    assert_eq!(
        pixel(&composed, 80, 16),
        pixel(&original, 80, 16),
        "blank cell preserves clear"
    );
    let mut wrong = background.clone();
    wrong[1].instance.region = 1;
    target
        .render_into(|_, view, size| {
            compositor.draw_with_background(view, size, &shapes, &wrong, &actors, &camera())
        })
        .unwrap();
    let changed = target.read_rgba8();
    assert_eq!(pixel(&changed, 36, 4), [0, 0, 255, 255]);
    for y in 0..64 {
        for x in 0..96 {
            if !(32..64).contains(&x) || y >= 32 {
                assert_eq!(
                    pixel(&changed, x, y),
                    pixel(&composed, x, y),
                    "wrong region escaped its cell"
                );
            }
        }
    }
    wrong[1].instance.region = 999;
    assert!(
        target
            .render_into(|_, view, size| compositor.draw_with_background(
                view,
                size,
                &shapes,
                &wrong,
                &actors,
                &camera()
            ))
            .is_err()
    );
    assert_eq!(
        target.read_rgba8(),
        changed,
        "invalid later input must not submit a partial frame"
    );
}
