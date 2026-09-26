use phantom_core::{Result, err, rand};
use phantom_service::{Services, auth::uiaa::SESSION_ID_LENGTH};
use ruma::{
    CanonicalJsonValue, OwnedUserId, UserId,
    api::{
        IncomingRequest,
        client::uiaa::{AuthData, AuthFlow, AuthType, UiaaInfo},
    },
};

use crate::router::Ruma;

pub async fn authenticate_uiaa<T>(services: &Services, request: &Ruma<T>) -> Result<OwnedUserId>
where
    T: IncomingRequest + Send + Sync,
{
    let sender_user = request
        .sender_user
        .as_deref()
        .ok_or_else(|| err!(Request(MissingToken("Missing access token."))))?;

    let sender_device = request.sender_device()?;
    let mut uiaainfo = UiaaInfo::new(flows(services, sender_user).await);

    let auth_data = request
        .json_body
        .as_ref()
        .and_then(CanonicalJsonValue::as_object)
        .and_then(|body| body.get("auth"))
        .cloned()
        .map(|auth| serde_json::from_value::<AuthData>(auth.into()))
        .transpose()?;

    if let Some(auth_data) = auth_data {
        let (completed, uiaainfo) = services
            .uiaa
            .try_auth(sender_user, sender_device, &auth_data, &uiaainfo)
            .await?;

        return if completed {
            Ok(sender_user.to_owned())
        } else {
            Err(uiaainfo.into())
        };
    }

    let json_body = request
        .json_body
        .as_ref()
        .ok_or_else(|| err!(Request(NotJson("JSON body is not valid"))))?;

    uiaainfo.session = Some(rand::string(SESSION_ID_LENGTH));
    services
        .uiaa
        .create(sender_user, sender_device, &uiaainfo, json_body)?;

    Err(uiaainfo.into())
}

async fn flows(services: &Services, user_id: &UserId) -> Vec<AuthFlow> {
    let has_password = services
        .users
        .password_hash(user_id)
        .await
        .is_ok_and(|hash| !hash.is_empty());

    has_password
        .then(|| AuthFlow::new(vec![AuthType::Password]))
        .into_iter()
        .collect()
}
