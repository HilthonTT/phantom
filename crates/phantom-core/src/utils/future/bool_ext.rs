//! Extended external extensions to futures::FutureExt

use std::marker::Unpin;

use futures::{
    Future, FutureExt,
    future::{select_ok, try_join, try_join_all, try_select},
};
