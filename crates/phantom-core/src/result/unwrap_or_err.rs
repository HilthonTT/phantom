use std::convert::identity;

use super::Result;

pub trait UnwrapOrErr<T> {
    fn unwrap_or_err(self) -> T;
}

impl<T> UnwrapOrErr<T> for Result<T, T> {
    #[inline]
    fn unwrap_or_err(self) -> T {
        self.unwrap_or_else(identity::<T>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_side_is_returned_as_is() {
        let ok: Result<u8, u8> = Ok(1);
        let err: Result<u8, u8> = Err(2);

        assert_eq!(ok.unwrap_or_err(), 1);
        assert_eq!(err.unwrap_or_err(), 2);
    }
}
