//! Scalar material contract and mandatory headless GPU acceptance.
//! GPU cases fail on missing adapters; a skipped GPU is not an accepted result.
#![cfg(feature = "metallic-roughness")]
#![allow(clippy::float_arithmetic)]
#[path = "material_shading/oracle.rs"]
mod oracle;
use orr_render::orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, PipelineDesc, Rhi,
    TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions, decode_rgba16f_texel,
};
use orr_render::{MaterialShading, MaterialShadingError, ScalarMaterial};

#[test]
fn validated_scalars_reject_nonfinite_and_out_of_range_without_clamping() {
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01] {
        assert_eq!(
            ScalarMaterial::new(invalid, 0.5),
            Err(MaterialShadingError::InvalidMetallic)
        );
        assert_eq!(
            ScalarMaterial::new(0.5, invalid),
            Err(MaterialShadingError::InvalidRoughness)
        );
    }
    for value in [0.0, 0.001, 0.5, 1.0] {
        let material = ScalarMaterial::new(value, value).unwrap();
        assert_eq!(material.metallic(), value);
        assert_eq!(material.roughness(), value);
    }
    assert_eq!(MaterialShading::default(), MaterialShading::LegacyDiffuse);
}

#[test]
fn independent_oracle_has_analytic_limits_reciprocity_and_no_energy_gain() {
    let n = [0.0, 0.0, 1.0];
    let base = [0.2, 0.5, 0.9];
    for metal in [0.0, 1.0] {
        let response = oracle::brdf(n, n, n, base, metal, 1.0);
        for i in 0..3 {
            let expected = if metal == 0.0 {
                (0.96 * base[i] + 0.01) / std::f64::consts::PI
            } else {
                base[i] / (4.0 * std::f64::consts::PI)
            };
            assert!((response[i] - expected).abs() < 1e-12);
        }
    }
    for rough in [0.25, 0.5, 1.0] {
        for metal in [0.0, 0.5, 1.0] {
            let v = oracle::unit([0.7, 0.2, 1.0]);
            let l = oracle::unit([-0.4, 0.3, 1.0]);
            let a = oracle::brdf(n, v, l, base, metal, rough);
            let b = oracle::brdf(n, l, v, base, metal, rough);
            for i in 0..3 {
                assert!((a[i] - b[i]).abs() < 1e-12);
            }
            // Midpoint integration in cos(theta), phi. Test moderate roughness
            // where this fixed quadrature resolves the lobe; no smooth-lobe claim.
            let mut energy = [0.0; 3];
            for y in 0..128 {
                let z = (f64::from(y) + 0.5) / 128.0;
                for x in 0..256 {
                    let phi = (f64::from(x) + 0.5) * std::f64::consts::TAU / 256.0;
                    let radius = (1.0 - z * z).sqrt();
                    let l = [radius * phi.cos(), radius * phi.sin(), z];
                    let f = oracle::brdf(n, v, l, [1.0; 3], metal, rough);
                    for i in 0..3 {
                        energy[i] += f[i] * z * std::f64::consts::TAU / (128.0 * 256.0);
                    }
                }
            }
            assert!(
                energy
                    .iter()
                    .all(|e| e.is_finite() && *e >= 0.0 && *e <= 1.005),
                "energy {energy:?}"
            );
        }
    }
}

static GPU_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn gpu() -> (std::sync::MutexGuard<'static, ()>, Wgpu) {
    let guard = GPU_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("mandatory material GPU adapter");
    let info = gpu.adapter().get_info();
    eprintln!(
        "material acceptance adapter: name={:?} backend={:?} device_type={:?} vendor={} device={} driver={:?} driver_info={:?} software={}",
        info.name,
        info.backend,
        info.device_type,
        info.vendor,
        info.device,
        info.driver,
        info.driver_info,
        gpu.is_software()
    );
    (guard, gpu)
}
fn target(g: &Wgpu, size: u32, format: TextureFormat) -> <Wgpu as Rhi>::Texture {
    g.create_texture(&TextureDesc {
        label: "material acceptance",
        width: size,
        height: size,
        format,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    })
}
fn near(actual: [f32; 4], expected: [f64; 3]) {
    for i in 0..3 {
        let tolerance = (expected[i].abs() * 0.006).max(0.0001);
        assert!(
            actual[i].is_finite()
                && actual[i] >= 0.0
                && (f64::from(actual[i]) - expected[i]).abs() <= tolerance,
            "actual {actual:?}, expected {expected:?}"
        );
    }
    assert_eq!(actual[3], 1.0);
}

