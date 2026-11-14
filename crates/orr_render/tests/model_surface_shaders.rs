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
        renderer.set_instance_surfaces(&[Some(ModelSurface::new(id, shader.default_values()))], 0.0);
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
    renderer.set_instance_surfaces(&[Some(ModelSurface::new(tint, values(&[1.0, 0.0, 0.0]))), None], 0.0);
    let image = render(&g, &mut scene, &mut renderer, &instances, &flat());
    near(px(&image, -0.5, 0.0), [255, 0, 0], "the first instance reads its tint");
    let gray = px(&image, 0.5, 0.0);
    near(gray, [128, 128, 128], "the second instance keeps the built-in look");

    // Saving the shader file again replaces the pipelines under the same id.
    renderer.replace_surface(tint, &SurfaceShader::compile("tint", &TINT.replace("s.albedo = params.tint;", "s.albedo = params.tint.bgr;")).unwrap());
    let image = render(&g, &mut scene, &mut renderer, &instances, &flat());
    near(px(&image, -0.5, 0.0), [0, 0, 255], "the replaced shader draws");

    // An unknown id and an empty list both keep the built-in look.
    renderer.set_instance_surfaces(&[Some(ModelSurface::new(orr_render::SurfaceId(7), values(&[])))], 0.0);
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
    renderer.set_instance_surfaces(&[Some(ModelSurface::new(id, values(&[])))], 0.5);
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
    renderer.set_instance_surfaces(&[Some(ModelSurface::new(white, values(&[]))), None], 0.0);
    let image = render(&g, &mut scene, &mut renderer, &instances, &sunny());
    let lit = px(&image, 0.7, 0.0);
    let shadowed = px(&image, 0.1, 0.0);
    assert!(lit[0] > 200, "the white surface is lit by the sun: {lit:?}");
    assert!(shadowed[0] + 60 < lit[0], "the caster's shadow falls on the surface: shadowed {shadowed:?}, lit {lit:?}");
}

/// 4 x 4 numbers in quadrants: red and green in the top rows, blue and white below.
fn quadrants() -> orr_render::SurfaceTexture {
    let mut rgba = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            rgba.extend(match (x < 2, y < 2) {
                (true, true) => [255, 0, 0, 255],
                (false, true) => [0, 255, 0, 255],
                (true, false) => [0, 0, 255, 255],
                (false, false) => [255, 255, 255, 255],
            });
        }
    }
    orr_render::SurfaceTexture::from_rgba8(orr_render::TextureKind::Data, 4, 4, &rgba).unwrap()
}

const PICTURE: &str = "// @texture picture data = 0, 0, 1, 1
fn surface(in: SurfaceInput) -> Surface {
    var s = default_surface(in);
    s.albedo = picture(in.world.xy + vec2<f32>(0.5)).rgb;
    return s;
}
";

#[test]
fn a_model_surface_reads_its_picked_texture_and_falls_back_to_the_declared_default() {
    let Some(g) = gpu() else { return };
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    let mut renderer = ModelRenderer::new(g.clone(), FORMAT, square()).unwrap();
    let shader = SurfaceShader::compile("picture", PICTURE).unwrap();
    let id = renderer.add_surface(&shader);
    let texture = renderer.add_texture(&quadrants());
    let mut surface = ModelSurface::new(id, values(&[]));
    surface.textures[0] = Some(texture);
    renderer.set_instance_surfaces(&[Some(surface)], 0.0);
    let instances = [at(0.0, 0.0, 0.0, 1.0)];
    let image = render(&g, &mut scene, &mut renderer, &instances, &flat());
    near(px(&image, -0.25, -0.25), [255, 0, 0], "texture row 0 at the bottom left");
    near(px(&image, 0.25, -0.25), [0, 255, 0], "texture row 0 at the bottom right");
    near(px(&image, -0.25, 0.25), [0, 0, 255], "texture row 3 at the top left");
    near(px(&image, 0.25, 0.25), [255, 255, 255], "texture row 3 at the top right");

    // A replaced image keeps its id.
    renderer.replace_texture(texture, &orr_render::SurfaceTexture::solid(orr_render::TextureKind::Data, [0.0, 1.0, 0.0, 1.0]));
    let image = render(&g, &mut scene, &mut renderer, &instances, &flat());
    near(px(&image, -0.25, -0.25), [0, 255, 0], "the replaced image");

    // No image and an unknown id both read the declared default.
    for chosen in [None, Some(orr_render::TextureId(9))] {
        surface.textures[0] = chosen;
        renderer.set_instance_surfaces(&[Some(surface)], 0.0);
        let image = render(&g, &mut scene, &mut renderer, &instances, &flat());
        near(px(&image, 0.25, 0.25), [0, 0, 255], "the declared default");
    }
}

