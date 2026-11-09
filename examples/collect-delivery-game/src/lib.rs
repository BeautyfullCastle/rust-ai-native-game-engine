//! An independently registered static game using the public engine starter APIs.
pub mod game;
pub mod registration;

use orr_bridge::FrameView;
use orr_view::{RenderItem, Shape, Style, Transform2};
use serde_json::{Value, json};

use game::{CollectDeliveryGame, CollectDeliveryState, Courier};

impl orr_static_starter::presentation::StandaloneGame for CollectDeliveryGame {
    fn presentation_hint() -> &'static str {
        "Press one arrow key to move one tile. Space picks up or delivers. Red loses. Release, then press Space after a win or loss to restart."
    }

    fn held_input(keys: orr_static_starter::game_contract::HeldKeys) -> Self::Input {
        // A directional press is one discrete movement pulse even if Space is
        // held at the same time; it cannot also collect/deliver accidentally.
        let action = if keys.x != 0 || keys.y != 0 {
            game::MODE_NAVIGATION_PULSE
        } else if keys.action {
            game::MODE_PRIMARY
        } else {
            game::MODE_RELEASED
        };
        game::CollectInput {
            dx: i32::from(keys.x.clamp(-1, 1)),
            dy: i32::from(keys.y.clamp(-1, 1)),
            action,
        }
    }

    fn input_value(input: &Self::Input) -> Value {
        json!({"dx":input.dx,"dy":input.dy,"action":input.action})
    }

    fn extract(frame: FrameView<'_>, out: &mut Vec<RenderItem>) {
        out.clear();
        let state = frame.singleton::<CollectDeliveryState>();
        if state.item_available != 0 {
            out.push(circle(
                orr_ecs::Entity::NONE,
                state.pickup,
                [1.0, 0.82, 0.1, 1.0],
                0.34,
            ));
        }
        out.push(circle(
            orr_ecs::Entity::NONE,
            state.destination,
            [0.2, 0.85, 0.25, 1.0],
            0.42,
        ));
        out.push(circle(
            orr_ecs::Entity::NONE,
            state.hazard,
            [0.9, 0.15, 0.12, 1.0],
            0.42,
        ));
        if let Some((entity, courier)) = frame.iter::<Courier>().next() {
            let color = if state.lost != 0 {
                [0.95, 0.2, 0.2, 1.0]
            } else if state.won != 0 {
                [0.15, 0.95, 0.3, 1.0]
            } else if state.carrying != 0 {
                [0.85, 0.35, 1.0, 1.0]
            } else {
                [0.25, 0.55, 1.0, 1.0]
            };
            out.push(circle(entity, courier.position, color, 0.3));
        }
    }

    fn state_value(frame: FrameView<'_>) -> Value {
        let state = frame.singleton::<CollectDeliveryState>();
        let courier = frame.iter::<Courier>().next().expect("admitted courier").1;
        let pair = |position: orr_fp::FPVec2| [position.x.raw(), position.y.raw()];
        json!({
            "position": pair(courier.position),
            "start": pair(state.start),
            "pickup": pair(state.pickup),
            "destination": pair(state.destination),
            "hazard": pair(state.hazard),
            "item_available": state.item_available,
            "carrying": state.carrying,
            "won": state.won,
            "lost": state.lost,
            "reserved": state.reserved,
            "action_held": state.action_held
        })
    }
}

fn circle(
    entity: orr_ecs::Entity,
    position: orr_fp::FPVec2,
    color: [f32; 4],
    size: f32,
) -> RenderItem {
    RenderItem {
        entity,
        transform: Transform2 {
            pos: orr_view::fp_to_vec2(position),
            rot: 0.0,
        },
        style: Style {
            shape: Shape::Circle,
            size,
            half_y: 0.0,
            color,
        },
    }
}
