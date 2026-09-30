//! Exact, integer-only decimal conversion for fixed-point values.
//!
//! Reading uses `orr_fp`'s own parser (`FP::parse`), so a value in a data
//! file becomes the same bits as the `fp!` literal in code. There is no
//! `f32` or `f64` anywhere on this path. Writing produces the shortest
//! decimal text that reads back to exactly the same raw value.
//!
//! Q48.16 has 16 fractional bits, so every value has an exact decimal
//! expansion with at most 16 digits after the point. The writer tries 0, 1,
//! 2, ... digits and stops at the first text that the parser maps back to
//! the same raw value.

use orr_fp::FP;

/// Longest number text the strict number grammar accepts.
const MAX_NUMBER_LEN: usize = 48;

/// Why a number text was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberError {
    /// The text is not a plain decimal number (`-12`, `0.5`, `3.25`).
    NotDecimal,
    /// The value does not fit the type.
    Overflow,
}

/// True if `s` is `-?(0|[1-9][0-9]*)(\.[0-9]+)?`, at most 48 bytes.
///
/// This is stricter than `FP::parse`, which also accepts spaces, `+`, `.5`
/// and `5.`. Data files use exactly one spelling of a number.
pub fn is_plain_decimal(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > MAX_NUMBER_LEN {
        return false;
    }
    let mut i = 0;
    if b[0] == b'-' {
        i = 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_len = i - int_start;
    if int_len == 0 || (int_len > 1 && b[int_start] == b'0') {
        return false;
    }
    if i == b.len() {
        return true;
    }
    if b[i] != b'.' {
        return false;
    }
    i += 1;
    let frac_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    i == b.len() && i > frac_start
}

/// True if `s` is `-?(0|[1-9][0-9]*)`, at most 48 bytes.
pub fn is_plain_integer(s: &str) -> bool {
    is_plain_decimal(s) && !s.contains('.')
}

/// Parses a plain decimal number into a raw `FP` (Q48.16). Rounds to the
/// nearest `1/65536`, ties away from zero, as `orr_fp` does.
pub fn parse_fp(s: &str) -> Result<FP, NumberError> {
    if !is_plain_decimal(s) {
        return Err(NumberError::NotDecimal);
    }
    match FP::parse(s) {
        Ok(v) => {
            // `i64::MIN` has no decimal spelling that parses, keep the range symmetric.
            if v.raw() == i64::MIN {
                Err(NumberError::Overflow)
            } else {
                Ok(v)
            }
        }
        Err(_) => Err(NumberError::Overflow),
    }
}

/// Parses a plain decimal number into a raw `FP32` value (Q16.16).
pub fn parse_fp32_raw(s: &str) -> Result<i32, NumberError> {
    let v = parse_fp(s)?;
    i32::try_from(v.raw()).map_err(|_| NumberError::Overflow)
}

/// Parses a plain integer.
pub fn parse_int(s: &str) -> Result<i128, NumberError> {
    if !is_plain_integer(s) {
        return Err(NumberError::NotDecimal);
    }
    s.parse::<i128>().map_err(|_| NumberError::Overflow)
}

/// Shortest decimal text that `parse_fp` reads back to `v`.
///
/// Integers print without a point (`3`), others with the fewest digits
/// (`0.05`, `1.0000152587890625`). Negative values start with `-`.
pub fn fp_to_decimal(v: FP) -> String {
    raw_to_decimal(v.raw())
}

/// Same as [`fp_to_decimal`] for a Q16.16 raw value.
pub fn fp32_raw_to_decimal(raw: i32) -> String {
    raw_to_decimal(i64::from(raw))
}

fn raw_to_decimal(raw: i64) -> String {
    let neg = raw < 0;
    let mag: u128 = u128::from(raw.unsigned_abs());
    for digits in 0u32..=16 {
        let pow = 10u128.pow(digits);
        // round(mag * 10^digits / 65536), ties up (the parser rounds ties away from zero).
        let scaled = (mag * pow + 32_768) >> 16;
        let text = format_scaled(neg, scaled, digits, pow);
        if let Ok(back) = FP::parse(&text) {
            if back.raw() == raw {
                return text;
            }
        }
    }
    // Sixteen digits are exact for every value that parses; only `i64::MIN`
    // (which no text parses to) gets here.
    format_scaled(neg, (mag * 10u128.pow(16)) >> 16, 16, 10u128.pow(16))
}

fn format_scaled(neg: bool, scaled: u128, digits: u32, pow: u128) -> String {
    let mut out = String::new();
    if neg && scaled != 0 {
        out.push('-');
    }
    let int = scaled / pow;
    let frac = scaled % pow;
    out.push_str(&int.to_string());
    if digits > 0 {
        out.push('.');
        let f = frac.to_string();
        for _ in f.len()..digits as usize {
            out.push('0');
        }
        out.push_str(&f);
    }
    out
}

/// Text of an integer with the given value.
pub fn int_to_decimal(v: i128) -> String {
    v.to_string()
}
