//! Exact decimal conversion of fixed-point values.

use orr_fp::FP;
use orr_reflect::decimal::{self, NumberError};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 11
    }
}

#[test]
fn known_values_have_the_expected_text() {
    let cases: [(i64, &str); 10] = [
        (0, "0"),
        (65536, "1"),
        (-65536, "-1"),
        (32768, "0.5"),
        (3277, "0.05"),
        (1, "0.00002"),
        (-1, "-0.00002"),
        (65537, "1.00002"),
        (65536 * 100 + 16384, "100.25"),
        (6554, "0.1"),
    ];
    for (raw, text) in cases {
        assert_eq!(decimal::fp_to_decimal(FP::from_raw(raw)), text, "raw {raw}");
        assert_eq!(decimal::parse_fp(text).unwrap().raw(), raw, "text {text}");
    }
}

#[test]
fn every_small_raw_value_round_trips_and_is_shortest() {
    for raw in -140_000i64..=140_000 {
        check(raw);
    }
}

#[test]
fn many_random_raw_values_round_trip() {
    let mut rng = Lcg(0x5eed);
    for _ in 0..300_000 {
        let bits = rng.next() % 63;
        let mut raw = (rng.next() as i64) & ((1i64 << bits.max(1)) - 1);
        if rng.next() & 1 == 1 {
            raw = -raw;
        }
        if raw == i64::MIN {
            continue;
        }
        check(raw);
    }
    for raw in [i64::MAX, i64::MAX - 1, -i64::MAX, 1 << 62, -(1 << 62), (1 << 47) << 16 >> 1] {
        check(raw);
    }
}

fn check(raw: i64) {
    let text = decimal::fp_to_decimal(FP::from_raw(raw));
    let back = decimal::parse_fp(&text).unwrap_or_else(|e| panic!("raw {raw}: text {text:?} does not parse: {e:?}"));
    assert_eq!(back.raw(), raw, "raw {raw} written as {text}");
    // Shortest: no trailing zero after the point, and one digit less does not round trip.
    if let Some((_, frac)) = text.split_once('.') {
        assert!(!frac.ends_with('0'), "{text} has a trailing zero");
        let shorter = decimal::fp_to_decimal(FP::from_raw(raw));
        assert_eq!(shorter, text);
    }
}

#[test]
fn fp32_values_round_trip() {
    for raw in (i32::MIN..=i32::MAX).step_by(9973).chain([i32::MIN, i32::MAX, 0, 1, -1]) {
        let text = decimal::fp32_raw_to_decimal(raw);
        assert_eq!(decimal::parse_fp32_raw(&text).unwrap(), raw, "{text}");
    }
    assert_eq!(decimal::parse_fp32_raw("32768"), Err(NumberError::Overflow));
    assert_eq!(decimal::parse_fp32_raw("-32768").unwrap(), i32::MIN);
}

#[test]
fn parsing_uses_no_floats_and_rounds_to_nearest() {
    // 0.1 is not a multiple of 1/65536: 6553.6 rounds to 6554.
    assert_eq!(decimal::parse_fp("0.1").unwrap().raw(), 6554);
    // Ties round away from zero (same as the fp! macro).
    assert_eq!(decimal::parse_fp("0.000007629394531250").unwrap().raw(), 1);
    assert_eq!(decimal::parse_fp("-0.000007629394531250").unwrap().raw(), -1);
    assert_eq!(orr_fp::fp!(0.1).raw(), decimal::parse_fp("0.1").unwrap().raw());
    assert_eq!(orr_fp::fp!(-123.456).raw(), decimal::parse_fp("-123.456").unwrap().raw());
}

#[test]
fn only_plain_decimals_are_numbers() {
    for bad in ["", "+1", ".5", "5.", "1e3", "1E3", "0x10", "1_000", " 1", "1 ", "00", "01", "-", "--1", "1.2.3", "nan", "inf", "١٢", "1,5"] {
        assert_eq!(decimal::parse_fp(bad), Err(NumberError::NotDecimal), "{bad:?}");
    }
    for good in ["0", "-0", "7", "-7", "0.5", "-0.5", "10.25", "123456789.123456789"] {
        assert!(decimal::parse_fp(good).is_ok(), "{good:?}");
    }
    assert_eq!(decimal::parse_fp("140737488355328"), Err(NumberError::Overflow));
    assert_eq!(decimal::parse_fp("140737488355327").unwrap().raw() >> 16, 140737488355327);
    assert_eq!(decimal::parse_int("12").unwrap(), 12);
    assert_eq!(decimal::parse_int("1.5"), Err(NumberError::NotDecimal));
    assert_eq!(decimal::parse_int("340282366920938463463374607431768211456"), Err(NumberError::Overflow));
}
