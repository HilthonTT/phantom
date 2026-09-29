use axum::extract::State;
use futures::{StreamExt, future::ready};
use phantom_core::{Err, Result, err, rand};
use phantom_service::Services;
use ruma::{
    OwnedServerName, UserId,
    api::federation::query::{get_profile_information, get_room_information},
};

use serde_json::Value as JsonValue;

use crate::router::Ruma;

pub(crate) async fn get_room_information_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_room_information::v1::Request>,
) -> Result<get_room_information::v1::Response> {
    let room_id = services
        .rooms
        .alias
        .resolve_local_alias(&body.room_alias)
        .await
        .map_err(|_| err!(Request(NotFound("Room alias not found."))))?;

    let mut servers: Vec<OwnedServerName> = services
        .rooms
        .state_cache
        .room_servers(&room_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    servers.sort_unstable();
    servers.dedup();

    rand::shuffle(&mut servers);

    if let Some(server_index) = servers
        .iter()
        .position(|server| server == services.server_state.server_name())
    {
        servers.swap_remove(server_index);
        servers.insert(0, services.server_state.server_name().to_owned());
    }

    Ok(get_room_information::v1::Response::new(room_id, servers))
}

pub(crate) async fn get_profile_information_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_profile_information::v1::Request>,
) -> Result<get_profile_information::v1::Response> {
    if !services
        .server
        .config
        .federation
        .allow_inbound_profile_lookup_federation_requests
    {
        return Err!(Request(Forbidden(
            "Profile lookup over federation is not allowed on this homeserver.",
        )));
    }

    if !services.server_state.user_is_local(&body.user_id) {
        return Err!(Request(InvalidParam(
            "User does not belong to this server.",
        )));
    }

    if !services.users.exists(&body.user_id).await {
        return Err!(Request(NotFound("Profile was not found.")));
    }

    let mut response = get_profile_information::v1::Response::new();

    match &body.field {
        Some(field) => {
            if let Some(value) = profile_field(&services, &body.user_id, field.as_str()).await {
                response.set(field.to_string(), value);
            }
        }
        None => {
            for field in DEDICATED_PROFILE_FIELDS {
                if let Some(value) = profile_field(&services, &body.user_id, field).await {
                    response.set(field.to_owned(), value);
                }
            }

            services
                .profile
                .all_profile_keys(&body.user_id)
                .for_each(|(field, value)| {
                    response.set(field, value);
                    ready(())
                })
                .await;
        }
    }

    Ok(response)
}

/// Profile fields kept in their own tables rather than the generic profile-key
/// table.
const DEDICATED_PROFILE_FIELDS: [&str; 3] = ["displayname", "avatar_url", BLURHASH_FIELD];

const BLURHASH_FIELD: &str = "xyz.amorgan.blurhash";

async fn profile_field(services: &Services, user_id: &UserId, field: &str) -> Option<JsonValue> {
    let profile = &services.profile;

    match field {
        "displayname" => profile.displayname(user_id).await.ok().map(Into::into),
        "avatar_url" => profile
            .avatar_url(user_id)
            .await
            .ok()
            .map(|url| url.to_string().into()),
        BLURHASH_FIELD => profile.blurhash(user_id).await.ok().map(Into::into),
        field => profile.profile_key(user_id, field).await.ok(),
    }
}
