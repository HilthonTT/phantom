mod account;
mod account_data;
mod alias;
mod appservice;
mod backup;
mod capabilities;
mod context;
mod device;
mod directory;
mod events;
mod filter;
mod keys;
mod media;
mod media_legacy;
mod membership;
mod message;
mod openid;
mod phantom;
mod presence;
mod profile;
mod push;
mod read_marker;
mod redact;
mod register;
mod relations;
mod report;
mod room;
mod search;
mod send;
mod session;
mod space;
mod state;
mod sync;
mod tag;
mod thirdparty;
mod threads;
mod to_device;
mod typing;
mod unstable;
mod user_directory;
mod utils;
mod versions;
mod voip;
mod well_known;

use axum::{Router, routing::get};
use phantom_core::{Config, matrix::Event};
use phantom_service::{Services, rooms::timeline::PdusIterItem};
use ruma::UserId;

pub(crate) use self::message::{annotate_membership, is_ignored_pdu, with_membership};
use crate::router::{RouterExt, State};

/// generated device ID length
const DEVICE_ID_LENGTH: usize = 10;

/// generated user access token length
const TOKEN_LENGTH: usize = phantom_service::accounts::users::TOKEN_LENGTH;

/// generated user session ID length
const SESSION_ID_LENGTH: usize = phantom_service::auth::uiaa::SESSION_ID_LENGTH;

