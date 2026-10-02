use http::header::AUTHORIZATION;
use phantom_core::{Result, time::timepoint_has_passed};
use phantom_service::{
    Services, accounts::users::is_refresh_token, ops::appservice::RegistrationInfo,
};
use ruma::{OwnedDeviceId, OwnedUserId};

use super::RawRequest;

pub enum Token {
    None,
    Invalid,
    /// A recognised access token past its expiry; the client must refresh.
    Expired,
    User(OwnedUserId, OwnedDeviceId),
    Appservice(Box<RegistrationInfo>),
}

impl Token {
    pub(super) async fn find(services: &Services, request: &RawRequest) -> Result<Self> {
        let Some(access_token) = access_token(request) else {
            return Ok(Self::None);
        };

        if let Some(info) = services.appservice.find_from_token(access_token).await {
            return Ok(Self::Appservice(Box::new(info)));
        }

        // A refresh token resolves through the same index but authorizes nothing.
        if is_refresh_token(access_token) {
            return Ok(Self::Invalid);
        }

        match services.users.find_from_token(access_token).await {
            Ok((.., Some(expires_at))) if timepoint_has_passed(expires_at) => Ok(Self::Expired),
            Ok((user_id, device_id, _)) => Ok(Self::User(user_id, device_id)),
            Err(e) if e.is_not_found() => Ok(Self::Invalid),
            Err(e) => Err(e),
        }
    }
}

fn access_token(request: &RawRequest) -> Option<&str> {
    request
        .parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim())
        .or(request.query.access_token.as_deref())
}
