//! Checks that the table-driven `sin_cos` equals the CORDIC reference at
//! every input, and regenerates `src/trig_tables.rs`. This file is a test,
//! not simulation code, so plain float arithmetic is fine here.
#![allow(clippy::float_arithmetic)]

use orr_fp::FP;
use std::fmt::Write;

/// Nodes at `i / 1024` rad for `i in 0..=1609` (must match `SEG_SHIFT`).
const NODES: usize = 1610;

/// Nodes at `i / 1024` rad for `i in 0..=1024` of the `atan` table.
const ATAN_NODES: usize = 1025;

fn node_table(f: fn(f64) -> f64) -> Vec<i64> {
    (0..NODES)
        .map(|i| (f(i as f64 / 1024.0) * 4_294_967_296.0).round() as i64)
        .collect()
}

fn write_array(out: &mut String, name: &str, ty: &str, items: &[String], per_line: usize) {
    writeln!(out, "pub(crate) const {name}: [{ty}; {}] = [", items.len()).unwrap();
    for chunk in items.chunks(per_line) {
        writeln!(out, "    {},", chunk.join(", ")).unwrap();
    }
    writeln!(out, "];\n").unwrap();
}

#[test]
fn lut_sin_cos_equals_cordic_on_first_quadrant() {
    for raw in 0..=FP::HALF_PI.raw() {
        let a = FP::from_raw(raw);
        assert_eq!(a.sin_cos(), a.sin_cos_cordic(), "raw {raw}");
    }
}

#[test]
fn lut_sin_cos_equals_cordic_on_wide_range() {
    // Covers all quadrants, negative angles and multi-turn wrapping.
    let lim = FP::TWO_PI.raw() * 4;
    let mut raw = -lim;
    while raw <= lim {
        let a = FP::from_raw(raw);
        assert_eq!(a.sin_cos(), a.sin_cos_cordic(), "raw {raw}");
        raw += 7;
    }
    for a in [FP::MAX, FP::MIN, FP::from_raw(i64::MAX - 12345)] {
        assert_eq!(a.sin_cos(), a.sin_cos_cordic());
    }
}

#[test]
fn lut_atan_equals_cordic_on_reduced_domain() {
    // `atan` reduces every input to `t` in `[0, 1]`; check all of it, both signs.
    for raw in 0..=FP::ONE.raw() {
        let t = FP::from_raw(raw);
        assert_eq!(t.atan(), t.atan_cordic(), "raw {raw}");
        assert_eq!((-t).atan(), (-t).atan_cordic(), "raw -{raw}");
    }
}

#[test]
fn lut_atan_equals_cordic_on_wide_range() {
    // Dense just past 1 (where `1/t` is not exact), then geometric and random.
    for raw in 65_537..4_000_000 {
        let t = FP::from_raw(raw);
        assert_eq!(t.atan(), t.atan_cordic(), "raw {raw}");
        assert_eq!((-t).atan(), (-t).atan_cordic(), "raw -{raw}");
    }
    let mut seed = 0x243f6a8885a308d3_u64;
    for _ in 0..3_000_000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let shift = (seed >> 58) as u32; // 0..64
        let raw = ((seed << 1) as i64) >> shift;
        let t = FP::from_raw(raw);
        assert_eq!(t.atan(), t.atan_cordic(), "raw {raw}");
    }
    for t in [FP::MAX, FP::MIN, FP::from_raw(FP::MIN.raw() + 1), FP::from_raw(1), FP::from_raw(-1)] {
        assert_eq!(t.atan(), t.atan_cordic(), "raw {}", t.raw());
    }
}

#[test]
fn lut_atan2_equals_cordic() {
    let edges = [
        0,
        1,
        -1,
        2,
        65_535,
        65_536,
        65_537,
        -65_536,
        1 << 47,
        (1 << 47) - 1,
        -(1 << 47),
        -(1 << 47) - 1,
        (1 << 47) + 1,
        i64::MAX,
        i64::MIN,
        i64::MIN + 1,
        i64::MAX - 1,
    ];
    for &y in &edges {
        for &x in &edges {
            check_atan2(y, x);
        }
    }
    let mut seed = 0x13198a2e03707344_u64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        // Random magnitude from 0 to 63 bits, random sign.
        let shift = (seed >> 58) as u32;
        ((seed << 1) as i64) >> shift
    };
    for _ in 0..4_000_000 {
        let (y, x) = (next(), next());
        check_atan2(y, x);
    }
    // Small dense grid, including the axes.
    for y in -300..=300 {
        for x in -300..=300 {
            check_atan2(y * 37, x * 41);
        }
    }
}

fn check_atan2(y: i64, x: i64) {
    let (fy, fx) = (FP::from_raw(y), FP::from_raw(x));
    assert_eq!(fy.atan2(fx), fy.atan2_cordic(fx), "y {y} x {x}");
}

