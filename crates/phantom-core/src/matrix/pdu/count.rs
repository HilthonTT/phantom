#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::as_conversions
)]

use std::{cmp::Ordering, fmt, fmt::Display, str::FromStr};

use ruma::api::Direction;

use crate::{Error, Result, err};

#[derive(Eq, Clone, Copy, Debug)]
pub enum Count {
    Normal(u64),
    Backfilled(i64),
}

impl Count {
    #[inline]
    #[must_use]
    pub fn from_unsigned(unsigned: u64) -> Self {
        Self::from_signed(unsigned as i64)
    }

    #[inline]
    #[must_use]
    pub fn from_signed(signed: i64) -> Self {
        match signed {
            i64::MIN..=0 => Self::Backfilled(signed),
            _ => Self::Normal(signed as u64),
        }
    }

    #[inline]
    #[must_use]
    pub fn into_unsigned(self) -> u64 {
        self.debug_assert_valid();
        match self {
            Self::Normal(i) => i,
            Self::Backfilled(i) => i as u64,
        }
    }

    #[inline]
    #[must_use]
    pub fn into_signed(self) -> i64 {
        self.debug_assert_valid();
        match self {
            Self::Normal(i) => i as i64,
            Self::Backfilled(i) => i,
        }
    }

    #[inline]
    #[must_use]
    pub fn into_normal(self) -> Self {
        self.debug_assert_valid();
        match self {
            Self::Normal(i) => Self::Normal(i),
            Self::Backfilled(_) => Self::Normal(0),
        }
    }

    #[inline]
    pub fn checked_inc(self, dir: Direction) -> Result<Self, Error> {
        match dir {
            Direction::Forward => self.checked_add(1),
            Direction::Backward => self.checked_sub(1),
        }
    }

    #[inline]
    pub fn checked_add(self, add: u64) -> Result<Self, Error> {
        Ok(match self {
            Self::Normal(i) => Self::Normal(
                i.checked_add(add)
                    .ok_or_else(|| err!(Arithmetic("Count::Normal overflow")))?,
            ),
            Self::Backfilled(i) => Self::Backfilled(
                i.checked_add(add as i64)
                    .ok_or_else(|| err!(Arithmetic("Count::Backfilled overflow")))?,
            ),
        })
    }

    #[inline]
    pub fn checked_sub(self, sub: u64) -> Result<Self, Error> {
        Ok(match self {
            Self::Normal(i) => Self::Normal(
                i.checked_sub(sub)
                    .ok_or_else(|| err!(Arithmetic("Count::Normal underflow")))?,
            ),
            Self::Backfilled(i) => Self::Backfilled(
                i.checked_sub(sub as i64)
                    .ok_or_else(|| err!(Arithmetic("Count::Backfilled underflow")))?,
            ),
        })
    }

    #[inline]
    #[must_use]
    pub fn saturating_inc(self, dir: Direction) -> Self {
        match dir {
            Direction::Forward => self.saturating_add(1),
            Direction::Backward => self.saturating_sub(1),
        }
    }

    #[inline]
    #[must_use]
    pub fn saturating_add(self, add: u64) -> Self {
        match self {
            Self::Normal(i) => Self::Normal(i.saturating_add(add)),
            Self::Backfilled(i) => Self::Backfilled(i.saturating_add(add as i64)),
        }
    }

    #[inline]
    #[must_use]
    pub fn saturating_sub(self, sub: u64) -> Self {
        match self {
            Self::Normal(i) => Self::Normal(i.saturating_sub(sub)),
            Self::Backfilled(i) => Self::Backfilled(i.saturating_sub(sub as i64)),
        }
    }

    #[inline]
    #[must_use]
    pub const fn min() -> Self {
        Self::Backfilled(i64::MIN)
    }

    #[inline]
    #[must_use]
    pub const fn max() -> Self {
        Self::Normal(i64::MAX as u64)
    }

    #[inline]
    pub(crate) fn debug_assert_valid(&self) {
        if let Self::Backfilled(i) = self {
            debug_assert!(*i <= 0, "Backfilled sequence must be negative");
        }
    }
}

impl Display for Count {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        self.debug_assert_valid();
        match self {
            Self::Normal(i) => write!(f, "{i}"),
            Self::Backfilled(i) => write!(f, "{i}"),
        }
    }
}

impl From<i64> for Count {
    #[inline]
    fn from(signed: i64) -> Self {
        Self::from_signed(signed)
    }
}

impl From<u64> for Count {
    #[inline]
    fn from(unsigned: u64) -> Self {
        Self::from_unsigned(unsigned)
    }
}

impl FromStr for Count {
    type Err = Error;

    fn from_str(token: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_signed(token.parse()?))
    }
}

// Diverges from upstream, which derived these. `from_signed(0)` yields
// `Backfilled(0)` while `Default` is `Normal(0)`, and arithmetic can land on
// either, so a derived equality disagreed with `Ord` (which compares the
// signed value) for zero. Equality and hashing go by the same value.
impl PartialEq for Count {
    fn eq(&self, other: &Self) -> bool {
        self.into_signed() == other.into_signed()
    }
}

impl std::hash::Hash for Count {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.into_signed().hash(state);
    }
}

impl PartialOrd for Count {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Count {
    fn cmp(&self, other: &Self) -> Ordering {
        self.into_signed().cmp(&other.into_signed())
    }
}

impl Default for Count {
    fn default() -> Self {
        Self::Normal(0)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::Count;

    /// Zero can be reached as `Normal(0)` or `Backfilled(0)`; equality,
    /// hashing and ordering all have to agree that it is one value.
    #[test]
    fn zero_is_equal_however_it_was_made() {
        let from_signed = Count::from_signed(0);
        let from_default = Count::default();

        assert_eq!(from_signed, from_default);
        assert_eq!(from_signed.cmp(&from_default), std::cmp::Ordering::Equal);

        let set: HashSet<Count> = [from_signed, from_default].into_iter().collect();
        assert_eq!(set.len(), 1);
    }
}
