use phantom_core::{Err, Result, err};
use phantom_service::Services;
use ruma::{
    OwnedUserId, UserId,
    api::client::{
        session::login::v3::{ApplicationService, Request},
        uiaa,
    },
};

use crate::router::Ruma;

pub(super) fn handle_login(
    services: &Services,
    body: &Ruma<Request>,
    info: &ApplicationService,
) -> Result<OwnedUserId> {
    #[expect(deprecated)]
    let (identifier, user) = (&info.identifier, &info.user);

    let Some(ref info) = body.appservice_info else {
        return Err!(Request(MissingToken("Missing appservice token.")));
    };

    let user_id = match identifier {
        Some(uiaa::UserIdentifier::Matrix(identifier)) => Some(&identifier.user),
        _ => None,
    }
    .or(user.as_ref())
    .ok_or_else(|| {
        err!(Request(Unknown(debug_warn!(
            message = format_args!("Valid identifier or username was not provided (invalid or unsupported login type?)"),
            ?body.login_info
        ))))
    })?;

    let user_id =
        UserId::parse_with_server_name(user_id.as_str(), services.server_state.server_name())
            .map_err(|e| err!(Request(InvalidUsername(warn!("Username is invalid: {e}")))))?;

    if !services.server_state.user_is_local(&user_id) {
        return Err!(Request(Unknown(
            "User ID does not belong to this homeserver"
        )));
    }

    let emergency_mode_enabled = services.config.admin.emergency_password.is_some();

    if !info.is_user_match(&user_id) && !emergency_mode_enabled {
        return Err!(Request(Exclusive(
            "Username is not in an appservice namespace."
        )));
    }

    Ok(user_id)
}