#[test]
fn gpu_shared_brdf_matches_independent_f64_oracle_at_endpoints_and_grazing() {
    let (_guard, g) = gpu();
    let source = format!(
        "{}\n{}",
        include_str!("../src/material_shading.wgsl"),
        r#"
struct Case { n: vec4<f32>, v: vec4<f32>, l: vec4<f32>, base: vec4<f32>, params: vec4<f32> };
@group(0) @binding(0) var<uniform> c: Case;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1., -1.), vec2<f32>(3., -1.), vec2<f32>(-1., 3.));
    return vec4<f32>(p[i], 0., 1.);
}
@fragment fn fs() -> @location(0) vec4<f32> {
    return vec4<f32>(mr_brdf(c.n.xyz, c.v.xyz, c.l.xyz, c.base.xyz, c.params.x, c.params.y) * max(dot(c.n.xyz, c.l.xyz), 0.0), 1.0);
}
"#
    );
    let shader = g.create_shader("material BRDF oracle", &source);
    let pipeline = g.create_pipeline(&PipelineDesc::color(
        "material BRDF oracle",
        &shader,
        ("vs", "fs"),
        &[],
        TextureFormat::Rgba16Float,
        Blend::Opaque,
    ));
    let uniform = g.create_buffer(&BufferDesc {
        label: "material case",
        size: 80,
        usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
    });
    let bind = g.create_bind_group(
        &pipeline,
        0,
        &[Binding::Uniform {
            binding: 0,
            buffer: &uniform,
        }],
    );
    let texture = target(&g, 1, TextureFormat::Rgba16Float);
    let view = g.create_texture_view(&texture, None);
    for rough in [0.0, 0.045, 0.3, 1.0] {
        for metal in [0.0, 0.5, 1.0] {
            for (v, l) in [
                ([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]),
                ([0.0, 0.0, 1.0], [0.3, 0.0, 1.0]),
                ([1.0, 0.0, 0.001], [-0.5, 0.0, 1.0]),
                ([0.0; 3], [0.0, 0.0, 1.0]),
                ([0.0, 0.0, 1.0], [0.0, 0.0, -1.0]),
            ] {
                let v = oracle::unit(v).map(|x| x as f32);
                let l = oracle::unit(l).map(|x| x as f32);
                let base = [0.2_f32, 0.5, 0.9];
                let values = [
                    [0.0, 0.0, 1.0, 0.0],
                    [v[0], v[1], v[2], 0.0],
                    [l[0], l[1], l[2], 0.0],
                    [base[0], base[1], base[2], 0.0],
                    [metal, rough, 0.0, 0.0],
                ];
                g.write_buffer(&uniform, 0, bytemuck::cast_slice(&values));
                let mut encoder = g.create_encoder("material oracle");
                g.encode_render_pass(
                    &mut encoder,
                    "material oracle",
                    &ColorAttachment {
                        view: &view,
                        resolve: None,
                        clear: Some([0.0; 4]),
                    },
                    &[
                        Command::SetPipeline(&pipeline),
                        Command::SetBindGroup(0, &bind),
                        Command::Draw {
                            vertices: 0..3,
                            instances: 0..1,
                        },
                    ],
                );
                g.submit(encoder);
                let bytes = g.read_texture(&texture);
                let response = oracle::brdf(
                    [0.0, 0.0, 1.0],
                    v.map(f64::from),
                    l.map(f64::from),
                    base.map(f64::from),
                    f64::from(metal),
                    f64::from(rough),
                );
                near(
                    decode_rgba16f_texel(bytes[..8].try_into().unwrap()),
                    response.map(|x| x * f64::from(l[2]).max(0.0)),
                );
            }
        }
    }
}

fn panel() -> orr_model::StaticModel {
    use orr_model::{
        Dependency, IDENTITY, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap,
    };
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: "material/panel.glb".into(),
        dependencies: vec![Dependency {
            uri: "$source".into(),
            sha256: "0".repeat(64),
        }],
        primitives: vec![Primitive {
            id: "material/panel.glb#node=0/mesh=0/primitive=0".into(),
            material: 0,
            transform: IDENTITY,
            vertices: [
                [-0.9, -0.9, 0.0],
                [0.9, -0.9, 0.0],
                [0.9, 0.9, 0.0],
                [-0.9, 0.9, 0.0],
            ]
            .into_iter()
            .map(|position| Vertex {
                position,
                normal: [0.0, 0.0, 1.0],
                uv: [0.5, 0.5],
            })
            .collect(),
            indices: vec![0, 1, 2, 0, 2, 3],
        }],
        materials: vec![Material {
            base_color: [0.7, 0.4, 0.9, 0.2],
            image: 0,
            linear_filter: true,
            wrap_s: Wrap::Clamp,
            wrap_t: Wrap::Clamp,
        }],
        images: vec![Image {
            width: 1,
            height: 1,
            rgba8: vec![128, 64, 192, 17],
        }],
    })
    .unwrap()
}

#[test]
fn panel_fixture_is_admitted_and_cooked_without_a_gpu() {
    let model = panel();
    let cooked = model.to_bytes().unwrap();
    let reloaded = orr_model::StaticModel::from_bytes(&cooked).unwrap();
    assert_eq!(reloaded.source(), model.source());
    assert_eq!(reloaded.source().primitives.len(), 1);
    assert_eq!(reloaded.source().materials.len(), 1);
}

