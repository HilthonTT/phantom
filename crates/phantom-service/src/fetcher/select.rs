use std::sync::Arc;

use arrayvec::ArrayVec;
use async_trait::async_trait;
use futures::{Stream, StreamExt, future::Either, stream::empty};
use phantom_core::{
    implement,
    rand::index,
    stream::{IterStream, ReadyExt, Tools as StreamTools},
};
use ruma::{EventId, OwnedServerName, RoomId, ServerName};

use super::opts::{Op, Opts};
use crate::{
    Services,
    federation::{Candidates, WhenAllBackedOff},
    moderation::Restriction,
};

const ROUTE_FANOUT: usize = 5;

#[async_trait]
pub(super) trait Select: Send + Sync {
    async fn candidates(&self, opts: &Opts) -> Candidates;
}

pub(super) struct RoomCandidates {
    pub(super) services: Arc<Services>,
}

#[async_trait]
impl Select for RoomCandidates {
    #[tracing::instrument(
		level = "trace",
		skip_all,
		fields(
			room_id = ?opts.room_id,
		),
	)]
    async fn candidates(&self, opts: &Opts) -> Candidates {
        if !opts.candidates.is_empty() {
            return self.ranked_override(opts).await;
        }

        let authority = self.authority_server(opts).await;

        let mxid_hosts = [
            opts.event_id.as_deref().and_then(EventId::server_name),
            opts.room_id.as_deref().and_then(RoomId::server_name),
        ]
        .into_iter()
        .flatten()
        .map(ToOwned::to_owned);

        let popular = match opts.room_id.as_deref() {
            None => Either::Right(empty::<OwnedServerName>()),
            Some(room_id) => Either::Left(self.route_by_popularity(room_id).await),
        };

        let eligible = opts
            .hint
            .clone()
            .into_iter()
            .chain(authority)
            .stream()
            .chain(popular)
            .chain(mxid_hosts.stream())
            .ready_filter(|server| self.is_eligible(server));

        self.rank_unique(eligible).await
    }
}

#[implement(RoomCandidates)]
#[tracing::instrument(level = "trace", skip_all)]
async fn ranked_override(&self, opts: &Opts) -> Candidates {
    let eligible = opts
        .hint
        .iter()
        .chain(opts.candidates.iter())
        .filter(|&server| self.is_eligible(server))
        .cloned()
        .stream();

    self.rank_unique(eligible).await
}

#[implement(RoomCandidates)]
async fn rank_unique<S>(&self, eligible: S) -> Candidates
where
    S: Stream<Item = OwnedServerName> + Send,
{
    let ordered: Candidates = eligible.ready_fold(Candidates::new(), push_unique).await;

    self.services
        .federation
        .rank_candidates(ordered, WhenAllBackedOff::Attempt)
        .await
}

fn push_unique(mut ordered: Candidates, server: OwnedServerName) -> Candidates {
    if !ordered.contains(&server) {
        ordered.push(server);
    }

    ordered
}

#[implement(RoomCandidates)]
#[tracing::instrument(level = "trace", skip_all)]
async fn authority_server(&self, opts: &Opts) -> Option<OwnedServerName> {
    let room_id = opts.room_id.as_deref()?;

    if !matches!(opts.op, Op::AuthEvent | Op::AuthChain) {
        return None;
    }

    self.services
        .rooms
        .state_cache
        .most_powerful_user_server(room_id)
        .await
}

#[implement(RoomCandidates)]
#[tracing::instrument(level = "trace", skip_all)]
async fn route_by_popularity<'a>(
    &'a self,
    room_id: &'a RoomId,
) -> impl Stream<Item = OwnedServerName> + Send + 'a {
    let sampled: ArrayVec<OwnedServerName, ROUTE_FANOUT> = self
        .services
        .rooms
        .state_cache
        .room_members(room_id)
        .sample_by(|user| user.server_name().to_owned())
        .await;

    if sampled.is_empty() {
        return Either::Right(
            self.services
                .rooms
                .state_cache
                .room_servers(room_id)
                .map(ToOwned::to_owned),
        );
    }

    Either::Left(sampled.into_iter().stream())
}

#[implement(RoomCandidates)]
#[expect(dead_code)]
async fn route_uniformly<'a>(
    &'a self,
    room_id: &'a RoomId,
) -> impl Stream<Item = OwnedServerName> + Send + 'a {
    let count = self
        .services
        .rooms
        .state_cache
        .room_servers(room_id)
        .count()
        .await;

    let offset = index(count);

    self.services
        .rooms
        .state_cache
        .room_servers(room_id)
        .map(ToOwned::to_owned)
        .skip(offset)
        .take(ROUTE_FANOUT)
}

#[implement(RoomCandidates)]
fn is_eligible(&self, server: &ServerName) -> bool {
    !self.services.server_state.server_is_ours(server)
        && !self
            .services
            .moderation
            .forbids(server, Restriction::Federation)
}

#[cfg(test)]
mod tests {
    use ruma::owned_server_name;

    use super::{Candidates, push_unique};

    #[test]
    fn push_unique_keeps_first_occurrence() {
        let pool = [
            owned_server_name!("a.test"),
            owned_server_name!("b.test"),
            owned_server_name!("a.test"),
            owned_server_name!("c.test"),
            owned_server_name!("b.test"),
        ];

        let deduped: Candidates = pool.into_iter().fold(Candidates::new(), push_unique);

        let names: Vec<&str> = deduped.iter().map(AsRef::as_ref).collect();

        assert_eq!(names, ["a.test", "b.test", "c.test"]);
    }
}
