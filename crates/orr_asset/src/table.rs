use crate::{check_id, AssetError, AssetRef, TypedRef, MAX_SIM_RECORDS};
use orr_fp::FP;
mod sealed {
    pub trait Sealed {}
}

/// Supported immutable simulation asset schema. Sealed to the v1 registry.
pub trait SimAsset: sealed::Sealed {
    /// Stable registry ID, independent of Rust names/hashes.
    const TYPE_ID: u32;
    /// Exact supported schema version; automatic migration is not performed.
    const SCHEMA_VERSION: u32;
    /// Validate in-memory asset semantics before constructing a table.
    fn validate(&self) -> Result<(), AssetError>;
}

/// Fixed-point movement per tick, with `0 < speed_per_tick <= 16`.
///
/// Its canonical payload is exactly one signed raw Q48.16 `i64`, little-endian.
/// Fields are private so successful construction preserves this invariant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MotionProfileV1 {
    speed_per_tick: FP,
}
impl MotionProfileV1 {
    /// Construct from deterministic fixed-point data; usable in generated statics.
    pub const fn new(speed_per_tick: FP) -> Result<Self, AssetError> {
        if speed_per_tick.raw() <= 0 || speed_per_tick.raw() > 16 * 65536 {
            return Err(AssetError::InvalidMotion);
        }
        Ok(Self { speed_per_tick })
    }
    /// The validated movement constant.
    pub const fn speed_per_tick(self) -> FP {
        self.speed_per_tick
    }
    /// Decode exactly eight bytes without casts, allocation, or float parsing.
    pub fn decode(bytes: &[u8]) -> Result<Self, AssetError> {
        let raw: [u8; 8] = bytes.try_into().map_err(|_| AssetError::LengthMismatch)?;
        Self::new(FP::from_raw(i64::from_le_bytes(raw)))
    }
    /// Canonical raw fixed-point payload, including its exact endian spelling.
    pub const fn encode(self) -> [u8; 8] {
        self.speed_per_tick.raw().to_le_bytes()
    }
}
impl sealed::Sealed for MotionProfileV1 {}
impl SimAsset for MotionProfileV1 {
    const TYPE_ID: u32 = crate::MOTION_PROFILE_TYPE_ID;
    const SCHEMA_VERSION: u32 = 1;
    fn validate(&self) -> Result<(), AssetError> {
        Self::new(self.speed_per_tick).map(|_| ())
    }
}

/// Validated immutable borrowed records, sorted by numeric GUID.
///
/// Construction checks bounds/order/values once; resolve performs binary search
/// with no allocation, I/O, lock, or mutation. Caller-owned records remain
/// immutably borrowed for this table's lifetime; no process-global store exists.
#[derive(Debug)]
pub struct SimTable<'a, T> {
    entries: &'a [(AssetRef, T)],
}
impl<'a, T: SimAsset> SimTable<'a, T> {
    /// Borrow canonical records; reject over-budget, null, duplicate or unsorted IDs.
    pub fn new(entries: &'a [(AssetRef, T)]) -> Result<Self, AssetError> {
        if entries.len() > MAX_SIM_RECORDS {
            return Err(AssetError::BudgetExceeded);
        }
        let mut previous = 0;
        for (id, value) in entries {
            previous = check_id(previous, *id)?;
            value.validate()?;
        }
        Ok(Self { entries })
    }
    /// Canonical records for inspection/re-encoding by a bundle verifier.
    pub fn entries(&self) -> &'a [(AssetRef, T)] {
        self.entries
    }
    /// Resolve by stable GUID. Metadata typing alone does not imply membership.
    pub fn resolve(&self, id: TypedRef<T>) -> Result<&'a T, AssetError> {
        self.entries
            .binary_search_by_key(&id.raw(), |(id, _)| *id)
            .map(|index| &self.entries[index].1)
            .map_err(|_| AssetError::MissingAsset)
    }
}
