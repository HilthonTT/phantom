mod appservice;
mod logout;
mod password;
mod refresh;
mod token;

use axum::extract::State;
use phantom_core::{Err, Result, info, rand, stream::ReadyExt};
use phantom_service::accounts::users::generate_refresh_token;
use ruma::{
    OwnedDeviceId,
    api::client::session::{
        get_login_types::{
            self,
            v3::{ApplicationServiceLoginType, LoginType, PasswordLoginType, TokenLoginType},
        },
        login::{
            self,
            v3::{DiscoveryInfo, HomeserverInfo, LoginInfo},
        },
    },
};

pub(crate) use self::{
    logout::{logout_all_route, logout_route},
    refresh::refresh_token_route,
    token::login_token_route,
};
use super::DEVICE_ID_LENGTH;
use crate::router::{ClientIp, Ruma};

/// # `GET /_matrix/client/v3/login`
///
/// Get the supported login types of this server. One of these should be used as
/// the `type` field when logging in.
#[tracing::instrument(skip_all, fields(%client), name = "login")]
pub(crate) async fn get_login_types_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    _body: Ruma<get_login_types::v3::Request>,
) -> Result<get_login_types::v3::Response> {
    let config = &services.config.client;

    let appservice = Some(LoginType::ApplicationService(
        ApplicationServiceLoginType::default(),
    ));

    let mut token = TokenLoginType::default();
    token.get_login_token = config.login_via_existing_session;
    let token = config.login_via_token.then_some(LoginType::Token(token));

    let password = config
        .login_with_password
        .then(|| LoginType::Password(PasswordLoginType::default()));

    let flows = [appservice, token, password]
        .into_iter()
        .flatten()
        .collect();

    Ok(get_login_types::v3::Response::new(flows))
}

/// # `POST /_matrix/client/v3/login`
///
/// Authenticates the user and returns an access token it can use in subsequent
/// requests.
///
/// - If `device_id` is known: issues an additional access token for that device
/// - If `device_id` is unknown: creates a new device
/// - Returns access token that is associated with the user and device
#[tracing::instrument(
    name = "login",
    level = "debug",
    skip_all,
    fields(client = %client_ip),
)]
pub(crate) async fn login_route(
    State(services): State<crate::router::State>,
    ClientIp(client_ip): ClientIp,
    body: Ruma<login::v3::Request>,
) -> Result<login::v3::Response> {
    let config = &services.config.client;
    let user_id = match &body.login_info {
        LoginInfo::Password(info) if config.login_with_password => {
            password::handle_login(&services, &body, info).await?
        }
        LoginInfo::Token(info) if config.login_via_token => {
            token::handle_login(&services, &body, info).await?
        }
        LoginInfo::ApplicationService(info) => appservice::handle_login(&services, &body, info)?,
        _ => {
            return Err!(Request(Unknown(debug_warn!(
                "Invalid or unsupported login type"
            ))));
        }
    };

    // Appservice users are often passwordless, which reads as deactivated.
    if !matches!(body.login_info, LoginInfo::ApplicationService(_))
        && services
            .users
            .is_deactivated(&user_id)
            .await
            .unwrap_or(false)
    {
        return Err!(Request(UserDeactivated(
            "This account has been deactivated."
        )));
    }

    let (access_token, expires_in) = services
        .users
        .generate_access_token(body.body.refresh_token);

    let refresh_token = expires_in.is_some().then(generate_refresh_token);

    let existing_device = match &body.device_id {
        Some(device_id) => {
            services
                .users
                .all_device_ids(&user_id)
                .ready_any(|existing_device_id| existing_device_id == device_id)
                .await
        }
        None => false,
    };

    let device_id = match &body.device_id {
        Some(device_id) if existing_device => device_id.clone(),
        device_id => {
            let device_id = device_id
                .clone()
                .unwrap_or_else(|| OwnedDeviceId::from(rand::string(DEVICE_ID_LENGTH)));

            services
                .users
                .create_device(
                    &user_id,
                    &device_id,
                    &access_token,
                    body.initial_device_display_name.clone(),
                    Some(client_ip.to_string()),
                )
                .await?;

            device_id
        }
    };

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

    info!("{user_id} logged in");

    // Send client well-known information when configured, so the client can reconfigure itself.
    let well_known: Option<DiscoveryInfo> = services
        .config
        .auth
        .well_known_client
        .as_ref()
        .map(ToString::to_string)
        .map(HomeserverInfo::new)
        .map(DiscoveryInfo::new);

    let mut response = login::v3::Response::new(user_id, access_token, device_id);
    response.well_known = well_known;
    response.expires_in = expires_in;
    response.refresh_token = refresh_token;

    Ok(response)
}