pub fn register(router: Router<State>, config: &Config) -> Router<State> {
    let router = router
        .ruma_route(&versions::get_supported_versions_route)
        .ruma_route(&register::get_register_available_route)
        .ruma_route(&register::register_route)
        .ruma_route(&register::check_registration_token_validity)
        .ruma_route(&session::get_login_types_route)
        .ruma_route(&session::login_route)
        .ruma_route(&session::login_token_route)
        .ruma_route(&session::refresh_token_route)
        .ruma_route(&session::logout_route)
        .ruma_route(&session::logout_all_route)
        .ruma_route(&account::change_password_route)
        .ruma_route(&account::deactivate_route)
        .ruma_route(&account::third_party_route)
        .ruma_route(&account::add_3pid_route)
        .ruma_route(&account::delete_3pid_route)
        .ruma_route(&account::request_3pid_management_token_via_email_route)
        .ruma_route(&account::request_3pid_management_token_via_msisdn_route)
        .ruma_route(&account::request_registration_token_via_email_route)
        .ruma_route(&account::request_password_change_token_via_email_route)
        .route(
            "/_phantom/3pid/email/validate",
            get(account::get_email_validate_route).post(account::post_email_validate_route),
        )
        .ruma_route(&well_known::well_known_client)
        .ruma_route(&well_known::well_known_support)
        .ruma_route(&capabilities::get_capabilities_route)
        .ruma_route(&account::whoami_route)
        .ruma_route(&filter::get_filter_route)
        .ruma_route(&filter::create_filter_route)
        .ruma_route(&account_data::set_global_account_data_route)
        .ruma_route(&account_data::set_room_account_data_route)
        .ruma_route(&account_data::get_global_account_data_route)
        .ruma_route(&account_data::get_room_account_data_route)
        .ruma_route(&tag::get_tags_route)
        .ruma_route(&tag::update_tag_route)
        .ruma_route(&tag::delete_tag_route)
        .ruma_route(&openid::create_openid_token_route)
        .ruma_route(&voip::turn_server_route)
        .ruma_route(&device::get_devices_route)
        .ruma_route(&device::get_device_route)
        .ruma_route(&device::update_device_route)
        .ruma_route(&device::delete_device_route)
        .ruma_route(&device::delete_devices_route)
        .ruma_route(&keys::upload_keys_route)
        .ruma_route(&keys::get_keys_route)
        .ruma_route(&keys::claim_keys_route)
        .ruma_route(&keys::upload_signing_keys_route)
        .ruma_route(&keys::upload_signatures_route)
        .ruma_route(&keys::get_key_changes_route)
        .ruma_route(&backup::create_backup_version_route)
        .ruma_route(&backup::update_backup_version_route)
        .ruma_route(&backup::delete_backup_version_route)
        .ruma_route(&backup::get_latest_backup_info_route)
        .ruma_route(&backup::get_backup_info_route)
        .ruma_route(&backup::add_backup_keys_route)
        .ruma_route(&backup::add_backup_keys_for_room_route)
        .ruma_route(&backup::add_backup_keys_for_session_route)
        .ruma_route(&backup::delete_backup_keys_for_room_route)
        .ruma_route(&backup::delete_backup_keys_for_session_route)
        .ruma_route(&backup::delete_backup_keys_route)
        .ruma_route(&backup::get_backup_keys_for_room_route)
        .ruma_route(&backup::get_backup_keys_for_session_route)
        .ruma_route(&backup::get_backup_keys_route)
        .ruma_route(&to_device::send_event_to_device_route)
        .ruma_route(&profile::get_profile_route)
        .ruma_route(&profile::get_profile_field_route)
        .ruma_route(&profile::set_profile_field_route)
        .ruma_route(&profile::delete_profile_field_route)
        .ruma_route(&presence::set_presence_route)
        .ruma_route(&presence::get_presence_route)
        .ruma_route(&typing::create_typing_event_route)
        .ruma_route(&user_directory::search_users_route)
        .ruma_route(&unstable::get_mutual_rooms_route)
        .ruma_route(&appservice::appservice_ping)
        .ruma_route(&report::report_event_route)
        .ruma_route(&report::report_room_route)
        .ruma_route(&report::report_user_route)
        .ruma_route(&thirdparty::get_protocols_route)
        .ruma_route(&thirdparty::get_protocol_route)
        .ruma_route(&thirdparty::get_user_for_protocol_route)
        .ruma_route(&thirdparty::get_location_for_protocol_route)
        .ruma_route(&thirdparty::get_user_for_user_id_route)
        .ruma_route(&thirdparty::get_location_for_room_alias_route)
        .ruma_route(&push::get_pushrules_all_route)
        .ruma_route(&push::get_pushrules_global_route)
        .ruma_route(&push::set_pushrule_route)
        .ruma_route(&push::get_pushrule_route)
        .ruma_route(&push::set_pushrule_enabled_route)
        .ruma_route(&push::get_pushrule_enabled_route)
        .ruma_route(&push::get_pushrule_actions_route)
        .ruma_route(&push::set_pushrule_actions_route)
        .ruma_route(&push::delete_pushrule_route)
        .ruma_route(&push::get_pushers_route)
        .ruma_route(&push::set_pushers_route)
        .ruma_route(&push::get_notifications_route)
        .ruma_route(&read_marker::set_read_marker_route)
        .ruma_route(&read_marker::create_receipt_route)
        .ruma_route(&room::create_room_route)
        .ruma_route(&redact::redact_event_route)
        .ruma_route(&alias::create_alias_route)
        .ruma_route(&alias::delete_alias_route)
        .ruma_route(&alias::get_alias_route)
        .ruma_route(&membership::join_room_by_id_route)
        .ruma_route(&membership::join_room_by_id_or_alias_route)
        .ruma_route(&membership::joined_members_route)
        .ruma_route(&membership::knock_room_route)
        .ruma_route(&membership::leave_room_route)
        .ruma_route(&membership::forget_room_route)
        .ruma_route(&membership::joined_rooms_route)
        .ruma_route(&membership::kick_user_route)
        .ruma_route(&membership::ban_user_route)
        .ruma_route(&membership::unban_user_route)
        .ruma_route(&membership::invite_user_route)
        .ruma_route(&membership::get_member_events_route)
        .ruma_route(&directory::set_room_visibility_route)
        .ruma_route(&directory::get_room_visibility_route)
        .ruma_route(&directory::get_public_rooms_route)
        .ruma_route(&directory::get_public_rooms_filtered_route)
        .ruma_route(&room::upgrade_room_route)
        .ruma_route(&room::get_room_summary)
        .route(
            "/_matrix/client/unstable/im.nheko.summary/rooms/{room_id_or_alias}/summary",
            get(room::get_room_summary_legacy),
        )
        .ruma_route(&room::get_room_event_route)
        .ruma_route(&room::get_room_aliases_route)
        .ruma_route(&send::send_message_event_route)
        .ruma_route(&state::send_state_event_for_key_route)
        .ruma_route(&state::get_state_events_route)
        .ruma_route(&state::get_state_events_for_key_route)
        // Ruma doesn't have support for multiple paths for a single endpoint yet, and these
        // routes share one Ruma request / response type pair with
        // {get,send}_state_event_for_key_route
        .route(
            "/_matrix/client/r0/rooms/{room_id}/state/{event_type}",
            get(state::get_state_events_for_empty_key_route)
                .put(state::send_state_event_for_empty_key_route),
        )
        .route(
            "/_matrix/client/v3/rooms/{room_id}/state/{event_type}",
            get(state::get_state_events_for_empty_key_route)
                .put(state::send_state_event_for_empty_key_route),
        )
        // These two endpoints allow trailing slashes
        .route(
            "/_matrix/client/r0/rooms/{room_id}/state/{event_type}/",
            get(state::get_state_events_for_empty_key_route)
                .put(state::send_state_event_for_empty_key_route),
        )
        .route(
            "/_matrix/client/v3/rooms/{room_id}/state/{event_type}/",
            get(state::get_state_events_for_empty_key_route)
                .put(state::send_state_event_for_empty_key_route),
        )
        .ruma_route(&events::events_route)
        .ruma_route(&sync::sync_events_route)
        .ruma_route(&sync::sync_events_v5_route)
        .ruma_route(&context::get_context_route)
        .ruma_route(&room::get_event_by_timestamp_route)
        .ruma_route(&message::get_message_events_route)
        .ruma_route(&search::search_events_route)
        .ruma_route(&threads::get_threads_route)
        .ruma_route(&relations::get_relating_events_with_rel_type_and_event_type_route)
        .ruma_route(&relations::get_relating_events_with_rel_type_route)
        .ruma_route(&relations::get_relating_events_route)
        .ruma_route(&space::get_hierarchy_route)
        .ruma_route(&media::create_content_route)
        .ruma_route(&media::create_mxc_uri_route)
        .ruma_route(&media::create_content_async_route)
        .ruma_route(&media::get_media_preview_route)
        .ruma_route(&media::get_media_config_route)
        .ruma_route(&media::get_content_thumbnail_route)
        .ruma_route(&media::get_content_route)
        .ruma_route(&media::get_content_as_filename_route)
        .ruma_route(&media_legacy::get_media_config_legacy_route)
        .ruma_route(&media_legacy::get_media_preview_legacy_route)
        .ruma_route(&media_legacy::get_content_legacy_route)
        .ruma_route(&media_legacy::get_content_as_filename_legacy_route)
        .ruma_route(&media_legacy::get_content_thumbnail_legacy_route)
        .route(
            "/_phantom/server_version",
            get(phantom::phantom_server_version),
        );

    // The local user count is withheld for privacy when federation is off.
    if config.federation.allow_federation {
        router.route(
            "/_phantom/local_user_count",
            get(phantom::phantom_local_user_count),
        )
    } else {
        router
    }
}

/// Keeps a timeline item only when the user may see its event.
///
/// The room's history visibility is read as it stood at that event rather than
/// as it stands at the time of the request.
#[inline]
async fn visibility_filter(
    services: &Services,
    item: PdusIterItem,
    user_id: &UserId,
) -> Option<PdusIterItem> {
    let (_, pdu) = &item;

    services
        .rooms
        .state_accessor
        .user_can_see_event(user_id, pdu.room_id(), pdu.event_id())
        .await
        .then_some(item)
}
