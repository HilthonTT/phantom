use std::{cmp, time::Duration};

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

#[inline]
#[must_use]
pub fn exponential_backoff_streak_cap(window: Duration, max: Duration) -> u32 {
    let window = window.as_secs();
    let max = max.as_secs();

    if window == 0 {
        return 1;
    }

    let ratio = max.div_ceil(window);

    u32::try_from(ratio.isqrt().saturating_add(1)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        continue_exponential_backoff, continue_exponential_backoff_secs,
        exponential_backoff_streak_cap,
    };

    #[test]
    fn backoff_grows_with_the_try_count() {
        let min = Duration::from_secs(1);
        let max = Duration::from_secs(60);

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

        assert!(!continue_exponential_backoff_secs(1, 60, Duration::ZERO, 0));
    }

    #[test]
    fn the_streak_cap_is_where_the_curve_saturates() {
        let window = Duration::from_secs(180);
        let max = Duration::from_secs(86400);

        let cap = exponential_backoff_streak_cap(window, max);
        assert_eq!(cap, 22);

        assert!(continue_exponential_backoff(
            window,
            max,
            max - Duration::from_secs(1),
            cap
        ));
        assert!(!continue_exponential_backoff(window, max, max, cap));
    }

    #[test]
    fn a_streak_past_the_cap_changes_nothing() {
        let window = Duration::from_secs(180);
        let max = Duration::from_secs(86400);
        let cap = exponential_backoff_streak_cap(window, max);

        for elapsed in [0, 1000, 86399, 86400, 90000] {
            let elapsed = Duration::from_secs(elapsed);

            assert_eq!(
                continue_exponential_backoff(window, max, elapsed, cap),
                continue_exponential_backoff(window, max, elapsed, cap * 10),
            );
        }
    }

    #[test]
    fn a_zero_window_saturates_immediately() {
        assert_eq!(
            exponential_backoff_streak_cap(Duration::ZERO, Duration::from_secs(86400)),
            1
        );
    }
}
