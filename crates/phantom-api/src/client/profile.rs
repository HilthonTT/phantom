use axum::extract::State;
use futures::{StreamExt, future::join3};
use phantom_core::{Err, Result, err, matrix::pdu::PduBuilder, warn};
use phantom_service::Services;
use ruma::{
    OwnedMxcUri, OwnedRoomId, UserId,
    api::{
        client::profile::{
            delete_profile_field, get_profile,
            get_profile_field::{self, v3::Response as GetProfileFieldResponse},
            set_profile_field,
        },
        federation::query::get_profile_information,
    },
    events::room::member::RoomMemberEventContent,
    profile::{ProfileFieldName, ProfileFieldValue},
};
use serde_json::Value as JsonValue;

use crate::{
    client::utils::{may_set_displayname, ping_presence},
    router::Ruma,
};

/// Profile key holding the time zone under its unstable MSC4175 name, where
/// the profile service keeps it.
const TIMEZONE_KEY: &str = "us.cloke.msc4175.tz";

/// # `GET /_matrix/client/v3/profile/{userId}`
///
/// Returns the displayname, avatar_url, blurhash, tz and custom fields of the
/// user.
///
/// - A remote user's profile is fetched over federation.
pub(crate) async fn get_profile_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_profile::v3::Request>,
) -> Result<get_profile::v3::Response> {
    shared_rooms_check(&services, &body, &body.user_id).await?;

    if !services.server_state.user_is_local(&body.user_id) {
        let response = fetch_remote_profile(&services, &body.user_id, None).await?;

        return Ok(response.into_iter().collect());
    }

    if !services.users.exists(&body.user_id).await {
        return Err!(Request(NotFound("Profile was not found.")));
    }

    Ok(local_profile(&services, &body.user_id).await)
}

/// # `GET /_matrix/client/v3/profile/{userId}/{field}`
///
/// Gets the profile key-value field of a user, as per MSC4133.
///
/// - An unset `displayname` or `avatar_url` is a 200 with the field omitted, as
///   before Matrix 1.16; other unset fields are a 404 per MSC4133.
pub(crate) async fn get_profile_field_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_profile_field::v3::Request>,
) -> Result<GetProfileFieldResponse> {
    shared_rooms_check(&services, &body, &body.user_id).await?;

    let value = if services.server_state.user_is_local(&body.user_id) {
        if !services.users.exists(&body.user_id).await {
            return Err!(Request(NotFound("Profile was not found.")));
        }

        get_field(&services, &body.user_id, &body.field).await
    } else {
        fetch_remote_profile(&services, &body.user_id, Some(body.field.clone()))
            .await?
            .get(body.field.as_str())
            .cloned()
    };

    let legacy = matches!(
        body.field,
        ProfileFieldName::AvatarUrl | ProfileFieldName::DisplayName
    );

    let value = match value {
        Some(value) => Some(
            ProfileFieldValue::new(body.field.as_str(), value)
                .map_err(|e| err!(Database("Invalid profile value for {}: {e}", body.field)))?,
        ),
        None if legacy => None,
        None => return Err!(Request(NotFound("Profile field was not found."))),
    };

    let mut response = GetProfileFieldResponse::default();
    response.value = value;

    Ok(response)
}

/// # `PUT /_matrix/client/v3/profile/{user_id}/{field}`
///
/// Updates the profile key-value field of a user, as per MSC4133.
///
/// A new `displayname` or `avatar_url` is also sent to every room the user
/// has joined.
pub(crate) async fn set_profile_field_route(
    State(services): State<crate::router::State>,
    body: Ruma<set_profile_field::v3::Request>,
) -> Result<set_profile_field::v3::Response> {
    let field = body.value.field_name();

    write_check(&services, &body, &body.user_id, &field).await?;

    let value = body.value.value().into_owned();
    set_field(&services, &body.user_id, &field, Some(value)).await?;

    ping_presence(&services, &body, &body.user_id).await?;

    Ok(set_profile_field::v3::Response::new())
}

/// # `DELETE /_matrix/client/v3/profile/{user_id}/{field}`
///
/// Deletes the profile key-value field of a user, as per MSC4133.
///
/// A cleared `displayname` or `avatar_url` is also sent to every room the
/// user has joined.
pub(crate) async fn delete_profile_field_route(
    State(services): State<crate::router::State>,
    body: Ruma<delete_profile_field::v3::Request>,
) -> Result<delete_profile_field::v3::Response> {
    write_check(&services, &body, &body.user_id, &body.field).await?;

    set_field(&services, &body.user_id, &body.field, None).await?;

    ping_presence(&services, &body, &body.user_id).await?;

    Ok(delete_profile_field::v3::Response::new())
}

/// Collects every field of a local user's profile.
pub(crate) async fn local_profile(
    services: &Services,
    user_id: &UserId,
) -> get_profile::v3::Response {
    let profile = &services.profile;
    let (displayname, avatar_url, blurhash) = join3(
        profile.displayname(user_id),
        profile.avatar_url(user_id),
        profile.blurhash(user_id),
    )
    .await;

    let mut response: get_profile::v3::Response = profile
        .all_profile_keys(user_id)
        .map(|(key, value)| match key.as_str() {
            TIMEZONE_KEY => (ProfileFieldName::TimeZone.as_str().to_owned(), value),
            _ => (key, value),
        })
        .collect()
        .await;

    let legacy = [
        (
            ProfileFieldName::DisplayName,
            displayname.ok().map(JsonValue::from),
        ),
        (
            ProfileFieldName::AvatarUrl,
            avatar_url.ok().map(|url| JsonValue::from(url.to_string())),
        ),
        (
            ProfileFieldName::from("xyz.amorgan.blurhash"),
            blurhash.ok().map(JsonValue::from),
        ),
    ];

    for (field, value) in legacy {
        if let Some(value) = value {
            response.set(field.as_str().to_owned(), value);
        }
    }

    response
}

