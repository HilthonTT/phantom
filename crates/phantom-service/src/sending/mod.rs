use std::{
    fmt::Debug,
    hash::{DefaultHasher, Hash, Hasher},
    iter::once,
    sync::Arc,
};

use async_trait::async_trait;
use futures::{FutureExt, Stream, StreamExt};
use phantom_core::{
    Result, debug, err, error,
    math::usize_from_u64_truncated,
    server::Server,
    stream::{ReadyExt, TryReadyExt},
    sys::compute::available_parallelism,
    warn,
};
use ruma::{
    RoomId, ServerName, UserId,
    api::{OutgoingRequest, appservice::Registration},
};
use tokio::{task, task::JoinSet};
