//! Helpers shared by the byte formats in this crate (`.orrp` replays and
//! late-join messages), which both read untrusted input.

/// Upper bound on how much larger a valid lz4 block can be than its
/// compressed form (one 255 run-length byte can stand for 255 output bytes).
const LZ4_MAX_RATIO: u64 = 255;

/// Decompresses a `lz4_flex::block::compress_prepend_size` buffer without
/// trusting its size prefix: a prefix larger than any valid block of this
/// length could produce is rejected before anything is allocated, so the
/// allocation is bounded by the input length (times [`LZ4_MAX_RATIO`]).
pub(crate) fn decompress_bounded(compressed: &[u8]) -> Result<Vec<u8>, String> {
    let Some((prefix, block)) = compressed.split_first_chunk::<4>() else {
        return Err("missing size prefix".to_string());
    };
    let size = u32::from_le_bytes(*prefix);
    if u64::from(size) > block.len() as u64 * LZ4_MAX_RATIO {
        return Err(format!("declared size {size} impossible for {} compressed bytes", block.len()));
    }
    lz4_flex::block::decompress(block, size as usize).map_err(|e| e.to_string())
}
