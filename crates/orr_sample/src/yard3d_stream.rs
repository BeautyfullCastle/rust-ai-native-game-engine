//! Yard3D as a 3D view stream (`orr_viewstream`).
//!
//! The stream uses the same extractor as the native sample view, so ERP
//! clients see the authoritative frame with matching shapes, floor placement,
//! transforms, and materials.

use orr_bridge::{Bridge, FrameView};
use orr_ecs::Frame;
use orr_physics3d::{BODY_DYNAMIC, Body};
use orr_reflect::Reflect;
use orr_view::{Extracted3, fp_to_vec3};
use orr_viewstream::{
    FrameMeta, InputLayout, KindDef, PropType, Pumped3, Schema, StreamKinds3, StreamProducer,
    ViewStreamSource3,
};

use crate::yard3d_game::{NoCommand, TICK_RATE, Yard3D, YardInput};
use crate::yard3d_view::YardExtractor;

/// Static terrain and walls.
pub const KIND_STATIC: u16 = 0;
/// Simulated movable bodies, with a `speed` property in world units per second.
pub const KIND_DYNAMIC: u16 = 1;

/// A reflection-only mirror keeps the stream's input offsets tied to YardInput's
/// actual C layout without changing the simulation type or its encoding.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Reflect)]
struct YardInputLayout {
    /// Held button bits: 1 shoot, 2 spawn box, 4 spawn ball, 8 spawn capsule.
    buttons: u32,
    /// Reserved; write zero.
    _pad: u32,
    /// Camera ray origin in centimeters.
    origin: [i32; 3],
    /// Camera ray direction in thousandths of a unit.
    dir: [i32; 3],
}

const _: () = {
    assert!(std::mem::size_of::<YardInput>() == 32);
    assert!(std::mem::size_of::<YardInput>() == std::mem::size_of::<YardInputLayout>());
    assert!(
        std::mem::offset_of!(YardInput, buttons) == std::mem::offset_of!(YardInputLayout, buttons)
    );
    assert!(std::mem::offset_of!(YardInput, _pad) == std::mem::offset_of!(YardInputLayout, _pad));
    assert!(
        std::mem::offset_of!(YardInput, origin) == std::mem::offset_of!(YardInputLayout, origin)
    );
    assert!(std::mem::offset_of!(YardInput, dir) == std::mem::offset_of!(YardInputLayout, dir));
};

fn yard_input_layout() -> InputLayout {
    orr_viewstream::input_layout_of::<YardInputLayout>()
}

#[derive(Clone, Copy, Debug, Default)]
struct YardKinds;

impl StreamKinds3 for YardKinds {
    fn classify(&self, frame: FrameView<'_>, item: &Extracted3, props: &mut Vec<u32>) -> u16 {
        match frame.get::<Body>(item.entity) {
            Some(body) if body.kind == BODY_DYNAMIC => {
                props.push(fp_to_vec3(body.vel).length().to_bits());
                KIND_DYNAMIC
            }
            _ => KIND_STATIC,
        }
    }
}

/// The schema advertised to clients connecting to Yard3D's view stream.
pub fn yard3d_view_schema(build_id: u64, player_count: u8) -> Schema {
    Schema {
        game: "Yard3D".to_string(),
        dimensions: 3,
        build_id,
        tick_rate: TICK_RATE,
        player_count,
        kinds: vec![
            KindDef::new(KIND_STATIC, "static"),
            KindDef::new(KIND_DYNAMIC, "dynamic").with_prop("speed", PropType::F32),
        ],
        input: yard_input_layout(),
        // Yard3D has no one-off commands, but its command encoding is the
        // four-byte NoCommand Pod used by the session protocol.
        command_size: std::mem::size_of::<NoCommand>(),
        events: Vec::new(),
    }
}

/// A Yard3D view producer for direct frames or a snapshot-based Bridge stream.
pub struct Yard3dStreamProducer {
    source: ViewStreamSource3<YardExtractor, YardKinds>,
}

impl Yard3dStreamProducer {
    /// Creates a producer for a Yard3D host build and player count.
    pub fn new(build_id: u64, player_count: u8) -> Self {
        Self {
            source: ViewStreamSource3::new(
                YardExtractor,
                YardKinds,
                yard3d_view_schema(build_id, player_count),
            ),
        }
    }

    /// Polls a Yard3D bridge and preserves its snapshot, rollback and recovery
    /// metadata in the 3D stream. Each poll is consumed exactly once by the
    /// underlying source, including a bounded-mailbox reset baseline.
    pub fn pump<B: Bridge<Yard3D> + ?Sized>(&mut self, bridge: &mut B) -> Pumped3 {
        self.source.pump(bridge)
    }
}

impl StreamProducer for Yard3dStreamProducer {
    fn schema(&self) -> &Schema {
        self.source.schema()
    }

