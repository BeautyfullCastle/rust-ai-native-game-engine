use crate::particle_renderer::ParticleRenderer;
use orr_view::particles::{ParticleBlend, ParticleDraw};
fn particle() -> ParticleDraw {
    ParticleDraw {
        key: bytemuck::Zeroable::zeroed(),
        position: orr_view::Vec3::ZERO,
        size: 1.0,
        color: [1.0; 4],
        blend: ParticleBlend::Alpha,
    }
}
fn composed(
    scene: &mut ImportedSceneRenderer<Mock>,
    renderer: &mut ParticleRenderer<Mock>,
    size: (u32, u32),
    items: &[ParticleDraw],
    batches: &mut [ImportedBatch<'_, Mock>],
) -> Result<(), ImportedSceneError> {
    scene.draw_with_particles(
        ImportedSceneTarget {
            view: &0,
            size,
            format: FORMAT,
            sample_count: 1,
        },
        &Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0),
        &Lighting {
            shadows: false,
            ..Lighting::default()
        },
        &PointLightSettings::default(),
        batches,
        Some(ParticleBatch { renderer, items }),
    )
}
#[test]
fn particle_pass_loads_shared_depth_on_one_encoder_before_hdr() {
    let rhi = Mock::default();
    let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
    scene.post_process.enabled = true;
    let mut particles = ParticleRenderer::new(rhi.clone(), scene.scene_format()).unwrap();
    rhi.take();
    composed(&mut scene, &mut particles, (64, 64), &[particle()], &mut []).unwrap();
    let e = rhi.take();
    assert_eq!((e.encoders, e.submits), (1, 1));
    assert_eq!(e.passes.len(), 2);
    assert_eq!(e.passes[0].0, e.passes[1].0);
    assert!(e.passes[0].1 && e.passes[0].2);
    assert!(!e.passes[1].1 && !e.passes[1].2 && e.passes[1].3);
    assert!(!e.post_passes.is_empty());
}
#[test]
fn invalid_trailing_particle_rejects_resize_before_any_scene_effect() {
    let rhi = Mock::default();
    let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
    scene.post_process.enabled = true;
    let mut particles = ParticleRenderer::new(rhi.clone(), scene.scene_format()).unwrap();
    composed(&mut scene, &mut particles, (64, 64), &[particle()], &mut []).unwrap();
    let generations = (scene.depth_generation, scene.post_process_generation);
    rhi.take();
    let invalid = ParticleDraw {
        size: f32::NAN,
        ..particle()
    };
    assert!(matches!(
        composed(
            &mut scene,
            &mut particles,
            (128, 128),
            &[particle(), invalid],
            &mut []
        ),
        Err(ImportedSceneError::Particles(_))
    ));
    assert_eq!(
        generations,
        (scene.depth_generation, scene.post_process_generation)
    );
    assert_eq!(rhi.take(), Effects::default());
}
#[test]
fn hdr_scene_format_mismatch_rejects_before_allocations_or_writes() {
    let rhi = Mock::default();
    let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
    scene.post_process.enabled = true;
    let mut particles = ParticleRenderer::new(rhi.clone(), FORMAT).unwrap();
    rhi.take();
    assert!(matches!(
        composed(&mut scene, &mut particles, (64, 64), &[particle()], &mut []),
        Err(ImportedSceneError::FormatMismatch)
    ));
    assert_eq!(rhi.take(), Effects::default());
    assert_eq!(
        (scene.depth_generation, scene.post_process_generation),
        (0, 0)
    );
}
#[test]
fn malformed_final_geometry_after_valid_particles_is_atomic() {
    let rhi = Mock::default();
    let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
    let mut particles = ParticleRenderer::new(rhi.clone(), FORMAT).unwrap();
    let mut geometry = Renderer3D::new(rhi.clone(), FORMAT);
    let mut list = RenderList3D::new();
    list.sphere(
        [f32::NAN, 0.0, 0.0],
        crate::IDENTITY_ROT,
        1.0,
        &crate::Material::new([1.0; 3]),
    );
    rhi.take();
    assert!(composed(
        &mut scene,
        &mut particles,
        (64, 64),
        &[particle()],
        &mut [ImportedBatch::Procedural {
            renderer: &mut geometry,
            list: &list
        }]
    )
    .is_err());
    assert_eq!(rhi.take(), Effects::default());
    assert_eq!(
        (scene.depth_generation, scene.post_process_generation),
        (0, 0)
    );
}
#[test]
fn unchanged_default_and_empty_particle_routes_have_equal_frame_effects() {
    let rhi = Mock::default();
    let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
    let mut particles = ParticleRenderer::new(rhi.clone(), FORMAT).unwrap();
    draw(&mut scene, (64, 64), &mut []).unwrap();
    rhi.take();
    draw(&mut scene, (64, 64), &mut []).unwrap();
    let default = rhi.take();
    composed(&mut scene, &mut particles, (64, 64), &[], &mut []).unwrap();
    assert_eq!(default, rhi.take());
}
fn alternating() -> Vec<ParticleDraw> {
    (0..1024)
        .map(|n| ParticleDraw {
            blend: if n % 2 == 0 {
                ParticleBlend::Alpha
            } else {
                ParticleBlend::Additive
            },
            ..particle()
        })
        .collect()
}
#[test]
fn aggregate_draws_accept_4096_and_reject_4097_before_frame_changes() {
    let rhi = Mock::default();
    let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
    let mut particles = ParticleRenderer::new(rhi.clone(), FORMAT).unwrap();
    let mut models: Vec<_> = (0..7).map(|_| model(&rhi, FORMAT)).collect();
    assert_eq!(models[0].draw_count(), 2);
    let placements = vec![StaticInstance::default(); 256];
    let extra = [StaticInstance::default()];
    let items = alternating();
    let mut batches: Vec<_> = models
        .iter_mut()
        .enumerate()
        .map(|(n, renderer)| ImportedBatch::StaticInstances {
            renderer,
            instances: if n < 6 { &placements } else { &extra },
        })
        .collect();
    // (6 * 256 + 1) * 2 geometry draws + 1022 particle runs = 4096.
    composed(
        &mut scene,
        &mut particles,
        (64, 64),
        &items[..1022],
        &mut batches,
    )
    .unwrap();
    let generation = scene.depth_generation;
    rhi.take();
    assert!(matches!(
        composed(
            &mut scene,
            &mut particles,
            (128, 128),
            &items[..1023],
            &mut batches
        ),
        Err(ImportedSceneError::DrawLimit)
    ));
    assert_eq!(scene.depth_generation, generation);
    assert_eq!(rhi.take(), Effects::default());
}
#[test]
fn full_geometry_batch_budget_rejects_extra_particle_pass_atomically() {
    let rhi = Mock::default();
    let mut scene = ImportedSceneRenderer::new(rhi.clone(), FORMAT).unwrap();
    let mut particles = ParticleRenderer::new(rhi.clone(), FORMAT).unwrap();
    let mut geometry: Vec<_> = (0..256)
        .map(|_| Renderer3D::new(rhi.clone(), FORMAT))
        .collect();
    let list = RenderList3D::new();
    let mut batches: Vec<_> = geometry
        .iter_mut()
        .map(|renderer| ImportedBatch::Procedural {
            renderer,
            list: &list,
        })
        .collect();
    rhi.take();
    composed(&mut scene, &mut particles, (64, 64), &[], &mut batches).unwrap();
    let generation = scene.depth_generation;
    rhi.take();
    assert!(matches!(
        composed(
            &mut scene,
            &mut particles,
            (64, 64),
            &[particle()],
            &mut batches
        ),
        Err(ImportedSceneError::BatchLimit)
    ));
    assert_eq!(rhi.take(), Effects::default());
    assert_eq!(scene.depth_generation, generation);
}
