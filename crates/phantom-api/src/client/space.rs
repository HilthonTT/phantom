use axum::extract::State;
use phantom_core::Result;
use phantom_service::rooms::spaces::Asker;
use ruma::{UInt, api::client::space::get_hierarchy};

use crate::router::Ruma;

/// # `GET /_matrix/client/v1/rooms/{room_id}/hierarchy`
///
/// Paginates over the space tree in a depth-first manner to locate child rooms
/// of a given space.
pub(crate) async fn get_hierarchy_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_hierarchy::v1::Request>,
) -> Result<get_hierarchy::v1::Response> {
    let limit = body
        .limit
        .unwrap_or_else(|| UInt::from(10_u32))
        .min(UInt::from(100_u32));

    let max_depth = body
        .max_depth
        .unwrap_or_else(|| UInt::from(3_u32))
        .min(UInt::from(10_u32));

    let hierarchy = services
        .rooms
        .spaces
        .client_hierarchy(
            Asker::User(body.sender_user()),
            &body.room_id,
            limit.into(),
            max_depth.into(),
            body.suggested_only,
            body.from.as_deref(),
        )
        .await?;

    let mut response = get_hierarchy::v1::Response::new();
    response.rooms = hierarchy.rooms;
    response.next_batch = hierarchy.next_batch;

    Ok(response)
}