#[test]
fn static_override_linear_color_orthographic_and_legacy_contract() {
    use orr_render::{Camera3D, Lighting, ModelRenderError, ModelRenderer};
    let (_guard, g) = gpu();
    let model = panel();
    let original = model.to_bytes().unwrap();
    let cam = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 1.0);
    let light = Lighting {
        direction: [0.0, 0.0, -1.0],
        color: [1.0; 3],
        intensity: 0.7,
        ambient: 0.0,
        shadows: false,
        exposure: 9.0,
        tonemap: true,
        ..Default::default()
    };
    assert!(matches!(
        ModelRenderer::new_with_materials(
            g.clone(),
            TextureFormat::Rgba16Float,
            model.clone(),
            &[]
        ),
        Err(ModelRenderError::InvalidMaterial(
            MaterialShadingError::MaterialCount
        ))
    ));
    for format in [
        TextureFormat::Rgba16Float,
        TextureFormat::Rgba8Unorm,
        TextureFormat::Rgba8UnormSrgb,
    ] {
        let texture = target(&g, 32, format);
        let view = g.create_texture_view(&texture, None);
        let mut legacy = ModelRenderer::new(g.clone(), format, model.clone()).unwrap();
        let mut explicit = ModelRenderer::new_with_materials(
            g.clone(),
            format,
            model.clone(),
            &[MaterialShading::LegacyDiffuse],
        )
        .unwrap();
        assert_eq!(legacy.material_shading(), &[MaterialShading::LegacyDiffuse]);
        legacy.draw(&view, (32, 32), &cam, &light).unwrap();
        let before = g.read_texture(&texture);
        explicit.draw(&view, (32, 32), &cam, &light).unwrap();
        assert_eq!(
            before,
            g.read_texture(&texture),
            "legacy constructor contract {format:?}"
        );
    }
    let texture = target(&g, 32, TextureFormat::Rgba16Float);
    let view = g.create_texture_view(&texture, None);
    for metal in [0.0, 1.0] {
        let m = MaterialShading::MetallicRoughness(ScalarMaterial::new(metal, 0.5).unwrap());
        let mut renderer = ModelRenderer::new_with_materials(
            g.clone(),
            TextureFormat::Rgba16Float,
            model.clone(),
            &[m],
        )
        .unwrap();
        renderer.draw(&view, (32, 32), &cam, &light).unwrap();
        let pixels = g.read_texture(&texture);
        let base = [128, 64, 192].map(oracle::srgb_decode);
        let factor = [0.7_f32, 0.4, 0.9].map(f64::from);
        let base = std::array::from_fn(|i| base[i] * factor[i]);
        let response = oracle::brdf(
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            base,
            f64::from(metal),
            0.5,
        )
        .map(|x| x * f64::from(light.intensity));
        // Widely separated fragments have identical V in orthographic projection.
        // HDR must ignore exposure/tone mapping and OPAQUE must ignore alpha.
        for (x, y) in [(8, 8), (16, 16), (24, 24)] {
            let offset = (y * 32 + x) * 8;
            near(
                decode_rgba16f_texel(pixels[offset..offset + 8].try_into().unwrap()),
                response,
            );
        }
    }
    assert_eq!(
        model.to_bytes().unwrap(),
        original,
        "runtime override mutated immutable asset"
    );
}

#[cfg(feature = "animation")]
#[test]
fn skinned_material_matches_deformed_static_geometry_and_preserves_legacy() {
    use orr_model::{ModelSource, StaticModel, animation_import};
    use orr_render::{Camera3D, Lighting, ModelRenderer, SkinnedInstance, SkinnedModelRenderer};
    let (_guard, g) = gpu();
    let model = animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("fixture embeds dependencies"),
    )
    .unwrap();
    let pose = model.rest_pose().unwrap();
    let source = model.source();
    let static_model = StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: source.asset_id.clone(),
        dependencies: source.dependencies.clone(),
        primitives: model.deform(&pose).unwrap(),
        materials: source.materials.clone(),
        images: source.images.clone(),
    })
    .unwrap();
    let camera = Camera3D::orthographic([0.0, 0.5, 5.0], [0.0, 0.5, 0.0], 1.5);
    let lighting = Lighting {
        direction: [0.3, -0.1, -1.0],
        intensity: 0.7,
        ambient: 0.1,
        shadows: false,
        tonemap: false,
        exposure: 1.0,
        ..Default::default()
    };
    let texture = target(&g, 64, TextureFormat::Rgba16Float);
    let view = g.create_texture_view(&texture, None);
    let instances = [SkinnedInstance::new(&pose)];
    let mut legacy =
        SkinnedModelRenderer::new(g.clone(), TextureFormat::Rgba16Float, model.clone()).unwrap();
    legacy.clear = [0.0, 0.0, 0.0, 1.0];
    legacy
        .draw(&view, (64, 64), &camera, &lighting, &instances)
        .unwrap();
    let legacy_pixels = g.read_texture(&texture);
    for shading in [
        MaterialShading::LegacyDiffuse,
        MaterialShading::MetallicRoughness(ScalarMaterial::new(0.7, 0.45).unwrap()),
    ] {
        let materials = vec![shading; source.materials.len()];
        let mut skinned = SkinnedModelRenderer::new_with_materials(
            g.clone(),
            TextureFormat::Rgba16Float,
            model.clone(),
            &materials,
        )
        .unwrap();
        let mut static_renderer = ModelRenderer::new_with_materials(
            g.clone(),
            TextureFormat::Rgba16Float,
            static_model.clone(),
            &materials,
        )
        .unwrap();
        skinned.clear = [0.0, 0.0, 0.0, 1.0];
        static_renderer.clear = skinned.clear;
        skinned
            .draw(&view, (64, 64), &camera, &lighting, &instances)
            .unwrap();
        let animated_pixels = g.read_texture(&texture);
        if shading == MaterialShading::LegacyDiffuse {
            assert_eq!(legacy_pixels, animated_pixels);
        } else {
            assert_ne!(
                legacy_pixels, animated_pixels,
                "override must visibly change the fixture"
            );
        }
        static_renderer
            .draw(&view, (64, 64), &camera, &lighting)
            .unwrap();
        let static_pixels = g.read_texture(&texture);
        let mut foreground = 0;
        for (a, b) in animated_pixels
            .chunks_exact(8)
            .zip(static_pixels.chunks_exact(8))
        {
            let a = decode_rgba16f_texel(a.try_into().unwrap());
            let b = decode_rgba16f_texel(b.try_into().unwrap());
            if b[..3].iter().any(|x| *x > 0.0) {
                foreground += 1;
            }
            near(a, [f64::from(b[0]), f64::from(b[1]), f64::from(b[2])]);
        }
        assert!(foreground > 50, "fixture rendered no meaningful geometry");
    }
}

