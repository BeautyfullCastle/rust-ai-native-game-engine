use orr_fp::{fp, FP};

#[test]
fn literal_macro_basic() {
    assert_eq!(fp!(1.5), FP::from_raw(98304));
    assert_eq!(fp!(-0.25), FP::from_raw(-16384));
    assert_eq!(fp!(3), FP::from_int(3));
    assert_eq!(fp!(0), FP::ZERO);
    assert_eq!(fp!(1), FP::ONE);
    assert_eq!(fp!(-1), FP::MINUS_ONE);
}

#[test]
fn parse_matches_macro() {
    assert_eq!(FP::parse("1.5").unwrap(), fp!(1.5));
    assert_eq!(FP::parse("-0.25").unwrap(), fp!(-0.25));
    assert_eq!(FP::parse("3").unwrap(), fp!(3));
    assert_eq!(FP::parse("+3").unwrap(), fp!(3));
    assert_eq!(FP::parse(".5").unwrap(), fp!(0.5));
    assert_eq!(FP::parse("-.5").unwrap(), fp!(-0.5));
}

#[test]
fn parse_errors() {
    assert!(FP::parse("").is_err());
    assert!(FP::parse("abc").is_err());
    assert!(FP::parse("1.2.3").is_err());
    assert!(FP::parse("1x").is_err());
    assert!(FP::parse("-").is_err());
}

#[test]
fn parse_rounds_to_nearest_ties_away_from_zero() {
    // 1/3 rounds to nearest 1/65536.
    let v = FP::parse("0.5000076294").unwrap(); // just above 32768.5/65536
    assert_eq!(v.raw(), 32769);
    let v2 = FP::parse("0.5").unwrap();
    assert_eq!(v2.raw(), 32768);
}

#[test]
fn display_round_trip_exact_values() {
    assert_eq!(fp!(1.5).to_string(), "1.5");
    assert_eq!(fp!(-0.25).to_string(), "-0.25");
    assert_eq!(fp!(3).to_string(), "3");
    assert_eq!(FP::ZERO.to_string(), "0");
    assert_eq!(fp!(-1).to_string(), "-1");
    // 1/8 is exactly representable in Q48.16.
    assert_eq!(fp!(0.125).to_string(), "0.125");
}

#[test]
fn display_trims_trailing_zeros() {
    // 0.5 in raw units is exactly representable.
    let v = FP::from_raw(32768);
    assert_eq!(v.to_string(), "0.5");
}

#[test]
fn display_parse_round_trip_fuzz() {
    for raw in [0i64, 1, -1, 65536, -65536, 98304, 12345, -54321, 1000000, -999999] {
        let v = FP::from_raw(raw);
        let s = v.to_string();
        let parsed = FP::parse(&s).unwrap();
        // Display rounds to 5 decimal digits (~ raw/13 granularity), so
        // round-tripping through it should land within a handful of raw
        // units, not necessarily be bit-exact.
        let diff = (parsed.raw() - v.raw()).abs();
        assert!(diff <= 4, "raw={raw} s={s} parsed={parsed:?} diff={diff}");
    }
}

#[test]
fn from_str_trait() {
    let v: FP = "2.5".parse().unwrap();
    assert_eq!(v, fp!(2.5));
}
