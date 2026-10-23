//! Frozen compatibility requirements for registered, reflected-input games.
use serde_json::{json, Value as J};

/// Expected input contract, validated on both control and frame connections.
/// Fields are private so invalid contracts cannot bypass construction checks.
#[derive(Clone, Debug, PartialEq)]
pub struct ExpectedInputDescriptor {
    schema: J,
    value_format: String,
    max_players: u8,
}

impl ExpectedInputDescriptor {
    /// Builds an expectation for the engine's supported managed-input protocol.
    pub fn new(
        schema: J,
        value_format: impl Into<String>,
        max_players: u8,
    ) -> Result<Self, String> {
        let value_format = value_format.into();
        if !schema.is_object() {
            return Err("expected input schema must be an object".into());
        }
        if value_format != crate::methods::VALUE_FORMAT {
            return Err("unsupported expected input value format".into());
        }
        if !(1..=16).contains(&max_players) {
            return Err("expected input player limit must be 1..=16".into());
        }
        Ok(Self {
            schema,
            value_format,
            max_players,
        })
    }

    pub fn schema(&self) -> &J {
        &self.schema
    }
    pub fn value_format(&self) -> &str {
        &self.value_format
    }
    pub fn max_players(&self) -> u8 {
        self.max_players
    }

    /// Validates the result of `registry.input` without accepting missing fields.
    pub fn validate(&self, actual: &J) -> Result<(), String> {
        for (key, expected) in [
            ("schema", self.schema.clone()),
            ("value_format", json!(self.value_format)),
            ("max_players", json!(self.max_players)),
            ("managed_held", crate::input::ManagedHeld::descriptor()),
        ] {
            if actual.get(key) != Some(&expected) {
                return Err(format!(
                    "registry.input {key} does not match the expected input contract"
                ));
            }
        }
        Ok(())
    }
}
