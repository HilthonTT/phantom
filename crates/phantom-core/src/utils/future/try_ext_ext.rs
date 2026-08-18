//! Extended external extensions to futures::TryFutureExt
#![allow(clippy::type_complexity)]
// is_ok() has to consume *self rather than borrow. This extension is for a
// caller only ever caring about result status while discarding all contents.
#![allow(clippy::wrong_self_convention)]

use std::marker::Unpin;

use futures::{
    TryFuture, TryFutureExt, future,
    future::{MapOkOrElse, TrySelect, UnwrapOrElse},
};
