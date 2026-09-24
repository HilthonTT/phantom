//! Coalesced, failover federation fetch of raw event bytes.
//!
//! [`Service::fetch`] is the entry point; behind it a single worker task owns
//! every in-flight fetch and the dedup map, so no lock guards them. The
//! per-fetch work splits across the submodules: candidate selection, the
//! federation transport, and response validation.

mod error;
mod inflight;
mod opts;
mod select;
mod transport;
mod validate;
mod worker;

use std::sync::Arc;

use async_trait::async_trait;
use futures::channel::oneshot;
use loole::{Receiver, Sender, unbounded};
use phantom_core::{Result, implement};
use tokio::sync::watch;

pub use self::opts::{EventWindow, FanoutGrowth, Op, Opts, Outcome};
use self::{
    error::Failure,
    inflight::{Key, SharedResult, Subscription},
    select::{RoomCandidates, Select},
    transport::{FederationTransport, Transport},
};
use crate::{
    Dep,
    net::{federation, server_keys},
    ops::{config, moderation, server_state},
    rooms::state_cache,
};

/// Upper bound on concurrent in-flight fetches across all keys.
const REQUESTS_MAX: usize = 100;

/// Coordinates coalesced, validated federation fetches with candidate failover.
///
/// A single worker owns all in-flight state and admits at most
/// [`REQUESTS_MAX`] distinct fetches at once; additional keys wait in its
/// pending queue. Fetches have no whole-operation deadline or per-server
/// fairness scheduler.
pub struct Service {
    services: Arc<Services>,
    channel: (Sender<Msg>, Receiver<Msg>),
    transport: Arc<dyn Transport>,
    select: Arc<dyn Select>,
    capacity: usize,
}

/// The services a fetch reaches, shared by the worker, the transport and the
/// candidate selector.
///
/// Diverges from tuwunel, whose fetcher holds its `OnceServices`: phantom has
/// no such handle, so the dependencies are named here as `Dep`s instead.
struct Services {
    config: Dep<config::Service>,
    federation: Dep<federation::Service>,
    moderation: Dep<moderation::Service>,
    server_keys: Dep<server_keys::Service>,
    server_state: Dep<server_state::Service>,
    state_cache: Dep<state_cache::Service>,
}

/// Request to the worker. The worker replies with a subscription to the
/// coalesced result, deferring the reply under backpressure until a slot frees.
struct Msg {
    key: Key,
    reply: oneshot::Sender<Subscription>,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let services = Arc::new(Services {
            config: args.depend::<config::Service>("ops::config"),
            federation: args.depend::<federation::Service>("net::federation"),
            moderation: args.depend::<moderation::Service>("ops::moderation"),
            server_keys: args.depend::<server_keys::Service>("net::server_keys"),
            server_state: args.depend::<server_state::Service>("ops::server_state"),
            state_cache: args.depend::<state_cache::Service>("rooms::state_cache"),
        });

        let transport: Arc<dyn Transport> = Arc::new(FederationTransport {
            services: services.clone(),
        });

        let select: Arc<dyn Select> = Arc::new(RoomCandidates {
            services: services.clone(),
        });

        Ok(Arc::new(Self {
            services,
            channel: unbounded(),
            transport,
            select,
            capacity: REQUESTS_MAX,
        }))
    }

    async fn worker(self: Arc<Self>) -> Result<()> {
        self.run_worker().await;

        Ok(())
    }

    fn interrupt(&self) {
        let (sender, _) = &self.channel;
        if !sender.is_closed() {
            sender.close();
        }
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Fetches raw response bytes over federation with coalescing and failover.
///
/// Requests coalesce only when their complete option identities match, except
/// missing-event windows are order independent. The future resolves when one
/// response passes every enabled check; candidate exhaustion, attempt limits,
/// or round limits otherwise end it with failure.
#[implement(Service)]
#[tracing::instrument(
	level = "debug",
	skip_all,
	fields(
		op = ?opts.op,
		room_id = ?opts.room_id,
		event_id = ?opts.event_id,
	),
)]
pub async fn fetch(&self, opts: Opts) -> Result<Arc<Outcome>> {
    let key = Key::new(opts);
    let (reply, reply_rx) = oneshot::channel();

    self.channel
        .0
        .send(Msg { key, reply })
        .map_err(|_| Failure::Cancelled)?;

    // Hold the strong interest token across the wait; its drop cancels the fetch.
    let (rx, _interest) = reply_rx.await.map_err(|_| Failure::Cancelled)?;

    await_result(rx).await.map_err(Into::into)
}

async fn await_result(mut rx: watch::Receiver<Option<SharedResult>>) -> SharedResult {
    rx.wait_for(Option::is_some)
        .await
        .map_or(Err(Failure::Cancelled), |value| {
            value.clone().expect("present by predicate")
        })
}
