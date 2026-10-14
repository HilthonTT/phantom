//! One token bucket per key, for every service that rate-limits a caller.
//!
//! Four services each carried their own copy of this arithmetic, and three had
//! drifted from the most careful one: `oauth` swept expired buckets but never
//! evicted when the sweep freed nothing, so a table full of *fresh* buckets
//! stopped admitting new clients altogether, and `media` bounded its table not
//! at all. Holding the refill, the cap and the eviction in one place is what
//! keeps them from drifting apart again.

use std::{
    borrow::Borrow,
    collections::HashMap,
    hash::Hash,
    sync::Mutex,
    time::{Duration, Instant},
};

use http::StatusCode;
use phantom_core::{Error, Result};
use ruma::api::error::{ErrorKind, LimitExceededErrorData, RetryAfter};

/// Each key's last debit and the allowance remaining to it.
pub(crate) type Ratelimiter<K> = Mutex<HashMap<K, (Instant, f64)>>;

/// Ceiling on how many buckets a table retains before it evicts.
///
/// A bucket is two `f64`s and an `Instant` behind a hash key, so 65536 of them
/// is a bounded cost; the point of the cap is that an attacker cycling keys
/// cannot grow the table without bound.
pub(crate) const DEFAULT_MAP_CAP: usize = 1 << 16;

/// A bucket's refill rate, its ceiling, and what to say once it is empty.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limit {
    /// Tokens restored per second.
    pub(crate) rate: f64,

    /// Ceiling on accumulated tokens, and the allowance a new bucket opens with.
    pub(crate) burst: f64,

    /// Message carried by the `M_LIMIT_EXCEEDED` raised once a bucket is empty.
    pub(crate) message: &'static str,
}

impl Limit {
    /// Whether this limit is configured to admit everything.
    ///
    /// A non-positive rate or burst means the operator turned the limit off,
    /// which callers check before touching the table at all.
    pub(crate) fn is_disabled(self) -> bool {
        self.rate <= 0.0 || self.burst <= 0.0
    }
}

/// Debit one token from `key`'s bucket, failing when it has none left.
///
/// `make_key` builds the owned key, and is only called when the bucket is
/// missing, so the common path of an existing bucket allocates nothing.
pub(crate) fn check<K, Q>(
    table: &Ratelimiter<K>,
    key: &Q,
    make_key: impl FnOnce() -> K,
    limit: Limit,
) -> Result
where
    K: Borrow<Q> + Clone + Eq + Hash,
    Q: Eq + Hash + ?Sized,
{
    check_at(table, key, make_key, limit, Instant::now(), DEFAULT_MAP_CAP)
}

/// [`check`], with the clock and the table cap supplied by the caller.
///
/// Tests drive time forward explicitly rather than sleeping, and a service
/// whose table wants a tighter bound than [`DEFAULT_MAP_CAP`] passes its own.
pub(crate) fn check_at<K, Q>(
    table: &Ratelimiter<K>,
    key: &Q,
    make_key: impl FnOnce() -> K,
    limit: Limit,
    now: Instant,
    cap: usize,
) -> Result
where
    K: Borrow<Q> + Clone + Eq + Hash,
    Q: Eq + Hash + ?Sized,
{
    let mut buckets = table.lock()?;
    debug_assert!(cap > 0, "rate-limit table cap must be positive");
    debug_assert!(buckets.len() <= cap, "rate-limit table exceeded its cap");

    if let Some(bucket) = buckets.get_mut(key) {
        return debit(bucket, limit, now);
    }

    if buckets.len() >= cap {
        evict(&mut buckets, limit, now, cap);
    }

    let bucket = buckets
        .entry(make_key())
        .or_insert_with(|| (now, limit.burst));

    debit(bucket, limit, now)
}

