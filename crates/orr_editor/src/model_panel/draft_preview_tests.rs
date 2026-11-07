use super::*;
fn context() -> Context {
    Context {
        scene: "scene.json".into(),
        epoch: 1,
        checksum: 2,
        guid: "guid".into(),
        entity: Entity {
            index: 3,
            version: 4,
        },
        project: "project".into(),
        package: "package".into(),
        asset: "asset".into(),
        kind: ModelKind::Static,
        package_digest: "a".repeat(64),
        source_hash: "b".repeat(64),
        body_bits: vec![0; 7],
        transform_bits: vec![0; 10],
        document: vec![1],
        binding_path: "binding.json".into(),
        reserved_scene_models: 0,
        #[cfg(feature = "animated-models")]
        animation: (0, "Once".into(), "0".into()),
    }
}
fn placement() -> DraftPlacement {
    let source = orr_model::ModelSource {
        format: "orr_static_model".into(),
        version: 1,
        asset_id: "fixture.model.json".into(),
        dependencies: vec![orr_model::Dependency {
            uri: "$source".into(),
            sha256: "a".repeat(64),
        }],
        primitives: vec![orr_model::Primitive {
            id: "fixture.model.json#node=0".into(),
            vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
                .map(|position| orr_model::Vertex {
                    position,
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0; 2],
                })
                .to_vec(),
            indices: vec![0, 1, 2],
            material: 0,
            transform: orr_model::IDENTITY,
        }],
        materials: vec![orr_model::Material {
            base_color: [1.0; 4],
            image: 0,
            linear_filter: false,
            wrap_s: orr_model::Wrap::Clamp,
            wrap_t: orr_model::Wrap::Clamp,
        }],
        images: vec![orr_model::Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4],
        }],
    };
    DraftPlacement::Static(ModelPlacement {
        entity: context().entity,
        model: std::sync::Arc::new(orr_model::StaticModel::new(source).unwrap()),
        instance: orr_render::StaticInstance::default(),
    })
}
#[test]
fn unchanged_context_keeps_preview_but_mismatch_retires_permanently() {
    let c = context();
    let mut s = State {
        accepted: Some((c.clone(), placement())),
    };
    s.sync(Some(&c));
    assert!(s.accepted.is_some());
    let mut changed = c.clone();
    changed.epoch += 1;
    s.sync(Some(&changed));
    assert!(s.accepted.is_none());
    s.sync(Some(&c));
    assert!(s.accepted.is_none());
}
#[test]
fn all_scene_selection_asset_draft_and_binding_changes_are_pinned() {
    let c = context();
    let mut variants = Vec::new();
    macro_rules! change {
        ($f:ident,$v:expr) => {{
            let mut x = c.clone();
            x.$f = $v;
            variants.push(x);
        }};
    }
    change!(scene, "other".into());
    change!(checksum, 3);
    change!(guid, "other".into());
    change!(
        entity,
        Entity {
            index: 3,
            version: 5
        }
    );
    change!(project, "other".into());
    change!(package, "other".into());
    change!(asset, "other".into());
    change!(source_hash, "other".into());
    change!(package_digest, "other".into());
    change!(body_bits, vec![1; 7]);
    change!(transform_bits, vec![1; 10]);
    change!(document, vec![2]);
    change!(binding_path, "different-bindings.json".into());
    change!(reserved_scene_models, 1);
    change!(kind, ModelKind::Animated);
    #[cfg(feature = "animated-models")]
    change!(animation, (1, "Loop".into(), "0.5".into()));
    for x in variants {
        let mut s = State {
            accepted: Some((c.clone(), placement())),
        };
        s.sync(Some(&x));
        assert!(s.accepted.is_none());
    }
}
#[test]
fn invalid_context_clears_preview_and_entity_accessor_is_exact() {
    let p = placement();
    assert_eq!(p.entity(), context().entity);
    let copy = p.duplicate();
    assert_eq!(copy.entity(), p.entity());
    let mut s = State {
        accepted: Some((context(), p)),
    };
    s.sync(None);
    assert!(s.accepted.is_none());
}
