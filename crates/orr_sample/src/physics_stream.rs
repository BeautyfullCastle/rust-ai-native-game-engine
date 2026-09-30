//! The physics scene as a view stream (`orr_viewstream`): the game's
//! vocabulary of entity kinds, its schema and a ready producer. The entities
//! and their look are exactly what the Rust viewport draws
//! ([`PhysExtractor`]); this only names kinds and adds properties.
//!
//! Used by the ERP host (`orr_remote::sample`) for the `viewstream` topic and
//! by the C ABI (`orr_ffi`).

use orr_bridge::FrameView;
use orr_physics::{Body, BODY_DYNAMIC, BODY_KINEMATIC};
use orr_view::{fp_to_vec2, Extracted, InterpMode};
use orr_viewstream::{input_layout_of, EventDef, KindDef, PropType, Schema, StreamKinds, ViewStreamSource};

use crate::physics_game::{NoCommand, PaddleTag, PhysEvent, PhysInput, TICK_RATE};
use crate::physics_view::PhysExtractor;

/// Entity kind: a wall or an obstacle that never moves.
pub const KIND_STATIC: u16 = 0;
/// Entity kind: a ball or a box pushed around by physics. Property `speed` (f32, world units per second).
pub const KIND_DYNAMIC: u16 = 1;
/// Entity kind: a moving bar of the mixer scene (kinematic, no owner).
pub const KIND_BAR: u16 = 2;
/// Entity kind: the paddle of a player. Property `slot` (u32).
pub const KIND_PADDLE: u16 = 3;

/// Sorts the bodies of the physics scene into the kinds above.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhysKinds;

impl StreamKinds for PhysKinds {
    fn classify(&self, frame: FrameView<'_>, item: &Extracted, props: &mut Vec<u32>) -> u16 {
        let Some(body) = frame.get::<Body>(item.entity) else { return KIND_STATIC };
        match body.kind {
            BODY_DYNAMIC => {
                props.push(fp_to_vec2(body.vel).length().to_bits());
                KIND_DYNAMIC
            }
            BODY_KINEMATIC => match frame.get::<PaddleTag>(item.entity) {
                Some(tag) => {
                    props.push(tag.slot);
                    KIND_PADDLE
                }
                None => KIND_BAR,
            },
            _ => KIND_STATIC,
        }
    }
}

/// The schema of the physics scene.
pub fn phys_schema(build_id: u64, player_count: u8, tick_rate: u32) -> Schema {
    Schema {
        game: "PhysGame".to_string(),
        build_id,
        tick_rate,
        player_count,
        kinds: vec![
            KindDef::new(KIND_STATIC, "static"),
            KindDef::new(KIND_DYNAMIC, "dynamic").with_prop("speed", PropType::F32),
            KindDef::new(KIND_BAR, "bar"),
            KindDef::new(KIND_PADDLE, "paddle").with_prop("slot", PropType::U32),
        ],
        input: input_layout_of::<PhysInput>(),
        command_size: std::mem::size_of::<NoCommand>(),
        events: vec![EventDef { id: 0, name: "trigger".to_string(), payload_size: std::mem::size_of::<PhysEvent>() }],
    }
}

/// A view stream source for the physics scene. Every paddle is shown
/// predicted (one simulation, no remote peers).
pub fn phys_stream_source(build_id: u64, player_count: u8) -> ViewStreamSource<PhysExtractor, PhysKinds> {
    let extractor = PhysExtractor { remote_mode: InterpMode::Prediction, local_slot: 0 };
    ViewStreamSource::new(extractor, PhysKinds, phys_schema(build_id, player_count, TICK_RATE))
}
