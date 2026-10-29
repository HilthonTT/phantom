mod admin;
mod appservice;
mod federation;
mod scheme;
mod token;
mod uiaa;

use http::HeaderMap;
use phantom_core::Result;
use phantom_service::{Services, ops::appservice::RegistrationInfo};
use ruma::{OwnedDeviceId, OwnedServerName, OwnedUserId};

use self::token::Token;
pub use self::{
    admin::{AdminAuth, AdminWitness},
    scheme::Authenticate,
    uiaa::authenticate_uiaa,
};
use super::raw_request::RawRequest;

#[derive(Debug, Default)]
pub struct Auth {
    pub(super) origin: Option<OwnedServerName>,
    pub(super) sender_user: Option<OwnedUserId>,
    pub(super) sender_device: Option<OwnedDeviceId>,
    pub(super) appservice_info: Option<RegistrationInfo>,
}

impl Auth {
    fn user(user_id: OwnedUserId, device_id: OwnedDeviceId) -> Self {
        Self {
            sender_user: Some(user_id),
            sender_device: Some(device_id),
            ..Self::default()
        }
    }

    fn appservice(info: RegistrationInfo) -> Self {
        Self {
            appservice_info: Some(info),
            ..Self::default()
        }
    }

    fn server(origin: OwnedServerName) -> Self {
        Self {
            origin: Some(origin),
            ..Self::default()
        }
    }
}

#[tracing::instrument(level = "trace", skip_all, err(level = "debug"))]
pub(super) async fn authenticate<S>(services: &Services, request: &RawRequest) -> Result<Auth>
where
    S: Authenticate,
{
    let token = Token::find(services, request).await?;

    S::authenticate(services, request, token).await
}

/// The local user a bearer token names, for a handler that reads its own body
/// and so cannot take a `Ruma` extractor. No token is `None`; an unknown or
/// expired one is refused, as `AccessTokenOptional` refuses it.
pub async fn optional_user(
    services: &Services,
    headers: &HeaderMap,
) -> Result<Option<OwnedUserId>> {
    match Token::lookup(services, token::bearer(headers)).await? {
        Token::User(user_id, _) => Ok(Some(user_id)),
        Token::Invalid => scheme::unknown_token(),
        Token::Expired => scheme::expired_token(),
        Token::None | Token::Appservice(_) => Ok(None),
    }
}
