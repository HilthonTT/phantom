use std::{fmt::Write, net::IpAddr};

use axum::extract::State;
use phantom_core::{Err, Error, Result, debug_info, debug_warn, info, warn};
use phantom_service::{
    Services, accounts::users::generate_refresh_token, auth::threepid::Association,
};
use ruma::{
    DeviceId, MilliSecondsSinceUnixEpoch, OwnedDeviceId, OwnedUserId, UserId,
    api::client::{
        account::register::{self, LoginType, RegistrationKind},
        uiaa::{AuthFlow, AuthType, UiaaInfo},
    },
    events::{
        GlobalAccountDataEventType,
        push_rules::{PushRulesEvent, PushRulesEventContent},
    },
    push::Ruleset,
    thirdparty::Medium,
};

use super::{DEVICE_ID_LENGTH, SESSION_ID_LENGTH, is_matrix_appservice_irc};
use crate::router::{ClientIp, Ruma};

const RANDOM_USER_ID_LENGTH: usize = 10;

/// # `POST /_matrix/client/v3/register`
///
/// Register an account on this homeserver.
///
/// You can use [`GET
/// /_matrix/client/v3/register/available`](fn.get_register_available_route.
/// html) to check if the user id is valid and available.
///
/// - Only works if registration is enabled
/// - If type is guest: ignores all parameters except
///   initial_device_display_name
/// - If sender is not appservice: Requires UIAA (but we only use a dummy stage)
/// - If type is not guest and no username is given: Always fails after UIAA
///   check
/// - Creates a new account and populates it with default account data
/// - If `inhibit_login` is false: Creates a device and returns device id and
///   access_token
#[expect(clippy::doc_markdown)]
#[tracing::instrument(skip_all, fields(%client), name = "register")]
pub(crate) async fn register_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<register::v3::Request>,
) -> Result<register::v3::Response> {
    let is_guest = body.kind == RegistrationKind::Guest;
    let emergency_mode_enabled = services.config.admin.emergency_password.is_some();

    gate_registration_allowed(&services, &body, is_guest)?;

    let user_id =
        resolve_registration_user_id(&services, &body, is_guest, emergency_mode_enabled).await?;

    check_appservice_namespace(&services, &body, &user_id, emergency_mode_enabled).await?;

    let email_association = enforce_uiaa(&services, &body, is_guest).await?;

    let password = if is_guest {
        None
    } else {
        body.password.as_deref()
    };

    full_register(&services, &user_id, password).await?;

    bind_registration_email(&services, &user_id, email_association.as_ref()).await;

    if !is_guest && body.inhibit_login {
        return Ok(register::v3::Response::new(user_id));
    }

    let device_id = match (is_guest, &body.device_id) {
        (false, Some(device_id)) => device_id.clone(),
        _ => OwnedDeviceId::from(phantom_core::rand::string(DEVICE_ID_LENGTH)),
    };

    // Generate new token for the device
    let (access_token, expires_in) = services.users.generate_access_token(body.refresh_token);

    // Generate a new refresh_token if requested by client
    let refresh_token = expires_in.is_some().then(generate_refresh_token);

    // Create device for this account
    services
        .users
        .create_device(
            &user_id,
            &device_id,
            &access_token,
            body.initial_device_display_name.clone(),
            Some(client.to_string()),
        )
        .await?;

    services
        .users
        .set_access_token(
            &user_id,
            &device_id,
            &access_token,
            expires_in,
            refresh_token.as_deref(),
        )
        .await?;

    debug_info!(%user_id, %device_id, "User account was created");

    if body.appservice_info.is_none()
        && (!is_guest || services.config.client.log_guest_registrations)
    {
        announce_new_user(&user_id, &body, is_guest, &client).await?;
    }

    let mut response = register::v3::Response::new(user_id);
    response.device_id = Some(device_id);
    response.access_token = Some(access_token);
    response.refresh_token = refresh_token;
    response.expires_in = expires_in;

    Ok(response)
}

