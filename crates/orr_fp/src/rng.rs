use bytemuck::{Pod, Zeroable};

use crate::fp::FP;

const MULTIPLIER: u64 = 6_364_136_223_846_793_005;

/// A deterministic, integer-only pseudo-random number generator (PCG32:
/// O'Neill's "permuted congruential generator", 64-bit state / 32-bit
/// output) for simulation use.
///
/// PCG32 is a simple linear-congruential core followed by a fixed
/// bit-permutation output function; every step is plain `u64`
/// multiply/add/shift/xor/rotate, so a given `(seed, stream)` produces the
/// exact same sequence on every platform. It is *not* cryptographically
/// secure — it's chosen for speed, small state (16 bytes, `Pod`) and
/// excellent statistical quality for gameplay use, not for anything
/// security-sensitive.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Pod, Zeroable, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FrameRng {
    state: u64,
    inc: u64,
}

impl FrameRng {
    /// Create a new generator from a 64-bit seed, on stream `0`.
    #[must_use]
    pub fn new(seed: u64) -> FrameRng {
        FrameRng::with_stream(seed, 0)
    }

    /// Create a new generator from a 64-bit seed on a specific stream id.
    /// Two generators with the same seed but different stream ids produce
    /// different, statistically independent sequences.
    #[must_use]
    pub fn with_stream(seed: u64, stream_id: u64) -> FrameRng {
        let mut rng = FrameRng { state: 0, inc: (stream_id << 1) | 1 };
        let _ = rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        let _ = rng.next_u32();
        rng
    }

    /// Next 32 random bits.
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(MULTIPLIER).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Next 64 random bits (two `next_u32` calls combined).
    pub fn next_u64(&mut self) -> u64 {
        let hi = self.next_u32() as u64;
        let lo = self.next_u32() as u64;
        (hi << 32) | lo
    }

    /// A uniformly-distributed `i32` in the half-open range `[lo, hi)`.
    /// Uses a modulo reduction (a small, well-documented bias for spans
    /// that don't evenly divide `2^32` is accepted here in exchange for
    /// simplicity/speed; use a wider span if that bias matters for your
    /// use case). Panics if `hi <= lo`.
    pub fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        assert!(hi > lo, "FrameRng::range_i32: empty or inverted range");
        let span = (hi as i64 - lo as i64) as u64;
        lo + (self.next_u32() as u64 % span) as i32
    }

    /// A uniformly-distributed [`FP`] in `[0, 1)`, at the generator's
    /// native `1/65536` resolution (the top 16 bits of a `next_u32` call
    /// become the fixed-point fractional bits directly).
    pub fn next_fp01(&mut self) -> FP {
        FP::from_raw((self.next_u32() >> 16) as i64)
    }

    /// A uniformly-distributed [`FP`] in `[lo, hi)`.
    pub fn range_fp(&mut self, lo: FP, hi: FP) -> FP {
        lo + self.next_fp01() * (hi - lo)
    }

    /// Derive a new, independent generator stream, seeded deterministically
    /// from this generator's current state plus `stream_id`. Useful for
    /// giving each subsystem (e.g. "enemy spawns", "loot rolls") its own
    /// RNG stream without them perturbing each other's sequences, while
    /// staying fully deterministic given the parent seed.
    pub fn fork(&mut self, stream_id: u64) -> FrameRng {
        let seed = self.next_u64();
        FrameRng::with_stream(seed, stream_id)
    }
}
