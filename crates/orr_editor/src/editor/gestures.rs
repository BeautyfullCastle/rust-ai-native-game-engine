//! Connection-owned ERP transactions, advanced only by correlated replies.
//! No optimistic document/cache state. At most eight gestures, 64 unsent
//! distinct fields each, and one transaction request in flight. Coalescing
//! removes the older identical field and appends its latest absolute value:
//! this also preserves ordering relative to overlapping parent/child paths.
//! ERP has no transaction token; observed expiry aborts, but a server timeout
//! racing a later patch cannot be fenced without extending that protocol.

use std::collections::VecDeque;

use super::*;

const MAX_GESTURES: usize = 8;
const MAX_FIELDS: usize = 64;
const REPLY_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Default)]
pub(super) struct Gestures {
    queue: VecDeque<Gesture>,
    input: Input,
    next: u64,
}

#[derive(Default)]
enum Input {
    #[default]
    Idle,
    Active(u64),
    Rejected,
}

struct Gesture {
    id: u64,
    label: String,
    phase: Phase,
    edits: VecDeque<Edit>,
    ended: bool,
    cancelled: bool,
    lost: bool,
    sent_at: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Queued,
    Beginning,
    Ready,
    Patching,
    Finishing,
}

struct Edit {
    what: String,
    method: &'static str,
    params: J,
}

pub(super) enum Reply {
    Begin,
    Patch(String),
    Finish { rollback: bool },
}

impl Gestures {
    pub(super) fn busy(&self) -> bool {
        !self.queue.is_empty()
    }

    pub(super) fn has_input(&self) -> bool {
        !matches!(self.input, Input::Idle)
    }
}

// Synchronous non-gesture mutations must not overtake a queued gesture or
// inadvertently become part of its connection-owned transaction.
pub(super) fn is_read(method: &str) -> bool {
    matches!(method, "rpc.discover" | "registry.types" | "registry.schema" | "registry.input"
        | "world.query" | "world.get" | "world.singleton.get" | "history.list" | "sim.state"
        | "activity.list" | "proposal.list" | "proposal.get" | "proposal.preview")
}

impl Editor {
    /// Begins a nonblocking edit-mode drag. Accepted drags are serialized as
    /// separate undo entries. Repeated Begin during a drag is a no-op. A full
    /// queue visibly refuses the entire new drag until its matching End.
    pub fn begin_edit(&mut self, label: &str) {
        if self.is_viewer() {
            self.error("replay Viewer is read-only");
            return;
        }
        if self.gesture.has_input() || self.sim.mode != Mode::Edit {
            return;
        }
        if self.down.is_some() || self.gesture.queue.len() == MAX_GESTURES {
            self.gesture.input = Input::Rejected;
            self.error("cannot begin drag: disconnected or the edit queue is full");
            return;
        }
        self.gesture.next += 1;
        let id = self.gesture.next;
        self.gesture.input = Input::Active(id);
        self.gesture.queue.push_back(Gesture {
            id, label: label.into(), phase: Phase::Queued, edits: VecDeque::new(),
            ended: false, cancelled: false, lost: false, sent_at: None,
        });
        self.advance_gesture();
    }

    /// Releases the drag without waiting. Commit follows begin and every
    /// acknowledged patch, including when release preceded the begin reply.
    pub fn end_edit(&mut self) {
        if let Input::Active(id) = std::mem::take(&mut self.gesture.input) {
            if let Some(g) = self.gesture.queue.iter_mut().find(|g| g.id == id) {
                g.ended = true;
            }
        }
        self.advance_gesture();
    }

    /// Cancels the current drag, including a begin still awaiting its reply.
    /// Already released gestures retain their commit intent. An in-flight
    /// patch is acknowledged before rollback; no later queued patch is sent.
    pub fn cancel_edit(&mut self) {
        if let Input::Active(id) = self.gesture.input {
            self.gesture.input = Input::Rejected;
            if let Some(g) = self.gesture.queue.iter_mut().find(|g| g.id == id) {
                g.cancelled = true;
                g.ended = true;
                g.edits.clear();
            }
        }
        self.advance_gesture();
    }

    /// True while a drag or its asynchronous completion is pending.
    pub fn in_gesture(&self) -> bool {
        self.gesture.has_input() || self.gesture.busy()
    }

