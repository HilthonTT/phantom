//! The shapes data takes on its way into and out of a map: the key and value
//! types, the handle a read returns, and the batched and transactional writes.

pub(crate) mod cork;
pub(crate) mod handle;
pub mod keyval;
pub(crate) mod txn;
