//! Extensions to [`futures::Future`] and [`futures::TryFuture`].

mod bool;
mod option;
mod option_stream;
mod try_ext;
mod until;

pub use self::{
    bool::{BoolExt, and, or},
    option::OptionExt,
    option_stream::OptionStream,
    try_ext::TryExt,
    until::UntilExt,
};
