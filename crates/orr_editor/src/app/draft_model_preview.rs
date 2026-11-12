//! Compose a selected-object draft without changing any unrelated placement.
#[cfg(feature = "animated-models")]
use crate::viewport3d::AnimatedPlacement;
use crate::{model_panel::DraftPlacement, viewport3d::ModelPlacement};
pub(super) fn compose(
    models: &mut Vec<ModelPlacement>,
    #[cfg(feature = "animated-models")] animated: &mut Vec<AnimatedPlacement>,
    draft: Option<DraftPlacement>,
) {
    let Some(draft) = draft else {
        return;
    };
    let entity = draft.entity();
    models.retain(|placement| placement.entity != entity);
    #[cfg(feature = "animated-models")]
    animated.retain(|placement| placement.entity != entity);
    match draft {
        DraftPlacement::Static(placement) => models.push(placement),
        #[cfg(feature = "animated-models")]
        DraftPlacement::Animated(placement) => animated.push(placement),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use orr_ecs::Entity;
    use std::sync::Arc;
    fn entity(index: u32, version: u32) -> Entity {
        Entity { index, version }
    }
    fn static_placement(entity: Entity) -> ModelPlacement {
        let model = orr_model::import::import_with_resolver(
            "fixture.glb",
            include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
            |_| panic!("embedded fixture only"),
        )
        .unwrap();
        ModelPlacement {
            entity,
            model: Arc::new(model),
            instance: Default::default(),
        }
    }
    #[cfg(feature = "animated-models")]
    fn animated_placement(entity: Entity) -> AnimatedPlacement {
        let model = Arc::new(
            orr_model::animation::AnimatedModel::from_bytes(include_bytes!(
                "../../../../assets/room_character_demo/courier.orrmodel.json"
            ))
            .unwrap(),
        );
        let pose = model.sample_clip(1, 0.25).unwrap();
        AnimatedPlacement {
            entity,
            model,
            instance: orr_render::math3::Mat4::IDENTITY.0,
            pose,
        }
    }
    #[test]
    fn absent_draft_preserves_existing_content_order_and_arc_identity() {
        let mut models = vec![
            static_placement(entity(1, 0)),
            static_placement(entity(2, 0)),
        ];
        let retained = models[0].model.clone();
        let instance = models[0].instance;
        #[cfg(feature = "animated-models")]
        let mut animated = vec![animated_placement(entity(3, 0))];
        compose(
            &mut models,
            #[cfg(feature = "animated-models")]
            &mut animated,
            None,
        );
        assert_eq!(
            models.iter().map(|p| p.entity).collect::<Vec<_>>(),
            vec![entity(1, 0), entity(2, 0)]
        );
        assert!(Arc::ptr_eq(&models[0].model, &retained));
        assert_eq!(models[0].instance, instance);
        #[cfg(feature = "animated-models")]
        assert_eq!(animated.len(), 1);
    }
    #[test]
    fn static_draft_replaces_all_exact_entity_duplicates_preserving_other_generations() {
        let target = entity(7, 3);
        let mut models = vec![
            static_placement(target),
            static_placement(entity(7, 4)),
            static_placement(target),
            static_placement(entity(8, 0)),
        ];
        let untouched = models[1].model.clone();
        let mut draft = static_placement(target);
        draft.instance.translation = [3.0, 4.0, 5.0];
        #[cfg(feature = "animated-models")]
        let mut animated = vec![animated_placement(target), animated_placement(entity(9, 0))];
        compose(
            &mut models,
            #[cfg(feature = "animated-models")]
            &mut animated,
            Some(DraftPlacement::Static(draft)),
        );
        assert_eq!(
            models.iter().map(|p| p.entity).collect::<Vec<_>>(),
            vec![entity(7, 4), entity(8, 0), target]
        );
        assert!(Arc::ptr_eq(&models[0].model, &untouched));
        assert_eq!(models.last().unwrap().instance.translation, [3.0, 4.0, 5.0]);
        #[cfg(feature = "animated-models")]
        assert_eq!(
            animated.iter().map(|p| p.entity).collect::<Vec<_>>(),
            vec![entity(9, 0)]
        );
    }
    #[cfg(feature = "animated-models")]
    #[test]
    fn animated_draft_replaces_static_and_animated_target_only_and_keeps_sample_pose() {
        let target = entity(2, 1);
        let mut models = vec![static_placement(target), static_placement(entity(3, 1))];
        let mut animated = vec![animated_placement(target), animated_placement(entity(4, 1))];
        let untouched = animated[1].model.clone();
        let draft = animated_placement(target);
        let pose = draft.pose.clone();
        compose(
            &mut models,
            &mut animated,
            Some(DraftPlacement::Animated(draft)),
        );
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].entity, entity(3, 1));
        assert_eq!(animated.len(), 2);
        assert_eq!(animated[0].entity, entity(4, 1));
        assert!(Arc::ptr_eq(&animated[0].model, &untouched));
        assert_eq!(animated[1].entity, target);
        assert_eq!(animated[1].pose, pose);
    }
}
