//! Extended external extensions to futures::FutureExt

use std::marker::Unpin;

use futures::{Future, future, future::Select};