#[cfg(feature = "imported-scene")]
#[test]
fn composed_opposing_grazing_sun_point_and_exposure_remain_finite_in_ldr() {
    use orr_render::{
        Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting,
        ModelRenderer, PointLight, PointLightSettings,
    };
    let (_guard, g) = gpu();
    let mut source = panel().source().clone();
    source.materials[0].base_color = [1.0; 4];
    source.images[0].rgba8.fill(255);
    // Geometrically front-facing panel with admitted authored grazing normals.
    // Rotate the reviewer's N/V/L configuration so rasterization stays robust.
    for vertex in &mut source.primitives[0].vertices {
        vertex.normal = [1.0, 0.0, 1e-9];
    }
    let model = orr_model::StaticModel::new(source).unwrap();
    let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 1.0);
    let lighting = Lighting {
        direction: [-2e-9, 0.0, 1.0],
        color: [1e4; 3],
        intensity: 1e4,
        ambient: 0.0,
        exposure: 1e4,
        tonemap: true,
        shadows: false,
        ..Default::default()
    };
    // A distant point preserves the tiny directional component despite fragment
    // interpolation error. Range and position stay inside admitted limits.
    let point = PointLightSettings {
        point_light: Some(PointLight {
            position: [0.002, 0.0, -1e6],
            color: [1e4; 3],
            intensity: 1e4,
            range: 1e9,
        }),
    };
    point.validate().unwrap();
    let expected = oracle::brdf(
        [1.0, 0.0, 1e-9],
        [0.0, 0.0, 1.0],
        [2e-9, 0.0, -1.0],
        [1.0; 3],
        1.0,
        0.045,
    )[0] * 1e-9
        * 1e8
        * (1.0 + 0.999_f64.powi(2))
        * 1e4;
    assert!(expected.is_finite() && expected > 1e19);
    assert!(
        ((expected as f32) * (2.51 * expected as f32 + 0.03)).is_infinite(),
        "regression must overflow original ACES numerator"
    );
    for format in [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb] {
        let texture = target(&g, 33, format);
        let view = g.create_texture_view(&texture, None);
        let mut scene = ImportedSceneRenderer::new(g.clone(), format).unwrap();
        assert!(
            !scene.post_process.enabled,
            "test the direct LDR branch, not HDR saturation"
        );
        let mut renderer = ModelRenderer::new_with_materials(
            g.clone(),
            format,
            model.clone(),
            &[MaterialShading::MetallicRoughness(
                ScalarMaterial::new(1.0, 0.045).unwrap(),
            )],
        )
        .unwrap();
        scene
            .draw(
                ImportedSceneTarget {
                    view: &view,
                    size: (33, 33),
                    format,
                    sample_count: 1,
                },
                &camera,
                &lighting,
                &point,
                &mut [ImportedBatch::Static(&mut renderer)],
            )
            .unwrap();
        let pixels = g.read_texture(&texture);
        let offset = (16 * 33 + 16) * 4;
        assert_eq!(
            &pixels[offset..offset + 4],
            &[255, 255, 255, 255],
            "finite saturated LDR {format:?}"
        );
    }
}

#[test]
fn perspective_material_uses_fragment_view_direction_against_analytic_plane_oracle() {
    use orr_render::{Camera3D, Lighting, ModelRenderer};
    let (_guard, g) = gpu();
    const SIZE: u32 = 65;
    // Symmetric 90-degree projection, square viewport, z=0 plane, eye z=2.5.
    // The oracle below uses pixel centers and tan(pi/4), with no engine matrix,
    // inverse projection, camera ray, or world_to_screen helper.
    let camera = Camera3D::perspective([0.0, 0.0, 2.5], [0.0; 3], 90.0);
    let lighting = Lighting {
        direction: [-0.25, 0.0, -1.0],
        color: [0.9, 0.7, 1.0],
        intensity: 0.4,
        ambient: 0.0,
        shadows: false,
        exposure: 1.0,
        tonemap: false,
        ..Default::default()
    };
    let light = oracle::unit([0.25, 0.0, 1.0]);
    let color = [128, 64, 192].map(oracle::srgb_decode);
    let factor = [0.7_f32, 0.4, 0.9].map(f64::from);
    let base = std::array::from_fn(|i| color[i] * factor[i]);
    let texture = target(&g, SIZE, TextureFormat::Rgba16Float);
    let view = g.create_texture_view(&texture, None);
    for metal in [0.0, 1.0] {
        let shading = MaterialShading::MetallicRoughness(ScalarMaterial::new(metal, 0.25).unwrap());
        let mut renderer = ModelRenderer::new_with_materials(
            g.clone(),
            TextureFormat::Rgba16Float,
            panel(),
            &[shading],
        )
        .unwrap();
        renderer
            .draw(&view, (SIZE, SIZE), &camera, &lighting)
            .unwrap();
        let pixels = g.read_texture(&texture);
        let response = |v| {
            let brdf = oracle::brdf([0.0, 0.0, 1.0], v, light, base, f64::from(metal), 0.25);
            std::array::from_fn::<_, 3, _>(|i| {
                brdf[i] * light[2] * f64::from(lighting.color[i]) * f64::from(lighting.intensity)
            })
        };
        let wrong_constant_view = response([0.0, 0.0, 1.0]);
        let mut strongest_control_difference = 0.0_f64;
        for (x, y) in [(24_u32, 32_u32), (32, 32), (40, 32), (32, 24), (32, 40)] {
            let nx = 2.0 * (f64::from(x) + 0.5) / f64::from(SIZE) - 1.0;
            let ny = 1.0 - 2.0 * (f64::from(y) + 0.5) / f64::from(SIZE);
            let world = [
                nx * 2.5 * std::f64::consts::FRAC_PI_4.tan(),
                ny * 2.5 * std::f64::consts::FRAC_PI_4.tan(),
                0.0,
            ];
            assert!(
                world[0].abs() < 0.8 && world[1].abs() < 0.8,
                "sample must be well inside the authored plane"
            );
            let v = oracle::unit([-world[0], -world[1], 2.5]);
            let expected = response(v);
            let offset = ((y * SIZE + x) * 8) as usize;
            near(
                decode_rgba16f_texel(pixels[offset..offset + 8].try_into().unwrap()),
                expected,
            );
            strongest_control_difference =
                strongest_control_difference.max((expected[0] - wrong_constant_view[0]).abs());
        }
        // This fixture must reject using the orthographic constant V everywhere,
        // for both a dielectric and a metal, not merely pass near-normal pixels.
        assert!(
            strongest_control_difference > 0.05,
            "weak perspective control for metallic={metal}: {strongest_control_difference}"
        );
    }
}

