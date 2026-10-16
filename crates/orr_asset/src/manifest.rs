use crate::{check_id, AssetError, AssetRef, SimAsset, TypedRef};

/// ORAM manifest version supported by this codec.
pub const MANIFEST_VERSION: u32 = 1;
/// Registry ID for `sim.motion_profile/1`.
pub const MOTION_PROFILE_TYPE_ID: u32 = 1;
/// Registry ID for `view.impact_pcm16/1`.
pub const IMPACT_PCM16_TYPE_ID: u32 = 2;
/// Fixed header byte length.
pub const HEADER_LEN: usize = 16;
/// Fixed entry byte length (including 32 opaque digest bytes).
pub const ENTRY_LEN: usize = 56;
/// Maximum actual manifest bytes (128 KiB), checked before parsing.
pub const MAX_MANIFEST_BYTES: usize = 128 * 1024;
/// Maximum live simulation records per manifest/table.
pub const MAX_SIM_RECORDS: usize = 64;
/// Maximum live view records per manifest.
pub const MAX_VIEW_RECORDS: usize = 16;
/// Maximum sum of declared simulation payload sizes (64 KiB).
pub const MAX_SIM_PAYLOAD_BYTES: u64 = 64 * 1024;
/// Maximum sum of declared cooked view payload sizes (2 MiB).
pub const MAX_VIEW_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024;
/// Maximum frames in a v1 mono PCM payload (one second at 48 kHz).
pub const MAX_PCM_FRAMES: u32 = 48000;

/// Separate simulation and view manifest namespaces.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Domain {
    /// Immutable simulation constants.
    Sim = 1,
    /// Presentation-only data.
    View = 2,
}
impl Domain {
    fn from_raw(raw: u32) -> Result<Self, AssetError> {
        match raw {
            1 => Ok(Self::Sim),
            2 => Ok(Self::View),
            _ => Err(AssetError::WrongDomain),
        }
    }
    fn max_records(self) -> usize {
        match self {
            Self::Sim => MAX_SIM_RECORDS,
            Self::View => MAX_VIEW_RECORDS,
        }
    }
    fn max_payload(self) -> u64 {
        match self {
            Self::Sim => MAX_SIM_PAYLOAD_BYTES,
            Self::View => MAX_VIEW_PAYLOAD_BYTES,
        }
    }
}

/// Wire metadata. Public fields support cookers, but require validation on use.
/// Digest bytes are opaque: zero/all-one hashes are structurally valid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestEntry {
    /// Stable nonnull GUID, not a content hash.
    pub id: AssetRef,
    /// Registry type identifier.
    pub type_id: u32,
    /// Exact schema version.
    pub schema_version: u32,
    /// Exact canonical payload length, never a native `usize` on the wire.
    pub payload_len: u64,
    /// SHA-256 of exact payload bytes, to be computed/verified outside this core.
    pub payload_sha256: [u8; 32],
}
impl ManifestEntry {
    /// Validate ID, supported type/schema, domain, and type-specific byte length.
    /// Does not inspect an artifact or authenticate/verify its hash.
    pub fn validate(&self, domain: Domain) -> Result<(), AssetError> {
        if self.id.is_null() {
            return Err(AssetError::InvalidId);
        }
        let expected_domain = match self.type_id {
            MOTION_PROFILE_TYPE_ID => Domain::Sim,
            IMPACT_PCM16_TYPE_ID => Domain::View,
            _ => return Err(AssetError::WrongType),
        };
        if domain != expected_domain {
            return Err(AssetError::WrongDomain);
        }
        if self.schema_version != 1 {
            return Err(AssetError::UnsupportedVersion);
        }
        if self.payload_len > domain.max_payload() {
            return Err(AssetError::BudgetExceeded);
        }
        match domain {
            Domain::Sim if self.payload_len != 8 => return Err(AssetError::LengthMismatch),
            Domain::View
                if self.payload_len < 10
                    || self.payload_len > 8 + u64::from(MAX_PCM_FRAMES) * 2
                    || self.payload_len % 2 != 0 =>
            {
                return Err(AssetError::LengthMismatch)
            }
            _ => (),
        }
        Ok(())
    }
    fn decode(bytes: &[u8]) -> Self {
        // Only called on an exact ENTRY_LEN chunk admitted by Manifest::decode.
        let mut digest = [0; 32];
        digest.copy_from_slice(&bytes[24..56]);
        Self {
            id: AssetRef::from_raw(u64_at(bytes, 0)),
            type_id: u32_at(bytes, 8),
            schema_version: u32_at(bytes, 12),
            payload_len: u64_at(bytes, 16),
            payload_sha256: digest,
        }
    }
    fn encode(self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.id.get().to_le_bytes());
        out[8..12].copy_from_slice(&self.type_id.to_le_bytes());
        out[12..16].copy_from_slice(&self.schema_version.to_le_bytes());
        out[16..24].copy_from_slice(&self.payload_len.to_le_bytes());
        out[24..56].copy_from_slice(&self.payload_sha256);
    }
}
fn u32_at(bytes: &[u8], start: usize) -> u32 {
    let mut value = [0; 4];
    value.copy_from_slice(&bytes[start..start + 4]);
    u32::from_le_bytes(value)
}
fn u64_at(bytes: &[u8], start: usize) -> u64 {
    let mut value = [0; 8];
    value.copy_from_slice(&bytes[start..start + 8]);
    u64::from_le_bytes(value)
}
fn validate_entries(
    domain: Domain,
    entries: impl Iterator<Item = ManifestEntry>,
) -> Result<(), AssetError> {
    let mut previous = 0;
    let mut total = 0u64;
    for entry in entries {
        entry.validate(domain)?;
        previous = check_id(previous, entry.id)?;
        total = total
            .checked_add(entry.payload_len)
            .ok_or(AssetError::BudgetExceeded)?;
        if total > domain.max_payload() {
            return Err(AssetError::BudgetExceeded);
        }
    }
    Ok(())
}

