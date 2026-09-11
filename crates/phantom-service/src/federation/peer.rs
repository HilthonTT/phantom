use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures::Stream;
use phantom_core::{
    Error,
    http::StatusCode,
    implement,
    stream::{ReadyExt, TryIgnore},
    time::now_secs,
};
use phantom_database::Interfix;
use ruma::{OwnedServerName, ServerName, api::error::ErrorBody};

use super::Service;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Classification {
    #[default]
    Transient,
    Permanent,
}

impl Classification {
    #[inline]
    #[must_use]
    fn from_byte(byte: u8) -> Self {
        match byte {
            1 => Self::Permanent,
            _ => Self::Transient,
        }
    }
}

impl From<Classification> for u8 {
    #[inline]
    fn from(class: Classification) -> Self {
        match class {
            Classification::Transient => 0,
            Classification::Permanent => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShouldAttempt {
    Yes,

    No { earliest_retry: SystemTime },

    Deprioritize,
}

pub(super) struct Backoff {
    pub(super) class: Classification,

    pub(super) anchor_secs: u64,

    pub(super) streak: u32,

    pub(super) now: u64,

    pub(super) window_secs: u64,
    pub(super) grace_secs: u64,
    pub(super) max_backoff_secs: u64,
}

#[derive(Clone, Copy)]
pub(super) struct Streak {
    pub(super) class: Classification,
    pub(super) anchor_secs: u64,
    pub(super) oldest_bucket: u64,
    pub(super) latest_bucket: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct PeerBackoff {
    pub class: Classification,

    pub anchor_secs: u64,

    pub oldest_secs: u64,

    pub delay_secs: u64,
}

#[implement(Service)]
pub async fn record_success(&self, server: &ServerName) {
    self.statuses.del_prefix(&(server, Interfix)).await;
}

#[implement(Service)]
#[tracing::instrument(level = "trace", skip(self), fields(%server))]
pub async fn note_peer_alive(&self, server: &ServerName) -> bool {
    let sad = self.peer_has_failures(server).await;

    if sad {
        self.statuses.del_prefix(&(server, Interfix)).await;
    }

    sad
}

#[implement(Service)]
#[tracing::instrument(level = "trace", skip(self), fields(%server))]
pub async fn peer_has_failures(&self, server: &ServerName) -> bool {
    self.statuses
        .raw_stream_prefix(&prefix_of(server))
        .ignore_err()
        .ready_any(|_| true)
        .await
}

#[implement(Service)]
pub fn record_failure(&self, server: &ServerName, class: Classification) {
    let mut value = [0_u8; 9];
    value[0] = u8::from(class);
    value[1..].copy_from_slice(&now_secs().to_be_bytes());

    self.statuses
        .put_raw((server, self.current_bucket()), value)
        .ok();
}

#[implement(Service)]
#[tracing::instrument(level = "trace", skip(self), fields(%server))]
pub async fn should_attempt(&self, server: &ServerName) -> ShouldAttempt {
    let Some(streak) = self.peer_streak(server).await else {
        return ShouldAttempt::Yes;
    };

    attempt_verdict(&self.backoff(streak))
}

#[implement(Service)]
pub async fn peer_backoff(&self, server: &ServerName) -> Option<PeerBackoff> {
    self.peer_streak(server)
        .await
        .map(|streak| self.peer_backoff_from(streak))
}

#[implement(Service)]
pub async fn peer_backoffs(&self) -> BTreeMap<OwnedServerName, PeerBackoff> {
    let window_secs = self.window_secs;

    self.statuses
        .stream()
        .ignore_err()
        .ready_fold(
            Vec::<(&str, Streak)>::new(),
            |mut runs, ((server, bucket), value): ((&str, u64), &[u8])| {
                match runs.last_mut() {
                    Some((last, streak)) if *last == server => {
                        *streak = fold_streak(window_secs, Some(*streak), bucket, value);
                    }
                    _ => runs.push((server, fold_streak(window_secs, None, bucket, value))),
                }

                runs
            },
        )
        .await
        .into_iter()
        .filter_map(|(server, streak)| {
            let server = OwnedServerName::try_from(server).ok()?;

            Some((server, self.peer_backoff_from(streak)))
        })
        .collect()
}

#[implement(Service)]
pub fn peer_snapshot(
    &self,
) -> impl Stream<Item = (&ServerName, SystemTime, Classification)> + Send + '_ {
    self.statuses.stream().ignore_err().ready_filter_map(
        move |((server, bucket), value): ((&str, u64), &[u8])| {
            let server = <&ServerName>::try_from(server).ok()?;

            Some((server, self.bucket_start(bucket), classify(value)))
        },
    )
}

#[implement(Service)]
#[inline]
#[must_use]
fn current_bucket(&self) -> u64 {
    now_secs().checked_div(self.window_secs.max(1)).unwrap_or(0)
}

#[implement(Service)]
#[inline]
#[must_use]
fn bucket_start(&self, bucket: u64) -> SystemTime {
    let offset = bucket.saturating_mul(self.window_secs);

    UNIX_EPOCH
        .checked_add(Duration::from_secs(offset))
        .unwrap_or(UNIX_EPOCH)
}

#[implement(Service)]
#[inline]
#[must_use]
fn streak(&self, latest_bucket: u64, oldest_bucket: u64) -> u32 {
    let span = latest_bucket
        .saturating_sub(oldest_bucket)
        .saturating_add(1);

    u32::try_from(span).unwrap_or(u32::MAX).min(self.n_max)
}

#[implement(Service)]
async fn peer_streak(&self, server: &ServerName) -> Option<Streak> {
    let window_secs = self.window_secs;

    self.statuses
        .stream_prefix(&(server, Interfix))
        .ignore_err()
        .ready_fold(None, |state, ((_, bucket), value): ((&str, u64), &[u8])| {
            Some(fold_streak(window_secs, state, bucket, value))
        })
        .await
}

#[implement(Service)]
fn backoff(&self, streak: Streak) -> Backoff {
    Backoff {
        class: streak.class,
        anchor_secs: streak.anchor_secs,
        streak: self.streak(streak.latest_bucket, streak.oldest_bucket),
        now: now_secs(),
        window_secs: self.window_secs,
        grace_secs: self.grace.as_secs(),
        max_backoff_secs: self.max_backoff.as_secs(),
    }
}

#[implement(Service)]
fn peer_backoff_from(&self, streak: Streak) -> PeerBackoff {
    PeerBackoff {
        class: streak.class,
        anchor_secs: streak.anchor_secs,
        oldest_secs: streak.oldest_bucket.saturating_mul(self.window_secs),
        delay_secs: self.backoff(streak).delay_secs(),
    }
}

fn prefix_of(server: &ServerName) -> Vec<u8> {
    let mut prefix = server.as_str().as_bytes().to_vec();
    prefix.push(phantom_database::SEP);

    prefix
}

#[must_use]
pub(super) fn attempt_verdict(backoff: &Backoff) -> ShouldAttempt {
    let earliest_secs = backoff.anchor_secs.saturating_add(backoff.delay_secs());

    if backoff.now < earliest_secs {
        return ShouldAttempt::No {
            earliest_retry: UNIX_EPOCH
                .checked_add(Duration::from_secs(earliest_secs))
                .unwrap_or_else(SystemTime::now),
        };
    }

    ShouldAttempt::Deprioritize
}

impl Backoff {
    #[must_use]
    pub(super) fn delay_secs(&self) -> u64 {
        let max = self.max_backoff_secs;

        match self.class {
            Classification::Permanent => max,
            Classification::Transient if self.streak <= 1 && self.grace_secs != 0 => {
                self.grace_secs.min(max)
            }
            Classification::Transient => self
                .window_secs
                .saturating_mul(u64::from(self.streak))
                .saturating_mul(u64::from(self.streak))
                .min(max),
        }
    }
}

#[must_use]
pub(super) fn fold_streak(
    window_secs: u64,
    state: Option<Streak>,
    bucket: u64,
    value: &[u8],
) -> Streak {
    let anchor_secs = failure_secs(value).unwrap_or_else(|| bucket.saturating_mul(window_secs));

    let oldest_bucket = state.map_or(bucket, |streak| streak.oldest_bucket);

    Streak {
        class: classify(value),
        anchor_secs,
        oldest_bucket,
        latest_bucket: bucket,
    }
}

#[inline]
#[must_use]
pub(super) fn classify(bytes: &[u8]) -> Classification {
    bytes
        .first()
        .copied()
        .map_or(Classification::Transient, Classification::from_byte)
}

#[must_use]
pub(super) fn failure_secs(bytes: &[u8]) -> Option<u64> {
    bytes
        .get(1..9)
        .and_then(|tail| tail.try_into().ok())
        .map(u64::from_be_bytes)
}

#[must_use]
pub(super) fn classify_error(error: &Error) -> Option<Classification> {
    let Error::Federation(_, response) = error else {
        return Some(Classification::Transient);
    };

    let status = response.status_code;

    match status {
        _ if status == StatusCode::GONE => Some(Classification::Permanent),
        _ if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS => {
            Some(Classification::Transient)
        }
        _ if matches!(response.body, ErrorBody::NotJson { .. }) => Some(Classification::Transient),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Backoff, Classification, ShouldAttempt, Streak, attempt_verdict, classify, failure_secs,
        fold_streak,
    };

    const WINDOW: u64 = 180;
    const MAX: u64 = 86400;
    const GRACE: u64 = 30;

    fn backoff(class: Classification, streak: u32, anchor_secs: u64, now: u64) -> Backoff {
        Backoff {
            class,
            anchor_secs,
            streak,
            now,
            window_secs: WINDOW,
            grace_secs: GRACE,
            max_backoff_secs: MAX,
        }
    }

    #[test]
    fn a_first_failure_gets_the_grace_tier() {
        let backoff = backoff(Classification::Transient, 1, 1_000, 1_000);

        assert_eq!(backoff.delay_secs(), GRACE);
        assert!(matches!(
            attempt_verdict(&backoff),
            ShouldAttempt::No { .. }
        ));
    }

    #[test]
    fn the_grace_tier_expires() {
        let backoff = backoff(Classification::Transient, 1, 1_000, 1_000 + GRACE);

        assert_eq!(attempt_verdict(&backoff), ShouldAttempt::Deprioritize);
    }

    #[test]
    fn the_curve_is_quadratic_past_the_grace() {
        assert_eq!(
            backoff(Classification::Transient, 2, 0, 0).delay_secs(),
            WINDOW * 4
        );
        assert_eq!(
            backoff(Classification::Transient, 3, 0, 0).delay_secs(),
            WINDOW * 9
        );
    }

    #[test]
    fn the_curve_saturates_at_the_ceiling() {
        assert_eq!(
            backoff(Classification::Transient, 10_000, 0, 0).delay_secs(),
            MAX
        );
    }

    #[test]
    fn a_permanent_failure_goes_straight_to_the_ceiling() {
        assert_eq!(
            backoff(Classification::Permanent, 1, 0, 0).delay_secs(),
            MAX
        );
    }

    #[test]
    fn a_zero_grace_starts_on_the_curve() {
        let mut backoff = backoff(Classification::Transient, 1, 0, 0);
        backoff.grace_secs = 0;

        assert_eq!(backoff.delay_secs(), WINDOW);
    }

    #[test]
    fn an_elapsed_backoff_is_deprioritized_not_clean() {
        let backoff = backoff(Classification::Transient, 4, 0, MAX);

        assert_eq!(attempt_verdict(&backoff), ShouldAttempt::Deprioritize);
    }

    #[test]
    fn the_streak_folds_oldest_to_newest() {
        let value = |class: Classification, secs: u64| {
            let mut value = [0_u8; 9];
            value[0] = u8::from(class);
            value[1..].copy_from_slice(&secs.to_be_bytes());
            value
        };

        let first = fold_streak(WINDOW, None, 10, &value(Classification::Transient, 1_800));
        let second = fold_streak(
            WINDOW,
            Some(first),
            12,
            &value(Classification::Permanent, 2_160),
        );

        assert_eq!(second.oldest_bucket, 10);
        assert_eq!(second.latest_bucket, 12);
        assert_eq!(second.anchor_secs, 2_160);
        assert_eq!(second.class, Classification::Permanent);
    }

    #[test]
    fn a_row_without_a_timestamp_dates_from_its_bucket() {
        let streak: Streak = fold_streak(WINDOW, None, 10, &[0_u8]);

        assert_eq!(streak.anchor_secs, 10 * WINDOW);
        assert_eq!(failure_secs(&[0_u8]), None);
    }

    #[test]
    fn an_unknown_classification_byte_reads_as_transient() {
        assert_eq!(classify(&[7_u8]), Classification::Transient);
        assert_eq!(classify(&[]), Classification::Transient);
    }
}
