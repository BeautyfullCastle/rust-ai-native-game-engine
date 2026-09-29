use std::sync::Arc;

use orr_ecs::Frame;
use orr_session::RollbackInfo;

use crate::event::BridgeStats;
use crate::frame_view::FrameView;

pub(crate) struct SnapshotData {
    pub(crate) seq: u64,
    pub(crate) tick: u64,
    pub(crate) verified_tick: u64,
    pub(crate) tick_rate: u32,
    pub(crate) predicted: Arc<Frame>,
    pub(crate) predicted_prev: Option<Arc<Frame>>,
    pub(crate) verified: Option<Arc<Frame>>,
    pub(crate) stats: BridgeStats,
    pub(crate) last_rollback: Option<RollbackInfo>,
}

/// An immutable copy of what the view needs, published by the sim side
/// whenever the predicted head, the verified tick or history (rollback)
/// changes (design doc 5.2, "FrameView").
///
/// A snapshot never changes after it was published, and cloning it is a
/// pointer copy. It offers three frames:
///
/// - [`predicted`](Self::predicted): the newest simulated tick (`tick()`),
///   which may still be corrected by a rollback.
/// - [`predicted_prev`](Self::predicted_prev): the tick before it, as the
///   sim believes it now (after any rollback). The view interpolates
///   between the two.
/// - [`verified`](Self::verified): the newest fully confirmed tick.
///   It never changes afterwards; remote entities can be shown from it.
#[derive(Clone)]
pub struct Snapshot(pub(crate) Arc<SnapshotData>);

impl Snapshot {
    /// Increases by one for every publish. Equal `seq` means the same snapshot.
    pub fn seq(&self) -> u64 {
        self.0.seq
    }

    /// The predicted head tick.
    pub fn tick(&self) -> u64 {
        self.0.tick
    }

    pub fn verified_tick(&self) -> u64 {
        self.0.verified_tick
    }

    pub fn tick_rate(&self) -> u32 {
        self.0.tick_rate
    }

    pub fn predicted(&self) -> FrameView<'_> {
        FrameView::new(&self.0.predicted)
    }

    /// The frame of `tick() - 1`, or `None` at tick 0.
    pub fn predicted_prev(&self) -> Option<FrameView<'_>> {
        self.0.predicted_prev.as_deref().map(FrameView::new)
    }

    pub fn verified(&self) -> Option<FrameView<'_>> {
        self.0.verified.as_deref().map(FrameView::new)
    }

    pub fn stats(&self) -> BridgeStats {
        self.0.stats
    }

    /// The latest rollback (of the whole session, not of this publish).
    pub fn last_rollback(&self) -> Option<RollbackInfo> {
        self.0.last_rollback
    }
}
