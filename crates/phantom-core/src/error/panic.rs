//! Carrying a panic through [`Error`] and back out again.

use std::{
    any::Any,
    panic::{RefUnwindSafe, UnwindSafe, panic_any},
};

use super::Error;

impl UnwindSafe for Error {}
impl RefUnwindSafe for Error {}

impl Error {
    #[inline]
    pub fn panic(self) -> ! {
        panic_any(self.into_panic())
    }

    #[must_use]
    #[inline]
    pub fn from_panic(e: Box<dyn Any + Send>) -> Self {
        Self::Panic(panic_str(&e), e)
    }

    #[inline]
    pub fn into_panic(self) -> Box<dyn Any + Send + 'static> {
        match self {
            Self::Panic(_, e) | Self::PanicAny(e) => e,
            Self::JoinError(e) => e.into_panic(),
            _ => Box::new(self),
        }
    }

    /// Get the panic message string.
    #[inline]
    pub fn panic_str(self) -> Option<&'static str> {
        self.is_panic().then_some(panic_str(&self.into_panic()))
    }

    /// Check if the Error is trafficking a panic object.
    #[inline]
    pub fn is_panic(&self) -> bool {
        match &self {
            Self::Panic(..) | Self::PanicAny(..) => true,
            Self::JoinError(e) => e.is_panic(),
            _ => false,
        }
    }
}

/// The `&str` a panic carried, or `""` if it carried a formatted message.
#[must_use]
pub fn panic_str(p: &(dyn Any + Send)) -> &'static str {
    p.downcast_ref::<&str>().copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_str_reads_str_payloads() {
        let payload = std::panic::catch_unwind(|| panic!("boom")).expect_err("panicked");
        assert_eq!(panic_str(&*payload), "boom");
    }
}