/// Whether `key`'s bucket holds a token, without spending it.
///
/// For a limit charged only on failure: the check comes before the attempt,
/// and [`check`] debits once the attempt is known to have failed. A key with
/// no bucket has its whole burst, so nothing is inserted.
pub(crate) fn peek<K, Q>(table: &Ratelimiter<K>, key: &Q, limit: Limit) -> Result
where
    K: Borrow<Q> + Eq + Hash,
    Q: Eq + Hash + ?Sized,
{
    peek_at(table, key, limit, Instant::now())
}

/// [`peek`], with the clock supplied by the caller.
pub(crate) fn peek_at<K, Q>(table: &Ratelimiter<K>, key: &Q, limit: Limit, now: Instant) -> Result
where
    K: Borrow<Q> + Eq + Hash,
    Q: Eq + Hash + ?Sized,
{
    let buckets = table.lock()?;

    match buckets.get(key) {
        Some(&(last, tokens)) => match refill(last, tokens, limit, now) {
            tokens if tokens < 1.0 => Err(exceeded(limit, tokens)),
            _ => Ok(()),
        },
        None => Ok(()),
    }
}

/// Make room in a full table, first by sweeping, then by dropping the oldest.
///
/// The sweep drops buckets that have refilled to their ceiling, since those are
/// indistinguishable from a caller that never appeared. When every bucket is
/// still in debt the sweep frees nothing, and the least recently used one is
/// evicted so a new caller is not refused for want of a slot — the step
/// `oauth`'s copy was missing.
fn evict<K>(buckets: &mut HashMap<K, (Instant, f64)>, limit: Limit, now: Instant, cap: usize)
where
    K: Clone + Eq + Hash,
{
    let mut oldest = None;

    buckets.retain(|key, bucket| {
        let (last, tokens) = *bucket;
        let refilled = now
            .duration_since(last)
            .as_secs_f64()
            .mul_add(limit.rate, tokens);

        let retain = refilled < limit.burst;

        if retain
            && oldest
                .as_ref()
                .is_none_or(|(_, oldest_at)| last < *oldest_at)
        {
            oldest = Some((key.clone(), last));
        }

        retain
    });

    if buckets.len() >= cap
        && let Some((oldest, _)) = oldest
    {
        buckets.remove::<K>(&oldest);
    }
}

/// Refill a bucket for the time elapsed, then spend one token from it.
fn debit(bucket: &mut (Instant, f64), limit: Limit, now: Instant) -> Result {
    let (last_time, tokens) = bucket;
    let new_tokens = refill(*last_time, *tokens, limit, now);

    if new_tokens < 1.0 {
        return Err(exceeded(limit, new_tokens));
    }

    *last_time = now;
    *tokens = new_tokens - 1.0;

    Ok(())
}

/// A bucket's tokens after refilling for the time since `last`.
fn refill(last: Instant, tokens: f64, limit: Limit, now: Instant) -> f64 {
    now.duration_since(last)
        .as_secs_f64()
        .mul_add(limit.rate, tokens)
        .min(limit.burst)
}

/// The `M_LIMIT_EXCEEDED` for a bucket holding `tokens`, telling the client
/// how long until it holds one whole token again.
fn exceeded(limit: Limit, tokens: f64) -> Error {
    let mut data = LimitExceededErrorData::new();
    data.retry_after = retry_after(limit, tokens).map(RetryAfter::Delay);

    Error::Request(
        ErrorKind::LimitExceeded(data),
        limit.message.into(),
        StatusCode::TOO_MANY_REQUESTS,
    )
}

