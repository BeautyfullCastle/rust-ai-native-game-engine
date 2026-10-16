//! Every bench case at a fixed iteration count produces a pinned checksum. The same test runs on
//! x86_64, wasm32-wasip1 and aarch64 (qemu) in CI: the wasm/narrow fast paths of `orr_fp` and the
//! physics code must give the same bits as the native build. Values were recorded on the code
//! before the wasm optimizations (M6 step 4) wherever the case existed then (the 12 cases);
//! a change here means a simulation or checksum-format change. ORRF v2
//! intentionally rebaselines only the five Frame-derived results to hash
//! complete encoded state; all seven FP-only results remain unchanged.
//! See `docs/frame-compatibility.md` for the migration boundary.

use orr_wasm_bench::{cases, run_fresh};

/// `(case, iterations, checksum)`.
const GOLDEN: &[(&str, u64, u64)] = &[
    ("fp_mul", 5000, 0x4f8ee65bdf0c7f67),
    ("fp_mul_small", 5000, 0x0000012e0269c7ea),
    ("fp_div", 5000, 0x0030cdef59fd4e29),
    ("fp_sqrt", 5000, 0x000000160bbbd95c),
    ("fp_sin_cos", 5000, 0xffffffffe79fd2fe),
    ("fp_atan2", 5000, 0xffffffffffeb3831),
    ("fp_vec3_normalize", 5000, 0xffffffffffe92d88),
    ("ecs_integrate_100k", 3, 0x4c6642b3d4caa6bc),
    ("ecs_checksum_100k", 2, 0xdda4ba9f777d8d94),
    ("ecs_copy_100k", 2, 0xeed25d4fbbbec6ca),
    ("physics2d_1000_tick", 20, 0x12dcd11a181432fa),
    ("physics3d_1000_tick", 10, 0xe50d5b7c80b2e3ff),
];

#[test]
fn golden_bench_checksums() {
    let all = cases();
    let mut bad = Vec::new();
    for &(name, iters, want) in GOLDEN {
        let case = all.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no case {name}"));
        let got = run_fresh(case, iters);
        println!("(\"{name}\", {iters}, {got:#018x}),");
        if got != want {
            bad.push(name);
        }
    }
    assert!(bad.is_empty(), "checksums differ for {bad:?}");
}

#[test]
fn every_case_is_pinned() {
    for c in cases() {
        assert!(GOLDEN.iter().any(|g| g.0 == c.name), "case {} has no golden checksum", c.name);
    }
}
