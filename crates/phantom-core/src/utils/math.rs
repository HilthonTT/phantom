//! Checked arithmetic and fallible numeric conversion.

use crate::{Error, Result};

/// Fallible numeric conversion usable in combinator position, e.g.
/// `get::<u32>(key).and_then(math::try_into)`.
#[inline]
pub fn try_into<Dst, Src>(src: Src) -> Result<Dst>
where
    Dst: TryFrom<Src>,
    Error: From<<Dst as TryFrom<Src>>::Error>,
{
    Dst::try_from(src).map_err(Into::into)
}

/// Checked arithmetic yielding [`Error::Overflow`] rather than panicking or
/// wrapping.
pub trait Tried: Sized {
    fn try_add(self, rhs: Self) -> Result<Self>;
    fn try_sub(self, rhs: Self) -> Result<Self>;
    fn try_mul(self, rhs: Self) -> Result<Self>;
}

macro_rules! impl_tried {
    ($($ty:ty),+ $(,)?) => {$(
        impl Tried for $ty {
            #[inline]
            fn try_add(self, rhs: Self) -> Result<Self> {
                self.checked_add(rhs).ok_or(Error::Overflow)
            }

            #[inline]
            fn try_sub(self, rhs: Self) -> Result<Self> {
                self.checked_sub(rhs).ok_or(Error::Overflow)
            }

            #[inline]
            fn try_mul(self, rhs: Self) -> Result<Self> {
                self.checked_mul(rhs).ok_or(Error::Overflow)
            }
        }
    )+};
}

impl_tried!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);
