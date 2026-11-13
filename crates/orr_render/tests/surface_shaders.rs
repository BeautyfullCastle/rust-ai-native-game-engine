//! Surface shaders on a real GPU: the shader decides the color, parameters
//! reach it per instance, the engine still lights, shadows and composes it.
//!
//! Without an adapter the tests print SKIP. Set `ORR_REQUIRE_GPU=1` to fail instead.
#![allow(clippy::float_arithmetic)]

use orr_render::orr_rhi::{TextureFormat, Wgpu, WgpuOptions};
use orr_render::{
    Camera3D, Lighting, Material, OffscreenTarget, RenderList3D, Renderer3D, Settings3D, SurfaceShader, IDENTITY_ROT, MAX_SURFACE_FLOATS,
};

const SIZE: (u32, u32) = (96, 96);

fn gpu() -> Option<Wgpu> {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(g) => Some(g),
        Err(e) => {
            assert!(std::env::var_os("ORR_REQUIRE_GPU").is_none(), "ORR_REQUIRE_GPU is set but no adapter: {e}");
            eprintln!("SKIP: no GPU adapter ({e})");
            None
        }
    }
}

/// Ambient light only: the shaded color equals the albedo.
fn flat_light() -> Lighting {
    Lighting { intensity: 0.0, ambient: 1.0, sky: [1.0; 3], ground: [1.0; 3], shadows: false, tonemap: false, ..Lighting::default() }
}

fn render(gpu: &Wgpu, renderer: &mut Renderer3D<Wgpu>, list: &RenderList3D, camera: &Camera3D) -> Vec<u8> {
    let target = OffscreenTarget::new(gpu, SIZE.0, SIZE.1, TextureFormat::Rgba8Unorm);
    target.render3d(renderer, list, camera);
    target.read_rgba8()
}

fn px(img: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE.0 + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2]]
}

const SOLID: &str = "// @param tint color = 0, 1, 0
fn surface(in: SurfaceInput) -> Surface {
    var s = default_surface(in);
    s.albedo = params.tint;
    return s;
}
";

const HALVES: &str = "// Left half red, right half blue, split at local x = 0.
fn surface(in: SurfaceInput) -> Surface {
    var s = default_surface(in);
    s.albedo = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), in.local.x < 0.0);
    return s;
}
";

fn params(values: &[f32]) -> [f32; MAX_SURFACE_FLOATS] {
    let mut out = [0.0; MAX_SURFACE_FLOATS];
    out[..values.len()].copy_from_slice(values);
    out
}

#[test]
fn the_surface_function_decides_the_color_and_reads_per_instance_parameters() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = Renderer3D::with_settings(gpu.clone(), TextureFormat::Rgba8Unorm, Settings3D { msaa: 1, ..Settings3D::LOW });
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    let solid = renderer.add_surface(&SurfaceShader::compile("solid", SOLID).unwrap());
    let halves = renderer.add_surface(&SurfaceShader::compile("halves", HALVES).unwrap());
    assert_eq!((solid.0, halves.0), (0, 1), "ids count up from zero");
    let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0);

    // Two boxes with the same shader and different parameters, and one with the built-in material.
    let mut list = RenderList3D::new();
    list.lighting = flat_light();
    let gray = Material::new([0.5, 0.5, 0.5]).rough(1.0);
    list.with_surface(solid, params(&[1.0, 0.0, 0.0]), |list| list.cuboid([-1.3, 0.0, 0.0], IDENTITY_ROT, [0.5; 3], &gray));
    list.with_surface(solid, params(&[0.0, 0.0, 1.0]), |list| list.cuboid([0.0, 0.0, 0.0], IDENTITY_ROT, [0.5; 3], &gray));
    list.cuboid([1.3, 0.0, 0.0], IDENTITY_ROT, [0.5; 3], &gray);
    assert_eq!((list.boxes.len(), list.surfaces.len()), (1, 2));
    let img = render(&gpu, &mut renderer, &list, &camera);
    let (left, middle, right) = (px(&img, 17, 48), px(&img, 48, 48), px(&img, 79, 48));
    assert_eq!(left, [255, 0, 0], "first instance's tint");
    assert_eq!(middle, [0, 0, 255], "second instance's tint");
    assert!(right[0] == right[1] && right[1] == right[2] && right[0] > 100, "the built-in material is unchanged: {right:?}");

    // The shader can use the position in the shape's own space.
    list.clear();
    list.with_surface(halves, params(&[]), |list| list.cuboid([0.0; 3], IDENTITY_ROT, [0.9; 3], &gray));
    let img = render(&gpu, &mut renderer, &list, &camera);
    assert_eq!(px(&img, 30, 48), [255, 0, 0]);
    assert_eq!(px(&img, 66, 48), [0, 0, 255]);

    // An id the renderer does not have is skipped, not drawn with another shader.
    list.clear();
    list.with_surface(orr_render::SurfaceId(7), params(&[]), |list| list.cuboid([0.0; 3], IDENTITY_ROT, [0.9; 3], &gray));
    assert_eq!(px(&render(&gpu, &mut renderer, &list, &camera), 48, 48), [0, 0, 0]);
}

