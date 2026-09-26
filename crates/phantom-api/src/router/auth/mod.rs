mod appservice;
mod federation;
mod scheme;
mod token;
mod uiaa;

use phantom_core::Result;
use phantom_service::{Services, ops::appservice::RegistrationInfo};
use ruma::{OwnedDeviceId, OwnedServerName, OwnedUserId};

use self::token::Token;
pub use self::{scheme::Authenticate, uiaa::authenticate_uiaa};
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
