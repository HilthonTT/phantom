pub mod exponential_backoff;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{Result, err};

#[inline]
#[must_use]
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
pub fn now_millis() -> u64 {
    UNIX_EPOCH
        .elapsed()
        .expect("positive duration after epoch")
        .as_millis() as u64
}

#[inline]
pub fn parse_timepoint_ago(ago: &str) -> Result<SystemTime> {
    timepoint_ago(parse_duration(ago)?)
}

#[inline]
pub fn timepoint_ago(duration: Duration) -> Result<SystemTime> {
    SystemTime::now()
        .checked_sub(duration)
        .ok_or_else(|| err!(Arithmetic("Duration {duration:?} is too large")))
}

#[inline]
pub fn timepoint_from_now(duration: Duration) -> Result<SystemTime> {
    SystemTime::now()
        .checked_add(duration)
        .ok_or_else(|| err!(Arithmetic("Duration {duration:?} is too large")))
}

#[inline]
pub fn parse_duration(duration: &str) -> Result<Duration> {
    cyborgtime::parse_duration(duration)
        .map_err(|error| err!("'{duration:?}' is not a valid duration string: {error:?}"))
}

#[must_use]
pub fn rfc2822_from_seconds(epoch: i64) -> String {
    use chrono::{DateTime, Utc};

    DateTime::<Utc>::from_timestamp(epoch, 0)
        .unwrap_or_default()
        .to_rfc2822()
}

#[must_use]
pub fn format(ts: SystemTime, str: &str) -> String {
    use chrono::{DateTime, Utc};

    let dt: DateTime<Utc> = ts.into();
    dt.format(str).to_string()
}

#[must_use]
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn pretty(d: Duration) -> String {
    use Unit::*;

    let fmt = |w, f, u| format!("{w}.{f:02} {u}");
    let gen64 = |w, f, u| fmt(w, (f * 100.0) as u32, u);
    let gen128 = |w, f, u| gen64(u64::try_from(w).expect("u128 to u64"), f, u);
    match whole_and_frac(d) {
        (Days(whole), frac) => gen64(whole, frac, "days"),
        (Hours(whole), frac) => gen64(whole, frac, "hours"),
        (Mins(whole), frac) => gen64(whole, frac, "minutes"),
        (Secs(whole), frac) => gen64(whole, frac, "seconds"),
        (Millis(whole), frac) => gen128(whole, frac, "milliseconds"),
        (Micros(whole), frac) => gen128(whole, frac, "microseconds"),
        (Nanos(whole), frac) => gen128(whole, frac, "nanoseconds"),
    }
}

/// Return a pair of (whole part, frac part) from a duration where. The whole
/// part is the largest Unit containing a non-zero value, the frac part is a
/// rational remainder left over.
#[must_use]
#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
pub fn whole_and_frac(d: Duration) -> (Unit, f64) {
    use Unit::*;

    let whole = whole_unit(d);
    (
        whole,
        match whole {
            Days(_) => (d.as_secs() % 86_400) as f64 / 86_400.0,
            Hours(_) => (d.as_secs() % 3_600) as f64 / 3_600.0,
            Mins(_) => (d.as_secs() % 60) as f64 / 60.0,
            Secs(_) => f64::from(d.subsec_millis()) / 1000.0,
            Millis(_) => f64::from(d.subsec_micros() % 1_000) / 1000.0,
            Micros(_) => f64::from(d.subsec_nanos() % 1_000) / 1000.0,
            Nanos(_) => 0.0,
        },
    )
}

/// Return the largest Unit which represents the duration. The value is
/// rounded-down, but never zero.
#[must_use]
pub fn whole_unit(d: Duration) -> Unit {
    use Unit::*;

    match d.as_secs() {
        86_400.. => Days(d.as_secs() / 86_400),
        3_600..=86_399 => Hours(d.as_secs() / 3_600),
        60..=3_599 => Mins(d.as_secs() / 60),

        _ => match d.as_micros() {
            1_000_000.. => Secs(d.as_secs()),
            1_000..=999_999 => Millis(d.subsec_millis().into()),

            _ => match d.as_nanos() {
                1_000.. => Micros(d.subsec_micros().into()),

                _ => Nanos(d.subsec_nanos().into()),
            },
        },
    }
}

#[derive(Eq, PartialEq, Clone, Copy, Debug)]
pub enum Unit {
    Days(u64),
    Hours(u64),
    Mins(u64),
    Secs(u64),
    Millis(u128),
    Micros(u128),
    Nanos(u128),
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Unit, pretty, whole_and_frac, whole_unit};

    #[test]
    fn whole_unit_picks_largest_nonzero() {
        assert_eq!(whole_unit(Duration::from_secs(90_000)), Unit::Days(1));
        assert_eq!(whole_unit(Duration::from_secs(5_400)), Unit::Hours(1));
        assert_eq!(whole_unit(Duration::from_secs(90)), Unit::Mins(1));
        assert_eq!(whole_unit(Duration::from_millis(1_500)), Unit::Secs(1));
        assert_eq!(whole_unit(Duration::from_micros(1_500)), Unit::Millis(1));
        assert_eq!(whole_unit(Duration::from_nanos(1_500)), Unit::Micros(1));
        assert_eq!(whole_unit(Duration::from_nanos(999)), Unit::Nanos(999));
    }

    /// The remainder is a fraction of the whole unit, so it must stay below 1
    /// even for the sub-second units, whose `subsec_*` accessors are relative
    /// to the second rather than to the unit above them.
    #[test]
    fn frac_is_always_below_one() {
        let cases = [
            Duration::from_secs(90_000),
            Duration::from_secs(5_400),
            Duration::from_secs(90),
            Duration::from_millis(1_500),
            Duration::from_micros(1_500),
            Duration::from_micros(999_500),
            Duration::from_nanos(1_500),
            Duration::from_nanos(999_500),
        ];

        for d in cases {
            let (_, frac) = whole_and_frac(d);
            assert!((0.0..1.0).contains(&frac), "{d:?} gave frac {frac}");
        }
    }

    #[test]
    fn pretty_formats_two_fractional_digits() {
        assert_eq!(pretty(Duration::from_micros(1_500)), "1.50 milliseconds");
        assert_eq!(pretty(Duration::from_nanos(1_500)), "1.50 microseconds");
        assert_eq!(pretty(Duration::from_millis(1_050)), "1.05 seconds");
        assert_eq!(pretty(Duration::from_secs(90)), "1.50 minutes");
    }
}
