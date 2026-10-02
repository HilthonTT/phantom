use axum::{Json, extract::State, response::IntoResponse};
use phantom_core::Result;
use serde::Serialize;

#[derive(Serialize)]
struct AuthIssuerResponse {
    issuer: String,
}

pub(crate) async fn auth_issuer_route(
    State(services): State<crate::router::State>,
) -> Result<impl IntoResponse> {
    let issuer = services.oauth.get_server()?.issuer_url()?;

    Ok(Json(AuthIssuerResponse { issuer }))
}
