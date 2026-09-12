use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
    sync::{Arc, Weak},
};

use bytes::Bytes;
use futures::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use phantom_core::{debug_warn, implement, trace};
use ruma::OwnedServerName;
use tokio::sync::watch::channel;

use super::{
    Msg, Opts, Outcome, Service,
    error::{Attempted, Failure},
    inflight::{Inflight, Key, SharedResult},
};

type FetchFuture<'a> = BoxFuture<'a, (Key, SharedResult)>;
type FetchFutures<'a> = FuturesUnordered<FetchFuture<'a>>;

#[implement(Service)]
pub(super) async fn run_worker(self: Arc<Self>) {
    let mut inflight: HashMap<Key, Inflight> = HashMap::new();
    let mut pending: VecDeque<Msg> = VecDeque::new();
    let mut futures: FetchFutures<'_> = FuturesUnordered::new();

    self.work_loop(&mut inflight, &mut pending, &mut futures)
        .await;
}

#[implement(Service)]
async fn work_loop<'a>(
    &'a self,
    inflight: &mut HashMap<Key, Inflight>,
    pending: &mut VecDeque<Msg>,
    futures: &mut FetchFutures<'a>,
) {
    let rx = self.channel.1.clone();
    while !rx.is_closed() {
        while let Ok(msg) = rx.try_recv() {
            self.on_request(msg, inflight, pending, futures);
        }

        tokio::select! {
            Some((key, result)) = futures.next() =>
                self.on_complete(key, result, inflight, pending, futures),
            msg = rx.recv_async() => match msg {
                | Ok(msg) => self.on_request(msg, inflight, pending, futures),
                | Err(_) => break,
            },
        }
    }
}

#[implement(Service)]
fn on_request<'a>(
    &'a self,
    msg: Msg,
    inflight: &mut HashMap<Key, Inflight>,
    pending: &mut VecDeque<Msg>,
    futures: &FetchFutures<'a>,
) {
    let Some(entry) = inflight.get_mut(&msg.key) else {
        if futures.len() >= self.capacity {
            pending.push_back(msg);
        } else {
            self.dispatch(msg, inflight, futures);
        }

        return;
    };

    match entry.interest.upgrade() {
        Some(strong) => {
            msg.reply.send((entry.tx.subscribe(), strong)).ok();
        }
        None => {
            let interest = Arc::new(());
            entry.interest = Arc::downgrade(&interest);
            msg.reply.send((entry.tx.subscribe(), interest)).ok();
        }
    }
}

#[implement(Service)]
fn dispatch<'a>(
    &'a self,
    msg: Msg,
    inflight: &mut HashMap<Key, Inflight>,
    futures: &FetchFutures<'a>,
) {
    let Msg { key, reply } = msg;
    let interest = Arc::new(());
    let (tx, rx) = channel(None);

    if reply.send((rx, interest.clone())).is_err() {
        return;
    }

    let opts = key.opts();
    let weak = Arc::downgrade(&interest);
    inflight.insert(
        key.clone(),
        Inflight {
            tx,
            interest: weak.clone(),
            opts: opts.clone(),
        },
    );

    self.push_attempt(futures, key, opts, weak);
}

#[implement(Service)]
fn push_attempt<'a>(
    &'a self,
    futures: &FetchFutures<'a>,
    key: Key,
    opts: Arc<Opts>,
    weak: Weak<()>,
) {
    futures.push(async move { (key, self.run_attempts(&opts, &weak).await) }.boxed());
}

#[implement(Service)]
fn on_complete<'a>(
    &'a self,
    key: Key,
    result: SharedResult,
    inflight: &mut HashMap<Key, Inflight>,
    pending: &mut VecDeque<Msg>,
    futures: &FetchFutures<'a>,
) {
    let Some(entry) = inflight.get(&key) else {
        return;
    };

    if matches!(&result, Err(Failure::Cancelled)) && entry.interest.upgrade().is_some() {
        let opts = entry.opts.clone();
        let weak = entry.interest.clone();
        self.push_attempt(futures, key, opts, weak);
        return;
    }

    entry.tx.send(Some(result)).ok();
    inflight.remove(&key);

    while futures.len() < self.capacity {
        let Some(msg) = pending.pop_front() else {
            break;
        };

        self.on_request(msg, inflight, pending, futures);
    }
}

#[implement(Service)]
#[tracing::instrument(
	name = "attempts",
	level = "debug",
	skip_all,
	fields(
		op = ?opts.op,
		room_id = ?opts.room_id,
		event_id = ?opts.event_id,
	),
)]
async fn run_attempts(&self, opts: &Opts, interest: &Weak<()>) -> SharedResult {
    let candidates = self.select.candidates(opts).await;
    if candidates.is_empty() {
        return Err(Failure::NoCandidates);
    }

    let count = candidates.len();
    let limit = opts.attempt_limit.map_or(count, |n| n.get().min(count));

    let federation = &self.services.server.config.federation;
    let max_width = effective_cap(opts.fanout_max_width, federation.fetch_fanout_max_width);
    let max_rounds = effective_cap(opts.fanout_rounds, federation.fetch_fanout_rounds);

    let mut attempted: Attempted = Attempted::new();
    let mut remaining = candidates.into_iter();
    let mut round: usize = 0;

    while attempted.len() < limit {
        if interest.strong_count() == 0 {
            return Err(Failure::Cancelled);
        }

        if round >= max_rounds {
            break;
        }

        let budget = limit.saturating_sub(attempted.len());
        let width = opts
            .fanout_growth
            .round_width(round)
            .min(max_width)
            .min(budget);

        let mut racing: FuturesUnordered<_> = remaining
            .by_ref()
            .take(width)
            .map(|server| self.attempt(server, opts))
            .collect();

        if racing.is_empty() {
            break;
        }

        while let Some((server, bytes)) = racing.next().await {
            let Some(bytes) = bytes else {
                attempted.push(server);

                if interest.strong_count() == 0 {
                    return Err(Failure::Cancelled);
                }

                continue;
            };

            trace!(%server, "fetch satisfied");
            return Ok(Arc::new(Outcome {
                bytes,
                origin: server,
            }));
        }

        round = round.saturating_add(1);
    }

    Err(Failure::NotFound { attempted })
}

#[implement(Service)]
#[tracing::instrument(
	name = "attempt",
	level = "trace",
	skip_all,
	fields(%server),
)]
async fn attempt(&self, server: OwnedServerName, opts: &Opts) -> (OwnedServerName, Option<Bytes>) {
    let Some(bytes) = self
        .transport
        .fetch_raw(opts.op, &server, opts)
        .await
        .inspect_err(|error| debug_warn!(%server, "fetch attempt failed: {error}"))
        .ok()
    else {
        return (server, None);
    };

    let valid = self
        .validate(opts, &bytes)
        .await
        .inspect_err(|error| debug_warn!(%server, "rejecting poisoned response: {error}"))
        .is_ok();

    (server, valid.then_some(bytes))
}

fn effective_cap(requested: Option<NonZeroUsize>, configured: usize) -> usize {
    let configured = if configured > 0 {
        configured
    } else {
        usize::MAX
    };

    requested.map_or(configured, |n| n.get().min(configured))
}