#[cfg(feature = "animation")]
fn tilted_strip() -> orr_model::animation::AnimatedModel {
    use orr_model::{
        animation::{AnimatedModel, ChannelValues, Interpolation},
        animation_import,
    };
    let original = animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("fixture embeds dependencies"),
    )
    .unwrap();
    let mut source = original.source().clone();
    // Fence the authored rig for the closed-form reference below. This is not
    // a general-purpose skeleton evaluator, and never calls a matrix helper.
    assert_eq!(source.skins.len(), 1);
    assert_eq!(source.skins[0].joints, [1, 2]);
    assert_eq!(source.nodes.len(), 4);
    assert_eq!(source.skins[0].inverse_bind_matrices.len(), 2);
    for node in &source.nodes {
        assert_eq!(node.rest.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(node.rest.scale, [1.0; 3]);
    }
    assert_eq!(source.nodes[1].rest.translation, [0.0; 3]);
    assert_eq!(source.nodes[0].rest.translation, [0.3, 0.2, 0.0]);
    assert_eq!(source.nodes[0].children, [1]);
    assert_eq!(source.nodes[1].children, [2]);
    assert_eq!(source.nodes[2].rest.translation, [0.0, 1.5, 0.0]);
    assert_eq!(source.nodes[3].rest.translation, [7.0, 0.0, 0.0]);
    for (matrix, pivot) in source.skins[0]
        .inverse_bind_matrices
        .iter()
        .zip([[0.3, 0.2], [0.3, 1.7]])
    {
        let mut expected = orr_model::IDENTITY;
        expected[3] = [-pivot[0], -pivot[1], 0.0, 1.0];
        assert_eq!(*matrix, expected);
    }
    let channel = &source.clips[0].channels;
    assert_eq!(channel.len(), 1);
    assert_eq!(channel[0].node, 2);
    assert_eq!(channel[0].interpolation, Interpolation::Linear);
    assert_eq!(channel[0].times, [0.0, 1.0, 2.0]);
    let ChannelValues::Rotation(rotations) = &channel[0].values else {
        panic!("rotation fixture");
    };
    assert_eq!(rotations.len(), 3);
    assert_eq!(rotations[0], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(rotations[2], rotations[0]);
    assert_eq!(rotations[1], [0.0, 0.0, 0.5, 0.75_f32.sqrt()]);
    assert_eq!(source.primitives.len(), 1);
    assert_eq!(source.primitives[0].skin, Some(0));
    assert_eq!(source.primitives[0].node, 3);
    for vertex in &mut source.primitives[0].vertices {
        assert_eq!(vertex.joints, [0, 1, 0, 0]);
        vertex.vertex.normal = [0.6, 0.0, 0.8];
    }
    // Uniform authored texels isolate normal/material response from nearest
    // texture boundaries; the original disk fixture is untouched.
    for image in &mut source.images {
        for texel in image.rgba8.chunks_exact_mut(4) {
            texel.copy_from_slice(&[128, 96, 192, 255]);
        }
    }
    AnimatedModel::new(source).unwrap()
}

#[cfg(feature = "animation")]
fn analytic_strip(
    model: &orr_model::animation::AnimatedModel,
    angle: f64,
) -> orr_model::StaticModel {
    use orr_model::{ModelSource, Primitive, StaticModel};
    let source = model.source();
    let (s, c) = angle.sin_cos();
    let primitives = source
        .primitives
        .iter()
        .map(|primitive| Primitive {
            id: primitive.id.clone(),
            indices: primitive.indices.clone(),
            material: primitive.material,
            transform: orr_model::IDENTITY,
            vertices: primitive
                .vertices
                .iter()
                .map(|input| {
                    let weights = input.weights.map(f64::from);
                    let sum: f64 = weights.iter().sum();
                    assert!(sum.is_finite() && sum > 0.0);
                    let w = weights[1] / sum;
                    let [x, y, z] = input.vertex.position.map(f64::from);
                    // Upper joint rotates around world bind pivot (.3,1.7); lower
                    // palette is identity. Mesh-node translation must not be reapplied.
                    let qx = 0.3 + c * (x - 0.3) - s * (y - 1.7);
                    let qy = 1.7 + s * (x - 0.3) + c * (y - 1.7);
                    let a = 1.0 - w + w * c;
                    let b = w * s;
                    let determinant = a * a + b * b;
                    let [nx, ny, nz] = input.vertex.normal.map(f64::from);
                    // Inverse-transpose of the BLENDED XY matrix, not blended
                    // inverse-transposes and not direct position-matrix multiplication.
                    let normal = oracle::unit([
                        (a * nx - b * ny) / determinant,
                        (b * nx + a * ny) / determinant,
                        nz,
                    ]);
                    orr_model::Vertex {
                        position: [(1.0 - w) * x + w * qx, (1.0 - w) * y + w * qy, z]
                            .map(|v| v as f32),
                        normal: normal.map(|v| v as f32),
                        uv: input.vertex.uv,
                    }
                })
                .collect(),
        })
        .collect();
    StaticModel::new(ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: source.asset_id.clone(),
        dependencies: source.dependencies.clone(),
        primitives,
        materials: source.materials.clone(),
        images: source.images.clone(),
    })
    .unwrap()
}

