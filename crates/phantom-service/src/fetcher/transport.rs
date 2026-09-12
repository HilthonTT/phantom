use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use phantom_core::{Err, Result, err};
use ruma::{
    MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedRoomId, ServerName, UInt,
    api::federation::{
        authorization::get_event_authorization::v1::Request as EventAuthRequest,
        backfill::get_backfill::v1::Request as BackfillRequest,
        event::{
            get_event::v1::Request as EventRequest,
            get_event_by_timestamp::v1::Request as TimestampRequest,
            get_missing_events::v1::Request as MissingEventsRequest,
            get_room_state_ids::v1::Request as StateIdsRequest,
        },
    },
};

use super::opts::{Op, Opts};
use crate::Services;

#[async_trait]
pub(super) trait Transport: Send + Sync {
    async fn fetch_raw(&self, op: Op, server: &ServerName, opts: &Opts) -> Result<Bytes>;
}

pub(super) struct FederationTransport {
    pub(super) services: Arc<Services>,
}

#[async_trait]
impl Transport for FederationTransport {
    #[tracing::instrument(
		level = "debug",
		skip(self, opts),
		fields(
			%server,
		),
	)]
    async fn fetch_raw(&self, op: Op, server: &ServerName, opts: &Opts) -> Result<Bytes> {
        let federation = &self.services.federation;

        match op {
            Op::Event | Op::AuthEvent => {
                let event_id = require_event_id(opts)?;
                let res = federation
                    .execute(server, EventRequest::new(event_id))
                    .await?;

                Ok(Bytes::copy_from_slice(res.pdu.get().as_bytes()))
            }
            Op::AuthChain => {
                let event_id = require_event_id(opts)?;
                let room_id = require_room_id(opts)?;
                let res = federation
                    .execute(server, EventAuthRequest::new(room_id, event_id))
                    .await?;

                to_bytes(&res.auth_chain)
            }
            Op::Backfill => {
                let event_id = require_event_id(opts)?;
                let room_id = require_room_id(opts)?;
                let res = federation
                    .execute(
                        server,
                        BackfillRequest::new(room_id, vec![event_id], batch_limit(opts)),
                    )
                    .await?;

                to_bytes(&res.pdus)
            }
            Op::StateIds => {
                let event_id = require_event_id(opts)?;
                let room_id = require_room_id(opts)?;
                let res = federation
                    .execute(server, StateIdsRequest::new(event_id, room_id))
                    .await?;

                to_bytes(&serde_json::json!({
                    "auth_chain_ids": res.auth_chain_ids,
                    "pdu_ids": res.pdu_ids,
                }))
            }
            Op::MissingEvents => {
                require_latest_events(opts)?;
                let room_id = require_room_id(opts)?;
                let req = MissingEventsRequest::new(
                    room_id,
                    opts.earliest_events.to_vec(),
                    opts.latest_events.to_vec(),
                );

                let res = federation.execute(server, req).await?;

                to_bytes(&res.events)
            }
            Op::TimestampToEvent => {
                let room_id = require_room_id(opts)?;
                let ts = require_ts(opts)?;
                let res = federation
                    .execute(
                        server,
                        TimestampRequest::new(room_id, ts, opts.dir.unwrap_or_default()),
                    )
                    .await?;

                to_bytes(&serde_json::json!({
                    "event_id": res.event_id,
                    "origin_server_ts": res.origin_server_ts,
                }))
            }
        }
    }
}

fn require_event_id(opts: &Opts) -> Result<OwnedEventId> {
    opts.event_id.clone().ok_or_else(|| {
        err!(Request(InvalidParam(
            "event_id is required for op {:?}",
            opts.op
        )))
    })
}

fn require_room_id(opts: &Opts) -> Result<OwnedRoomId> {
    opts.room_id.clone().ok_or_else(|| {
        err!(Request(InvalidParam(
            "room_id is required for op {:?}",
            opts.op
        )))
    })
}

fn require_ts(opts: &Opts) -> Result<MilliSecondsSinceUnixEpoch> {
    opts.ts
        .ok_or_else(|| err!(Request(InvalidParam("ts is required for op {:?}", opts.op))))
}

fn require_latest_events(opts: &Opts) -> Result {
    if opts.latest_events.is_empty() {
        return Err!(Request(InvalidParam(
            "latest_events is required for op {:?}",
            opts.op
        )));
    }

    Ok(())
}

fn batch_limit(opts: &Opts) -> UInt {
    opts.backfill_limit.map_or_else(
        || UInt::from(10_u8),
        |n| UInt::new_saturating(u64::try_from(n.get()).unwrap_or(u64::MAX)),
    )
}

fn to_bytes<T: serde::Serialize>(value: &T) -> Result<Bytes> {
    serde_json::to_vec(value).map(Bytes::from).map_err(|e| {
        err!(BadServerResponse(
            "failed to re-encode federation response: {e}"
        ))
    })
}
