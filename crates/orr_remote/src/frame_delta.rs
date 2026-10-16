//! In-memory prototype for delta encoding serialized ECS frames.
//!
//! This module provides the stateful codec used by the negotiated ORRS v2
//! frame-record path. It splices the ORRF v2 body after normalizing away the
//! encoded tick and trailing checksum, then restores those fields from the
//! target stamp. It keeps at most one complete baseline. The current frame
//! decoder remains the authority for schema and checksum validation after
//! reconstruction.

use std::fmt;
use std::sync::Arc;

use orr_ecs::{ComponentRegistry, Frame, FrameDecodeError};

/// Identifies the stream and timeline in which a frame was produced.
///
/// A new stream generation is required after reconnecting or resetting the
/// producer. The play and timeline epochs fence session and timeline changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameScope {
    pub stream_generation: u64,
    pub play_epoch: u64,
    pub timeline_epoch: u64,
}

/// Identity of a complete serialized frame within its scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameStamp {
    pub scope: FrameScope,
    pub tick: u64,
    pub frame_checksum: u64,
}

/// One complete reset frame or a splice relative to one exact previous frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameRecord {
    /// Complete `Frame::to_bytes()` data. Applying it replaces the baseline.
    Full {
        target: FrameStamp,
        bytes: Arc<[u8]>,
    },
    /// Replace the bytes between a common prefix and suffix of normalized
    /// ORRF v2 bodies (the tick and trailing checksum are restored from target).
    Delta {
        base: FrameStamp,
        target: FrameStamp,
        /// Total target ORRF v2 byte length, including tick and checksum.
        target_len: u64,
        /// Common prefix length in the normalized body.
        prefix_len: u64,
        /// Common suffix length in the normalized body.
        suffix_len: u64,
        /// Replacement bytes in the normalized body.
        inserted: Vec<u8>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Baseline {
    stamp: FrameStamp,
    bytes: Arc<[u8]>,
}

/// Stateful encoder with one bounded full-frame baseline.
pub struct Encoder {
    max_frame_bytes: usize,
    max_baseline_bytes: usize,
    baseline: Option<Baseline>,
}

/// A prepared encoder step. The current baseline is unchanged until the
/// caller admits one candidate and commits this object.
pub struct PreparedEncode {
    expected_base: Option<FrameStamp>,
    baseline: Option<Baseline>,
    full_record: FrameRecord,
    delta_record: Option<FrameRecord>,
    target_stamp: FrameStamp,
}

impl PreparedEncode {
    /// Complete reset candidate for this target frame.
    pub fn full_record(&self) -> &FrameRecord {
        &self.full_record
    }

    /// Compatible splice candidate, if a forward same-scope baseline exists.
    pub fn delta_record(&self) -> Option<&FrameRecord> {
        self.delta_record.as_ref()
    }

    /// Stamp installed if this preparation is committed.
    pub fn target_stamp(&self) -> FrameStamp {
        self.target_stamp
    }
}

impl Encoder {
    /// Creates an encoder with separate frame and retained-baseline limits.
    pub fn new(max_frame_bytes: usize, max_baseline_bytes: usize) -> Self {
        Self {
            max_frame_bytes,
            max_baseline_bytes,
            baseline: None,
        }
    }

    /// Prepares both reset and, when possible, delta candidates without
    /// advancing the retained baseline. A transport can compare complete
    /// compressed messages and commit only after successful queue admission.
    pub fn prepare(&self, frame: &Frame, scope: FrameScope) -> Result<PreparedEncode, EncodeError> {
        let bytes = frame.to_bytes();
        if bytes.len() > self.max_frame_bytes {
            return Err(EncodeError::FrameTooLarge {
                actual: bytes.len(),
                max: self.max_frame_bytes,
            });
        }

        let stamp = FrameStamp {
            scope,
            tick: frame.tick(),
            frame_checksum: frame.checksum(),
        };
        let target: Arc<[u8]> = Arc::from(bytes);
        let normalized_target =
            normalize_orff_v2(&target).ok_or(EncodeError::InvalidOrffV2Frame)?;

        let full_record = FrameRecord::Full {
            target: stamp,
            bytes: Arc::clone(&target),
        };
        let delta_record = match self.baseline.as_ref() {
            Some(base) if base.stamp.scope == stamp.scope && base.stamp.tick < stamp.tick => {
                let normalized_base =
                    normalize_orff_v2(&base.bytes).ok_or(EncodeError::InvalidOrffV2Frame)?;
                let (prefix_len, suffix_len) = common_edges(&normalized_base, &normalized_target);
                let insert_end = normalized_target.len() - suffix_len;
                let inserted = normalized_target[prefix_len..insert_end].to_vec();
                Some(FrameRecord::Delta {
                    base: base.stamp,
                    target: stamp,
                    target_len: usize_as_u64(target.len())?,
                    prefix_len: usize_as_u64(prefix_len)?,
                    suffix_len: usize_as_u64(suffix_len)?,
                    inserted,
                })
            }
            _ => None,
        };

        Ok(PreparedEncode {
            expected_base: self.baseline.as_ref().map(|base| base.stamp),
            baseline: (target.len() <= self.max_baseline_bytes).then_some(Baseline {
                stamp,
                bytes: target,
            }),
            full_record,
            delta_record,
            target_stamp: stamp,
        })
    }

    /// Commits a prepared target baseline if no intervening encoder operation
    /// changed the base it was prepared against. This operation only moves
    /// already-allocated values.
    pub fn commit(&mut self, prepared: PreparedEncode) -> Result<(), EncodeError> {
        let actual_base = self.baseline.as_ref().map(|base| base.stamp);
        if actual_base != prepared.expected_base {
            return Err(EncodeError::PreparedBaseChanged {
                expected: prepared.expected_base,
                actual: actual_base,
            });
        }
        if let Some(baseline) = &prepared.baseline {
            debug_assert_eq!(baseline.stamp, prepared.target_stamp());
        }
        self.baseline = prepared.baseline;
        Ok(())
    }

    /// Encodes a frame using the historical uncompressed-size heuristic.
    /// Negotiated transport callers should use `prepare`, compare complete
    /// compressed candidate messages, and call `commit` after admission.
    pub fn encode(&mut self, frame: &Frame, scope: FrameScope) -> Result<FrameRecord, EncodeError> {
        let prepared = self.prepare(frame, scope)?;
        let record = match prepared.delta_record() {
            Some(delta)
                if estimated_record_size(delta) < estimated_record_size(prepared.full_record()) =>
            {
                delta.clone()
            }
            _ => prepared.full_record().clone(),
        };
        self.commit(prepared)?;
        Ok(record)
    }

    /// Returns the exact stamp of the currently retained baseline, if any.
    pub fn baseline_stamp(&self) -> Option<FrameStamp> {
        self.baseline.as_ref().map(|base| base.stamp)
    }

    /// Returns the number of complete serialized bytes retained as a baseline.
    pub fn retained_baseline_bytes(&self) -> usize {
        self.baseline.as_ref().map_or(0, |base| base.bytes.len())
    }

    /// Drops the current baseline. The next encoded frame is full.
    pub fn reset(&mut self) {
        self.baseline = None;
    }
}

/// Stateful decoder with one bounded full-frame baseline.
pub struct Decoder {
    registry: Arc<ComponentRegistry>,
    max_frame_bytes: usize,
    max_baseline_bytes: usize,
    baseline: Option<Baseline>,
}

/// A fully validated decoded frame and its prospective baseline. No decoder
/// state changes until `Decoder::commit` succeeds.
pub struct PreparedDecode {
    expected_base: Option<FrameStamp>,
    baseline: Option<Baseline>,
    frame: Frame,
    target_stamp: FrameStamp,
}

impl PreparedDecode {
    /// Reconstructed frame available for outer delivery/mailbox validation.
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Stamp to be installed if this preparation is committed.
    pub fn target_stamp(&self) -> FrameStamp {
        self.target_stamp
    }
}

impl Decoder {
    /// Creates a decoder for the registry used by serialized frames.
    pub fn new(
        registry: Arc<ComponentRegistry>,
        max_frame_bytes: usize,
        max_baseline_bytes: usize,
    ) -> Self {
        Self {
            registry,
            max_frame_bytes,
            max_baseline_bytes,
            baseline: None,
        }
    }

    /// Validates and reconstructs a full reset or exact-base delta without
    /// changing the current baseline.
    pub fn prepare_decode(&self, record: &FrameRecord) -> Result<PreparedDecode, DecodeError> {
        let expected_base = self.baseline.as_ref().map(|base| base.stamp);
        match record {
            FrameRecord::Full { target, bytes } => {
                self.check_frame_size(bytes.len())?;
                let frame = self.decode_and_check_stamp(*target, bytes)?;
                Ok(PreparedDecode {
                    expected_base,
                    baseline: (bytes.len() <= self.max_baseline_bytes).then(|| Baseline {
                        stamp: *target,
                        bytes: Arc::clone(bytes),
                    }),
                    frame,
                    target_stamp: *target,
                })
            }
            FrameRecord::Delta {
                base,
                target,
                target_len,
                prefix_len,
                suffix_len,
                inserted,
            } => {
                if base.scope != target.scope {
                    return Err(DecodeError::NeedFull(NeedFullReason::ScopeChanged));
                }
                if target.tick <= base.tick {
                    return Err(DecodeError::NeedFull(NeedFullReason::NonForwardTick {
                        base_tick: base.tick,
                        target_tick: target.tick,
                    }));
                }

                let Some(current) = self.baseline.as_ref() else {
                    return Err(DecodeError::NeedFull(NeedFullReason::MissingBaseline));
                };
                if current.stamp.scope != target.scope {
                    return Err(DecodeError::NeedFull(NeedFullReason::ScopeChanged));
                }
                if current.stamp != *base {
                    return Err(DecodeError::NeedFull(NeedFullReason::StaleBase {
                        expected: *base,
                        got: current.stamp,
                    }));
                }

                let target_len = u64_as_usize(*target_len)?;
                let prefix_len = u64_as_usize(*prefix_len)?;
                let suffix_len = u64_as_usize(*suffix_len)?;
                self.check_frame_size(target_len)?;
                if target_len < ORRF_V2_MIN_BYTES {
                    return Err(DecodeError::InvalidDelta(DeltaError::FrameHeaderTooShort));
                }
                let normalized_target_len = target_len - 16;

                let normalized_base = normalize_orff_v2(&current.bytes)
                    .ok_or(DecodeError::InvalidDelta(DeltaError::FrameHeaderTooShort))?;
                let base_len = normalized_base.len();
                let covered = prefix_len
                    .checked_add(suffix_len)
                    .ok_or(DecodeError::InvalidDelta(DeltaError::LengthOverflow))?;
                if covered > base_len {
                    return Err(DecodeError::InvalidDelta(DeltaError::BaseRange));
                }
                let rebuilt_len = covered
                    .checked_add(inserted.len())
                    .ok_or(DecodeError::InvalidDelta(DeltaError::LengthOverflow))?;
                if rebuilt_len != normalized_target_len {
                    return Err(DecodeError::InvalidDelta(DeltaError::TargetLength));
                }
                if inserted.len() > self.max_frame_bytes {
                    return Err(DecodeError::InvalidDelta(DeltaError::InsertedBytesTooLarge));
                }

                let suffix_start = base_len - suffix_len;
                let mut normalized = Vec::with_capacity(normalized_target_len);
                normalized.extend_from_slice(&normalized_base[..prefix_len]);
                normalized.extend_from_slice(inserted);
                normalized.extend_from_slice(&normalized_base[suffix_start..]);
                let rebuilt = restore_orff_v2_stamp(&normalized, target_len, *target)?;

                let frame = self.decode_and_check_stamp(*target, &rebuilt)?;
                let baseline = (rebuilt.len() <= self.max_baseline_bytes).then(|| Baseline {
                    stamp: *target,
                    bytes: Arc::from(rebuilt),
                });
                Ok(PreparedDecode {
                    expected_base,
                    baseline,
                    frame,
                    target_stamp: *target,
                })
            }
        }
    }

    /// Commits a prepared decode if its captured base is still current. The
    /// returned Frame is moved out of the preparation; no allocation occurs.
    pub fn commit(&mut self, prepared: PreparedDecode) -> Result<Frame, DecodeError> {
        let actual_base = self.baseline.as_ref().map(|base| base.stamp);
        if actual_base != prepared.expected_base {
            return Err(DecodeError::PreparedBaseChanged {
                expected: prepared.expected_base,
                actual: actual_base,
            });
        }
        debug_assert_eq!(prepared.frame().tick(), prepared.target_stamp().tick);
        debug_assert_eq!(
            prepared.frame().checksum(),
            prepared.target_stamp().frame_checksum
        );
        self.baseline = prepared.baseline;
        Ok(prepared.frame)
    }

    /// Decodes and commits in one operation. Failed records never replace or
    /// partially mutate the current baseline.
    pub fn decode(&mut self, record: &FrameRecord) -> Result<Frame, DecodeError> {
        let prepared = self.prepare_decode(record)?;
        self.commit(prepared)
    }

    /// Returns the exact stamp of the currently retained baseline, if any.
    pub fn baseline_stamp(&self) -> Option<FrameStamp> {
        self.baseline.as_ref().map(|base| base.stamp)
    }

    /// Returns the number of complete serialized bytes retained as a baseline.
    pub fn retained_baseline_bytes(&self) -> usize {
        self.baseline.as_ref().map_or(0, |base| base.bytes.len())
    }

    /// Drops the current baseline. The next delta requires an explicit full.
    pub fn reset(&mut self) {
        self.baseline = None;
    }

    fn check_frame_size(&self, actual: usize) -> Result<(), DecodeError> {
        if actual > self.max_frame_bytes {
            return Err(DecodeError::FrameTooLarge {
                actual,
                max: self.max_frame_bytes,
            });
        }
        Ok(())
    }

    fn decode_and_check_stamp(
        &self,
        target: FrameStamp,
        bytes: &[u8],
    ) -> Result<Frame, DecodeError> {
        let frame = Frame::from_bytes(Arc::clone(&self.registry), bytes)
            .map_err(DecodeError::FrameDecode)?;
        if frame.tick() != target.tick {
            return Err(DecodeError::StampMismatch {
                expected: target,
                actual_tick: frame.tick(),
                actual_checksum: frame.checksum(),
            });
        }
        if frame.checksum() != target.frame_checksum {
            return Err(DecodeError::StampMismatch {
                expected: target,
                actual_tick: frame.tick(),
                actual_checksum: frame.checksum(),
            });
        }
        Ok(frame)
    }
}

fn estimated_record_size(record: &FrameRecord) -> usize {
    match record {
        FrameRecord::Full { bytes, .. } => FULL_RECORD_OVERHEAD.saturating_add(bytes.len()),
        FrameRecord::Delta { inserted, .. } => DELTA_RECORD_OVERHEAD.saturating_add(inserted.len()),
    }
}

/// Error while serializing or sizing an encoder input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EncodeError {
    /// The complete serialized frame exceeds the configured limit.
    FrameTooLarge { actual: usize, max: usize },
    /// A platform-sized length could not be represented in the record fields.
    LengthOutOfRange,
    /// `Frame::to_bytes()` did not have the ORRF v2 layout required here.
    InvalidOrffV2Frame,
    /// The encoder base changed after this record was prepared.
    PreparedBaseChanged {
        expected: Option<FrameStamp>,
        actual: Option<FrameStamp>,
    },
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameTooLarge { actual, max } => {
                write!(f, "serialized frame is {actual} bytes; limit is {max}")
            }
            Self::LengthOutOfRange => f.write_str("frame length does not fit the record format"),
            Self::InvalidOrffV2Frame => {
                f.write_str("serialized frame is not a complete ORRF v2 frame")
            }
            Self::PreparedBaseChanged { expected, actual } => write!(
                f,
                "encoder base changed while record was prepared: expected {expected:?}, got {actual:?}"
            ),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Why the decoder needs a new full reset frame before it can continue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NeedFullReason {
    MissingBaseline,
    StaleBase {
        /// Base stamp named by the incoming delta.
        expected: FrameStamp,
        /// Stamp currently retained by the decoder.
        got: FrameStamp,
    },
    ScopeChanged,
    NonForwardTick {
        base_tick: u64,
        target_tick: u64,
    },
}

/// Malformed splice details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeltaError {
    LengthOverflow,
    FrameHeaderTooShort,
    BaseRange,
    TargetLength,
    InsertedBytesTooLarge,
}

