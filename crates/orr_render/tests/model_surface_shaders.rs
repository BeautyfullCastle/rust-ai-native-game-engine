//! Surface shaders on static models in an imported scene: the same shader file
//! as on shapes, lit by the model's own lighting, chosen per instance.
#![cfg(feature = "imported-scene")]
#![allow(clippy::float_arithmetic)]

use orr_model::{StaticModel, Vertex, IDENTITY};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, ModelRenderer, ModelSurface,
    PointLightSettings, StaticInstance, SurfaceShader, MAX_SURFACE_FLOATS,
};

const SIZE: u32 = 64;
const FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

fn gpu() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(g) => Some(g),
        Err(e) => {
            assert!(std::env::var_os("ORR_REQUIRE_GPU").is_none(), "required GPU unavailable: {e}");
            eprintln!("SKIP: {e}");
            None
        }
    }
}

/// A unit square facing the camera with a solid gray texture, made from the imported test model.
fn square() -> StaticModel {
    let imported = orr_model::import::import_with_resolver(
        "fixtures/model.glb",
        include_bytes!("../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("fixture embeds every dependency"),
    )
    .unwrap();
    let mut source = StaticModel::from_bytes(&imported.to_bytes().unwrap()).unwrap().source().clone();
    source.primitives.truncate(1);
    let p = &mut source.primitives[0];
    p.vertices = [[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.5, 0.5, 0.0], [-0.5, 0.5, 0.0]]
        .into_iter()
        .map(|position| Vertex { position, normal: [0.0, 0.0, 1.0], uv: [0.25, 0.25] })
        .collect();
    p.indices = vec![0, 1, 2, 0, 2, 3];
    p.transform = IDENTITY;
    p.material = 0;
    source.materials[0].base_color = [1.0, 1.0, 1.0, 1.0];
    for image in &mut source.images {
        for pixel in image.rgba8.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[128, 128, 128, 255]);
        }
    }
    StaticModel::new(source).unwrap()
}

fn at(x: f32, y: f32, z: f32, scale: f32) -> StaticInstance {
    StaticInstance { translation: [x, y, z], scale: [scale; 3], ..Default::default() }
}

/// Only ambient light of strength 1: a surface's albedo is its color.
fn flat() -> Lighting {
    Lighting { sky: [1.0; 3], ground: [1.0; 3], ambient: 1.0, intensity: 0.0, shadows: false, tonemap: false, exposure: 1.0, ..Default::default() }
}

/// A sun from the upper left with shadows and a little ambient light.
fn sunny() -> Lighting {
    Lighting {
        direction: [0.6, 0.0, -0.8],
        color: [1.0; 3],
        intensity: 1.0,
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient: 0.1,
        shadows: true,
        tonemap: false,
        exposure: 1.0,
        ..Default::default()
    }
}

fn render(g: &Wgpu, scene: &mut ImportedSceneRenderer<Wgpu>, renderer: &mut ModelRenderer<Wgpu>, instances: &[StaticInstance], light: &Lighting) -> Vec<u8> {
    let texture = g.create_texture(&TextureDesc {
        label: "model surface test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 1.0);
    scene
        .draw(
            ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
            &camera,
            light,
            &PointLightSettings::default(),
            &mut [ImportedBatch::StaticInstances { renderer, instances }],
        )
        .unwrap();
    g.read_texture(&texture)
}

/// The pixel at world position (x, y) on the plane the camera looks at.
fn px(image: &[u8], x: f32, y: f32) -> [u8; 3] {
    let column = (((x + 1.0) * 0.5) * SIZE as f32) as usize;
    let row = (((1.0 - y) * 0.5) * SIZE as f32) as usize;
    let i = (row * SIZE as usize + column) * 4;
    [image[i], image[i + 1], image[i + 2]]
}

fn near(actual: [u8; 3], expected: [u8; 3], label: &str) {
    assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 3), "{label}: {actual:?}, expected {expected:?}");
}

fn values(numbers: &[f32]) -> [f32; MAX_SURFACE_FLOATS] {
    let mut out = [0.0; MAX_SURFACE_FLOATS];
    out[..numbers.len()].copy_from_slice(numbers);
    out
}

const TINT: &str = "// @param tint color = 1, 0, 0
fn surface(in: SurfaceInput) -> Surface {
    var s = default_surface(in);
    s.albedo = params.tint;
    return s;
}
";

