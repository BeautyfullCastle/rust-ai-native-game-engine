//! Typed sprite validation before create-only JSON publication.
//!
//! See `crate::json_output` for the trusted, quiescent destination contract.
use orr_sprite::{MAX_JSON_BYTES, SpriteDocument};
use std::path::Path;

pub(super) use crate::json_output::Published;
type Result<T> = std::result::Result<T, String>;

fn failure(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}")
}

fn validated_json(document: &SpriteDocument) -> Result<String> {
    let json = document
        .to_json()
        .map_err(|e| failure("serialize sprite", e))?;
    if json.len() > MAX_JSON_BYTES {
        return Err("sprite JSON exceeds byte limit".into());
    }
    let reloaded =
        SpriteDocument::from_json(&json).map_err(|e| failure("validate serialized sprite", e))?;
    if reloaded
        .to_json()
        .map_err(|e| failure("canonical sprite", e))?
        != json
    {
        return Err("sprite canonical reload mismatch".into());
    }
    Ok(json)
}

pub(super) fn publish_new(
    output: &Path,
    document: &SpriteDocument,
    check: impl FnMut() -> Result<()>,
) -> Result<Published> {
    let json = validated_json(document)?;
    crate::json_output::publish_new(output, json.as_bytes(), check)
}

#[cfg(test)]
use crate::json_output::Point;
#[cfg(test)]
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

// Keep the existing typed fault-injection coverage against the shared publisher.
#[cfg(test)]
fn publish(
    output: &Path,
    document: &SpriteDocument,
    check: impl FnMut() -> Result<()>,
    hook: impl FnMut(Point, &Path) -> Result<()>,
) -> Result<Published> {
    let json = validated_json(document)?;
    crate::json_output::publish_with_hook(output, json.as_bytes(), check, hook)
}

#[cfg(test)]
mod tests;