#[test]
fn a_texture_repeated_far_below_one_pixel_reads_its_smallest_mip_level() {
    let Some(g) = gpu() else { return };
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    let mut renderer = ModelRenderer::new(g.clone(), FORMAT, square()).unwrap();
    // A red and green checker of single pixels: every level below the first averages to half of each.
    let rgba: Vec<u8> = (0..16).flat_map(|i| if (i % 4 + i / 4) % 2 == 0 { [255, 0, 0, 255] } else { [0, 255, 0, 255] }).collect();
    let checker = renderer.add_texture(&orr_render::SurfaceTexture::from_rgba8(orr_render::TextureKind::Data, 4, 4, &rgba).unwrap());
    let shader = SurfaceShader::compile("tiled", &PICTURE.replace("picture(in.world.xy + vec2<f32>(0.5))", "picture(in.world.xy * 400.0)")).unwrap();
    let id = renderer.add_surface(&shader);
    let mut surface = ModelSurface::new(id, values(&[]));
    surface.textures[0] = Some(checker);
    renderer.set_instance_surfaces(&[Some(surface)], 0.0);
    let image = render(&g, &mut scene, &mut renderer, &[at(0.0, 0.0, 0.0, 1.0)], &flat());
    // Linear 0.5 is stored as sRGB 188.
    near(px(&image, 0.1, 0.1), [188, 188, 0], "the averaged level, not a single pixel");
}

/// The animated strip fixture with a bend about X in its clip.
#[cfg(feature = "animation")]
fn animated_strip() -> orr_model::animation::AnimatedModel {
    use orr_model::animation::{AnimatedModel, ChannelValues};
    let imported = orr_model::animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("fixture embeds all dependencies"),
    )
    .unwrap();
    let mut source = AnimatedModel::from_bytes(&imported.to_bytes().unwrap()).unwrap().source().clone();
    source.clips[0].channels[0].values = ChannelValues::Rotation(vec![[0.0, 0.0, 0.0, 1.0], [0.5, 0.0, 0.0, 0.75f32.sqrt()], [0.0, 0.0, 0.0, 1.0]]);
    AnimatedModel::new(source).unwrap()
}

/// Pixels covered by something other than the clear color.
#[cfg(feature = "animation")]
fn coverage(image: &[u8], background: [u8; 3]) -> Vec<bool> {
    image.chunks_exact(4).map(|p| [p[0], p[1], p[2]] != background).collect()
}

