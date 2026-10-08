use std::future::ready;

use axum::extract::State;
use phantom_core::{Result, matrix::state_res::RoomVersion};
use phantom_service::Services;
#[expect(deprecated)]
use ruma::api::client::discovery::get_capabilities::v3::{
    SetAvatarUrlCapability, SetDisplayNameCapability,
};
use ruma::{
    api::client::discovery::get_capabilities::v3::{
        AccountModerationCapability, Capabilities, ChangePasswordCapability,
        ForgetForcedUponLeaveCapability, GetLoginTokenCapability, ProfileFieldsCapability, Request,
        Response, RoomVersionStability, RoomVersionsCapability, ThirdPartyIdChangesCapability,
    },
    profile::ProfileFieldName,
};
use serde_json::json;

use crate::{client::utils::may_set_displayname, router::Ruma};

/// # `GET /_matrix/client/v3/capabilities`
///
/// Get information on the supported feature set and other relevant capabilities
/// of this server.
pub(crate) async fn get_capabilities_route(
    State(services): State<crate::router::State>,
    body: Ruma<Request>,
) -> Result<Response> {
    // MSC4323: advertise admin moderation only to admins; absence implies
    // neither suspend nor lock is available to the caller.
    let account_moderation = services.admin.user_is_admin(body.sender_user()).await;

    let set_displayname = may_set_displayname(&services, &body, || ready(account_moderation)).await;

    capabilities(&services, set_displayname, account_moderation).map(Response::new)
}

#[expect(deprecated)]
fn capabilities(
    services: &Services,
    set_displayname: bool,
    account_moderation: bool,
) -> Result<Capabilities> {
    let config = &services.config;

    let available = RoomVersion::supported()
        .map(|version| (version, RoomVersionStability::Stable))
        .collect();

    let default = config.client.default_room_version.clone();

    // Matrix 1.16 clients read the display name policy from m.profile_fields.
    let disallowed = (!set_displayname).then(|| vec![ProfileFieldName::DisplayName]);

    let mut capabilities = Capabilities::new();

    capabilities.room_versions = RoomVersionsCapability::new(default, available);

    // MSC3283: deprecated displayname/avatar capabilities for pre-1.16 clients.
    capabilities.set_displayname = SetDisplayNameCapability::new(set_displayname);
    capabilities.set_avatar_url = SetAvatarUrlCapability::new(true);

    // 3PID add/remove is available only when the email subsystem can send.
    capabilities.thirdparty_id_changes =
        ThirdPartyIdChangesCapability::new(services.sendmail.is_enabled());

    capabilities.get_login_token =
        GetLoginTokenCapability::new(config.client.login_via_existing_session);

    let mut profile_fields = ProfileFieldsCapability::new(true);
    profile_fields.disallowed = disallowed;
    capabilities.profile_fields = Some(profile_fields);

    capabilities.change_password = ChangePasswordCapability::new(config.client.login_with_password);

    capabilities.forget_forced_upon_leave =
        ForgetForcedUponLeaveCapability::new(config.rooms.forget_forced_upon_leave);

    capabilities.set(
        "org.matrix.msc4267.forget_forced_upon_leave",
        json!({"enabled": config.rooms.forget_forced_upon_leave}),
    )?;

    // MSC4452: enabled mirrors the per-URL gate; empty allowlists 403 every URL.
    capabilities.set(
        "io.element.msc4452.preview_url",
        json!({"enabled": preview_url_enabled(services)}),
    )?;

    if account_moderation {
        capabilities.account_moderation = AccountModerationCapability::new(true, true);
    }

    Ok(capabilities)
}

fn preview_url_enabled(services: &Services) -> bool {
    let media = &services.config.media;

    !media.url_preview_domain_contains_allowlist.is_empty()
        || !media.url_preview_domain_explicit_allowlist.is_empty()
        || !media.url_preview_url_contains_allowlist.is_empty()
}
