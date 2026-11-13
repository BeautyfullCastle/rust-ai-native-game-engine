//! Passes added to the imported scene renderer draw in their stages.
#![cfg(feature = "imported-scene")]
#![allow(clippy::float_arithmetic)]

use orr_model::{StaticModel, Vertex, IDENTITY};
use orr_render::orr_rhi::{
    Blend, ColorAttachment, Command, Compare, Cull, DepthAttachment, DepthState, PipelineDesc, Rhi, TextureDesc, TextureFormat, TextureUsage, Topology, Wgpu,
    WgpuOptions,
};
use orr_render::{Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, ModelRenderer, PassFrame, PassStage, PointLightSettings, RenderPass, StaticInstance};
use std::any::Any;

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

fn square() -> StaticModel {
    let imported = orr_model::import::import_with_resolver("fixtures/model.glb", include_bytes!("../../orr_model/tests/fixtures/model.glb"), |_| {
        panic!("fixture embeds every dependency")
    })
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
    source.materials[0].base_color = [1.0, 0.0, 0.0, 1.0];
    for image in &mut source.images {
        for pixel in image.rgba8.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    StaticModel::new(source).unwrap()
}

/// A sky in one color: a triangle at the far plane that only fills uncovered pixels.
struct FlatSky {
    pipeline: <Wgpu as Rhi>::Pipeline,
    prepared: u32,
    saw_shadow: bool,
}

const SKY: &str = "
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[i], 1.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0, 1.0, 0.0, 1.0); }
";

impl FlatSky {
    fn new(g: &Wgpu, format: TextureFormat) -> Self {
        let shader = g.create_shader("flat sky", SKY);
        let pipeline = g.create_pipeline(&PipelineDesc::<Wgpu> {
            label: "flat sky",
            shader: &shader,
            vs_entry: "vs",
            fs_entry: "fs",
            vertex_buffers: &[],
            color_format: Some(format),
            blend: Blend::Opaque,
            topology: Topology::TriangleList,
            cull: Cull::None,
            depth: Some(DepthState { format: TextureFormat::Depth32Float, write: false, compare: Compare::LessEqual, bias: 0, slope_bias: 0.0 }),
            samples: 1,
        });
        Self { pipeline, prepared: 0, saw_shadow: false }
    }
}

impl RenderPass<Wgpu> for FlatSky {
    fn name(&self) -> &str {
        "flat sky"
    }
    fn stages(&self) -> &[PassStage] {
        &[PassStage::Sky]
    }
    fn prepare(&mut self, _rhi: &Wgpu, frame: &PassFrame<'_, Wgpu>) {
        self.prepared += 1;
        self.saw_shadow |= frame.shadow.is_some();
    }
    fn encode(&self, rhi: &Wgpu, stage: PassStage, _frame: &PassFrame<'_, Wgpu>, encoder: &mut <Wgpu as Rhi>::Encoder, color: Option<&ColorAttachment<'_, Wgpu>>, depth: &DepthAttachment<'_, Wgpu>) {
        assert_eq!(stage, PassStage::Sky);
        rhi.encode_pass(encoder, "flat sky", color, Some(depth), &[Command::SetPipeline(&self.pipeline), Command::Draw { vertices: 0..3, instances: 0..1 }]);
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn render(g: &Wgpu, scene: &mut ImportedSceneRenderer<Wgpu>, renderer: &mut ModelRenderer<Wgpu>, light: &Lighting) -> Vec<u8> {
    let texture = g.create_texture(&TextureDesc {
        label: "pass test",
        width: SIZE,
        height: SIZE,
        format: FORMAT,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = g.create_texture_view(&texture, None);
    let instances = [StaticInstance::default()];
    scene
        .draw(
            ImportedSceneTarget { view: &view, size: (SIZE, SIZE), format: FORMAT, sample_count: 1 },
            &Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 1.0),
            light,
            &PointLightSettings::default(),
            &mut [ImportedBatch::StaticInstances { renderer, instances: &instances }],
        )
        .unwrap();
    g.read_texture(&texture)
}

fn px(image: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE + x) * 4) as usize;
    [image[i], image[i + 1], image[i + 2]]
}

#[test]
fn a_sky_pass_fills_only_what_the_scene_leaves_open_and_sees_the_shadow_map() {
    let Some(g) = gpu() else { return };
    let mut scene = ImportedSceneRenderer::new(g.clone(), FORMAT).unwrap();
    let mut renderer = ModelRenderer::new(g.clone(), FORMAT, square()).unwrap();
    scene.add_pass(Box::new(FlatSky::new(&g, FORMAT))).unwrap();
    let flat = Lighting { sky: [1.0; 3], ground: [1.0; 3], ambient: 1.0, intensity: 0.0, shadows: false, tonemap: false, exposure: 1.0, ..Default::default() };
    let image = render(&g, &mut scene, &mut renderer, &flat);
    assert_eq!(px(&image, 2, 2), [0, 255, 0], "open sky is the pass's color");
    let center = px(&image, 32, 32);
    assert!(center[0] > 200 && center[1] < 30, "the model stays in front: {center:?}");
    let sky = scene.pass_mut::<FlatSky>().expect("the pass is reachable by type");
    assert_eq!((sky.prepared, sky.saw_shadow), (1, false));

    // With sun shadows the pass is handed the shared map.
    let shadowed = Lighting { shadows: true, intensity: 1.0, shadow_radius: 5.0, ..flat };
    render(&g, &mut scene, &mut renderer, &shadowed);
    assert!(scene.pass_mut::<FlatSky>().unwrap().saw_shadow);

    // A removed pass draws no more.
    scene.remove_passes::<FlatSky>();
    let image = render(&g, &mut scene, &mut renderer, &flat);
    assert_ne!(px(&image, 2, 2), [0, 255, 0]);
}