/// Reads one field of a local user's profile.
async fn get_field(
    services: &Services,
    user_id: &UserId,
    field: &ProfileFieldName,
) -> Option<JsonValue> {
    let profile = &services.profile;

    match field {
        ProfileFieldName::DisplayName => profile.displayname(user_id).await.ok().map(Into::into),
        ProfileFieldName::AvatarUrl => profile
            .avatar_url(user_id)
            .await
            .ok()
            .map(|url| url.to_string().into()),
        ProfileFieldName::TimeZone => profile.timezone(user_id).await.ok().map(Into::into),
        _ => profile.profile_key(user_id, field.as_str()).await.ok(),
    }
}

/// Writes or clears one field of a local user's profile.
///
/// A display name or avatar change is sent on to the user's joined rooms.
async fn set_field(
    services: &Services,
    user_id: &UserId,
    field: &ProfileFieldName,
    value: Option<JsonValue>,
) -> Result {
    let profile = &services.profile;

    match field {
        ProfileFieldName::DisplayName => {
            let displayname = value.map(string_value).transpose()?;
            profile.set_displayname(user_id, displayname);
            update_member_events(services, user_id).await;
        }
        ProfileFieldName::AvatarUrl => {
            let avatar_url = value.map(string_value).transpose()?.map(OwnedMxcUri::from);
            profile.set_avatar_url(user_id, avatar_url);
            update_member_events(services, user_id).await;
        }
        ProfileFieldName::TimeZone => {
            let timezone = value.map(string_value).transpose()?;
            profile.set_timezone(user_id, timezone);
        }
        _ => profile.set_profile_key(user_id, field.as_str(), value),
    }

    Ok(())
}

fn string_value(value: JsonValue) -> Result<String> {
    match value {
        JsonValue::String(value) => Ok(value),
        _ => Err!(Request(BadJson("Profile field value must be a string."))),
    }
}

/// Sends the user's current display name and avatar into every joined room.
///
/// A room that refuses the update is logged and skipped; the profile itself is
/// already saved.
pub(crate) async fn update_member_events(services: &Services, user_id: &UserId) {
    let rooms: Vec<OwnedRoomId> = services
        .rooms
        .state_cache
        .rooms_joined(user_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    let (displayname, avatar_url) = futures::join!(
        services.profile.displayname(user_id),
        services.profile.avatar_url(user_id),
    );

    for room_id in rooms {
        let Ok(mut content) = services
            .rooms
            .state_accessor
            .room_state_get_content::<RoomMemberEventContent>(
                &room_id,
                &ruma::events::StateEventType::RoomMember,
                user_id.as_str(),
            )
            .await
        else {
            continue;
        };

        content.displayname = displayname.as_ref().ok().cloned();
        content.avatar_url = avatar_url.as_ref().ok().cloned();
        content.join_authorized_via_users_server = None;

        let state_lock = services.rooms.state.mutex.lock(&*room_id).await;

        if let Err(e) = services
            .rooms
            .timeline
            .build_and_append_pdu(
                PduBuilder::state(user_id.to_string(), &content),
                user_id,
                &room_id,
                &state_lock,
            )
            .await
        {
            warn!(%room_id, "Failed to update {user_id} profile in room: {e}");
        }
    }
}

/// Queries a remote user's profile over federation.
async fn fetch_remote_profile(
    services: &Services,
    user_id: &UserId,
    field: Option<ProfileFieldName>,
) -> Result<get_profile_information::v1::Response> {
    let mut request = get_profile_information::v1::Request::new(user_id.to_owned());
    request.field = field;

    services
        .federation
        .execute(user_id.server_name(), request)
        .await
        .map_err(|e| err!(Request(NotFound("Profile was not found: {e}"))))
}

/// Refuses a profile write the caller may not make.
///
/// Only the user or an appservice owning their namespace may write, and
/// display name writes are further gated by `enable_set_displayname`.
async fn write_check<T>(
    services: &Services,
    body: &Ruma<T>,
    user_id: &UserId,
    field: &ProfileFieldName,
) -> Result
where
    T: Sync,
{
    if body.sender_user() != user_id
        && !body
            .appservice_info
            .as_ref()
            .is_some_and(|registration| registration.is_user_match(user_id))
    {
        return Err!(Request(Forbidden(
            "You cannot update the profile of another user"
        )));
    }

    let is_admin = || services.admin.user_is_admin(body.sender_user());

    if *field == ProfileFieldName::DisplayName
        && !may_set_displayname(services, body, is_admin).await
    {
        return Err!(Request(Forbidden(
            "Setting display names has been disabled."
        )));
    }

    Ok(())
}

/// Refuses a profile read withheld by
/// `limit_profile_requests_to_users_who_share_rooms`.
///
/// Appservices and a user reading their own profile are exempt. The refusal
/// precedes the existence check, so it discloses nothing about the profile.
async fn shared_rooms_check<T>(services: &Services, body: &Ruma<T>, user_id: &UserId) -> Result
where
    T: Sync,
{
    if !services
        .config
        .client
        .limit_profile_requests_to_users_who_share_rooms
        || body.appservice_info.is_some()
    {
        return Ok(());
    }

    let visible = match body.sender_user.as_deref() {
        None => false,
        Some(sender_user) if sender_user == user_id => true,
        Some(sender_user) => {
            services
                .rooms
                .state_cache
                .user_sees_user(sender_user, user_id)
                .await
        }
    };

    if !visible {
        return Err!(Request(Forbidden("Profile isn't available.")));
    }

    Ok(())
}
