use http::header::AUTHORIZATION;
use phantom_core::Result;
use phantom_service::{Services, ops::appservice::RegistrationInfo};
use ruma::{OwnedDeviceId, OwnedUserId};

use super::RawRequest;

pub enum Token {
    None,
    Invalid,
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

        match services.users.find_from_token(access_token).await {
            Ok((user_id, device_id)) => Ok(Self::User(user_id, device_id)),
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
