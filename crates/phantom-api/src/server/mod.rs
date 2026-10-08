mod access;
mod directory;
mod events;
mod key;
mod media;
mod membership;
mod openid;
mod transaction;
mod version;
mod well_known;

use axum::{
    Router,
    response::IntoResponse,
    routing::{any, get},
};
use phantom_core::{Config, err};

use self::{
    access::AccessCheck,
    directory::{hierarchy, publicrooms, query, user},
    events::{backfill, event, event_auth, get_missing_events, state, state_ids, timestamp},
    membership::{invite, make_join, make_knock, make_leave, send_join, send_knock, send_leave},
};
use crate::router::{RouterExt, State};

pub fn register(router: Router<State>, config: &Config) -> Router<State> {
    let router = router
        .ruma_route(&well_known::well_known_server)
        .ruma_route(&openid::get_openid_userinfo_route);

    if !config.federation.allow_federation {
        return router
            .route("/_matrix/federation/{*path}", any(federation_disabled))
            .route("/_matrix/key/{*path}", any(federation_disabled));
    }

    router
        .ruma_route(&version::get_server_version_route)
        .route("/_matrix/key/v2/server", get(key::get_server_keys_route))
        .ruma_route(&publicrooms::get_public_rooms_route)
        .ruma_route(&publicrooms::get_public_rooms_filtered_route)
        .ruma_route(&transaction::send_transaction_message_route)
        .ruma_route(&event::get_event_route)
        .ruma_route(&timestamp::get_event_by_timestamp_route)
        .ruma_route(&backfill::get_backfill_route)
        .ruma_route(&get_missing_events::get_missing_events_route)
        .ruma_route(&event_auth::get_event_authorization_route)
        .ruma_route(&state::get_room_state_route)
        .ruma_route(&state_ids::get_room_state_ids_route)
        .ruma_route(&make_leave::create_leave_event_template_route)
        .ruma_route(&make_knock::create_knock_event_template_route)
        .ruma_route(&make_join::create_join_event_template_route)
        .ruma_route(&send_leave::create_leave_event_v2_route)
        .ruma_route(&send_knock::create_knock_event_v1_route)
        .ruma_route(&send_join::create_join_event_v2_route)
        .ruma_route(&invite::create_invite_route)
        .ruma_route(&user::get_devices_route)
        .ruma_route(&query::get_room_information_route)
        .ruma_route(&query::get_profile_information_route)
        .ruma_route(&user::get_keys_route)
        .ruma_route(&user::claim_keys_route)
        .ruma_route(&hierarchy::get_hierarchy_route)
        .ruma_route(&media::get_content_route)
        .ruma_route(&media::get_content_thumbnail_route)
}

async fn federation_disabled() -> impl IntoResponse {
    err!(Request(Forbidden("Federation is disabled.")))
}
