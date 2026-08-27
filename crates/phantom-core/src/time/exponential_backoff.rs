use std::{cmp, time::Duration};

/// Returns false if the exponential backoff has expired based on the inputs
#[inline]
#[must_use]
pub fn continue_exponential_backoff_secs(
    min: u64,
    max: u64,
    elapsed: Duration,
    tries: u32,
) -> bool {
    let min = Duration::from_secs(min);
    let max = Duration::from_secs(max);
    continue_exponential_backoff(min, max, elapsed, tries)
}

#[inline]
#[must_use]
pub fn continue_exponential_backoff(
    min: Duration,
    max: Duration,
    elapsed: Duration,
    tries: u32,
) -> bool {
    let min = min.saturating_mul(tries).saturating_mul(tries);
    let min = cmp::min(min, max);
    elapsed < min
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{continue_exponential_backoff, continue_exponential_backoff_secs};

    #[test]
    fn backoff_grows_with_the_try_count() {
        let min = Duration::from_secs(1);
        let max = Duration::from_secs(60);

        // Try 2 waits min * 2^2 = 4s.
        assert!(continue_exponential_backoff(
            min,
            max,
            Duration::from_secs(3),
            2
        ));
        assert!(!continue_exponential_backoff(
            min,
            max,
            Duration::from_secs(4),
            2
        ));

        // Try 3 waits min * 3^2 = 9s.
        assert!(continue_exponential_backoff(
            min,
            max,
            Duration::from_secs(8),
            3
        ));
        assert!(!continue_exponential_backoff(
            min,
            max,
            Duration::from_secs(9),
            3
        ));
    }

    #[test]
    fn backoff_is_clamped_to_max() {
        let min = Duration::from_secs(1);
        let max = Duration::from_secs(10);

        assert!(continue_exponential_backoff(
            min,
            max,
            Duration::from_secs(9),
            100
        ));
        assert!(!continue_exponential_backoff(
            min,
            max,
            Duration::from_secs(10),
            100
        ));
    }

    #[test]
    fn the_secs_wrapper_agrees() {
        assert!(continue_exponential_backoff_secs(
            1,
            60,
            Duration::from_secs(3),
            2
        ));
        assert!(!continue_exponential_backoff_secs(
            1,
            60,
            Duration::from_secs(4),
            2
        ));

        // Nothing has been tried yet, so nothing is being waited out.
        assert!(!continue_exponential_backoff_secs(1, 60, Duration::ZERO, 0));
    }
}