/// How long until a bucket holding `tokens` refills to one, rounded up to the
/// millisecond so a client retrying on time is not refused again; `None` when
/// the limit never refills.
fn retry_after(limit: Limit, tokens: f64) -> Option<Duration> {
    if limit.rate <= 0.0 {
        return None;
    }

    let secs = ((1.0 - tokens) / limit.rate).max(0.0);

    Duration::try_from_secs_f64((secs * 1000.0).ceil() / 1000.0).ok()
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex, time::Duration};

    use phantom_core::Error;
    use ruma::api::error::{ErrorKind, RetryAfter};

    use super::{Instant, Limit, Ratelimiter, check_at, peek_at};

    const LIMIT: Limit = Limit {
        rate: 1.0,
        burst: 3.0,
        message: "Too many requests.",
    };

    fn table<K>() -> Ratelimiter<K> {
        Mutex::new(HashMap::new())
    }

    fn check(table: &Ratelimiter<u32>, key: u32, now: Instant, cap: usize) -> bool {
        check_at(table, &key, || key, LIMIT, now, cap).is_ok()
    }

    #[test]
    fn a_burst_is_spent_then_refused() {
        let table = table();
        let now = Instant::now();

        for _ in 0..3 {
            assert!(check(&table, 1, now, 16));
        }

        assert!(
            !check(&table, 1, now, 16),
            "the fourth request has no token"
        );
    }

    #[test]
    fn tokens_return_as_time_passes() {
        let table = table();
        let now = Instant::now();

        for _ in 0..3 {
            assert!(check(&table, 1, now, 16));
        }

        assert!(!check(&table, 1, now, 16));
        assert!(
            check(&table, 1, now + Duration::from_secs(1), 16),
            "one second at one token per second buys one request"
        );
    }

    #[test]
    fn a_refilled_bucket_is_swept_to_make_room() {
        let table = table();
        let now = Instant::now();

        assert!(check(&table, 1, now, 2));
        assert!(check(&table, 2, now, 2));

        // Both buckets are back at their ceiling a minute later, so admitting a
        // third key costs nothing but the sweep.
        let later = now + Duration::from_secs(60);

        assert!(check(&table, 3, later, 2));
        assert_eq!(table.lock().expect("table is not poisoned").len(), 1);
    }

    #[test]
    fn a_full_table_of_fresh_buckets_still_admits_a_new_key() {
        let table = table();
        let now = Instant::now();

        // Spend every key down so that no bucket is sweepable.
        for key in 0..2 {
            for _ in 0..3 {
                assert!(check(&table, key, now, 2));
            }
        }

        // This is what `oauth`'s copy got wrong: the sweep frees nothing, and
        // without the oldest-bucket eviction the new key is refused outright.
        assert!(check(&table, 99, now, 2));

        let buckets = table.lock().expect("table is not poisoned");

        assert!(buckets.contains_key(&99), "the new key got a bucket");
        assert!(buckets.len() <= 2, "the table stayed within its cap");
    }

    #[test]
    fn a_refusal_says_when_to_retry() {
        let table = table();
        let now = Instant::now();

        for _ in 0..3 {
            assert!(check(&table, 1, now, 16));
        }

        let error = check_at(&table, &1, || 1, LIMIT, now, 16).expect_err("bucket is empty");
        let Error::Request(ErrorKind::LimitExceeded(data), ..) = error else {
            panic!("expected M_LIMIT_EXCEEDED, got {error:?}");
        };

        assert_eq!(
            data.retry_after,
            Some(RetryAfter::Delay(Duration::from_secs(1))),
            "an empty bucket at one token per second has one second to wait"
        );
    }

    #[test]
    fn peeking_spends_nothing() {
        let table = table();
        let now = Instant::now();

        assert!(peek_at(&table, &1, LIMIT, now).is_ok());
        assert!(
            table.lock().expect("table is not poisoned").is_empty(),
            "a peek at a missing bucket inserts none"
        );

        for _ in 0..3 {
            assert!(check(&table, 1, now, 16));
        }

        assert!(peek_at(&table, &1, LIMIT, now).is_err());
        assert!(peek_at(&table, &1, LIMIT, now + Duration::from_secs(1)).is_ok());
        assert!(
            peek_at(&table, &1, LIMIT, now + Duration::from_secs(1)).is_ok(),
            "a second peek still finds the token the first did not spend"
        );
    }

    #[test]
    fn a_disabled_limit_is_recognized() {
        assert!(Limit { rate: 0.0, ..LIMIT }.is_disabled());
        assert!(
            Limit {
                burst: 0.0,
                ..LIMIT
            }
            .is_disabled()
        );
        assert!(!LIMIT.is_disabled());
    }
}
