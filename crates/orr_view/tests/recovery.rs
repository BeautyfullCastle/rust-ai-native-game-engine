#![allow(clippy::float_arithmetic)]
use orr_bridge::{BridgeStats, FrameView, Snapshot, SnapshotParts};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_view::{
    Extracted, Extracted3, Extractor, Extractor3, InterpMode, Quat, Shape, Shape3, Style, Style3,
    Transform2, Transform3, Vec2, Vec3, ViewConfig, ViewLifecycle, ViewWorld, ViewWorld3,
};
use std::sync::Arc;

struct ByTick;
const ENTITY: Entity = Entity {
    index: 1,
    version: 0,
};
impl Extractor for ByTick {
    fn extract(&self, f: FrameView<'_>, out: &mut Vec<Extracted>) {
        out.push(Extracted {
            entity: ENTITY,
            transform: Transform2::new(Vec2::new(f.tick() as f32, 0.0), 0.0),
            mode: InterpMode::Prediction,
            style: Style {
                shape: Shape::Circle,
                size: 1.0,
                half_y: 0.0,
                color: [1.0; 4],
            },
        });
    }
}
impl Extractor3 for ByTick {
    fn extract(&self, f: FrameView<'_>, out: &mut Vec<Extracted3>) {
        out.push(Extracted3 {
            entity: ENTITY,
            transform: Transform3::new(Vec3::new(f.tick() as f32, 0.0, 0.0), Quat::IDENTITY),
            mode: InterpMode::Prediction,
            style: Style3::new(Shape3::Sphere { radius: 1.0 }, [1.0; 3]),
        });
    }
}
fn snapshot(tick: u64) -> Snapshot {
    let registry = ComponentRegistryBuilder::new().build();
    let mut frame = Frame::new(registry.clone());
    frame.set_tick(tick);
    let frame = Arc::new(frame);
    let mut prev = Frame::new(registry);
    prev.set_tick(tick.saturating_sub(1));
    Snapshot::from_parts(SnapshotParts {
        seq: tick + 1,
        tick,
        verified_tick: tick,
        tick_rate: 60,
        predicted: frame.clone(),
        predicted_prev: Some(Arc::new(prev)),
        verified: Some(frame),
        stats: BridgeStats::default(),
        last_rollback: None,
        timeline: None,
    })
}
#[test]
fn recovery_2d_shows_head_and_discards_old_lifecycle() {
    let mut view = ViewWorld::new(ByTick, ViewConfig::default());
    view.update(0.0, Some(&snapshot(1)));
    view.reset_from_snapshot(&snapshot(100));
    let mut items = Vec::new();
    view.render_items(&mut items);
    assert_eq!(
        items[0].transform.pos.x, 100.0,
        "show head, not previous tick"
    );
    assert_eq!(view.take_lifecycle(), vec![ViewLifecycle::Spawned(ENTITY)]);
    view.reset();
    assert!(view.take_lifecycle().is_empty());
}
#[test]
fn recovery_3d_shows_head_and_discards_old_lifecycle() {
    let mut view = ViewWorld3::new(ByTick, ViewConfig::default());
    view.update(0.0, Some(&snapshot(1)));
    view.reset_from_snapshot(&snapshot(100));
    let mut items = Vec::new();
    view.render_items(&mut items);
    assert_eq!(items[0].transform.pos.x, 100.0);
    assert_eq!(view.take_lifecycle(), vec![ViewLifecycle::Spawned(ENTITY)]);
    view.reset();
    assert!(view.take_lifecycle().is_empty());
}
