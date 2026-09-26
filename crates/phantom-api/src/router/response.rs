use axum::response::{IntoResponse, Response};
use bytes::BytesMut;
use http::StatusCode;
use http_body_util::Full;
use phantom_core::{Error, error};
use ruma::api::{OutgoingResponse, OutgoingResponseExt, client::uiaa::UiaaResponse};

pub struct RumaResponse<T>(pub T);

impl From<Error> for RumaResponse<UiaaResponse> {
    fn from(error: Error) -> Self {
        Self(error.into())
    }
}

impl<T> IntoResponse for RumaResponse<T>
where
    T: OutgoingResponse,
{
    fn into_response(self) -> Response {
        match self.0.try_into_http_response::<BytesMut>() {
            Ok(response) => response
                .map(|body| Full::new(body.freeze()))
                .into_response(),
            Err(e) => {
                error!("response error: {e}");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}
