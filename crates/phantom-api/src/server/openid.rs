use axum::extract::State;
use phantom_core::Result;
use ruma::api::federation::openid::get_openid_userinfo;

use crate::router::Ruma;

pub(crate) async fn get_openid_userinfo_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_openid_userinfo::v1::Request>,
) -> Result<get_openid_userinfo::v1::Response> {
    Ok(get_openid_userinfo::v1::Response::new(
        services
            .users
            .find_from_openid_token(&body.access_token)
            .await?,
    ))
}