#[test]
fn default_surface_on_a_model_draws_its_built_in_look_with_and_without_shadows() {
    let Some(g) = gpu() else { return };
    let shader = SurfaceShader::compile("same", "fn surface(in: SurfaceInput) -> Surface { return default_surface(in); }").unwrap();
    for light in [flat(), sunny()] {
        let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
        let mut renderer = ModelRenderer::new(g.clone(), FORMAT, square()).unwrap();
        let instances = [at(0.0, 0.0, 0.0, 1.5)];
        let built_in = render(&g, &mut scene, &mut renderer, &instances, &light);
        let id = renderer.add_surface(&shader);
        renderer.set_instance_surfaces(&[Some(ModelSurface { surface: id, params: shader.default_values() })], 0.0);
        let shaded = render(&g, &mut scene, &mut renderer, &instances, &light);
        let worst = built_in.iter().zip(&shaded).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 1, "shadows {}: a surface that returns default_surface differs by {worst}", light.shadows);
        assert!(px(&shaded, 0.0, 0.0)[0] > 20, "the model was drawn");
    }
}

#[test]
fn each_instance_picks_its_own_surface_and_a_replaced_shader_keeps_its_id() {
    let Some(g) = gpu() else { return };
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    let mut renderer = ModelRenderer::new(g.clone(), FORMAT, square()).unwrap();
    let instances = [at(-0.5, 0.0, 0.0, 0.8), at(0.5, 0.0, 0.0, 0.8)];
    let tint = renderer.add_surface(&SurfaceShader::compile("tint", TINT).unwrap());
    renderer.set_instance_surfaces(&[Some(ModelSurface { surface: tint, params: values(&[1.0, 0.0, 0.0]) }), None], 0.0);
    let image = render(&g, &mut scene, &mut renderer, &instances, &flat());
    near(px(&image, -0.5, 0.0), [255, 0, 0], "the first instance reads its tint");
    let gray = px(&image, 0.5, 0.0);
    near(gray, [128, 128, 128], "the second instance keeps the built-in look");

    // Saving the shader file again replaces the pipelines under the same id.
    renderer.replace_surface(tint, &SurfaceShader::compile("tint", &TINT.replace("s.albedo = params.tint;", "s.albedo = params.tint.bgr;")).unwrap());
    let image = render(&g, &mut scene, &mut renderer, &instances, &flat());
    near(px(&image, -0.5, 0.0), [0, 0, 255], "the replaced shader draws");

    // An unknown id and an empty list both keep the built-in look.
    renderer.set_instance_surfaces(&[Some(ModelSurface { surface: orr_render::SurfaceId(7), params: values(&[]) })], 0.0);
    let image = render(&g, &mut scene, &mut renderer, &instances, &flat());
    near(px(&image, -0.5, 0.0), gray, "an unknown surface id");
    renderer.set_instance_surfaces(&[], 0.0);
    let image = render(&g, &mut scene, &mut renderer, &instances, &flat());
    near(px(&image, -0.5, 0.0), gray, "no surfaces");
}

#[test]
fn uv_local_position_and_time_reach_the_surface_function() {
    let Some(g) = gpu() else { return };
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    let mut renderer = ModelRenderer::new(g.clone(), FORMAT, square()).unwrap();
    let shader = SurfaceShader::compile(
        "inputs",
        "fn surface(in: SurfaceInput) -> Surface { var s = default_surface(in); s.albedo = vec3<f32>(in.uv.x, in.local.x + 0.5, in.time); return s; }",
    )
    .unwrap();
    let id = renderer.add_surface(&shader);
    renderer.set_instance_surfaces(&[Some(ModelSurface { surface: id, params: values(&[]) })], 0.5);
    let image = render(&g, &mut scene, &mut renderer, &[at(0.0, 0.0, 0.0, 1.5)], &flat());
    // Linear 0.25 and 0.5 are stored as sRGB 137 and 188. local.x is 0 at the center of the mesh.
    near(px(&image, 0.0, 0.0), [137, 188, 188], "uv, local position and time");
}

#[test]
fn a_model_with_a_surface_shader_receives_the_shared_sun_shadow() {
    let Some(g) = gpu() else { return };
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    let mut renderer = ModelRenderer::new(g.clone(), FORMAT, square()).unwrap();
    let white = renderer.add_surface(&SurfaceShader::compile("white", "fn surface(in: SurfaceInput) -> Surface { var s = default_surface(in); s.albedo = vec3<f32>(1.0); return s; }").unwrap());
    // A large receiver and a small caster nearer the sun; the sun throws the caster's shadow to the right.
    let instances = [at(0.0, 0.0, 0.0, 1.8), at(-0.3, 0.0, 0.5, 0.3)];
    renderer.set_instance_surfaces(&[Some(ModelSurface { surface: white, params: values(&[]) }), None], 0.0);
    let image = render(&g, &mut scene, &mut renderer, &instances, &sunny());
    let lit = px(&image, 0.7, 0.0);
    let shadowed = px(&image, 0.1, 0.0);
    assert!(lit[0] > 200, "the white surface is lit by the sun: {lit:?}");
    assert!(shadowed[0] + 60 < lit[0], "the caster's shadow falls on the surface: shadowed {shadowed:?}, lit {lit:?}");
}
