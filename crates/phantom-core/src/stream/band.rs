use std::sync::atomic::{AtomicUsize, Ordering};

static WIDTH: AtomicUsize = AtomicUsize::new(32);

static AMPLIFICATION: AtomicUsize = AtomicUsize::new(1024);

pub const WIDTH_LIMIT: (usize, usize) = (1, 1024);

pub const AMPLIFICATION_LIMIT: (usize, usize) = (32, 32768);

pub fn set_width(width: usize) -> (usize, usize) {
    let width = width.clamp(WIDTH_LIMIT.0, WIDTH_LIMIT.1);

    (WIDTH.swap(width, Ordering::Relaxed), width)
}

pub fn set_amplification(amplification: usize) -> (usize, usize) {
    let amplification = amplification.clamp(AMPLIFICATION_LIMIT.0, AMPLIFICATION_LIMIT.1);

    (
        AMPLIFICATION.swap(amplification, Ordering::Relaxed),
        amplification,
    )
}

#[inline]
#[must_use]
pub fn automatic_width() -> usize {
    let width = WIDTH.load(Ordering::Relaxed);

    debug_assert!(width >= WIDTH_LIMIT.0, "WIDTH should not be zero");
    debug_assert!(width <= WIDTH_LIMIT.1, "WIDTH is probably too large");

    width
}

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
