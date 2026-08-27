//! Checked arithmetic and fallible numeric conversion.

use num_traits::ops::checked::{CheckedAdd, CheckedDiv, CheckedMul, CheckedRem, CheckedSub};
use std::{any::type_name, cmp};

pub use checked_ops::checked_ops;

use crate::{Err, Error, Result, err};

/// Checked arithmetic expression. Returns a Result<R, Error::Arithmetic>
#[macro_export]
macro_rules! checked {
	($($input:tt)+) => {
		$crate::math::checked_ops!($($input)+)
			.ok_or_else(|| $crate::err!(Arithmetic("operation overflowed or result invalid")))
	};
}

/// Checked arithmetic expression which panics on failure. This is for
/// expressions which do not meet the threshold for validated! but the caller
/// has no realistic expectation for error and no interest in cluttering the
/// callsite with result handling from checked!.
#[macro_export]
macro_rules! expected {
	($msg:literal, $($input:tt)+) => {
		$crate::checked!($($input)+).expect($msg)
	};

	($($input:tt)+) => {
		$crate::expected!("arithmetic expression expectation failure", $($input)+)
	};
}

/// Fallible numeric conversion usable in combinator position, e.g.
/// `get::<u32>(key).and_then(math::try_into)`.
#[inline]
pub fn try_into<Dst: TryFrom<Src>, Src>(src: Src) -> Result<Dst> {
    Dst::try_from(src).map_err(try_into_err::<Dst, Src>)
}

/// Checked arithmetic yielding [`Error::Arithmetic`] rather than panicking or
/// wrapping.
pub trait Tried: Sized {
    fn try_add(self, rhs: Self) -> Result<Self>;
    fn try_sub(self, rhs: Self) -> Result<Self>;
    fn try_mul(self, rhs: Self) -> Result<Self>;
}

fn try_into_err<Dst: TryFrom<Src>, Src>(e: <Dst as TryFrom<Src>>::Error) -> Error {
    drop(e);
    err!(Arithmetic(
        "failed to convert from {} to {}",
        type_name::<Src>(),
        type_name::<Dst>()
    ))
}

#[inline]
pub fn clamp<T: Ord>(val: T, min: T, max: T) -> T {
    cmp::min(cmp::max(val, min), max)
}

/// Truncating conversion from a float, rejecting the inputs that have no
/// `usize` to truncate to.
///
/// The reference implementation reaches for `to_int_unchecked`, which is
/// undefined behaviour for exactly the NaN and out-of-range inputs this
/// rejects. A saturating `as` cast costs nothing extra once the range has been
/// checked, so the unsafe block buys no speed here.
#[inline]
#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
pub fn usize_from_f64(val: f64) -> Result<usize> {
    // NaN is spelled out rather than left to a negated comparison, so that it
    // takes this branch instead of falling through to the cast.
    if val.is_nan() || val < 0.0 {
        return Err!(Arithmetic(
            "converting negative or NaN float to unsigned integer"
        ));
    }

    if val > usize::MAX as f64 {
        return Err!(Arithmetic("float exceeds the range of usize"));
    }

    Ok(val as usize)
}

#[inline]
#[must_use]
pub fn usize_from_ruma(val: ruma::UInt) -> usize {
    usize::try_from(val).expect("failed conversion from ruma::UInt to usize")
}

#[inline]
#[must_use]
pub fn ruma_from_u64(val: u64) -> ruma::UInt {
    ruma::UInt::try_from(val).expect("failed conversion from u64 to ruma::UInt")
}

#[inline]
#[must_use]
pub fn ruma_from_usize(val: usize) -> ruma::UInt {
    ruma::UInt::try_from(val).expect("failed conversion from usize to ruma::UInt")
}

#[inline]
#[must_use]
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
pub fn usize_from_u64_truncated(val: u64) -> usize {
    val as usize
}

macro_rules! impl_tried {
    ($($ty:ty),+ $(,)?) => {$(
        impl Tried for $ty {
            #[inline]
            fn try_add(self, rhs: Self) -> Result<Self> {
                self.checked_add(rhs).ok_or_else(|| err!(Arithmetic("integer overflow")))
            }

            #[inline]
            fn try_sub(self, rhs: Self) -> Result<Self> {
                self.checked_sub(rhs).ok_or_else(|| err!(Arithmetic("integer overflow")))
            }

            #[inline]
            fn try_mul(self, rhs: Self) -> Result<Self> {
                self.checked_mul(rhs).ok_or_else(|| err!(Arithmetic("integer overflow")))
            }
        }
    )+};
}

impl_tried!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize
);

pub trait Expected {
    #[inline]
    #[must_use]
    fn expected_add(self, rhs: Self) -> Self
    where
        Self: CheckedAdd + Sized,
    {
        expected!(self + rhs)
    }

    #[inline]
    #[must_use]
    fn expected_sub(self, rhs: Self) -> Self
    where
        Self: CheckedSub + Sized,
    {
        expected!(self - rhs)
    }

    #[inline]
    #[must_use]
    fn expected_mul(self, rhs: Self) -> Self
    where
        Self: CheckedMul + Sized,
    {
        expected!(self * rhs)
    }

    #[inline]
    #[must_use]
    fn expected_div(self, rhs: Self) -> Self
    where
        Self: CheckedDiv + Sized,
    {
        expected!(self / rhs)
    }

    #[inline]
    #[must_use]
    fn expected_rem(self, rhs: Self) -> Self
    where
        Self: CheckedRem + Sized,
    {
        expected!(self % rhs)
    }
}

impl<T> Expected for T {}