#[cfg(feature = "animation")]
#[test]
fn a_skinned_model_is_drawn_with_its_surface_shader_in_every_pose() {
    use orr_render::{SkinnedInstance, SkinnedModelRenderer};
    let Some(g) = gpu() else { return };
    let model = animated_strip();
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    let mut renderer = SkinnedModelRenderer::new(g.clone(), FORMAT, model.clone()).unwrap();
    let camera = Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 2.2);
    let draw = |scene: &mut ImportedSceneRenderer<Wgpu>, renderer: &mut SkinnedModelRenderer<Wgpu>, pose: &orr_model::animation::Pose| {
        let texture = g.create_texture(&TextureDesc { label: "skinned surface", width: SIZE, height: SIZE, format: FORMAT, usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC, sample_count: 1, view_formats: &[] });
        let view = g.create_texture_view(&texture, None);
        let instances = [SkinnedInstance::new(pose)];
        scene
            .draw(ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 }, &camera, &flat(), &PointLightSettings::default(), &mut [ImportedBatch::Skinned { renderer, instances: &instances }])
            .unwrap();
        g.read_texture(&texture)
    };
    let red = SurfaceShader::compile("red", "// @texture tint data = 1, 0, 0, 1\nfn surface(in: SurfaceInput) -> Surface { var s = default_surface(in); s.albedo = tint(in.uv).rgb; return s; }").unwrap();
    let id = renderer.add_surface(&red);
    let green = renderer.add_texture(&orr_render::SurfaceTexture::solid(orr_render::TextureKind::Data, [0.0, 1.0, 0.0, 1.0]));
    for pose in [model.rest_pose().unwrap(), model.sample_clip(0, 0.5).unwrap()] {
        renderer.set_instance_surfaces(&[], 0.0);
        let built_in = draw(&mut scene, &mut renderer, &pose);
        let background = [built_in[0], built_in[1], built_in[2]];
        renderer.set_instance_surfaces(&[Some(ModelSurface::new(id, values(&[])))], 0.0);
        let shaded = draw(&mut scene, &mut renderer, &pose);
        // The surface shader covers exactly the pixels of the skinned mesh in this pose.
        let covered = coverage(&built_in, background);
        assert!(covered.iter().filter(|c| **c).count() > 50, "the strip is on screen");
        assert_eq!(coverage(&shaded, background), covered, "same skinned shape");
        let first = covered.iter().position(|c| *c).unwrap() * 4;
        near([shaded[first], shaded[first + 1], shaded[first + 2]], [255, 0, 0], "the declared default");
        let mut picked = ModelSurface::new(id, values(&[]));
        picked.textures[0] = Some(green);
        renderer.set_instance_surfaces(&[Some(picked)], 0.0);
        let textured = draw(&mut scene, &mut renderer, &pose);
        near([textured[first], textured[first + 1], textured[first + 2]], [0, 255, 0], "the picked image");
    }
}

/// The test square with texture coordinates that follow its position, so u runs along +x.
fn square_with_uv() -> StaticModel {
    let mut source = square().source().clone();
    for v in &mut source.primitives[0].vertices {
        v.uv = [v.position[0] + 0.5, 0.5 - v.position[1]];
    }
    StaticModel::new(source).unwrap()
}

/// A normal map that tilts the normal by `tilt` along u (texel x), as a constant texel.
fn tilted_normal(tilt: &str) -> String {
    format!(
        "fn surface(in: SurfaceInput) -> Surface {{
    var s = default_surface(in);
    s.albedo = vec3<f32>(1.0);
    s.normal = normal_map(in, vec3<f32>(0.5 + {tilt}, 0.5, 0.85), 1.0);
    return s;
}}
"
    )
}

#[test]
fn a_normal_map_turns_the_lit_side_toward_or_away_from_the_sun() {
    let Some(g) = gpu() else { return };
    let mut brightness = Vec::new();
    for tilt in ["0.0", "0.35", "-0.35"] {
        let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
        let mut renderer = ModelRenderer::new(g.clone(), FORMAT, square_with_uv()).unwrap();
        let id = renderer.add_surface(&SurfaceShader::compile("normal map", &tilted_normal(tilt)).unwrap());
        renderer.set_instance_surfaces(&[Some(ModelSurface::new(id, values(&[])))], 0.0);
        let mut light = sunny();
        light.shadows = false;
        let image = render(&g, &mut scene, &mut renderer, &[at(0.0, 0.0, 0.0, 1.6)], &light);
        brightness.push(px(&image, 0.0, 0.0)[0]);
    }
    let [flat, away, toward] = [brightness[0], brightness[1], brightness[2]];
    // The sun travels toward +x: a normal bent toward -x faces it, one bent toward +x turns away.
    assert!(toward > flat + 10, "bent toward the sun is brighter: {toward} vs flat {flat}");
    assert!(away + 10 < flat, "bent away from the sun is darker: {away} vs flat {flat}");
}