#[test]
fn the_engine_lights_surface_shaders_and_they_cast_shadows() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = Renderer3D::with_settings(gpu.clone(), TextureFormat::Rgba8Unorm, Settings3D { msaa: 1, ..Settings3D::LOW });
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    let white = renderer.add_surface(&SurfaceShader::compile("white", "fn surface(in: SurfaceInput) -> Surface { var s = default_surface(in); s.albedo = vec3<f32>(1.0); return s; }").unwrap());
    // Looking straight down on a floor, with a box held above it and a sun from straight above.
    let camera = Camera3D::orthographic([0.0, 10.0, 0.01], [0.0; 3], 3.0);
    let sun = Lighting { direction: [0.0, -1.0, 0.0], intensity: 1.0, ambient: 0.05, shadows: true, shadow_radius: 6.0, tonemap: false, ..Lighting::default() };
    let floor = Material::new([0.8, 0.8, 0.8]).rough(1.0);
    let mut list = RenderList3D::new();
    list.lighting = sun;
    list.cuboid([0.0, -0.1, 0.0], IDENTITY_ROT, [3.0, 0.1, 3.0], &floor);
    // The box sits off to the side of the camera's view of the floor; only its shadow falls in the middle.
    list.with_surface(white, params(&[]), |list| list.cuboid([0.0, 1.5, 0.0], IDENTITY_ROT, [0.4; 3], &floor));
    let shadowed = render(&gpu, &mut renderer, &list, &camera);
    let corner = px(&shadowed, 8, 8);
    let box_top = px(&shadowed, 48, 48);
    assert!(box_top[0] > 200, "the sun lights the white top of the surface-shaded box: {box_top:?}");
    assert!(corner[0] > 150, "the floor away from the box is lit: {corner:?}");

    // The box's shadow: move the camera to see only the floor under the box.
    list.lighting.shadows = true;
    let below = Camera3D::orthographic([0.0, 1.0, 0.01], [0.0, -1.0, 0.0], 0.3);
    let mut floor_only = list.clone();
    let img = render(&gpu, &mut renderer, &floor_only, &below);
    let under = px(&img, 48, 48);
    floor_only.surfaces.clear();
    let without = px(&render(&gpu, &mut renderer, &floor_only, &below), 48, 48);
    assert!(under[0] + 60 < without[0], "the surface-shaded box casts a shadow: {under:?} with, {without:?} without");
}

