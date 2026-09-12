#![allow(dead_code)]

mod error;
mod inflight;
mod opts;
mod select;
mod transport;
mod validate;
mod worker;

use std::sync::Arc;

use futures::channel::oneshot;

use loole::{Receiver, Sender};

pub use self::opts::{EventWindow, FanoutGrowth, Op, Opts, Outcome};
use self::{
    error::Failure,
    inflight::{Key, Subscription},
    select::Select,
    transport::Transport,
};
use crate::Services;

const REQUESTS_MAX: usize = 100;

pub struct Service {
    services: Arc<Services>,
    channel: (Sender<Msg>, Receiver<Msg>),
    transport: Arc<dyn Transport>,
    select: Arc<dyn Select>,
    capacity: usize,
}

struct Msg {
    key: Key,
    reply: oneshot::Sender<Subscription>,
}
