mod opts;
mod select;
mod transport;

use std::sync::Arc;

use futures::channel::oneshot;

use loole::{Receiver, Sender, unbounded};

use self::{select::Select, transport::Transport};
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
