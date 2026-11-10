//! Bounded, strict schema-1 Room layout recipe shared by creation and editing.
//! Parsing is not scene admission or a proof that a puzzle can be completed.
use orr_fp::{FPVec3, FP};
use serde::Deserialize;
pub const MAX_BYTES: usize = 65536;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    schema: u32,
    pub player: [String; 2],
    pub key: [String; 2],
    pub exit: [String; 2],
    pub walls: Vec<Wall>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wall {
    pub center: [String; 2],
    pub half_extents: [String; 2],
}

impl Recipe {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Room layout exceeds 65536 bytes".into());
        }
        let recipe: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if recipe.schema != 1 || recipe.walls.len() > 64 {
            return Err("layout requires schema 1 and at most 64 walls".into());
        }
        for pair in [&recipe.player, &recipe.key, &recipe.exit] {
            position(pair)?;
        }
        for wall in &recipe.walls {
            position(&wall.center)?;
            position(&wall.half_extents)?;
        }
        Ok(recipe)
    }
    pub fn positions(&self) -> Result<[FPVec3; 3], String> {
        Ok([
            position(&self.player)?,
            position(&self.key)?,
            position(&self.exit)?,
        ])
    }
}

pub fn decimal(text: &str) -> Result<FP, String> {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let mut parts = unsigned.split('.');
    let integer = parts.next().unwrap_or("");
    let fraction = parts.next();
    if text.len() > 48
        || integer.is_empty()
        || !integer.bytes().all(|b| b.is_ascii_digit())
        || fraction.is_some_and(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
        || parts.next().is_some()
    {
        return Err("layout coordinates require plain decimal strings, at most 48 bytes".into());
    }
    text.parse::<FP>()
        .map_err(|e| format!("invalid fixed-point decimal: {e:?}"))
}
pub fn position(pair: &[String; 2]) -> Result<FPVec3, String> {
    Ok(FPVec3::new(
        decimal(&pair[0])?,
        FP::HALF,
        decimal(&pair[1])?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    const VALID: &str =
        r#"{"schema":1,"player":["-3","0"],"key":["-2","0"],"exit":["3","0"],"walls":[]}"#;
    #[test]
    fn bounded_strict_shared_recipe() {
        assert_eq!(
            Recipe::parse(VALID.as_bytes())
                .unwrap()
                .positions()
                .unwrap()[0]
                .x,
            FP::from_int(-3)
        );
        for bad in [
            VALID.replace("\"schema\":1", "\"schema\":1,\"schema\":1"),
            VALID.replace("\"schema\":1", "\"schema\":2"),
            VALID.replace("\"-3\"", "-3"),
            VALID.replace("\"-3\"", "\"1e0\""),
            VALID.replace("\"walls\":[]", "\"extra\":0,\"walls\":[]"),
            " ".repeat(MAX_BYTES + 1),
        ] {
            assert!(Recipe::parse(bad.as_bytes()).is_err(), "{bad}");
        }
        for bad in ["", "+1", ".5", "1.", "NaN", "1e0", "1.2.3"] {
            assert!(decimal(bad).is_err());
        }
    }
}
