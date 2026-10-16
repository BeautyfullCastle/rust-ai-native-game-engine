//! Lossless build-id parsing at the JavaScript boundary. Large u64 values
//! must arrive as text: a JavaScript Number may already have rounded them.

pub(crate) fn parse_build_id(text: &str) -> Result<u64, &'static str> {
    let text = text.trim();
    let (digits, radix) = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .map_or((text, 10), |s| (s, 16));
    if digits.is_empty()
        || !digits.bytes().all(|b| {
            if radix == 16 {
                b.is_ascii_hexdigit()
            } else {
                b.is_ascii_digit()
            }
        })
    {
        return Err("build_id must be an unsigned decimal or 0x-prefixed hexadecimal u64 string");
    }
    u64::from_str_radix(digits, radix).map_err(|_| "build_id is outside the u64 range")
}

pub(crate) fn numeric_build_id(value: f64) -> Result<u64, &'static str> {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    if !(0.0..=MAX_SAFE_INTEGER).contains(&value) || value as u64 as f64 != value {
        return Err("numeric build_id must be a nonnegative safe integer; use a decimal or 0x-prefixed string for a full u64");
    }
    Ok(value as u64)
}

#[cfg(test)]
mod tests {
    use super::{numeric_build_id, parse_build_id};

    #[test]
    fn decimal_and_hex_strings_preserve_all_u64_bits() {
        for id in [
            0,
            1,
            (1 << 53) - 1,
            1 << 53,
            (1 << 53) + 1,
            crate::ARENA_BUILD_ID,
            crate::PHYSICS_BUILD_ID,
            u64::MAX,
        ] {
            assert_eq!(parse_build_id(&id.to_string()), Ok(id));
            assert_eq!(parse_build_id(&format!("0x{id:x}")), Ok(id));
            assert_eq!(parse_build_id(&format!(" 0X{id:X} ")), Ok(id));
        }
    }

    #[test]
    fn malformed_or_overflowing_strings_are_rejected() {
        for bad in [
            "",
            " ",
            "0x",
            "-1",
            "+1",
            "0x-1",
            "0x+1",
            "1.5",
            "1e3",
            "0xGG",
            "NaN",
            "18446744073709551616",
            "0x10000000000000000",
        ] {
            assert!(parse_build_id(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn numeric_compatibility_requires_a_safe_nonnegative_integer() {
        for (number, id) in [
            (0.0, 0),
            (1.0, 1),
            (42.0, 42),
            (9_007_199_254_740_991.0, (1 << 53) - 1),
        ] {
            assert_eq!(numeric_build_id(number), Ok(id));
        }
        for bad in [
            -1.0,
            0.5,
            1.5,
            9_007_199_254_740_992.0,
            u64::MAX as f64,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert!(numeric_build_id(bad).is_err(), "accepted {bad:?}");
        }
        // Both new built-in IDs need the string path if explicitly supplied.
        assert!(numeric_build_id(crate::ARENA_BUILD_ID as f64).is_err());
        assert!(numeric_build_id(crate::PHYSICS_BUILD_ID as f64).is_err());
    }
}