    fn encode_frame(&mut self, cur: &Frame, prev: Option<&Frame>, meta: FrameMeta) -> Vec<u8> {
        self.source.encode_frame(cur, prev, meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yard3d_game::Yard3D;
    use orr_ecs::ComponentRegistryBuilder;
    use orr_fp::{FP, FPVec3, fp};
    use orr_physics3d::{Collider, Shape};
    use orr_sim::Game;
    use orr_viewstream::{
        FLAG_DISCONTINUITY, FLAG_ROLLED_BACK, MODE_NONE, MODE_PREDICTION, SHAPE3_PLANE,
        SHAPE3_SPHERE, STYLE_CHECKER, ViewFrame3,
    };

    fn registry() -> std::sync::Arc<orr_ecs::ComponentRegistry> {
        let mut builder = ComponentRegistryBuilder::new();
        Yard3D::register(&mut builder);
        builder.build()
    }

    fn frame(registry: std::sync::Arc<orr_ecs::ComponentRegistry>, ball_y: i32) -> Frame {
        let mut frame = Frame::new(registry);
        let floor = frame.spawn();
        let floor_shape = Shape::cuboid(fp!(26), fp!(1), fp!(26));
        assert!(
            frame
                .add(
                    floor,
                    Body::new_static(FPVec3::new(fp!(0), fp!(-1), fp!(0))),
                )
                .is_none()
        );
        assert!(frame.add(floor, Collider::new(floor_shape)).is_none());

        let ball = frame.spawn();
        let ball_shape = Shape::sphere(fp!(0.5));
        assert!(
            frame
                .add(
                    ball,
                    Body::new_dynamic(
                        FPVec3::new(fp!(2), FP::from_int(ball_y), fp!(3)),
                        &ball_shape,
                        fp!(1),
                    )
                    .with_velocity(FPVec3::new(fp!(3), fp!(4), fp!(0))),
                )
                .is_none()
        );
        assert!(frame.add(ball, Collider::new(ball_shape)).is_none());
        frame
    }

    #[test]
    fn yard_schema_has_real_input_offsets_and_four_byte_command() {
        let schema = yard3d_view_schema(0x1234, 2);
        assert_eq!(std::mem::size_of::<YardInput>(), 32);
        assert_eq!(std::mem::offset_of!(YardInput, buttons), 0);
        assert_eq!(std::mem::offset_of!(YardInput, _pad), 4);
        assert_eq!(std::mem::offset_of!(YardInput, origin), 8);
        assert_eq!(std::mem::offset_of!(YardInput, dir), 20);
        assert_eq!(schema.dimensions, 3);
        assert_eq!(schema.tick_rate, TICK_RATE);
        assert_eq!(schema.command_size, 4);
        assert_eq!(schema.input.size, 32);
        let fields = &schema.input.fields;
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[0]["name"], "buttons");
        assert_eq!(fields[0]["offset"], 0);
        assert_eq!(fields[0]["size"], 4);
        assert!(
            fields[0]["doc"]
                .as_str()
                .unwrap()
                .contains("1 shoot, 2 spawn box, 4 spawn ball, 8 spawn capsule")
        );
        assert_eq!(fields[1]["name"], "_pad");
        assert_eq!(fields[1]["offset"], 4);
        assert_eq!(fields[1]["size"], 4);
        assert_eq!(fields[2]["name"], "origin");
        assert_eq!(fields[2]["offset"], 8);
        assert_eq!(fields[2]["size"], 12);
        assert!(fields[2]["doc"].as_str().unwrap().contains("centimeters"));
        assert_eq!(fields[3]["name"], "dir");
        assert_eq!(fields[3]["offset"], 20);
        assert_eq!(fields[3]["size"], 12);
        assert!(fields[3]["doc"].as_str().unwrap().contains("thousandths"));
    }

    #[test]
    fn authoritative_frame_preserves_entities_floor_and_poses() {
        let registry = registry();
        let previous = frame(registry.clone(), 6);
        let current = frame(registry, 7);
        let mut producer = Yard3dStreamProducer::new(9, 2);
        let bytes = producer.encode_frame(
            &current,
            Some(&previous),
            FrameMeta {
                tick: 11,
                verified_tick: 10,
                ..FrameMeta::default()
            },
        );
        let view = ViewFrame3::decode(&bytes).unwrap();

        assert_eq!(view.entities.len(), 2);
        let floor = &view.entities[0];
        assert_eq!(floor.id, u64::from(0u32));
        assert_eq!(floor.shape, SHAPE3_PLANE);
        assert_eq!(floor.mode, MODE_NONE);
        assert_eq!(floor.cur.pos[1], 0.0, "floor is drawn at its top surface");
        assert_eq!(floor.style_flags & STYLE_CHECKER, STYLE_CHECKER);
        assert_eq!(floor.size, [26.0, 0.0, 26.0]);

        let ball = &view.entities[1];
        assert_eq!(ball.id, u64::from(1u32));
        assert_eq!(ball.shape, SHAPE3_SPHERE);
        assert_eq!(ball.mode, MODE_PREDICTION);
        assert_eq!(ball.prev.pos, [2.0, 6.0, 3.0]);
        assert_eq!(ball.cur.pos, [2.0, 7.0, 3.0]);
    }

    #[test]
    fn discontinuity_collapses_previous_pose_and_keeps_wire_flags() {
        let registry = registry();
        let previous = frame(registry.clone(), 6);
        let current = frame(registry, 7);
        let mut producer = Yard3dStreamProducer::new(9, 2);
        let bytes = producer.encode_frame(
            &current,
            Some(&previous),
            FrameMeta {
                tick: 11,
                verified_tick: 10,
                flags: FLAG_DISCONTINUITY,
                rollback: Some((8, 10)),
            },
        );
        let view = ViewFrame3::decode(&bytes).unwrap();
        assert!(view.has(FLAG_DISCONTINUITY | FLAG_ROLLED_BACK));
        assert!(view.entities.iter().all(|entity| entity.prev == entity.cur));
    }
}
