use std::cmp;

use axum::extract::State;
use futures::{FutureExt, StreamExt, TryStreamExt};
use phantom_core::{
    Result,
    math::usize_from_ruma_bounded,
    matrix::pdu::PduCount,
    stream::{IterStream, ReadyExt},
};
use ruma::{MilliSecondsSinceUnixEpoch, api::federation::backfill::get_backfill};

use super::AccessCheck;
use crate::router::Ruma;

const LIMIT_MAX: usize = 150;
const LIMIT_DEFAULT: usize = 50;

pub(crate) async fn get_backfill_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_backfill::v1::Request>,
) -> Result<get_backfill::v1::Response> {
    AccessCheck {
        services: &services,
        origin: body.origin(),
        room_id: &body.room_id,
        event_id: None,
    }
    .check()
    .await?;

    let limit = usize_from_ruma_bounded(body.limit, LIMIT_DEFAULT, LIMIT_MAX);

    let from = body
        .v
        .iter()
        .stream()
        .filter_map(|event_id| {
            services
                .rooms
                .timeline
                .get_pdu_count(event_id)
                .map(Result::ok)
        })
        .ready_fold(PduCount::min(), cmp::max)
        .await;

    let room_version = services
        .rooms
        .state
        .get_room_version(&body.room_id)
        .await
        .ok();

    let pdus = services
        .rooms
        .timeline
        .pdus_rev(None, &body.room_id, Some(from.saturating_add(1)))
        .try_filter_map(async |(_, pdu)| {
            Ok(services
                .rooms
                .state_accessor
                .server_can_see_event(body.origin(), &pdu.room_id, &pdu.event_id)
                .await
                .then_some(pdu))
        })
        .try_filter_map(async |pdu| {
            Ok(services
                .rooms
                .timeline
                .get_pdu_json(&pdu.event_id)
                .await
                .ok())
        })
        .take(limit)
        .and_then(|pdu| {
            services
                .federation
                .format_pdu(pdu, room_version.as_ref())
                .map(Ok)
        })
        .try_collect()
        .boxed()
        .await?;

    Ok(get_backfill::v1::Response::new(
        services.server_state.server_name().to_owned(),
        MilliSecondsSinceUnixEpoch::now(),
        pdus,
    ))
}
