//! A ready-made host loop for headless use, and the real-time pacer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use orr_edit::{EditorDoc, PlayController};
use orr_session::PlaySession;
use orr_sim::{Game, SimEvent};

use crate::dispatch::ErpTarget;
use crate::server::{ErpServer, PollReport};

/// Turns wall-clock time into ticks for a `PlaySession` that is playing.
/// Speed changes only how often ticks run, never a tick.
#[derive(Debug, Default)]
pub struct Pacer {
    /// Accumulated time, in nanoseconds scaled by 1000 (speed in permille).
    acc: u128,
}

impl Pacer {
    /// The most ticks run back to back after a stall; the rest of the lag is dropped.
    pub const MAX_CATCHUP: u32 = 8;

    /// Runs the ticks due after `elapsed` (only while the session
    /// `wants_tick`). Returns the sim events of those ticks.
    pub fn advance<G: Game>(&mut self, session: &mut PlaySession<G>, elapsed: Duration) -> Vec<SimEvent<G::Event>> {
        if !session.wants_tick() {
            self.acc = 0;
            return Vec::new();
        }
        let interval = 1_000_000_000u128 * 1000 / u128::from(session.tick_rate().max(1));
        self.acc += elapsed.as_nanos() * u128::from(session.speed().permille());
        let mut events = Vec::new();
        let mut ran = 0;
        while self.acc >= interval && ran < Self::MAX_CATCHUP {
            self.acc -= interval;
            events.append(&mut session.tick());
            ran += 1;
            if !session.wants_tick() {
                self.acc = 0;
                break;
            }
        }
        if ran == Self::MAX_CATCHUP {
            self.acc = 0;
        }
        events
    }
}

/// A host: a document, an optional play session, the ERP server and a
/// pacer. It owns everything that simulates or edits. `orr_remote_host` runs
/// one on its main thread; [`LocalHost`](crate::LocalHost) runs one on a
/// thread of a program that has a view (the editor), which then talks to it
/// through an in-process connection.
pub struct Host<G: Game> {
    /// The scene document (shared undo stack of every client).
    pub doc: EditorDoc,
    /// The running play session.
    pub play: Option<PlayController<G>>,
    /// The server.
    pub server: ErpServer,
    pacer: Pacer,
    last: Instant,
}

impl<G: Game> Host<G> {
    /// A host for `doc`, served by `server`.
    pub fn new(doc: EditorDoc, server: ErpServer) -> Self {
        Self { doc, play: None, server, pacer: Pacer::default(), last: Instant::now() }
    }

    /// One host frame: serve requests, run the ticks that are due, publish.
    pub fn frame(&mut self) -> PollReport {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last);
        self.last = now;
        let report = self.server.poll(&mut ErpTarget { doc: &mut self.doc, play: &mut self.play });
        assert!(!report.crash, "debug.panic: a client asked the host to panic");
        if let Some(pc) = self.play.as_mut() {
            let events = self.pacer.advance(pc.session_mut(), elapsed);
            if !events.is_empty() {
                self.server.push_events(&events);
            }
        }
        report
    }

    /// Runs frames until `stop` is set. A frame that had no request waits for
    /// the next one (at most `idle` while a play session runs by the clock,
    /// at most ten times that otherwise), so a request is served at once and
    /// an idle host sleeps. A frame with requests runs the next one at once.
    pub fn run(&mut self, stop: &AtomicBool, idle: Duration) {
        while !stop.load(Ordering::Relaxed) {
            let report = self.frame();
            if report.requests == 0 && !report.more {
                // A relay client has a session to pump every frame (its frames come from the network).
                let ticking = self.server.is_client_mode() || self.play.as_ref().is_some_and(|pc| pc.session().wants_tick());
                // A frame held back by a rate cap goes out a moment later: do not sleep through it.
                let wait = if ticking || report.frame_pending { idle } else { idle * 10 };
                self.server.wait_for_request(wait);
            }
        }
    }
}