#[cfg(feature = "animation")]
#[test]
fn tilted_strip_midpoint_and_key_match_independent_positions_and_normals_without_gpu() {
    let model = tilted_strip();
    let rest = analytic_strip(&model, 0.0);
    for (time, angle) in [
        (0.0, 0.0),
        (0.5, std::f64::consts::PI / 6.0),
        (1.0, std::f64::consts::PI / 3.0),
    ] {
        let expected = analytic_strip(&model, angle);
        let actual = model.deform(&model.sample_clip(0, time).unwrap()).unwrap();
        for (a, b) in actual[0]
            .vertices
            .iter()
            .zip(&expected.source().primitives[0].vertices)
        {
            for (x, y) in a
                .position
                .iter()
                .chain(&a.normal)
                .zip(b.position.iter().chain(&b.normal))
            {
                assert!((x - y).abs() < 2e-5, "time={time}: {x} != {y}");
            }
        }
        if time > 0.0 {
            let changed = &expected.source().primitives[0].vertices[6];
            let initial = &rest.source().primitives[0].vertices[6];
            assert!((changed.position[0] - initial.position[0]).abs() > 0.2);
            assert!((changed.normal[1] - initial.normal[1]).abs() > 0.2);
        }
    }
}

#[cfg(feature = "animation")]
#[test]
fn nonrest_skinned_material_matches_independent_tilted_normal_reference() {
    use orr_render::{Camera3D, Lighting, ModelRenderer, SkinnedInstance, SkinnedModelRenderer};
    let model = tilted_strip();
    let references: Vec<_> = [0.0, std::f64::consts::PI / 6.0, std::f64::consts::PI / 3.0]
        .map(|a| analytic_strip(&model, a))
        .into();
    // Admission, motion and normal-change assertions run before requesting GPU.
    tilted_strip_midpoint_and_key_match_independent_positions_and_normals_without_gpu();
    let (_guard, g) = gpu();
    let camera = Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 2.2);
    let lighting = Lighting {
        direction: [-0.3, -0.5, -1.0],
        intensity: 0.7,
        ambient: 0.1,
        shadows: false,
        tonemap: false,
        exposure: 1.0,
        ..Default::default()
    };
    let texture = target(&g, 128, TextureFormat::Rgba16Float);
    let view = g.create_texture_view(&texture, None);
    let mut rest_pixels: Option<Vec<u8>> = None;
    for (index, time) in [0.0, 0.5, 1.0].into_iter().enumerate() {
        let pose = model.sample_clip(0, time).unwrap();
        let instances = [SkinnedInstance::new(&pose)];
        let mut legacy =
            SkinnedModelRenderer::new(g.clone(), TextureFormat::Rgba16Float, model.clone())
                .unwrap();
        legacy.clear = [0.0, 0.0, 0.0, 1.0];
        legacy
            .draw(&view, (128, 128), &camera, &lighting, &instances)
            .unwrap();
        let legacy_pixels = g.read_texture(&texture);
        for shading in [
            MaterialShading::LegacyDiffuse,
            MaterialShading::MetallicRoughness(ScalarMaterial::new(0.7, 0.45).unwrap()),
        ] {
            let materials = vec![shading; model.source().materials.len()];
            let mut skinned = SkinnedModelRenderer::new_with_materials(
                g.clone(),
                TextureFormat::Rgba16Float,
                model.clone(),
                &materials,
            )
            .unwrap();
            let mut reference = ModelRenderer::new_with_materials(
                g.clone(),
                TextureFormat::Rgba16Float,
                references[index].clone(),
                &materials,
            )
            .unwrap();
            skinned.clear = legacy.clear;
            reference.clear = legacy.clear;
            skinned
                .draw(&view, (128, 128), &camera, &lighting, &instances)
                .unwrap();
            let pixels = g.read_texture(&texture);
            reference
                .draw(&view, (128, 128), &camera, &lighting)
                .unwrap();
            let expected = g.read_texture(&texture);
            let mut foreground = 0;
            let mut material_response = 0;
            for ((a, b), old) in pixels
                .chunks_exact(8)
                .zip(expected.chunks_exact(8))
                .zip(legacy_pixels.chunks_exact(8))
            {
                let a = decode_rgba16f_texel(a.try_into().unwrap());
                let b = decode_rgba16f_texel(b.try_into().unwrap());
                let old = decode_rgba16f_texel(old.try_into().unwrap());
                near(
                    a,
                    b[..3]
                        .try_into()
                        .map(|v: [f32; 3]| v.map(f64::from))
                        .unwrap(),
                );
                if b[0] > 0.001 {
                    foreground += 1;
                    if (a[0] - old[0]).abs() > 0.005 {
                        material_response += 1;
                    }
                }
            }
            assert!(foreground > 100, "time={time}: insufficient foreground");
            if shading == MaterialShading::LegacyDiffuse {
                assert_eq!(pixels, legacy_pixels);
            } else {
                assert!(
                    material_response > 50,
                    "time={time}: weak interior material response"
                );
                if let Some(rest) = &rest_pixels {
                    let mut shared_changed = 0;
                    for (a, b) in pixels.chunks_exact(8).zip(rest.chunks_exact(8)) {
                        let a = decode_rgba16f_texel(a.try_into().unwrap());
                        let b = decode_rgba16f_texel(b.try_into().unwrap());
                        if a[0] > 0.001 && b[0] > 0.001 && (a[0] - b[0]).abs() > 0.002 {
                            shared_changed += 1;
                        }
                    }
                    assert!(
                        shared_changed > 10,
                        "time={time}: no shared-foreground shading change"
                    );
                } else {
                    rest_pixels = Some(pixels);
                }
            }
        }
    }
}

