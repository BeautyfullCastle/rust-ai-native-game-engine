//! Opt-in, game-typed authoring input. Kept on the server rather than adding
//! fields to public host/config structs that embedders construct literally.

use std::sync::Arc;

use orr_reflect::{Reflect, TypeDesc, TypeRegistry};
use orr_sim::{Game, PlayerSlot};
use serde_json::{json, Value as J};

use crate::{json::json_to_value, methods, RpcError};

type DeriveCommands<G> =
    dyn Fn(PlayerSlot, &<G as Game>::Input) -> Vec<<G as Game>::Command> + Send + Sync;

pub(crate) struct StructuredInput<G: Game> {
    desc: TypeDesc,
    pub(crate) descriptor: J,
    pub(crate) max_players: u8,
    pub(crate) commands: Arc<DeriveCommands<G>>,
}

impl<G: Game> StructuredInput<G> {
    pub(crate) fn new(
        name: &'static str,
        max_players: u8,
        commands: impl Fn(PlayerSlot, &G::Input) -> Vec<G::Command> + Send + Sync + 'static,
    ) -> Self
    where
        G::Input: Reflect,
    {
        // This temporary registry generates only the input descriptor; input
        // is not a scene component and is never added to the game's registry.
        let mut types = TypeRegistry::new();
        types.register_component::<G::Input>(name);
        let schema: J =
            serde_json::from_str(&types.type_json_schema(name).expect("registered input"))
                .expect("reflection produces valid JSON Schema");
        Self {
            desc: G::Input::describe(),
            descriptor: json!({"schema": schema, "value_format": methods::VALUE_FORMAT, "max_players": max_players}),
            max_players,
            commands: Arc::new(commands),
        }
    }

    pub(crate) fn decode(&self, value: &J) -> Result<G::Input, RpcError> {
        let value = json_to_value(&self.desc, value, false).map_err(RpcError::params)?;
        // Padding/hidden fields are deterministic zeros, never caller bytes.
        let mut input = <G::Input as bytemuck::Zeroable>::zeroed();
        self.desc
            .set(bytemuck::bytes_of_mut(&mut input), &[], value)
            .map_err(|e| RpcError::params(e.to_string()))?;
        Ok(input)
    }
}
