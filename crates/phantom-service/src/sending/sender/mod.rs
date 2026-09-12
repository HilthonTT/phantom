mod dispatch;
mod edus;
mod select;

use std::{
    collections::HashMap,
    fmt::Debug,
    sync::Arc,
    time::{Duration, Instant},
};

use super::{Destination, Msg, SendingEvent, Service};
use futures::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use phantom_core::{Error, Result, debug, error, trace, tracing, warn};

#[derive(Debug)]
enum TransactionStatus {
    Running,
    Failed(u32, Instant),
    Retrying(u32),
}

type SendingError = (Destination, Error);
type SendingResult = Result<Destination, SendingError>;
type SendingFuture<'a> = BoxFuture<'a, SendingResult>;
type SendingFutures<'a> = FuturesUnordered<SendingFuture<'a>>;
type CurTransactionStatus = HashMap<Destination, TransactionStatus>;

const DEQUEUE_LIMIT: usize = 48;

pub const EDU_LIMIT: usize = 100;

impl Service {
    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn sender(self: Arc<Self>, id: usize) -> Result {
        let mut statuses: CurTransactionStatus = CurTransactionStatus::new();
        let mut futures: SendingFutures<'_> = FuturesUnordered::new();

        self.startup_netburst(id, &mut futures, &mut statuses)
            .boxed()
            .await;

        self.work_loop(id, &mut futures, &mut statuses).await;

        if !futures.is_empty() {
            self.finish_responses(&mut futures).boxed().await;
        }

        Ok(())
    }

    #[tracing::instrument(
        name = "work",
        level = "trace",
        skip_all,
        fields(
            futures = %futures.len(),
            statuses = %statuses.len(),
        ),
    )]
    async fn work_loop<'a>(
        &'a self,
        id: usize,
        futures: &mut SendingFutures<'a>,
        statuses: &mut CurTransactionStatus,
    ) {
        let receiver = self
            .channels
            .get(id)
            .map(|(_, receiver)| receiver.clone())
            .expect("Missing channel for sender worker");

        while !receiver.is_closed() {
            tokio::select! {
                Some(response) = futures.next() => {
                    self.handle_response(response, futures, statuses).await;
                },
                request = receiver.recv() => match request {
                    Ok(request) => self.handle_request(request, futures, statuses).await,
                    Err(_) => return,
                },
            }
        }
    }

    #[tracing::instrument(name = "response", level = "debug", skip_all)]
    async fn handle_response<'a>(
        &'a self,
        response: SendingResult,
        futures: &mut SendingFutures<'a>,
        statuses: &mut CurTransactionStatus,
    ) {
        match response {
            Ok(dest) => self.handle_response_ok(&dest, futures, statuses).await,
            Err((dest, e)) => Self::handle_response_err(dest, statuses, &e),
        }
    }

    fn handle_response_err(dest: Destination, statuses: &mut CurTransactionStatus, e: &Error) {
        debug!(dest = ?dest, "{e:?}");
        statuses.entry(dest).and_modify(|e| {
            *e = match e {
                TransactionStatus::Running => TransactionStatus::Failed(1, Instant::now()),
                &mut TransactionStatus::Retrying(ref n) => {
                    TransactionStatus::Failed(n.saturating_add(1), Instant::now())
                }
                TransactionStatus::Failed(..) => {
                    panic!("Request that was not even running failed?!")
                }
            }
        });
    }

    #[allow(clippy::needless_pass_by_ref_mut)]
    async fn handle_response_ok<'a>(
        &'a self,
        dest: &Destination,
        futures: &mut SendingFutures<'a>,
        statuses: &mut CurTransactionStatus,
    ) {
        let _cork = self.db.db.engine.cork_guard();
        self.db.delete_all_active_requests_for(dest).await;

        let new_events = self
            .db
            .queued_requests(dest)
            .take(DEQUEUE_LIMIT)
            .collect::<Vec<_>>()
            .await;

        if !new_events.is_empty() {
            if let Err(e) = self.db.mark_as_active(new_events.iter()) {
                error!(?dest, "Failed to mark queued events as active: {e}");
            }

            let new_events_vec = new_events.into_iter().map(|(_, event)| event).collect();
            futures.push(self.send_events(dest.clone(), new_events_vec));
        } else {
            statuses.remove(dest);
        }
    }

    #[allow(clippy::needless_pass_by_ref_mut)]
    #[tracing::instrument(name = "request", level = "debug", skip_all)]
    async fn handle_request<'a>(
        &'a self,
        msg: Msg,
        futures: &mut SendingFutures<'a>,
        statuses: &mut CurTransactionStatus,
    ) {
        let iv = vec![(msg.queue_id, msg.event)];
        if let Ok(Some(events)) = self.select_events(&msg.dest, iv, statuses).await {
            if !events.is_empty() {
                futures.push(self.send_events(msg.dest, events));
            } else {
                statuses.remove(&msg.dest);
            }
        }
    }

    #[tracing::instrument(
        name = "finish",
        level = "info",
        skip_all,
        fields(futures = %futures.len()),
    )]
    async fn finish_responses<'a>(&'a self, futures: &mut SendingFutures<'a>) {
        use tokio::{
            select,
            time::{Instant, sleep_until},
        };

        let timeout = self.server.config.network.sender_shutdown_timeout;
        let timeout = Duration::from_secs(timeout);
        let now = Instant::now();
        let deadline = now.checked_add(timeout).unwrap_or(now);
        loop {
            trace!("Waiting for {} requests to complete...", futures.len());
            select! {
                () = sleep_until(deadline) => return,
                response = futures.next() => match response {
                    Some(Ok(dest)) => self.db.delete_all_active_requests_for(&dest).await,
                    Some(_) => continue,
                    None => return,
                },
            }
        }
    }

    #[tracing::instrument(
        name = "netburst",
        level = "debug",
        skip_all,
        fields(futures = %futures.len()),
    )]
    #[allow(clippy::needless_pass_by_ref_mut)]
    async fn startup_netburst<'a>(
        &'a self,
        id: usize,
        futures: &mut SendingFutures<'a>,
        statuses: &mut CurTransactionStatus,
    ) {
        let keep =
            usize::try_from(self.server.config.network.startup_netburst_keep).unwrap_or(usize::MAX);
        let mut txns = HashMap::<Destination, Vec<SendingEvent>>::new();
        let mut active = self.db.active_requests().boxed();

        while let Some((key, event, dest)) = active.next().await {
            if self.shard_id(&dest) != id {
                continue;
            }

            let entry = txns.entry(dest.clone()).or_default();
            if self.server.config.network.startup_netburst_keep >= 0 && entry.len() >= keep {
                warn!(
                    "Dropping unsent event {dest:?} {:?}",
                    String::from_utf8_lossy(&key)
                );
                self.db.delete_active_request(&key);
            } else {
                entry.push(event);
            }
        }

        for (dest, events) in txns {
            if self.server.config.network.startup_netburst && !events.is_empty() {
                statuses.insert(dest.clone(), TransactionStatus::Running);
                futures.push(self.send_events(dest.clone(), events));
            }
        }
    }
}
