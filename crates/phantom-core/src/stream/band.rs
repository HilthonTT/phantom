//! The concurrency defaults the `broad_`/`wide_` combinators use.
//!
//! Most callsites do not want to pick a concurrency factor, and the right one
//! is a property of the host and the workload rather than of the callsite. They
//! are held here as live values so a running server can be tuned without a
//! restart.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Stream concurrency factor; this is a live value.
static WIDTH: AtomicUsize = AtomicUsize::new(32);

/// Stream throughput amplifier; this is a live value.
static AMPLIFICATION: AtomicUsize = AtomicUsize::new(1024);

/// Practicable limits on the stream width.
pub const WIDTH_LIMIT: (usize, usize) = (1, 1024);

/// Practicable limits on the stream amplifier.
pub const AMPLIFICATION_LIMIT: (usize, usize) = (32, 32768);

/// Sets the live concurrency factor, returning the width that was replaced and
/// the width actually set after [`WIDTH_LIMIT`] was applied.
pub fn set_width(width: usize) -> (usize, usize) {
    let width = width.clamp(WIDTH_LIMIT.0, WIDTH_LIMIT.1);

    (WIDTH.swap(width, Ordering::Relaxed), width)
}

/// Sets the live concurrency amplification, returning the amplification that
/// was replaced and the one actually set after [`AMPLIFICATION_LIMIT`] was
/// applied.
pub fn set_amplification(amplification: usize) -> (usize, usize) {
    let amplification = amplification.clamp(AMPLIFICATION_LIMIT.0, AMPLIFICATION_LIMIT.1);

    (
        AMPLIFICATION.swap(amplification, Ordering::Relaxed),
        amplification,
    )
}

/// The concurrency factor for stream operations where the caller did not supply
/// one, which is most of them.
#[inline]
#[must_use]
pub fn automatic_width() -> usize {
    let width = WIDTH.load(Ordering::Relaxed);

    debug_assert!(width >= WIDTH_LIMIT.0, "WIDTH should not be zero");
    debug_assert!(width <= WIDTH_LIMIT.1, "WIDTH is probably too large");

    width
}

/// The amplification for stream operations where the caller did not supply one.
#[inline]
#[must_use]
pub fn automatic_amplification() -> usize {
    let amplification = AMPLIFICATION.load(Ordering::Relaxed);

    debug_assert!(
        amplification >= AMPLIFICATION_LIMIT.0,
        "amplification is too low"
    );
    debug_assert!(
        amplification <= AMPLIFICATION_LIMIT.1,
        "amplification is too high"
    );

    amplification
}

/// The caller's concurrency factor, or [`automatic_width`] when they did not
/// supply one.
///
/// Zero is read as "unspecified" rather than passed on: a zero-capacity buffer
/// never polls the futures put into it, so a caller whose width came out of a
/// calculation would hang the stream instead of running it serially.
#[inline]
pub(super) fn width<N: Into<Option<usize>>>(n: N) -> usize {
    match n.into() {
        Some(n) if n > 0 => n,
        _ => automatic_width(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// The values are process-wide, so every test that reads or writes them
    /// takes this lock: the setter test would otherwise race the readers,
    /// which run in parallel with it.
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn an_unspecified_or_zero_width_falls_back_to_the_automatic_one() {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

        assert_eq!(width(None), automatic_width());
        assert_eq!(width(0), automatic_width());
        assert_eq!(width(4), 4);
    }

    #[test]
    fn setters_clamp_and_report_the_prior_value() {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let (previous, set) = set_width(WIDTH_LIMIT.1.saturating_mul(2));
        assert_eq!(set, WIDTH_LIMIT.1, "clamped to the upper limit");
        assert_eq!(automatic_width(), WIDTH_LIMIT.1);

        let (was, set) = set_width(0);
        assert_eq!(was, WIDTH_LIMIT.1, "the prior value is reported");
        assert_eq!(set, WIDTH_LIMIT.0, "clamped to the lower limit");

        set_width(previous);
        assert_eq!(automatic_width(), previous, "restored");

        let (previous, set) = set_amplification(0);
        assert_eq!(set, AMPLIFICATION_LIMIT.0);
        assert_eq!(automatic_amplification(), AMPLIFICATION_LIMIT.0);

        set_amplification(previous);
        assert_eq!(automatic_amplification(), previous, "restored");
    }
}