#[cfg(feature = "irradiance-probes")]
#[test]
fn composed_constant_probe_replaces_hemisphere_and_suppresses_metal_diffuse() {
    use orr_render::{
        Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, IrradianceGrid,
        IrradianceProvenance, Lighting, ModelRenderer, PointLightSettings,
    };
    let (_guard, g) = gpu();
    let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 1.0);
    let texture = target(&g, 65, TextureFormat::Rgba16Float);
    let view = g.create_texture_view(&texture, None);
    // Author SH0 directly. Expected E = c0 / sqrt(4*pi), then Lambert E/pi.
    // No engine constant_irradiance, sample, packed_uniform or SH basis helper.
    let c0 = [2.0_f32, 3.0, 4.0];
    let mut coefficients = [[0.0; 3]; 9];
    coefficients[0] = c0;
    let grid = IrradianceGrid {
        version: 1,
        enabled: true,
        provenance: IrradianceProvenance::Authored,
        dimensions: [4; 3],
        origin: [-3.0; 3],
        spacing: [2.0; 3],
        coefficients: vec![coefficients; 64],
    };
    grid.validate().unwrap();
    let probe =
        c0.map(|c| f64::from(c) / (4.0 * std::f64::consts::PI).sqrt() / std::f64::consts::PI);
    let light = oracle::unit([0.3, 0.1, 1.0]);
    let color = [128, 64, 192].map(oracle::srgb_decode);
    let factor = [0.7_f32, 0.4, 0.9].map(f64::from);
    let base = std::array::from_fn(|i| color[i] * factor[i]);
    let hemisphere = 0.1_f64; // (sky .6 + ground .2)/2 * ambient .25, N.y=0.
    let mut samples = [[[[0.0_f32; 4]; 2]; 2]; 2];
    for (metal_index, metal) in [0.0_f32, 1.0].into_iter().enumerate() {
        let shading = MaterialShading::MetallicRoughness(ScalarMaterial::new(metal, 0.45).unwrap());
        let mut renderer = ModelRenderer::new_with_materials(
            g.clone(),
            TextureFormat::Rgba16Float,
            panel(),
            &[shading],
        )
        .unwrap();
        let brdf = oracle::brdf(
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            light,
            base,
            f64::from(metal),
            f64::from(0.45_f32),
        );
        for (direct, direct_samples) in samples[metal_index].iter_mut().enumerate() {
            let lighting = Lighting {
                direction: [-0.3, -0.1, -1.0],
                color: [1.0; 3],
                intensity: if direct == 1 { 0.8 } else { 0.0 },
                sky: [0.6; 3],
                ground: [0.2; 3],
                ambient: 0.25,
                shadows: false,
                tonemap: false,
                exposure: 1.0,
                ..Default::default()
            };
            let sun = brdf.map(|b| b * light[2] * f64::from(lighting.intensity));
            for (enabled, sample) in direct_samples.iter_mut().enumerate() {
                let mut scene =
                    ImportedSceneRenderer::new(g.clone(), TextureFormat::Rgba16Float).unwrap();
                scene.irradiance = (enabled == 1).then(|| grid.clone());
                scene
                    .draw(
                        ImportedSceneTarget {
                            view: &view,
                            size: (65, 65),
                            format: TextureFormat::Rgba16Float,
                            sample_count: 1,
                        },
                        &camera,
                        &lighting,
                        &PointLightSettings::default(),
                        &mut [ImportedBatch::Static(&mut renderer)],
                    )
                    .unwrap();
                let pixels = g.read_texture(&texture);
                let indirect = if enabled == 1 { probe } else { [hemisphere; 3] };
                let expected = std::array::from_fn(|i| {
                    sun[i] + base[i] * (1.0 - f64::from(metal)) * 0.96 * indirect[i]
                });
                for (x, y) in [(16_u32, 32_u32), (32, 32), (48, 32)] {
                    // Analytic world X is in [-.5,.5], Y=Z=0. Grid q is in
                    // [1.25,1.75], hence every edge distance exceeds one cell.
                    let world_x = 2.0 * (f64::from(x) + 0.5) / 65.0 - 1.0;
                    assert!(world_x.abs() < 0.5);
                    let offset = ((y * 65 + x) * 8) as usize;
                    let actual =
                        decode_rgba16f_texel(pixels[offset..offset + 8].try_into().unwrap());
                    near(actual, expected);
                    if x == 32 {
                        *sample = actual;
                    }
                }
            }
        }
    }
    for (dielectric, metal) in samples[0].iter().zip(&samples[1]) {
        let dielectric_off = dielectric[0];
        let dielectric_on = dielectric[1];
        // Reject ignored probes. Blue has ample separation from the fallback.
        assert!((dielectric_on[2] - dielectric_off[2]).abs() > 0.05);
        // Reject additive hemisphere: the reference contains replacement only,
        // and the discarded fallback is much larger than readback tolerance.
        assert!(base[2] * 0.96 * hemisphere > 0.03);
        assert_eq!(metal[0], metal[1], "metal received diffuse probes");
    }
    // Metal direct-off really is black, but that alone would be vacuous. The
    // direct-on metal has nonzero independently checked GGX response.
    assert_eq!(samples[1][0][1], [0.0, 0.0, 0.0, 1.0]);
    assert!(samples[1][1][1][2] > 0.05);
    // A missing (1-metal) factor would visibly add this positive probe signal.
    assert!(base[2] * 0.96 * probe[2] > 0.1);
}