    pub(super) fn queue_gesture_edit(&mut self, what: String, method: &'static str, params: J) -> bool {
        let Input::Active(id) = self.gesture.input else { return false };
        let Some(g) = self.gesture.queue.iter_mut().find(|g| g.id == id) else { return false };
        if g.cancelled || g.lost {
            return false;
        }
        // Only an identical address may coalesce. Reappend to keep ordering
        // correct when a whole value and one of its children are both edited.
        g.edits.retain(|e| !(e.method == method && ["entity", "component", "name", "path"].iter().all(|k| e.params.get(k) == params.get(k))));
        if g.edits.len() == MAX_FIELDS {
            g.cancelled = true;
            g.ended = true;
            g.edits.clear();
            self.gesture.input = Input::Rejected;
            self.error("drag cancelled: too many distinct fields are pending");
            self.advance_gesture();
            return false;
        }
        g.edits.push_back(Edit { what, method, params });
        self.advance_gesture();
        true
    }

    fn advance_gesture(&mut self) {
        if self.down.is_some() {
            return;
        }
        loop {
            let Some(g) = self.gesture.queue.front_mut() else { return };
            if (g.phase == Phase::Queued && g.cancelled) || (g.phase == Phase::Ready && g.lost) {
                self.gesture.queue.pop_front();
                self.mark_edited();
                continue;
            }
            let (method, params, reply) = match g.phase {
                Phase::Queued => {
                    g.phase = Phase::Beginning;
                    ("tx.begin", json!({"label": g.label}), Reply::Begin)
                }
                Phase::Ready if g.cancelled => {
                    g.phase = Phase::Finishing;
                    ("tx.rollback", J::Null, Reply::Finish { rollback: true })
                }
                Phase::Ready if !g.edits.is_empty() => {
                    let e = g.edits.pop_front().expect("nonempty");
                    g.phase = Phase::Patching;
                    (e.method, e.params, Reply::Patch(e.what))
                }
                Phase::Ready if g.ended => {
                    g.phase = Phase::Finishing;
                    ("tx.commit", J::Null, Reply::Finish { rollback: false })
                }
                _ => return,
            };
            let id = g.id;
            g.sent_at = Some(Instant::now());
            self.post(method, params, Pend::Gesture(id, reply));
            return;
        }
    }

    pub(super) fn gesture_answer(&mut self, id: u64, reply: Reply, result: Result<J, orr_remote::RpcError>) {
        let Some(g) = self.gesture.queue.front_mut().filter(|g| g.id == id) else { return };
        g.sent_at = None;
        match reply {
            Reply::Begin => match result {
                Ok(_) => g.phase = Phase::Ready,
                Err(e) => {
                    self.reject_gesture_input(id);
                    self.gesture.queue.pop_front();
                    self.error(format!("begin edit: {}", e.message));
                    self.mark_edited();
                }
            },
            Reply::Patch(what) => {
                g.phase = Phase::Ready;
                if let Err(e) = result {
                    g.cancelled = true;
                    g.ended = true;
                    g.edits.clear();
                    self.reject_gesture_input(id);
                    self.error(format!("{what}: {}; cancelling drag", e.message));
                }
                self.mark_edited();
            }
            Reply::Finish { rollback } => match result {
                Ok(v) => {
                    if let Some(h) = v.get("history") {
                        self.history = History::from_json(h);
                    }
                    self.gesture.queue.pop_front();
                    self.mark_edited();
                }
                Err(e) if !rollback => {
                    g.phase = Phase::Ready;
                    g.cancelled = true;
                    g.edits.clear();
                    self.error(format!("commit edit: {}; attempting rollback", e.message));
                }
                Err(e) => {
                    // Without a confirmed close we cannot safely start another
                    // connection-owned transaction. Reconnection drops this one.
                    self.connection_lost(&format!("rollback was not confirmed: {}", e.message));
                }
            },
        }
        self.advance_gesture();
    }

    fn reject_gesture_input(&mut self, id: u64) {
        if matches!(self.gesture.input, Input::Active(active) if active == id) {
            self.gesture.input = Input::Rejected;
        }
    }

    pub(super) fn observe_gesture_transaction(&mut self, open: bool) {
        let Some(g) = self.gesture.queue.front_mut() else { return };
        // Notifications have no transaction/request ID. Buffered history from
        // other clients must not be attributed to a begin we have not yet
        // acknowledged. Ingest drains older notes before correlated replies.
        if !open && matches!(g.phase, Phase::Ready | Phase::Patching) && !g.lost {
            g.lost = true;
            g.edits.clear();
            let id = g.id;
            self.reject_gesture_input(id);
            self.error("drag cancelled: the host closed its transaction");
            self.advance_gesture();
        }
    }

    pub(super) fn check_gesture_timeout(&mut self) {
        if self.gesture.queue.front().and_then(|g| g.sent_at).is_some_and(|t| t.elapsed() >= REPLY_TIMEOUT) {
            self.connection_lost("edit reply timed out; reconnect before editing again");
        }
    }
}
