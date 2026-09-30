//! The `viewstream` topic's producer: the game's view stream source, held
//! by the host settings (see [`crate::HostLimits::view_stream`]).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use orr_viewstream::StreamProducer;

/// A shared handle to the game's [`StreamProducer`] (the game's extractor and
/// schema). Cloneable, so the settings stay `Clone`.
#[derive(Clone)]
pub struct ViewStreamHook(Arc<Mutex<dyn StreamProducer>>);

impl ViewStreamHook {
    /// Wraps a producer.
    pub fn new(producer: impl StreamProducer + 'static) -> Self {
        Self(Arc::new(Mutex::new(producer)))
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, dyn StreamProducer + 'static> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl core::fmt::Debug for ViewStreamHook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ViewStreamHook")
    }
}