fn gate_registration_allowed(
    services: &Services,
    body: &Ruma<register::v3::Request>,
    is_guest: bool,
) -> Result {
    let user = body.username.as_deref().unwrap_or("");
    let device_name = body.initial_device_display_name.as_deref().unwrap_or("");

    if !services.config.auth.allow_registration && body.appservice_info.is_none() {
        info!(
            %is_guest,
            %user,
            %device_name,
            "Rejecting registration attempt as registration is disabled"
        );

        return Err!(Request(Forbidden("Registration has been disabled.")));
    }

    if is_guest && !services.config.client.allow_guest_registration {
        debug_warn!(
            message = format_args!("Guest registration disabled, rejecting guest registration attempt"),
            %device_name
        );

        return Err!(Request(GuestAccessForbidden(
            "Guest registration is disabled."
        )));
    }

    Ok(())
}

async fn resolve_registration_user_id(
    services: &Services,
    body: &Ruma<register::v3::Request>,
    is_guest: bool,
    emergency_mode_enabled: bool,
) -> Result<OwnedUserId> {
    let (Some(username), false) = (body.username.as_ref(), is_guest) else {
        loop {
            let proposed_user_id = UserId::parse_with_server_name(
                phantom_core::rand::string(RANDOM_USER_ID_LENGTH).to_lowercase(),
                services.server_state.server_name(),
            )?;

            if !services.users.exists(&proposed_user_id).await {
                return Ok(proposed_user_id);
            }
        }
    };

    let is_irc = is_matrix_appservice_irc(body.appservice_info.as_ref());

    if services.config.rooms.forbidden_usernames.is_match(username) && !emergency_mode_enabled {
        return Err!(Request(Forbidden("Username is forbidden")));
    }

    // don't force the username lowercase if it's from matrix-appservice-irc
    let body_username = if is_irc {
        username.clone()
    } else {
        username.to_lowercase()
    };

    let proposed_user_id =
        match UserId::parse_with_server_name(&body_username, services.server_state.server_name()) {
            Ok(user_id) => {
                if let Err(e) = user_id.validate_strict() {
                    // unless the username is from the broken matrix appservice IRC bridge, or
                    // we are in emergency mode, we should follow synapse's behaviour on
                    // not allowing things like spaces and UTF-8 characters in usernames
                    if !is_irc && !emergency_mode_enabled {
                        return Err!(Request(InvalidUsername(debug_warn!(
                            "Username {body_username} contains disallowed characters or spaces: \
                             {e}"
                        ))));
                    }
                }

                user_id
            }
            Err(e) => {
                return Err!(Request(InvalidUsername(debug_warn!(
                    "Username {body_username} is not valid: {e}"
                ))));
            }
        };

    if services.users.exists(&proposed_user_id).await {
        return Err!(Request(UserInUse("User ID is not available.")));
    }

    Ok(proposed_user_id)
}

async fn check_appservice_namespace(
    services: &Services,
    body: &Ruma<register::v3::Request>,
    user_id: &UserId,
    emergency_mode_enabled: bool,
) -> Result {
    if body.body.login_type == Some(LoginType::ApplicationService) {
        match body.appservice_info {
            Some(ref info) => {
                if !info.is_user_match(user_id) && !emergency_mode_enabled {
                    return Err!(Request(Exclusive(
                        "Username is not in an appservice namespace."
                    )));
                }
            }
            _ => {
                return Err!(Request(MissingToken("Missing appservice token.")));
            }
        }
    } else if services.appservice.is_exclusive_user_id(user_id).await && !emergency_mode_enabled {
        return Err!(Request(Exclusive("Username is reserved by an appservice.")));
    }

    Ok(())
}

