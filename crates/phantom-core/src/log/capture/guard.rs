//! Scope guard returned by [`super::Capture::start`].

use std::sync::Arc;

use super::Capture;

/// Stops its capture when dropped.
///
/// Held rather than discarded — `let _ = capture.start()` drops it immediately
/// and captures nothing, which is why [`super::Capture::start`] is
/// `#[must_use]`.
pub struct Guard {
    pub(super) capture: Arc<Capture>,
}

impl Drop for Guard {
    #[inline]
    fn drop(&mut self) {
        self.capture.stop();
    }
}
