//! Byte format helpers for [`Frame::to_bytes`](crate::Frame::to_bytes) /
//! [`Frame::from_bytes`](crate::Frame::from_bytes).
//!
//! All integers are little-endian; every length and count is `u32` (never
//! `usize`, which is 32-bit on wasm32). Component bytes are the `Pod` bytes
//! as they sit in memory, so the format is only defined for little-endian
//! targets (every platform Orrery supports); big-endian fails to compile.
use bytemuck::Pod;

#[cfg(target_endian = "big")]
compile_error!("orr_ecs: the Frame byte format stores Pod bytes as-is and requires a little-endian target");

pub(crate) const MAGIC: [u8; 4] = *b"ORRF";
pub(crate) const FORMAT_VERSION: u32 = 1;

/// Why [`Frame::from_bytes`](crate::Frame::from_bytes) rejected its input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameDecodeError {
    /// The input ended before a complete frame was read.
    Truncated,
    BadMagic,
    UnsupportedVersion(u32),
    /// The stream registers a different number of `kind` types
    /// (`"component"`, `"singleton"` or `"list"`) than the registry.
    SchemaCount { kind: &'static str, expected: u32, found: u32 },
    /// The `index`-th `kind` type differs in name or byte size from the
    /// registry's type at the same registration index.
    SchemaEntry { kind: &'static str, index: u32 },
    /// The bytes parse but describe an impossible frame.
    Corrupt(&'static str),
    /// Extra bytes follow the end of the frame.
    TrailingBytes,
    /// The frame rebuilt from the stream hashes differently from the
    /// checksum stored at the end of the stream.
    ChecksumMismatch { expected: u64, actual: u64 },
}

impl core::fmt::Display for FrameDecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FrameDecodeError::Truncated => write!(f, "frame bytes truncated"),
            FrameDecodeError::BadMagic => write!(f, "not an ORRF frame"),
            FrameDecodeError::UnsupportedVersion(v) => write!(f, "unsupported frame format version {v}"),
            FrameDecodeError::SchemaCount { kind, expected, found } => {
                write!(f, "frame has {found} {kind} types, registry has {expected}")
            }
            FrameDecodeError::SchemaEntry { kind, index } => {
                write!(f, "{kind} type #{index} differs from the registry (name or size)")
            }
            FrameDecodeError::Corrupt(what) => write!(f, "corrupt frame: {what}"),
            FrameDecodeError::TrailingBytes => write!(f, "trailing bytes after frame"),
            FrameDecodeError::ChecksumMismatch { expected, actual } => {
                write!(f, "frame checksum mismatch: stored {expected:#x}, rebuilt {actual:#x}")
            }
        }
    }
}
impl std::error::Error for FrameDecodeError {}

pub(crate) fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
pub(crate) fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
/// Writes a length/count as `u32`. A collection this large cannot exist in a
/// `Frame` (ids and slots are `u32`), so overflow is an invariant violation.
pub(crate) fn put_len(out: &mut Vec<u8>, n: usize) {
    put_u32(out, u32::try_from(n).expect("orr_ecs: length exceeds u32 in frame serialization"));
}
pub(crate) fn put_pods<T: Pod>(out: &mut Vec<u8>, items: &[T]) {
    put_len(out, items.len());
    out.extend_from_slice(bytemuck::cast_slice(items));
}

/// Bounds-checked cursor over the input. Every read fails with
/// [`FrameDecodeError::Truncated`] instead of panicking, and no allocation is
/// sized from a count before the bytes it implies are known to exist.
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], FrameDecodeError> {
        if n > self.remaining() {
            return Err(FrameDecodeError::Truncated);
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, FrameDecodeError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u32(&mut self) -> Result<u32, FrameDecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, FrameDecodeError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    /// Reads `count` values of `T` (unaligned source, aligned result).
    pub(crate) fn pods<T: Pod>(&mut self, count: u32) -> Result<Vec<T>, FrameDecodeError> {
        let n = count as usize;
        let size = core::mem::size_of::<T>();
        let raw = if size == 0 {
            // Zero-sized elements occupy no bytes; bound the count by the
            // input length so a forged count cannot force a huge loop.
            if n > self.remaining() {
                return Err(FrameDecodeError::Truncated);
            }
            &[][..]
        } else {
            self.take(n.checked_mul(size).ok_or(FrameDecodeError::Truncated)?)?
        };
        let mut v = vec![<T as bytemuck::Zeroable>::zeroed(); n];
        bytemuck::cast_slice_mut::<T, u8>(&mut v).copy_from_slice(raw);
        Ok(v)
    }

    /// Reads a `u32` count followed by that many `T`.
    pub(crate) fn counted_pods<T: Pod>(&mut self) -> Result<Vec<T>, FrameDecodeError> {
        let n = self.u32()?;
        self.pods(n)
    }
}
