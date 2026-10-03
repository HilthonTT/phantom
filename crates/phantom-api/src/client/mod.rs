mod account;
mod account_data;
mod appservice;
mod backup;
mod capabilities;
mod device;
mod filter;
mod keys;
mod openid;
mod presence;
mod profile;
mod register;
mod report;
mod session;
mod tag;
mod thirdparty;
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

use crate::router::{RouterExt, State};

/// generated device ID length
const DEVICE_ID_LENGTH: usize = 10;

/// generated user access token length
const TOKEN_LENGTH: usize = phantom_service::accounts::users::TOKEN_LENGTH;

/// generated user session ID length
const SESSION_ID_LENGTH: usize = phantom_service::auth::uiaa::SESSION_ID_LENGTH;

pub fn register(router: Router<State>, _config: &Config) -> Router<State> {
    router
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
}

/// Keeps a timeline item only when the user may see its event.
///
/// The room's history visibility is read as it stood at that event rather than
/// as it stands at the time of the request.
#[allow(dead_code)]
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
