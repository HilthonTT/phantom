//! Turning Rust values into the bytes a column stores, and back.
//!
//! A key is written so that byte order is the order it iterates in, which is
//! what decides which ranges of it a caller can ask for — see
//! [`serialize::Interfix`] and [`serialize::Separator`]. A value is written
//! the same way unless it opts into [`serialize::Json`] or
//! [`serialize::Cbor`].
//!
//! Both halves are hand-written `serde` implementations rather than a derive,
//! because the wire format is the database's on-disk format: it is fixed by
//! what is already stored, not by what `serde` would choose.

pub mod deserialize;
pub mod serialize;

/// Rejects a `serde` type this format has no encoding for.
///
/// Every arm it guards is one no key or value in the schema reaches. Reaching
/// one is a programming error rather than bad data, so it aborts rather than
/// producing an error a caller would have to handle.
macro_rules! unhandled {
    ($msg:literal) => {
        unimplemented!($msg)
    };
}

// activate when stable; we're not ready for this yet
#[cfg(disable)] // #[cfg(not(debug_assertions))]
macro_rules! unhandled {
    ($msg:literal) => {
        // SAFETY: Eliminates branches for serializing and deserializing types never
        // encountered in the codebase. This can promote optimization and reduce
        // codegen. The developer must verify for every invoking callsite that the
        // unhandled type is in no way involved and could not possibly be encountered.
        unsafe {
            std::hint::unreachable_unchecked();
        }
    };
}

pub(crate) use unhandled;