/// Canonical encoded size; rejects record counts before multiplication.
pub fn manifest_encoded_len(domain: Domain, count: usize) -> Result<usize, AssetError> {
    if count > domain.max_records() {
        return Err(AssetError::BudgetExceeded);
    }
    let len = count
        .checked_mul(ENTRY_LEN)
        .and_then(|n| n.checked_add(HEADER_LEN))
        .ok_or(AssetError::BudgetExceeded)?;
    if len > MAX_MANIFEST_BYTES {
        return Err(AssetError::BudgetExceeded);
    }
    Ok(len)
}

/// Encode canonical entries into an exactly sized caller-owned buffer.
/// All validation precedes writes; failures leave `out` unchanged. Inputs must
/// already be sorted; silently sorting would hide duplicate/order mistakes.
pub fn encode_manifest(
    domain: Domain,
    entries: &[ManifestEntry],
    out: &mut [u8],
) -> Result<(), AssetError> {
    let len = manifest_encoded_len(domain, entries.len())?;
    if out.len() != len {
        return Err(AssetError::LengthMismatch);
    }
    validate_entries(domain, entries.iter().copied())?;
    let count = u32::try_from(entries.len()).map_err(|_| AssetError::BudgetExceeded)?;
    out[..4].copy_from_slice(b"ORAM");
    out[4..8].copy_from_slice(&MANIFEST_VERSION.to_le_bytes());
    out[8..12].copy_from_slice(&(domain as u32).to_le_bytes());
    out[12..16].copy_from_slice(&count.to_le_bytes());
    for (entry, chunk) in entries
        .iter()
        .zip(out[HEADER_LEN..].chunks_exact_mut(ENTRY_LEN))
    {
        entry.encode(chunk);
    }
    Ok(())
}

/// Validated borrowed canonical manifest bytes; no heap allocation or file I/O.
#[derive(Clone, Copy, Debug)]
pub struct Manifest<'a> {
    bytes: &'a [u8],
    domain: Domain,
}
impl<'a> Manifest<'a> {
    /// Check actual bytes, header, caller's domain, count, exact EOF, all entries,
    /// type/schema lengths and cumulative declared payload budgets.
    /// Actual artifact sizes/digests must still be checked by the bundle loader.
    pub fn decode(bytes: &'a [u8], expected_domain: Domain) -> Result<Self, AssetError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(AssetError::BudgetExceeded);
        }
        if bytes.len() < HEADER_LEN {
            return Err(AssetError::Truncated);
        }
        if &bytes[..4] != b"ORAM" {
            return Err(AssetError::InvalidMagic);
        }
        if u32_at(bytes, 4) != MANIFEST_VERSION {
            return Err(AssetError::UnsupportedVersion);
        }
        let domain = Domain::from_raw(u32_at(bytes, 8))?;
        if domain != expected_domain {
            return Err(AssetError::WrongDomain);
        }
        let count = usize::try_from(u32_at(bytes, 12)).map_err(|_| AssetError::BudgetExceeded)?;
        let len = manifest_encoded_len(domain, count)?;
        if bytes.len() < len {
            return Err(AssetError::Truncated);
        }
        if bytes.len() != len {
            return Err(AssetError::LengthMismatch);
        }
        let result = Self { bytes, domain };
        validate_entries(domain, result.entries())?;
        Ok(result)
    }
    /// Exact canonical bytes, suitable as hash input to an external SHA-256 tool.
    pub const fn as_bytes(self) -> &'a [u8] {
        self.bytes
    }
    /// The validated manifest domain.
    pub const fn domain(self) -> Domain {
        self.domain
    }
    /// Decode immutable entries in canonical numeric ID order.
    pub fn entries(self) -> impl ExactSizeIterator<Item = ManifestEntry> + 'a {
        self.bytes[HEADER_LEN..]
            .chunks_exact(ENTRY_LEN)
            .map(ManifestEntry::decode)
    }
    /// Find metadata by GUID (binary search, with no allocation).
    pub fn find(self, id: AssetRef) -> Result<ManifestEntry, AssetError> {
        if id.is_null() {
            return Err(AssetError::InvalidId);
        }
        let mut low = 0;
        let mut high = (self.bytes.len() - HEADER_LEN) / ENTRY_LEN;
        while low < high {
            let mid = low + (high - low) / 2;
            let start = HEADER_LEN + mid * ENTRY_LEN;
            let entry = ManifestEntry::decode(&self.bytes[start..start + ENTRY_LEN]);
            match entry.id.cmp(&id) {
                core::cmp::Ordering::Less => low = mid + 1,
                core::cmp::Ordering::Greater => high = mid,
                core::cmp::Ordering::Equal => return Ok(entry),
            }
        }
        Err(AssetError::MissingAsset)
    }
    /// Resolve raw GUID to a checked simulation type using manifest metadata.
    pub fn typed_ref<T: SimAsset>(self, id: AssetRef) -> Result<TypedRef<T>, AssetError> {
        if self.domain != Domain::Sim {
            return Err(AssetError::WrongDomain);
        }
        TypedRef::from_entry(&self.find(id)?)
    }
}
