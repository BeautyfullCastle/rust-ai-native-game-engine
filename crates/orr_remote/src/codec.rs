//! Tiny hex and base64 codecs (no dependency; used for raw bytes in JSON).

use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Lowercase hex text of `bytes`.
pub fn hex_encode(bytes: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(D[(b >> 4) as usize] as char);
        s.push(D[(b & 15) as usize] as char);
    }
    s
}

/// Bytes of a hex text (upper or lower case). `None` if malformed.
pub fn hex_decode(text: &str) -> Option<Vec<u8>> {
    let b = text.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    b.chunks(2).map(|p| Some(nib(p[0])? << 4 | nib(p[1])?)).collect()
}

/// Standard base64 with padding.
pub fn b64_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        s.push(B64[(n >> 18) as usize & 63] as char);
        s.push(B64[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    s
}

/// Bytes of a standard base64 text (padding required). `None` if malformed.
pub fn b64_decode(text: &str) -> Option<Vec<u8>> {
    match b64_decode_with_checkpoint(text, |_| Ok::<(), Infallible>(())) {
        Ok(decoded) => decoded,
        Err(never) => match never {},
    }
}

#[derive(Debug)]
pub(crate) struct DecodeCancelled;

/// No partial bytes escape on cancellation. Each quartet is cooperative;
/// a single reservation or allocation is not interruptible.
pub(crate) fn b64_decode_cancellable(text: &str, cancel: &AtomicBool) -> Result<Option<Vec<u8>>, DecodeCancelled> {
    b64_decode_with_checkpoint(text, |_| if cancel.load(Ordering::Relaxed) { Err(DecodeCancelled) } else { Ok(()) })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecodeCheckpoint {
    BeforeReserve,
    BeforeQuartet,
    Complete,
}

fn b64_decode_with_checkpoint<E>(text: &str, mut checkpoint: impl FnMut(DecodeCheckpoint) -> Result<(), E>) -> Result<Option<Vec<u8>>, E> {
    let b = text.as_bytes();
    if b.len() % 4 != 0 {
        return Ok(None);
    }
    let val = |c: u8| B64.iter().position(|&x| x == c).map(|p| p as u32);
    checkpoint(DecodeCheckpoint::BeforeReserve)?;
    let mut out = Vec::with_capacity(b.len() / 4 * 3);
    for (i, q) in b.chunks(4).enumerate() {
        checkpoint(DecodeCheckpoint::BeforeQuartet)?;
        let last = (i + 1) * 4 == b.len();
        let pad = q.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return Ok(None);
        }
        let mut n = 0u32;
        for &c in &q[..4 - pad] {
            let Some(value) = val(c) else { return Ok(None) };
            n = n << 6 | value;
        }
        n <<= 6 * pad as u32;
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    checkpoint(DecodeCheckpoint::Complete)?;
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(hex_decode(&hex_encode(&data)).unwrap(), data);
            assert_eq!(b64_decode(&b64_encode(&data)).unwrap(), data);
            assert_eq!(b64_decode_cancellable(&b64_encode(&data), &AtomicBool::new(false)).unwrap(), Some(data));
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
        assert_eq!(b64_encode(b"Ma"), "TWE=");
        assert_eq!(b64_decode("TWE=").unwrap(), b"Ma");
        assert!(b64_decode("TWE").is_none());
        assert!(b64_decode("T=Fu").is_none());
        assert!(hex_decode("zz").is_none());
    }

    #[test]
    fn base64_cancellation_discards_bytes_at_every_checkpoint() {
        for text in ["", "TQ==", "TWE=", "TWFu", "TWFuTWFuTWFu"] {
            assert!(b64_decode_cancellable(text, &AtomicBool::new(true)).is_err());
            let mut checkpoints = Vec::new();
            let expected = b64_decode_with_checkpoint(text, |point| {
                checkpoints.push(point);
                Ok::<(), DecodeCancelled>(())
            }).unwrap();
            assert_eq!(expected, b64_decode(text));
            assert_eq!(checkpoints.first(), Some(&DecodeCheckpoint::BeforeReserve));
            assert_eq!(checkpoints.last(), Some(&DecodeCheckpoint::Complete));
            assert_eq!(checkpoints.len(), text.len() / 4 + 2);

            // Includes before reservation, every quartet (middle and last),
            // and completion after the final quartet or an empty input.
            for stop in 0..checkpoints.len() {
                let cancel = AtomicBool::new(false);
                let mut visited = Vec::new();
                let result = b64_decode_with_checkpoint(text, |point| {
                    visited.push(point);
                    if visited.len() == stop + 1 {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    if cancel.load(Ordering::Relaxed) { Err(DecodeCancelled) } else { Ok(()) }
                });
                assert!(result.is_err(), "checkpoint {stop}: {visited:?}");
                assert_eq!(visited, checkpoints[..=stop], "no later work after cancellation");
            }
        }
    }
}
