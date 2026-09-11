pub mod deserialize;
pub mod serialize;

macro_rules! unhandled {
    ($msg:literal) => {
        unimplemented!($msg)
    };
}

#[cfg(disable)]
macro_rules! unhandled {
    ($msg:literal) => {
        unsafe {
            std::hint::unreachable_unchecked();
        }
    };
}

pub(crate) use unhandled;
