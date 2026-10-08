use axum::extract::State;
use phantom_core::{Err, Result};
use phantom_service::{Services, rooms::directory::PublicRoomsPage};
use ruma::{
    UInt,
    api::federation::directory::{get_public_rooms, get_public_rooms_filtered},
    directory::Filter,
};

use crate::router::{ClientIp, Ruma};

#[tracing::instrument(name = "publicrooms", level = "debug", skip_all, fields(%client))]
pub(crate) async fn get_public_rooms_filtered_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_public_rooms_filtered::v1::Request>,
) -> Result<get_public_rooms_filtered::v1::Response> {
    let page =
        public_rooms_page(&services, body.limit, body.since.as_deref(), &body.filter).await?;

    let mut response = get_public_rooms_filtered::v1::Response::new();
    response.chunk = page.chunk;
    response.prev_batch = page.prev_batch;
    response.next_batch = page.next_batch;
    response.total_room_count_estimate = page.total_room_count_estimate;

    Ok(response)
}

#[tracing::instrument(name = "publicrooms", level = "debug", skip_all, fields(%client))]
pub(crate) async fn get_public_rooms_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_public_rooms::v1::Request>,
) -> Result<get_public_rooms::v1::Response> {
    let page = public_rooms_page(
        &services,
        body.limit,
        body.since.as_deref(),
        &Filter::default(),
    )
    .await?;

    let mut response = get_public_rooms::v1::Response::new();
    response.chunk = page.chunk;
    response.prev_batch = page.prev_batch;
    response.next_batch = page.next_batch;
    response.total_room_count_estimate = page.total_room_count_estimate;

    Ok(response)
}

async fn public_rooms_page(
    services: &Services,
    limit: Option<UInt>,
    since: Option<&str>,
    filter: &Filter,
) -> Result<PublicRoomsPage> {
    if !services
        .server
        .config
        .federation
        .allow_public_room_directory_over_federation
    {
        return Err!(Request(Forbidden("Room directory is not public")));
    }

    services
        .rooms
        .directory
        .public_rooms_page(limit, since, filter)
        .await
}
