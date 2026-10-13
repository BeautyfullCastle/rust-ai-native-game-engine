// Mock call/descriptor evidence only: no adapter, actual GPU allocation census,
// physical residency, shader timing or cross-build performance measurement.
use crate::{MaterialShading, MaterialShadingError, ScalarMaterial};

fn static_asset() -> orr_model::StaticModel {
    orr_model::import::import_with_resolver(
        "fixtures/model.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap()
}

#[cfg(feature = "animation")]
fn animated_asset() -> orr_model::animation::AnimatedModel {
    orr_model::animation_import::import_with_resolver(
        "fixtures/animated_strip.glb",
        include_bytes!("../../../orr_model/tests/fixtures/animated_strip.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap()
}

#[test]
fn static_material_table_rejection_has_no_rhi_effects() {
    let asset = static_asset();
    let rhi = Mock::default();
    for count in [
        asset.source().materials.len() - 1,
        asset.source().materials.len() + 1,
    ] {
        let table = vec![MaterialShading::LegacyDiffuse; count];
        assert!(matches!(
            ModelRenderer::new_with_materials(rhi.clone(), FORMAT, asset.clone(), &table),
            Err(crate::ModelRenderError::InvalidMaterial(
                MaterialShadingError::MaterialCount
            ))
        ));
        assert_eq!(rhi.take(), Effects::default());
    }
    let table = vec![MaterialShading::LegacyDiffuse; asset.source().materials.len()];
    let _valid = ModelRenderer::new_with_materials(rhi.clone(), FORMAT, asset, &table).unwrap();
    let effects = rhi.take();
    assert!(
        effects.allocations > 0 && effects.writes > 0,
        "zero-effect mock must have a positive control"
    );
}

#[cfg(feature = "animation")]
#[test]
fn skinned_material_table_rejection_has_no_rhi_effects() {
    let asset = animated_asset();
    let rhi = Mock::default();
    for count in [
        asset.source().materials.len() - 1,
        asset.source().materials.len() + 1,
    ] {
        let table = vec![MaterialShading::LegacyDiffuse; count];
        assert!(matches!(
            SkinnedModelRenderer::new_with_materials(rhi.clone(), FORMAT, asset.clone(), &table),
            Err(crate::SkinnedRenderError::InvalidMaterial(
                MaterialShadingError::MaterialCount
            ))
        ));
        assert_eq!(rhi.take(), Effects::default());
    }
    let table = vec![MaterialShading::LegacyDiffuse; asset.source().materials.len()];
    let _valid =
        SkinnedModelRenderer::new_with_materials(rhi.clone(), FORMAT, asset, &table).unwrap();
    let effects = rhi.take();
    assert!(
        effects.allocations > 0 && effects.writes > 0,
        "zero-effect mock must have a positive control"
    );
}

fn assert_globals(effects: &Effects, label: &str) {
    let bytes = 288
        + if cfg!(feature = "irradiance-probes") {
            9280
        } else {
            0
        };
    let matching: Vec<_> = effects
        .buffers
        .iter()
        .filter(|(name, _, _)| name == label)
        .collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].1, bytes);
    assert!(bytes <= 16 * 1024);
    assert_eq!(
        effects
            .allocation_kinds
            .iter()
            .filter(|&&k| k == "shader")
            .count(),
        1
    );
    assert_eq!(
        effects
            .allocation_kinds
            .iter()
            .filter(|&&k| k == "pipeline")
            .count(),
        3
    );
    assert!(effects.allocation_kinds.contains(&"texture"));
    assert!(!effects.upload_lengths.is_empty());
}

fn assert_repeated_draw(effects: &Effects) {
    assert_eq!(effects.allocations, 0);
    assert!(effects.allocation_kinds.is_empty());
    assert!(effects.buffers.is_empty());
    assert!(effects.textures.is_empty());
    assert!(effects.texture_lifecycle.is_empty());
    assert!(effects.writes > 0);
    assert!(!effects.upload_lengths.is_empty());
    assert_eq!(effects.encoders, 1);
    assert_eq!(effects.submits, 1);
    assert_eq!(effects.passes.len(), 1);
}

fn static_trace(shading: Option<MaterialShading>) -> [Effects; 3] {
    let asset = static_asset();
    let rhi = Mock::default();
    let mut renderer = match shading {
        None => ModelRenderer::new(rhi.clone(), FORMAT, asset.clone()).unwrap(),
        Some(value) => ModelRenderer::new_with_materials(
            rhi.clone(),
            FORMAT,
            asset.clone(),
            &vec![value; asset.source().materials.len()],
        )
        .unwrap(),
    };
    let constructor = rhi.take();
    assert_globals(&constructor, "model globals");
    let objects: Vec<_> = constructor
        .buffers
        .iter()
        .filter(|(name, _, _)| name == "model object")
        .collect();
    assert_eq!(objects.len(), asset.source().primitives.len());
    assert!(objects.iter().all(|(_, size, _)| *size == 176));
    let camera = Camera3D::orthographic([0.0, 0.0, 5.0], [0.0; 3], 2.0);
    let lighting = Lighting {
        shadows: false,
        ..Default::default()
    };
    renderer.draw(&0, (32, 32), &camera, &lighting).unwrap();
    let first = rhi.take();
    assert!(first.allocations > 0, "first draw creates depth");
    renderer.draw(&0, (32, 32), &camera, &lighting).unwrap();
    let repeated = rhi.take();
    assert_repeated_draw(&repeated);
    [constructor, first, repeated]
}

#[test]
fn static_feature_on_legacy_and_material_resource_traces_match() {
    // Same build only: material-enabled legacy already has the enlarged uniforms.
    // Existing source/layout checks, not this equality, establish +32/+16 bytes.
    let old = static_trace(None);
    assert_eq!(old, static_trace(Some(MaterialShading::LegacyDiffuse)));
    assert_eq!(
        old,
        static_trace(Some(MaterialShading::MetallicRoughness(
            ScalarMaterial::new(0.7, 0.45).unwrap()
        )))
    );
}

#[cfg(feature = "animation")]
fn skinned_trace(shading: Option<MaterialShading>) -> [Effects; 3] {
    let asset = animated_asset();
    let pose = asset.sample_clip(0, 0.5).unwrap();
    let rhi = Mock::default();
    let mut renderer = match shading {
        None => SkinnedModelRenderer::new(rhi.clone(), FORMAT, asset.clone()).unwrap(),
        Some(value) => SkinnedModelRenderer::new_with_materials(
            rhi.clone(),
            FORMAT,
            asset.clone(),
            &vec![value; asset.source().materials.len()],
        )
        .unwrap(),
    };
    let constructor = rhi.take();
    assert_globals(&constructor, "skinned globals");
    assert!(
        !constructor
            .buffers
            .iter()
            .any(|(name, _, _)| name == "independent skinned instance palette")
    );
    let camera = Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 2.2);
    let lighting = Lighting {
        shadows: false,
        ..Default::default()
    };
    let instances = [SkinnedInstance::new(&pose)];
    renderer
        .draw(&0, (32, 32), &camera, &lighting, &instances)
        .unwrap();
    let first = rhi.take();
    let palettes: Vec<_> = first
        .buffers
        .iter()
        .filter(|(name, _, _)| name == "independent skinned instance palette")
        .collect();
    assert_eq!(palettes.len(), asset.source().primitives.len());
    assert!(palettes.iter().all(|(_, size, _)| *size == 4272));
    renderer
        .draw(&0, (32, 32), &camera, &lighting, &instances)
        .unwrap();
    let repeated = rhi.take();
    assert_repeated_draw(&repeated);
    [constructor, first, repeated]
}

#[cfg(feature = "animation")]
#[test]
fn skinned_feature_on_legacy_and_material_resource_traces_match() {
    let old = skinned_trace(None);
    assert_eq!(old, skinned_trace(Some(MaterialShading::LegacyDiffuse)));
    assert_eq!(
        old,
        skinned_trace(Some(MaterialShading::MetallicRoughness(
            ScalarMaterial::new(0.7, 0.45).unwrap()
        )))
    );
}
