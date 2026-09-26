use axum::{RequestPartsExt, body::Body, extract::Path};
use axum_extra::extract::cookie::CookieJar;
use bytes::Bytes;
use http::request::Parts;
use phantom_core::{Result, err};
use phantom_service::Services;
use serde::Deserialize;
use smallstr::SmallString;
use smallvec::SmallVec;

type UserIdParam = SmallString<[u8; 48]>;
type DeviceIdParam = SmallString<[u8; 24]>;
type PathParam = SmallString<[u8; 32]>;
type PathParams = SmallVec<[PathParam; 8]>;

#[derive(Debug, Default, Deserialize)]
pub struct QueryParams {
    pub(super) access_token: Option<String>,

    pub(super) user_id: Option<UserIdParam>,

    device_id: Option<DeviceIdParam>,

    #[serde(rename = "org.matrix.msc3202.device_id")]
    msc3202_device_id: Option<DeviceIdParam>,
}

impl QueryParams {
    pub(super) fn device_id(&self) -> Option<&str> {
        self.device_id
            .as_deref()
            .or(self.msc3202_device_id.as_deref())
    }
}

#[derive(Debug)]
pub struct RawRequest {
    pub(super) cookies: CookieJar,
    pub(super) path: PathParams,
    pub(super) query: QueryParams,
    pub(super) body: Bytes,
    pub(super) parts: Parts,
}

impl RawRequest {
    #[tracing::instrument(name = "parse", level = "trace", skip_all, err(level = "debug"))]
    pub(super) async fn parse(services: &Services, request: http::Request<Body>) -> Result<Self> {
        let (mut parts, body) = request.into_parts();

        let cookies = CookieJar::from_headers(&parts.headers);
        let Path(path) = parts.extract::<Path<PathParams>>().await?;

        let query = serde_html_form::from_str(parts.uri.query().unwrap_or_default())
            .map_err(|e| err!(Request(Unknown("Failed to read query parameters: {e}"))))?;

        let max_request_size = services.server.config.network.max_request_size;
        let body = axum::body::to_bytes(body, max_request_size)
            .await
            .map_err(|e| err!(Request(TooLarge("Request body too large: {e}"))))?;

        Ok(Self {
            cookies,
            path,
            query,
            body,
            parts,
        })
    }
}
