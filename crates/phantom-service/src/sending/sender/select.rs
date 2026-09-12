use super::{CurTransactionStatus, EDU_LIMIT, TransactionStatus};
use crate::sending::{Destination, SendingEvent, Service, data::QueueItem};
use phantom_core::{
    Result, stream::ReadyExt, time::exponential_backoff::continue_exponential_backoff_secs, tracing,
};

impl Service {
    #[tracing::instrument(
        name = "select",
        level = "debug",
        skip_all,
        fields(
            ?dest,
            new_events = %new_events.len(),
        ),
    )]
    pub(super) async fn select_events(
        &self,
        dest: &Destination,
        new_events: Vec<QueueItem>,
        statuses: &mut CurTransactionStatus,
    ) -> Result<Option<Vec<SendingEvent>>> {
        let (allow, retry) = self.select_events_current(dest, statuses)?;

        if !allow {
            return Ok(None);
        }

        let _cork = self.db.db.engine.cork_guard();
        let mut events = Vec::new();

        if retry {
            self.db
                .active_requests_for(dest)
                .ready_for_each(|(_, e)| events.push(e))
                .await;

            return Ok(Some(events));
        }

        let _cork = self.db.db.engine.cork_guard();
        if !new_events.is_empty() {
            self.db.mark_as_active(new_events.iter())?;
            for (_, e) in new_events {
                events.push(e);
            }
        }

        if let Destination::Federation(server_name) = dest
            && let Ok((select_edus, last_count)) = self.select_edus(server_name).await
        {
            debug_assert!(select_edus.len() <= EDU_LIMIT, "exceeded edus limit");
            let select_edus = select_edus.into_iter().map(SendingEvent::Edu);

            events.extend(select_edus);
            self.db.set_latest_educount(server_name, last_count)?;
        }

        Ok(Some(events))
    }

    fn select_events_current(
        &self,
        dest: &Destination,
        statuses: &mut CurTransactionStatus,
    ) -> Result<(bool, bool)> {
        let (mut allow, mut retry) = (true, false);
        statuses
            .entry(dest.clone())
            .and_modify(|e| match e {
                TransactionStatus::Failed(tries, time) => {
                    let min = self.server.config.network.sender_timeout;
                    let max = self.server.config.network.sender_retry_backoff_limit;
                    if continue_exponential_backoff_secs(min, max, time.elapsed(), *tries)
                        && !matches!(dest, Destination::Appservice(_))
                    {
                        allow = false;
                    } else {
                        retry = true;
                        *e = TransactionStatus::Retrying(*tries);
                    }
                }
                TransactionStatus::Running | TransactionStatus::Retrying(_) => {
                    allow = false;
                }
            })
            .or_insert(TransactionStatus::Running);

        Ok((allow, retry))
    }
}