/// Regenerates `src/trig_tables.rs`. Run twice after changing the node
/// layout (the first run writes the tables, the second computes the fixes
/// against them):
/// `cargo test -p orr_fp --release --test trig_lut -- --ignored`
#[test]
#[ignore]
fn generate_trig_tables() {
    let sin_tab = node_table(f64::sin);
    let cos_tab = node_table(f64::cos);

    let mut sin_fix: Vec<(u32, i8)> = Vec::new();
    let mut cos_fix: Vec<(u32, i8)> = Vec::new();
    for raw in 0..=FP::HALF_PI.raw() {
        let a = FP::from_raw(raw);
        let (cs, cc) = a.sin_cos_cordic();
        let (ls, lc) = a.sin_cos_lut_unpatched();
        let ds = cs.raw() - ls.raw();
        let dc = cc.raw() - lc.raw();
        if ds != 0 {
            sin_fix.push((raw as u32, ds as i8));
        }
        if dc != 0 {
            cos_fix.push((raw as u32, dc as i8));
        }
    }

    // atan: nodes at `i / 1024` for `t` in `[0, 1]`, plus slope tables.
    let atan_nodes = (0..ATAN_NODES).map(|i| i as f64 / 1024.0);
    let atan_tab: Vec<i64> = atan_nodes
        .clone()
        .map(|a| (a.atan() * 4_294_967_296.0).round() as i64)
        .collect();
    let atan_d1: Vec<u32> = atan_nodes
        .clone()
        .map(|a| (2_147_483_648.0 / (1.0 + a * a)).round() as u32)
        .collect();
    let atan_d2: Vec<u32> = atan_nodes
        .map(|a| (4_294_967_296.0 * a / ((1.0 + a * a) * (1.0 + a * a))).round() as u32)
        .collect();
    let mut atan_fix: Vec<(u32, i8)> = Vec::new();
    for raw in 0..=FP::ONE.raw() {
        let t = FP::from_raw(raw);
        let d = t.atan_cordic().raw() - t.atan_lut_unpatched().raw();
        if d != 0 {
            atan_fix.push((raw as u32, d as i8));
        }
    }

    let fmt_fix = |v: &[(u32, i8)]| -> Vec<String> {
        v.iter().map(|(r, d)| format!("({r}, {d})")).collect()
    };
    let fmt_tab = |v: &[i64]| -> Vec<String> { v.iter().map(|x| x.to_string()).collect() };
    let fmt_u32 = |v: &[u32]| -> Vec<String> { v.iter().map(|x| x.to_string()).collect() };

    let mut out = String::new();
    out.push_str(
        "//! Generated by `generate_trig_tables` in `tests/trig_lut.rs`; do not edit.\n\
         //!\n\
         //! `*_TAB`: `sin`/`cos` at `i / 1024` rad, times `2^32`, rounded to nearest.\n\
         //! `*_FIX`: sorted `(raw angle, delta)` pairs where the interpolated value\n\
         //! differs from the CORDIC result; `delta` is added to the interpolated raw.\n\
         //! `ATAN_*`: nodes at `i / 1024` for `i in 0..=1024`. `ATAN_TAB` is `atan(a)`\n\
         //! times `2^32`, `ATAN_D1` is `1/(1+a^2)` times `2^31`, `ATAN_D2` is\n\
         //! `a/(1+a^2)^2` times `2^32`, all rounded to nearest. `ATAN_FIX` is keyed by\n\
         //! raw `t` in `[0, 2^16]`.\n\n",
    );
    write_array(&mut out, "SIN_TAB", "i64", &fmt_tab(&sin_tab), 6);
    write_array(&mut out, "COS_TAB", "i64", &fmt_tab(&cos_tab), 6);
    write_array(&mut out, "SIN_FIX", "(u32, i8)", &fmt_fix(&sin_fix), 6);
    write_array(&mut out, "COS_FIX", "(u32, i8)", &fmt_fix(&cos_fix), 6);
    write_array(&mut out, "ATAN_TAB", "i64", &fmt_tab(&atan_tab), 6);
    write_array(&mut out, "ATAN_D1", "u32", &fmt_u32(&atan_d1), 8);
    write_array(&mut out, "ATAN_D2", "u32", &fmt_u32(&atan_d2), 8);
    write_array(&mut out, "ATAN_FIX", "(u32, i8)", &fmt_fix(&atan_fix), 6);

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/trig_tables.rs");
    std::fs::write(path, out).unwrap();
    eprintln!(
        "sin fixes: {}, cos fixes: {}, atan fixes: {}",
        sin_fix.len(),
        cos_fix.len(),
        atan_fix.len()
    );
}
