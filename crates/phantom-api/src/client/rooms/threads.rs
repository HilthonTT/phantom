use axum::extract::State;
use futures::StreamExt;
use phantom_core::{
    Err, Result, at,
    matrix::pdu::{PduCount, PduEvent},
    result::FlatOk,
    stream::WidebandExt,
};

use ruma::api::client::threads::get_threads;

use crate::{client::message::user_can_see_room, router::Ruma};

/// # `GET /_matrix/client/r0/rooms/{roomId}/threads`
pub(crate) async fn get_threads_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_threads::v1::Request>,
) -> Result<get_threads::v1::Response> {
    let sender_user = body.sender_user();
    let room_id = &body.room_id;

    if !services.rooms.metadata.exists(room_id).await {
        return Err!(Request(Forbidden("Room does not exist to this server")));
    }

    if !user_can_see_room(&services, sender_user, room_id).await {
        return Err!(Request(Forbidden(
            "You don't have permission to view this room."
        )));
    }

    // Use limit or else 10, with maximum 100
    let limit = body
        .limit
        .map(usize::try_from)
        .flat_ok()
        .unwrap_or(10)
        .min(100);

    let from: PduCount = body
        .from
        .as_deref()
        .map(str::parse)
        .transpose()?
        .unwrap_or_else(PduCount::max);

    // One extra row probes whether the list continues past this page. Threads
    // rooted by a user the requester ignores are left out.
    let mut threads: Vec<(PduCount, PduEvent)> = services
        .rooms
        .threads
        .threads_until(sender_user, room_id, from, &body.include)
        .await?
        .wide_filter_map(async |(count, pdu)| {
            services
                .rooms
                .state_accessor
                .user_can_see_event(sender_user, room_id, &pdu.event_id)
                .await
                .then_some((count, pdu))
        })
        .wide_filter_map(async |(count, pdu)| {
            (!services
                .users
                .user_is_ignored(&pdu.sender, sender_user)
                .await)
                .then_some((count, pdu))
        })
        .take(limit.saturating_add(1))
        .collect()
        .await;

    let more = threads.len() > limit;

    threads.truncate(limit);

    let next_batch = threads
        .last()
        .filter(|_| more)
        .map(at!(0))
        .as_ref()
        .map(ToString::to_string);

    let mut response = get_threads::v1::Response::new(
        threads
            .into_iter()
            .map(at!(1))
            .map(PduEvent::into_room_event)
            .collect(),
    );
    response.next_batch = next_batch;

    Ok(response)
}
