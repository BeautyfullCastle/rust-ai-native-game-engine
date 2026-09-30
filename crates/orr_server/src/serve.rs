//! The wall-clock driver of a [`RelayServer`]: a steady poll loop that
//! feeds real time to `update`. The server core itself never reads a clock;
//! this module is the one place that does.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};

use orr_proto::Endpoint;

use crate::{InputValidator, RelayServer};

/// Calls `server.update(now_us)` every `poll` (on a fixed schedule, so
/// the period does not drift) until `stop` is set. Server time starts at
/// zero. After every update `hook(server, now_us)` runs, to drain notes and
/// print statistics. Returns the last server time in microseconds.
pub fn run_wall_clock<E: Endpoint, V: InputValidator>(
    server: &mut RelayServer<E, V>,
    stop: &AtomicBool,
    poll: Duration,
    mut hook: impl FnMut(&mut RelayServer<E, V>, u64),
) -> u64 {
    let start = Instant::now();
    let mut next = start;
    let mut now_us = 0;
    while !stop.load(Relaxed) {
        now_us = start.elapsed().as_micros() as u64;
        server.update(now_us);
        hook(server, now_us);
        next += poll;
        let now = Instant::now();
        match next.checked_duration_since(now) {
            Some(wait) => std::thread::sleep(wait),
            // Behind schedule (a long hook or a stalled machine): do not try to catch up ticks of polling.
            None => next = now,
        }
    }
    now_us
}