async fn enforce_uiaa(
    services: &Services,
    body: &Ruma<register::v3::Request>,
    is_guest: bool,
) -> Result<Option<Association>> {
    if body.appservice_info.is_some() || is_guest {
        return Ok(None);
    }

    let token_required = services.registration_tokens.is_enabled().await;

    let smtp = &services.config.smtp;
    let email_required = smtp.connection_uri.is_some()
        && (smtp.require_email_for_registration
            || (token_required && smtp.require_email_for_token_registration));

    let stages: Vec<AuthType> = [
        token_required.then_some(AuthType::RegistrationToken),
        email_required.then_some(AuthType::EmailIdentity),
    ]
    .into_iter()
    .flatten()
    .collect();

    // A dummy stage still forces the client through UIA when nothing else does.
    let stages = if stages.is_empty() {
        vec![AuthType::Dummy]
    } else {
        stages
    };

    let mut uiaainfo = UiaaInfo::new(vec![AuthFlow::new(stages)]);

    let server_user = UserId::parse_with_server_name("", services.server_state.server_name())?;
    let server_device: &DeviceId = "".into();

    match &body.auth {
        Some(auth) => {
            let (worked, uiaainfo) = match email_required {
                true => {
                    services
                        .uiaa
                        .try_auth_registration(&server_user, server_device, auth, &uiaainfo)
                        .await?
                }
                false => {
                    services
                        .uiaa
                        .try_auth(&server_user, server_device, auth, &uiaainfo)
                        .await?
                }
            };

            if !worked {
                return Err(uiaainfo.into());
            }

            let session = uiaainfo.session.expect("session is always set");
            let claim = (server_user, server_device.to_owned(), session.into());

            let association = match email_required {
                false => None,
                true => {
                    let association = match services.threepid.redeem_claim(&claim).await {
                        Ok(association) => association,
                        Err(error)
                            if error.is_not_found() || matches!(&error, Error::Request(..)) =>
                        {
                            return Err!(Request(Forbidden("Invalid email identity proof.")));
                        }
                        Err(error) => return Err(error),
                    };

                    if association.medium != Medium::Email {
                        return Err!(Request(Forbidden("Invalid email identity proof.")));
                    }

                    Some(association)
                }
            };

            services
                .uiaa
                .update_uiaa_session(&claim.0, &claim.1, &claim.2, None)?;

            Ok(association)
        }
        _ => match body.json_body {
            None => Err!(Request(NotJson("JSON body is not valid"))),
            Some(ref json) => {
                uiaainfo.session = Some(phantom_core::rand::string(SESSION_ID_LENGTH));
                services
                    .uiaa
                    .create(&server_user, server_device, &uiaainfo, json)?;

                Err(uiaainfo.into())
            }
        },
    }
}

/// Bind the association spent before account creation.
///
/// Binding remains best effort after `full_register`; ownership of the proof
/// cannot be replayed even when the binding write fails.
async fn bind_registration_email(
    services: &Services,
    user_id: &UserId,
    association: Option<&Association>,
) {
    if !services.sendmail.is_enabled() {
        return;
    }

    let Some(association) = association else {
        return;
    };

    if let Err(e) = try_bind_registration_email(services, user_id, association).await {
        warn!(%user_id, "Skipping registration email binding: {e}");
    }
}

async fn try_bind_registration_email(
    services: &Services,
    user_id: &UserId,
    association: &Association,
) -> Result {
    if services
        .threepid
        .user_id_for_email(&association.address)
        .await?
        .is_some_and(|bound| bound != user_id)
    {
        warn!(%user_id, "Skipping registration email binding: address bound to another user");

        return Ok(());
    }

    let now = MilliSecondsSinceUnixEpoch::now();

    services
        .threepid
        .put_binding(user_id, &association.address, Medium::Email, now, now)
        .await?;

    Ok(())
}

async fn announce_new_user(
    user_id: &UserId,
    body: &Ruma<register::v3::Request>,
    is_guest: bool,
    client: &IpAddr,
) -> Result {
    let mut notice = String::from(if is_guest {
        "New guest user"
    } else {
        "New user"
    });

    write!(
        notice,
        " \"{user_id}\" registered on this server from IP {client}"
    )?;

    if let Some(device_name) = body.initial_device_display_name.as_deref() {
        write!(notice, " with device name {device_name}")?;
    }

    if is_guest {
        debug_info!("{notice}");
    } else {
        info!("{notice}");
    }

    Ok(())
}

/// Creates a local account with its initial profile and account data.
async fn full_register(services: &Services, user_id: &UserId, password: Option<&str>) -> Result {
    if services.users.exists(user_id).await {
        return Err!(Request(UserInUse("User ID is not available.")));
    }

    services.users.create(user_id, password)?;

    let suffix = services.config.auth.new_user_displayname_suffix.as_str();
    let displayname = match suffix {
        "" => user_id.localpart().to_owned(),
        suffix => format!("{} {suffix}", user_id.localpart()),
    };

    services.profile.set_displayname(user_id, Some(displayname));

    let push_rules =
        PushRulesEvent::new(PushRulesEventContent::new(Ruleset::server_default(user_id)));

    services
        .account_data
        .update(
            None,
            user_id,
            GlobalAccountDataEventType::PushRules.to_string().into(),
            &serde_json::to_value(push_rules)?,
        )
        .await
}