#[test]
fn shader_time_reaches_the_surface_function() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = Renderer3D::with_settings(gpu.clone(), TextureFormat::Rgba8Unorm, Settings3D { msaa: 1, ..Settings3D::LOW });
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    let pulse = renderer.add_surface(&SurfaceShader::compile("pulse", "fn surface(in: SurfaceInput) -> Surface { var s = default_surface(in); s.albedo = vec3<f32>(clamp(in.time / 10.0, 0.0, 1.0)); return s; }").unwrap());
    let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0);
    let mut list = RenderList3D::new();
    list.lighting = flat_light();
    // A matte shape: no ambient reflection is added on top of the albedo.
    list.with_surface(pulse, params(&[]), |list| list.cuboid([0.0; 3], IDENTITY_ROT, [0.9; 3], &Material::new([0.0; 3]).rough(1.0)));
    list.time = 0.0;
    let dark = px(&render(&gpu, &mut renderer, &list, &camera), 48, 48);
    list.time = 5.0;
    let half = px(&render(&gpu, &mut renderer, &list, &camera), 48, 48);
    assert_eq!(dark, [0, 0, 0]);
    // Linear 0.5 is stored as sRGB 188.
    assert!((186..=189).contains(&half[0]), "time 5 of 10 gives half gray: {half:?}");
}

#[cfg(feature = "imported-scene")]
#[test]
fn surface_shaders_draw_in_the_composed_scene_path() {
    use orr_render::orr_rhi::{Rhi, TextureDesc, TextureUsage};
    use orr_render::{ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, PointLightSettings};
    let Some(gpu) = gpu() else { return };
    let format = TextureFormat::Rgba8Unorm;
    let texture = gpu.create_texture(&TextureDesc {
        label: "composed surface target",
        width: SIZE.0,
        height: SIZE.1,
        format,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = gpu.create_texture_view(&texture, None);
    let mut scene = ImportedSceneRenderer::new(gpu.clone(), format).unwrap();
    scene.clear = [0.0, 0.0, 0.0, 1.0];
    let mut procedural = Renderer3D::with_settings(gpu.clone(), format, Settings3D::LOW);
    let solid = procedural.add_surface(&SurfaceShader::compile("solid", SOLID).unwrap());
    let mut list = RenderList3D::new();
    list.with_surface(solid, params(&[1.0, 0.0, 1.0]), |list| list.cuboid([0.0; 3], IDENTITY_ROT, [0.9; 3], &Material::new([0.5; 3]).rough(1.0)));
    let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0);
    for shadows in [false, true] {
        let lighting = Lighting { shadows, ..flat_light() };
        scene
            .draw(
                ImportedSceneTarget { view: &view, size: SIZE, format, sample_count: 1 },
                &camera,
                &lighting,
                &PointLightSettings::default(),
                &mut [ImportedBatch::Procedural { renderer: &mut procedural, list: &list }],
            )
            .unwrap();
        let img = gpu.read_texture(&texture);
        assert_eq!(px(&img, 48, 48), [255, 0, 255], "shadows {shadows}");
    }

    // In the composed path a surface-shaded box casts the shared sun shadow onto the floor.
    let mut list = RenderList3D::new();
    let floor = Material::new([0.8, 0.8, 0.8]).rough(1.0);
    list.cuboid([0.0, -0.1, 0.0], IDENTITY_ROT, [3.0, 0.1, 3.0], &floor);
    let caster = |list: &mut RenderList3D| list.with_surface(solid, params(&[1.0, 1.0, 1.0]), |list| list.cuboid([0.0, 1.5, 0.0], IDENTITY_ROT, [0.4; 3], &floor));
    let below = Camera3D::orthographic([0.0, 1.0, 0.01], [0.0, -1.0, 0.0], 0.3);
    let sun = Lighting { direction: [0.0, -1.0, 0.0], intensity: 1.0, ambient: 0.05, shadows: true, shadow_radius: 6.0, tonemap: false, ..Lighting::default() };
    let mut floor_under = |list: &RenderList3D, procedural: &mut Renderer3D<Wgpu>| {
        scene
            .draw(
                ImportedSceneTarget { view: &view, size: SIZE, format, sample_count: 1 },
                &below,
                &sun,
                &PointLightSettings::default(),
                &mut [ImportedBatch::Procedural { renderer: procedural, list }],
            )
            .unwrap();
        px(&gpu.read_texture(&texture), 48, 48)
    };
    let lit = floor_under(&list, &mut procedural);
    caster(&mut list);
    let shaded = floor_under(&list, &mut procedural);
    assert!(shaded[0] + 60 < lit[0], "the surface-shaded box shades the floor: {shaded:?} with, {lit:?} without");
}
