use std::ops::Deref;

use axum::{body::Body, extract::FromRequest};
use axum_extra::extract::cookie::CookieJar;
use bytes::Bytes;
use http::Method;
use phantom_core::{Err, Error, Result, err};
use phantom_service::{Services, ops::appservice::RegistrationInfo};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, DeviceId, OwnedDeviceId, OwnedServerName, OwnedUserId,
    ServerName, UserId,
    api::{IncomingRequest, IncomingRequestExt},
};
use serde_json::Value as JsonValue;
use smallvec::SmallVec;

use super::{
    State,
    auth::{self, Auth, Authenticate},
    raw_request::RawRequest,
};

#[derive(Debug)]
pub struct Ruma<T, const ADMIN: bool = false> {
    pub body: T,
    pub cookies: CookieJar,
    pub origin: Option<OwnedServerName>,
    pub sender_user: Option<OwnedUserId>,
    pub sender_device: Option<OwnedDeviceId>,
    pub appservice_info: Option<RegistrationInfo>,
    pub json_body: Option<CanonicalJsonValue>,
}

pub type RumaAdmin<T> = Ruma<T, true>;

impl<T, const ADMIN: bool> Ruma<T, ADMIN> {
    #[inline]
    pub fn sender_user(&self) -> &UserId {
        self.sender_user
            .as_deref()
            .expect("user must be authenticated for this handler")
    }

    #[inline]
    pub fn origin(&self) -> &ServerName {
        self.origin
            .as_deref()
            .expect("server must be authenticated for this handler")
    }

    #[inline]
    pub fn sender_device(&self) -> Result<&DeviceId> {
        self.sender_device.as_deref().ok_or_else(|| {
            err!(Request(Forbidden(
                "user must be authenticated and device identified"
            )))
        })
    }
}

impl<T, const ADMIN: bool> Deref for Ruma<T, ADMIN> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.body
    }
}

impl<T, const ADMIN: bool> FromRequest<State> for Ruma<T, ADMIN>
where
    T: IncomingRequest + Send + Sync + 'static,
    T::Authentication: Authenticate,
{
    type Rejection = Error;

    #[tracing::instrument(name = "ruma", level = "debug", skip_all, err(level = "debug"))]
    async fn from_request(request: http::Request<Body>, state: &State) -> Result<Self> {
        let request = RawRequest::parse(state, request).await?;

        let json_body = match parse_json(&request) {
            Err(e) if !ADMIN => return Err(e),
            json_body => json_body,
        };

        let auth = auth::authenticate::<T::Authentication>(state, &request).await?;

        if ADMIN {
            require_admin(state, &auth).await?;
        }

        build(state, request, json_body?, auth)
    }
}

async fn require_admin(services: &Services, auth: &Auth) -> Result {
    let is_admin = match auth.sender_user.as_deref() {
        Some(sender_user) => services.users.is_admin(sender_user).await,
        None => false,
    };

    if !is_admin {
        return Err!(Request(Forbidden(
            "Only server administrators can use this endpoint"
        )));
    }

    Ok(())
}

fn parse_json(request: &RawRequest) -> Result<Option<CanonicalJsonValue>> {
    if let Ok(json) = serde_json::from_slice(&request.body) {
        return Ok(Some(json));
    }

    let method = &request.parts.method;
    let is_json_endpoint = matches!(
        *method,
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    ) && !request.parts.uri.path().contains("/media/");

    if !is_json_endpoint {
        return Ok(None);
    }

    let is_empty = request.body.iter().all(u8::is_ascii_whitespace);

    if !is_empty {
        serde_json::from_slice::<JsonValue>(&request.body)
            .map_err(|_| err!(Request(NotJson("Request body is not valid JSON."))))?;
    }

    let empty_object = is_empty && matches!(*method, Method::POST | Method::DELETE);

    Ok(empty_object.then(|| CanonicalJsonValue::Object(CanonicalJsonObject::new())))
}

fn build<T, const ADMIN: bool>(
    services: &Services,
    request: RawRequest,
    json_body: Option<CanonicalJsonValue>,
    auth: Auth,
) -> Result<Ruma<T, ADMIN>>
where
    T: IncomingRequest,
{
    let RawRequest {
        cookies,
        path,
        body,
        parts,
        ..
    } = request;

    let (json_body, body) = match json_body
        .as_ref()
        .and_then(|json| merge_uiaa_session(services, json, &auth))
    {
        Some(merged) => {
            let body = serialize(&merged);
            (Some(merged), body)
        }
        None => (json_body, body),
    };

    let path_args: SmallVec<[&str; 8]> = path.iter().map(AsRef::as_ref).collect();
    let http_request = http::Request::from_parts(parts, body.as_ref());
    let body = T::try_from_http_request(http_request, &path_args)
        .map_err(|e| err!(Request(BadJson(debug_warn!("{e}")))))?;

    let Auth {
        origin,
        sender_user,
        sender_device,
        appservice_info,
    } = auth;

    Ok(Ruma {
        body,
        cookies,
        origin,
        sender_user,
        sender_device,
        appservice_info,
        json_body,
    })
}

fn merge_uiaa_session(
    services: &Services,
    json_body: &CanonicalJsonValue,
    auth: &Auth,
) -> Option<CanonicalJsonValue> {
    let CanonicalJsonValue::Object(current) = json_body else {
        return None;
    };

    let session = current.get("auth")?.as_object()?.get("session")?.as_str()?;

    let user_id = match &auth.sender_user {
        Some(sender_user) => sender_user.clone(),
        None => UserId::parse_with_server_name("", services.server_state.server_name()).ok()?,
    };

    let CanonicalJsonValue::Object(saved) =
        services
            .uiaa
            .get_uiaa_request(&user_id, auth.sender_device.as_deref(), session)?
    else {
        return None;
    };

    let merged = saved
        .into_iter()
        .fold(current.clone(), |mut merged, (key, value)| {
            merged.entry(key).or_insert(value);
            merged
        });

    Some(CanonicalJsonValue::Object(merged))
}

fn serialize(json: &CanonicalJsonValue) -> Bytes {
    serde_json::to_vec(json)
        .expect("canonical JSON is always serializable")
        .into()
}
