//! Integer-only decimal parsing and display for [`FP`].
//!
//! Both the [`fp!`](crate::fp) macro and [`FP::parse`] go through the same
//! `const fn` byte-level parser below, so compile-time literals and
//! runtime-parsed data-file values are parsed identically and
//! deterministically (no float parsing anywhere).

use core::fmt;
use core::str::FromStr;

use crate::fp::{ParseError, FP, SCALE};

/// Parse a decimal string (`[+-]? digits? ('.' digits?)?`, whitespace
/// tolerated between tokens) into a raw `FP`, rounding the fractional part
/// to the nearest `1/65536`, ties away from zero.
const fn parse_bytes(s: &[u8]) -> Result<FP, ParseError> {
    let n = s.len();
    let mut i = 0usize;

    while i < n && s[i] == b' ' {
        i += 1;
    }
    if i >= n {
        return Err(ParseError::Empty);
    }

    let mut neg = false;
    if s[i] == b'+' {
        i += 1;
    } else if s[i] == b'-' {
        neg = true;
        i += 1;
    }
    while i < n && s[i] == b' ' {
        i += 1;
    }

    let mut int_part: i64 = 0;
    let mut any_digit = false;
    while i < n {
        let c = s[i];
        if c == b' ' {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() {
            any_digit = true;
            let d = (c - b'0') as i64;
            let m = match int_part.checked_mul(10) {
                Some(v) => v,
                None => return Err(ParseError::Overflow),
            };
            let a = match m.checked_add(d) {
                Some(v) => v,
                None => return Err(ParseError::Overflow),
            };
            int_part = a;
            i += 1;
        } else {
            break;
        }
    }

    let mut frac_num: u64 = 0;
    let mut frac_den: u64 = 1;
    if i < n && s[i] == b'.' {
        i += 1;
        while i < n {
            let c = s[i];
            if c == b' ' {
                i += 1;
                continue;
            }
            if c.is_ascii_digit() {
                any_digit = true;
                // Cap accumulated digits so frac_den can't overflow u64;
                // extra digits beyond this are below FP's precision anyway.
                if frac_den <= 1_000_000_000_000_000_000u64 {
                    frac_num = frac_num * 10 + (c - b'0') as u64;
                    frac_den *= 10;
                }
                i += 1;
            } else {
                break;
            }
        }
    }

    while i < n && s[i] == b' ' {
        i += 1;
    }
    if i < n {
        if s[i] == b'.' {
            return Err(ParseError::TooManyDots);
        }
        return Err(ParseError::InvalidChar);
    }
    if !any_digit {
        return Err(ParseError::Empty);
    }

    let frac_raw: i64 = if frac_den == 1 {
        0
    } else {
        let numerator = (frac_num as u128) * (SCALE as u128);
        let denom = frac_den as u128;
        ((numerator + denom / 2) / denom) as i64
    };

    let scaled_int = match int_part.checked_mul(SCALE) {
        Some(v) => v,
        None => return Err(ParseError::Overflow),
    };
    let magnitude = match scaled_int.checked_add(frac_raw) {
        Some(v) => v,
        None => return Err(ParseError::Overflow),
    };
    let raw = if neg { -magnitude } else { magnitude };
    Ok(FP(raw))
}

impl FP {
    /// Parse a decimal literal's source text at compile time (or runtime;
    /// it is a plain `const fn`). Panics on invalid input — used by the
    /// [`fp!`](crate::fp) macro, which binds the result to a `const` so
    /// the panic (and the parse) happen at compile time.
    #[must_use]
    pub const fn from_decimal_str(s: &str) -> FP {
        match parse_bytes(s.as_bytes()) {
            Ok(v) => v,
            Err(_) => panic!("invalid FP decimal literal"),
        }
    }

    /// Parse a decimal string at runtime, e.g. while loading a data file.
    /// Uses the exact same integer-only routine as the `fp!` macro, so
    /// parsing is deterministic across platforms.
    pub fn parse(s: &str) -> Result<FP, ParseError> {
        parse_bytes(s.as_bytes())
    }
}

impl FromStr for FP {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<FP, ParseError> {
        FP::parse(s)
    }
}

impl fmt::Display for FP {
    /// Prints a decimal approximation of the value, at most 5 fractional
    /// digits, trailing zeros trimmed, using only integer arithmetic. This
    /// is a display convenience (not guaranteed to round-trip losslessly
    /// through [`FP::parse`] for values whose true decimal expansion needs
    /// more than 5 fractional digits).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let raw = self.0;
        let neg = raw < 0;
        let mag: u128 = if raw == i64::MIN {
            (i64::MAX as u128) + 1
        } else {
            raw.unsigned_abs() as u128
        };

        let mut int_part = mag >> 16;
        let frac_raw = mag & ((SCALE as u128) - 1);
        // round(frac_raw * 100000 / 65536), ties away from zero (frac_raw >= 0)
        let mut frac5 = (frac_raw * 100_000 + 32_768) / (SCALE as u128);
        if frac5 >= 100_000 {
            frac5 -= 100_000;
            int_part += 1;
        }

        if neg {
            write!(f, "-")?;
        }
        write!(f, "{int_part}")?;

        if frac5 != 0 {
            let mut digits = [0u8; 5];
            let mut t = frac5;
            for i in (0..5).rev() {
                digits[i] = (t % 10) as u8;
                t /= 10;
            }
            let mut last_nonzero: i32 = -1;
            for (i, d) in digits.iter().enumerate() {
                if *d != 0 {
                    last_nonzero = i as i32;
                }
            }
            write!(f, ".")?;
            for i in 0..=(last_nonzero as usize) {
                write!(f, "{}", digits[i])?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for FP {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}
