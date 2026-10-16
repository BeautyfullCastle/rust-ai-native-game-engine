use crate::{AssetError, ManifestEntry, SimAsset};
use bytemuck::{Pod, Zeroable};
use core::{fmt, marker::PhantomData, str::FromStr};

/// Stable GUID wire value. Zero is null/unset; every nonzero `u64` is a valid ID.
///
/// POD bytes are native-endian memory, not a portable encoding. Use little-endian
/// bytes on the wire. Authoring spelling is exactly `a_` plus 16 lowercase hex
/// digits; never transport the GUID through a floating-point JSON number.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Pod, Zeroable)]
pub struct AssetRef(u64);
impl AssetRef {
    /// Null/unset value, rejected by required references and live records.
    pub const NULL: Self = Self(0);
    /// Preserve a raw POD value, including null. This does not resolve/type it.
    pub const fn from_raw(id: u64) -> Self {
        Self(id)
    }
    /// The complete numeric GUID, with no truncation or index conversion.
    pub const fn get(self) -> u64 {
        self.0
    }
    /// Whether this value is null/unset.
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}
impl fmt::Display for AssetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a_{:016x}", self.0)
    }
}
impl FromStr for AssetRef {
    type Err = AssetError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        if bytes.len() != 18 || &bytes[..2] != b"a_" {
            return Err(AssetError::InvalidId);
        }
        let mut id = 0u64;
        for &byte in &bytes[2..] {
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                _ => return Err(AssetError::InvalidId),
            };
            id = (id << 4) | u64::from(digit);
        }
        Ok(Self(id))
    }
}

/// A nonnull reference whose manifest metadata matches `T`.
///
/// No public unchecked raw-to-typed conversion exists. This is structural type
/// validation, not proof of payload integrity or membership in a specific table.
///
/// Raw GUIDs cannot bypass metadata validation:
/// ```compile_fail
/// use orr_asset::{AssetRef, MotionProfileV1, TypedRef};
/// let typed: TypedRef<MotionProfileV1> = AssetRef::from_raw(1).into();
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedRef<T> {
    raw: AssetRef,
    marker: PhantomData<fn() -> T>,
}
impl<T: SimAsset> TypedRef<T> {
    /// Validate type/schema/domain/length/ID metadata before attaching a type.
    /// The entry may be supplied by a cooker; digests remain unverified here.
    pub fn from_entry(entry: &ManifestEntry) -> Result<Self, AssetError> {
        entry.validate(crate::Domain::Sim)?;
        if entry.type_id != T::TYPE_ID {
            return Err(AssetError::WrongType);
        }
        if entry.schema_version != T::SCHEMA_VERSION {
            return Err(AssetError::UnsupportedVersion);
        }
        Ok(Self {
            raw: entry.id,
            marker: PhantomData,
        })
    }
    /// Return the stable untyped POD GUID for storage in a Frame.
    pub const fn raw(self) -> AssetRef {
        self.raw
    }
}