/// Error while applying a full or delta record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// A full reset is required before another delta can be applied.
    NeedFull(NeedFullReason),
    /// The uncompressed target exceeds the configured frame limit.
    FrameTooLarge { actual: usize, max: usize },
    /// A delta's lengths or byte ranges are malformed.
    InvalidDelta(DeltaError),
    /// Reconstructed bytes failed the normal ORRF frame decoder.
    FrameDecode(FrameDecodeError),
    /// Frame bytes do not match the target tick/checksum stamp.
    StampMismatch {
        expected: FrameStamp,
        actual_tick: u64,
        actual_checksum: u64,
    },
    /// A u64 record length cannot fit this process's address space.
    LengthOutOfRange,
    /// The decoder base changed after a record was prepared.
    PreparedBaseChanged {
        expected: Option<FrameStamp>,
        actual: Option<FrameStamp>,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedFull(reason) => write!(
                f,
                "delta cannot be applied; full frame required: {reason:?}"
            ),
            Self::FrameTooLarge { actual, max } => {
                write!(f, "frame is {actual} bytes; limit is {max}")
            }
            Self::InvalidDelta(reason) => write!(f, "invalid frame delta: {reason:?}"),
            Self::FrameDecode(reason) => write!(f, "invalid reconstructed frame: {reason}"),
            Self::StampMismatch {
                expected,
                actual_tick,
                actual_checksum,
            } => write!(
                f,
                "frame stamp mismatch: expected tick {} checksum {:016x}, got tick {} checksum {:016x}",
                expected.tick, expected.frame_checksum, actual_tick, actual_checksum
            ),
            Self::LengthOutOfRange => f.write_str("delta length does not fit this process"),
            Self::PreparedBaseChanged { expected, actual } => write!(
                f,
                "decoder base changed while record was prepared: expected {expected:?}, got {actual:?}"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

// The record-size comparison is an explicit uncompressed in-memory estimate,
// not a wire-size or LZ4-performance claim. FrameStamp is five u64 values.
const FULL_RECORD_OVERHEAD: usize = 1 + 5 * 8;
const DELTA_RECORD_OVERHEAD: usize = 1 + 2 * (5 * 8) + 3 * 8;
const ORRF_V2_TICK_OFFSET: usize = 8;
const ORRF_V2_TICK_BYTES: usize = 8;
const ORRF_V2_CHECKSUM_BYTES: usize = 8;
const ORRF_V2_MIN_BYTES: usize = 24;

fn common_edges(base: &[u8], target: &[u8]) -> (usize, usize) {
    let common_limit = base.len().min(target.len());
    let mut prefix = 0;
    while prefix < common_limit && base[prefix] == target[prefix] {
        prefix += 1;
    }

    let suffix_limit = common_limit - prefix;
    let mut suffix = 0;
    while suffix < suffix_limit
        && base[base.len() - suffix - 1] == target[target.len() - suffix - 1]
    {
        suffix += 1;
    }
    (prefix, suffix)
}

/// Removes the volatile ORRF v2 tick and checksum fields before diffing.
fn normalize_orff_v2(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < ORRF_V2_MIN_BYTES
        || &bytes[..4] != b"ORRF"
        || u32::from_le_bytes(bytes[4..8].try_into().ok()?) != 2
    {
        return None;
    }

    let checksum_start = bytes.len().checked_sub(ORRF_V2_CHECKSUM_BYTES)?;
    let body_start = ORRF_V2_TICK_OFFSET.checked_add(ORRF_V2_TICK_BYTES)?;
    if checksum_start < body_start {
        return None;
    }
    let mut normalized =
        Vec::with_capacity(bytes.len() - ORRF_V2_TICK_BYTES - ORRF_V2_CHECKSUM_BYTES);
    normalized.extend_from_slice(&bytes[..ORRF_V2_TICK_OFFSET]);
    normalized.extend_from_slice(&bytes[body_start..checksum_start]);
    Some(normalized)
}

/// Restores the ORRF v2 tick and checksum fields omitted by normalization.
fn restore_orff_v2_stamp(
    normalized: &[u8],
    target_len: usize,
    target: FrameStamp,
) -> Result<Vec<u8>, DecodeError> {
    if target_len < ORRF_V2_MIN_BYTES || normalized.len() != target_len - 16 {
        return Err(DecodeError::InvalidDelta(DeltaError::FrameHeaderTooShort));
    }
    let mut bytes = Vec::with_capacity(target_len);
    bytes.extend_from_slice(&normalized[..ORRF_V2_TICK_OFFSET]);
    bytes.extend_from_slice(&target.tick.to_le_bytes());
    bytes.extend_from_slice(&normalized[ORRF_V2_TICK_OFFSET..]);
    bytes.extend_from_slice(&target.frame_checksum.to_le_bytes());
    Ok(bytes)
}

fn usize_as_u64(value: usize) -> Result<u64, EncodeError> {
    u64::try_from(value).map_err(|_| EncodeError::LengthOutOfRange)
}

fn u64_as_usize(value: u64) -> Result<usize, DecodeError> {
    usize::try_from(value).map_err(|_| DecodeError::LengthOutOfRange)
}
