use phantom_core::{Err, Result, err, implement, matrix::pdu::gen_event_id};
use ruma::{CanonicalJsonObject, RoomVersionId};
use serde::de::IgnoredAny;

use super::{Op, Opts};

#[implement(super::Service)]
#[tracing::instrument(name = "validate", level = "trace", skip_all)]
pub(super) async fn validate(&self, opts: &Opts, bytes: &[u8]) -> Result {
    if opts.check_conforms {
        match opts.op {
            Op::Backfill => serde_json::from_slice(bytes)
                .map(|pdus: Vec<IgnoredAny>| !pdus.is_empty())
                .map_err(|e| err!(BadServerResponse("malformed federation response: {e}")))
                .and_then(|populated| {
                    populated
                        .then_some(())
                        .ok_or_else(|| err!(BadServerResponse("empty backfill response")))
                }),
            _ => serde_json::from_slice(bytes)
                .map(|_: IgnoredAny| ())
                .map_err(|e| err!(BadServerResponse("malformed federation response: {e}"))),
        }?;
    }

    let deep = opts.check_event_id || opts.check_hashes || opts.check_signature;
    if matches!(opts.op, Op::Event | Op::AuthEvent) && deep {
        self.verify_pdu(opts, bytes).await?;
    }

    Ok(())
}

#[implement(super::Service)]
#[tracing::instrument(level = "trace", skip_all)]
async fn verify_pdu(&self, opts: &Opts, bytes: &[u8]) -> Result {
    let value: CanonicalJsonObject = serde_json::from_slice(bytes)
        .map_err(|e| err!(BadServerResponse("PDU is not a canonical JSON object: {e}")))?;

    let v11 = RoomVersionId::V11;
    let room_version = opts.room_version.as_ref().unwrap_or(&v11);

    if opts.check_event_id
        && let Some(expected) = opts.event_id.as_ref()
    {
        let calculated = gen_event_id(&value, room_version)?;
        if calculated != *expected {
            return Err!(BadServerResponse("server returned the wrong event id"));
        }
    }

    if opts.check_signature || opts.check_hashes {
        self.services
            .server_keys
            .verify_event(&value, Some(room_version))
            .await?;
    }

    Ok(())
}
