//! Per-phase timing of `step`:
//! `cargo run --release -p orr_physics --example profile -- <pile|shaker|field|mixed|mixed-shaker> <n> <warm> <measure>`.
//! The clock lives here, not in the sim crate. Optional env vars for
//! experiments: `SLEEP_LIN`, `SLEEP_ANG`, `SLEEP_TICKS`, `ITERS` override
//! the config; `RAIN=k` drops k bodies onto the scene after the warm-up;
//! `JITTER=1` prints how many bodies exceed some speeds.
//! Host-side tool: it uses `Instant` and floats to report, never in a sim.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
#[path = "../benches/scenes.rs"]
mod scenes;
use orr_physics::{step_probed, Body, Phase, PhysicsState, Scratch, BODY_DYNAMIC};
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let kind = a.get(1).map(String::as_str).unwrap_or("pile");
    let n: u32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let warm: u32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(300);
    let meas: u32 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(100);
    let shaken = kind == "shaker" || kind == "mixed-shaker";
    let mut f = match kind {
        "shaker" => scenes::build_shaker(n),
        "field" => scenes::build_field(n),
        "mixed" => scenes::build_mixed(n),
        "mixed-shaker" => scenes::build_mixed_shaker(n),
        _ => scenes::build(n),
    };
    {
        // Optional overrides for experiments: SLEEP_LIN, SLEEP_ANG (units, rad/s), SLEEP_TICKS, ITERS.
        let cfg = &mut f.singleton_mut::<PhysicsState>().config;
        let num = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f64>().ok());
        if let Some(v) = num("SLEEP_LIN") {
            cfg.sleep_linear_speed = orr_fp::FP::from_raw((v * 65536.0) as i64);
        }
        if let Some(v) = num("SLEEP_ANG") {
            cfg.sleep_angular_speed = orr_fp::FP::from_raw((v * 65536.0) as i64);
        }
        if let Some(v) = num("SLEEP_TICKS") {
            cfg.sleep_ticks = v as u32;
        }
        if let Some(v) = num("ITERS") {
            cfg.velocity_iterations = v as u32;
        }
    }
    let mut sc = Scratch::new();
    let mut ev = Vec::new();
    for _ in 0..warm {
        let t = f.tick() + 1;
        f.set_tick(t);
        if shaken {
            scenes::shake(&mut f, t);
        }
        step_probed(&mut f, &mut sc, &mut ev, &mut |_| {});
    }
    if std::env::var("JITTER").is_ok() {
        // Per-body worst speed over the next 90 ticks (on a copy).
        let mut g = f.clone();
        let mut sc2 = Scratch::new();
        let mut worst = std::collections::BTreeMap::<u32, (f64, f64)>::new();
        for _ in 0..90 {
            let t = g.tick() + 1;
            g.set_tick(t);
            if shaken {
                scenes::shake(&mut g, t);
            }
            step_probed(&mut g, &mut sc2, &mut ev, &mut |_| {});
            for (e, (b,)) in g.query::<(&Body,)>() {
                if b.kind == BODY_DYNAMIC {
                    let v = b.vel.length().raw() as f64 / 65536.0;
                    let w = b.omega.abs().raw() as f64 / 65536.0;
                    let x = worst.entry(e.index).or_insert((0.0, 0.0));
                    x.0 = x.0.max(v);
                    x.1 = x.1.max(w);
                }
            }
        }
        let cnt = |th: f64| worst.values().filter(|x| x.0 > th || x.1 > th).count();
        println!("bodies whose worst |v| or |w| over 90 ticks exceeded 0.05/0.1/0.2/0.5: {} {} {} {}", cnt(0.05), cnt(0.1), cnt(0.2), cnt(0.5));
        println!("stats after: {:?}", sc2.stats());
    }
    {
        let mut h = [0u32; 6];
        for (_, (b,)) in f.query::<(&Body,)>() {
            if b.kind == BODY_DYNAMIC {
                let t = b.sleep & 0x7fff_ffff;
                let k = if b.sleep >> 31 != 0 { 5 } else if t == 0 { 0 } else if t < 10 { 1 } else if t < 20 { 2 } else if t < 30 { 3 } else { 4 };
                h[k] += 1;
            }
        }
        println!("timer hist (0 / 1-9 / 10-19 / 20-29 / 30 awake / asleep): {h:?}");
    }
    if let Some(k) = std::env::var("RAIN").ok().and_then(|v| v.parse::<i32>().ok()) {
        // Drop k circles from just above the ground, spread over the field.
        for i in 0..k {
            let sh = orr_physics::Shape::circle(orr_fp::fp!(0.4));
            let x = orr_fp::FP::from_int((i - k / 2) * 9);
            let body = orr_physics::Body::new_dynamic(orr_fp::FPVec2::new(x, orr_fp::fp!(6)), &sh, orr_fp::FP::ONE);
            orr_physics::spawn_body(&mut f, body, orr_physics::Collider::new(sh));
        }
    }
    let names = ["gather", "xforms", "broad", "narrow", "sensors", "integ", "prepare", "solve", "sleep", "finish"];
    let mut tot = [0f64; 10];
    let mut total = 0f64;
    for _ in 0..meas {
        let t = f.tick() + 1;
        f.set_tick(t);
        if shaken {
            scenes::shake(&mut f, t);
        }
        let t0 = Instant::now();
        let mut last = t0;
        step_probed(&mut f, &mut sc, &mut ev, &mut |p: Phase| {
            let now = Instant::now();
            tot[p as usize] += (now - last).as_secs_f64();
            last = now;
        });
        total += (last - t0).as_secs_f64();
    }
    let ss = sc.stats();
    println!("{ss:?}");
    let st = *f.singleton::<PhysicsState>();
    let contacts = f.list(st.contacts).len();
    let (mut maxv, mut sumv, mut cnt, mut slow) = (0f64, 0f64, 0u32, 0u32);
    let (mut hv, mut hw) = ([0u32; 5], [0u32; 5]);
    for (_, (b,)) in f.query::<(&Body,)>() {
        if b.kind == BODY_DYNAMIC {
            let v = b.vel.length().raw() as f64 / 65536.0;
            maxv = maxv.max(v);
            sumv += v;
            cnt += 1;
            if v < 0.05 {
                slow += 1;
            }
            let w = b.omega.abs().raw() as f64 / 65536.0;
            let bucket = |x: f64| if x < 0.02 { 0 } else if x < 0.05 { 1 } else if x < 0.1 { 2 } else if x < 0.3 { 3 } else { 4 };
            hv[bucket(v)] += 1;
            hw[bucket(w)] += 1;
        }
    }
    println!(
        "{kind} n={n} warm={warm}: {:.0} us/step  contacts={contacts}  meanv={:.2} maxv={:.2} slow={slow}/{cnt}",
        total / meas as f64 * 1e6,
        sumv / cnt as f64,
        maxv
    );
    println!("speed hist <.02/<.05/<.1/<.3/rest: {hv:?}  omega: {hw:?}");
    for (i, nm) in names.iter().enumerate() {
        print!("{nm} {:.0}  ", tot[i] / meas as f64 * 1e6);
    }
    println!();
}
