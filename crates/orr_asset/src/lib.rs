//! Allocation-free, integer-only asset identity and ORAM v1 structural codec.
//!
//! This core does not load files, compute hashes, cook content, or bind releases.
//! SHA-256 fields are opaque bytes: successful decoding is **not** integrity or
//! authenticity verification. The caller must verify exact payload bytes and
//! their digests before admitting a bundle. See `docs/asset-pipeline-v1.md`.
//! Tables borrow immutable records and resolve by stable GUID, never by an index
//! stored in a Frame. Only [`AssetRef`] belongs in Frame POD state.
#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs, clippy::float_arithmetic, clippy::disallowed_types)]

mod manifest;
mod reference;
mod table;
pub use manifest::*;
pub use reference::*;
pub use table::*;

/// Structural validation failures. No variant implies digest verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetError {
    /// Null is not a live/required asset ID, or authoring spelling is invalid.
    InvalidId,
    /// Two consecutive canonical records have the same ID.
    DuplicateId,
    /// Records are not in strictly increasing numeric ID order.
    UnsortedIds,
    /// A referenced ID is absent from the selected table/manifest.
    MissingAsset,
    /// A type ID is not supported, or does not match the requested Rust type.
    WrongType,
    /// Manifest or asset schema version is unsupported.
    UnsupportedVersion,
    /// Manifest domain is unknown or incompatible with the type/caller.
    WrongDomain,
    /// Input is not an ORAM manifest.
    InvalidMagic,
    /// Header/entry/payload bytes are incomplete.
    Truncated,
    /// Declared, actual, or type-specific lengths do not match exactly.
    LengthMismatch,
    /// A fixed format or resource limit was exceeded (including size overflow).
    BudgetExceeded,
    /// Motion speed is outside the inclusive raw range 1..=1048576.
    InvalidMotion,
}

/// Reject null, duplicates, and noncanonical ordering without sorting inputs.
fn check_id(previous: u64, current: AssetRef) -> Result<u64, AssetError> {
    let id = current.get();
    if id == 0 {
        return Err(AssetError::InvalidId);
    }
    if id == previous {
        return Err(AssetError::DuplicateId);
    }
    if id < previous {
        return Err(AssetError::UnsortedIds);
    }
    Ok(id)
}