#[cfg(feature = "imported-scene")]
#[test]
fn composed_shadow_removes_only_sun_for_dielectric_and_metal() {
    use orr_render::{
        Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting,
        ModelRenderer, PointLight, PointLightSettings,
    };
    let mut source = panel().source().clone();
    let mut caster = source.primitives[0].clone();
    caster.id = "material/panel.glb#node=1/mesh=0/primitive=0".into();
    for vertex in &mut caster.vertices {
        vertex.position = [
            0.3 + vertex.position[0] * (0.1 / 0.9),
            vertex.position[1] * (0.25 / 0.9),
            0.5,
        ];
    }
    source.primitives.push(caster);
    let model = orr_model::StaticModel::new(source).unwrap();
    let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 1.0);
    let point = PointLight {
        position: [0.0, 0.0, 2.0],
        color: [0.8, 1.0, 0.6],
        intensity: 0.6,
        range: 4.0,
    };
    let points = PointLightSettings {
        point_light: Some(point),
    };
    points.validate().unwrap();
    let l = oracle::unit([0.6, 0.0, 1.0]);
    let color = [128, 64, 192].map(oracle::srgb_decode);
    let factor = [0.7_f32, 0.4, 0.9].map(f64::from);
    let base = std::array::from_fn(|i| color[i] * factor[i]);
    let (_guard, g) = gpu();
    let texture = target(&g, 65, TextureFormat::Rgba16Float);
    let view = g.create_texture_view(&texture, None);
    for metal in [0.0_f32, 1.0] {
        let material =
            MaterialShading::MetallicRoughness(ScalarMaterial::new(metal, 0.45).unwrap());
        let mut renderer = ModelRenderer::new_with_materials(
            g.clone(),
            TextureFormat::Rgba16Float,
            model.clone(),
            &[material],
        )
        .unwrap();
        let mut states = [[[0.0_f32; 4]; 2]; 2];
        let sun = oracle::brdf(
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            l,
            base,
            f64::from(metal),
            f64::from(0.45_f32),
        )
        .map(|b| b * l[2] * f64::from(0.8_f32));
        assert!(
            sun[2] > 0.02,
            "weak diffuse/specular sun control: metal={metal}"
        );
        for (shadow_index, samples) in states.iter_mut().enumerate() {
            let lighting = Lighting {
                direction: [-0.6, 0.0, -1.0],
                color: [1.0; 3],
                intensity: 0.8,
                sky: [0.4; 3],
                ground: [0.2; 3],
                ambient: 0.25,
                shadows: shadow_index == 1,
                shadow_center: [0.0, 0.0, 0.25],
                shadow_radius: 2.0,
                tonemap: false,
                exposure: 1.0,
            };
            let mut scene =
                ImportedSceneRenderer::new(g.clone(), TextureFormat::Rgba16Float).unwrap();
            scene
                .draw(
                    ImportedSceneTarget {
                        view: &view,
                        size: (65, 65),
                        format: TextureFormat::Rgba16Float,
                        sample_count: 1,
                    },
                    &camera,
                    &lighting,
                    &points,
                    &mut [ImportedBatch::Static(&mut renderer)],
                )
                .unwrap();
            assert_eq!(scene.shadow_map_size(), (shadow_index == 1).then_some(1024));
            let pixels = g.read_texture(&texture);
            for (sample, x) in samples.iter_mut().zip([32_u32, 16]) {
                let world_x = 2.0 * (f64::from(x) + 0.5) / 65.0 - 1.0;
                // Ray to sun reaches caster z=.5 at x+.3. Center is well
                // inside its [.2,.4] footprint; left pixel is well outside.
                // Camera rays at both samples miss the caster itself.
                let hit_x = world_x + 0.5 * 0.6;
                let blocked = hit_x > 0.2 && hit_x < 0.4;
                assert_eq!(blocked, x == 32);
                assert!((hit_x - 0.2).abs() > 0.09 && (hit_x - 0.4).abs() > 0.09);
                assert!(world_x < 0.1);
                let delta = [-world_x, 0.0, 2.0];
                let distance = oracle::dot(delta, delta).sqrt();
                let pl = oracle::unit(delta);
                let attenuation = (1.0 - distance / f64::from(point.range)).max(0.0).powi(2);
                let point_brdf = oracle::brdf(
                    [0.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0],
                    pl,
                    base,
                    f64::from(metal),
                    f64::from(0.45_f32),
                );
                let residual: [f64; 3] = std::array::from_fn(|i| {
                    point_brdf[i]
                        * pl[2]
                        * attenuation
                        * f64::from(point.intensity)
                        * f64::from(point.color[i])
                        + base[i] * (1.0 - f64::from(metal)) * 0.96 * 0.075
                });
                assert!(residual[2] > 0.02, "shadowed receiver must not be black");
                let visibility = if shadow_index == 1 && blocked {
                    0.0
                } else {
                    1.0
                };
                let expected = std::array::from_fn(|i| residual[i] + sun[i] * visibility);
                let offset = ((32 * 65 + x) * 8) as usize;
                *sample = decode_rgba16f_texel(pixels[offset..offset + 8].try_into().unwrap());
                near(*sample, expected);
            }
        }
        // Negatives reject ignored shadow maps and global darkening. Only the
        // analytically blocked sample loses the independently expected sunlight.
        near(
            [
                states[0][0][0] - states[1][0][0],
                states[0][0][1] - states[1][0][1],
                states[0][0][2] - states[1][0][2],
                1.0,
            ],
            sun,
        );
        assert!((states[0][0][2] - states[1][0][2]).abs() > 0.02);
        near(
            states[1][1],
            states[0][1][..3]
                .try_into()
                .map(|a: [f32; 3]| a.map(f64::from))
                .unwrap(),
        );
    }
}
