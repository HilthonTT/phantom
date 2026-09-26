use std::{
    mem::take,
    time::{Duration, SystemTime},
};

use axum::{Json, extract::State, response::IntoResponse};
use phantom_core::{Result, time::timepoint_from_now};
use ruma::{
    MilliSecondsSinceUnixEpoch,
    api::{
        OutgoingResponseExt,
        federation::discovery::{OldVerifyKey, ServerSigningKeys, get_server_keys},
    },
    serde::Raw,
};

pub(crate) async fn get_server_keys_route(
    State(services): State<crate::router::State>,
) -> Result<impl IntoResponse> {
    let server_name = services.server_state.server_name();
    let active_key_id = services.server_keys.active_key_id();
    let mut all_keys = services.server_keys.verify_keys_for(server_name).await;

    let verify_keys = all_keys
        .remove_entry(active_key_id)
        .expect("active verify_key is missing");

    let old_verify_keys = all_keys
        .into_iter()
        .map(|(id, key)| (id, OldVerifyKey::new(expires_ts(), key.key)))
        .collect();

    let mut server_key = ServerSigningKeys::new(server_name.to_owned(), valid_until_ts());
    server_key.verify_keys = [verify_keys].into();
    server_key.old_verify_keys = old_verify_keys;

    let server_key = Raw::new(&server_key)?;
    let mut response = get_server_keys::v2::Response::new(server_key)
        .try_into_http_response::<Vec<u8>>()
        .map(|mut response| take(response.body_mut()))
        .and_then(|body| serde_json::from_slice(&body).map_err(Into::into))?;

    services.server_keys.sign_json(&mut response)?;

    Ok(Json(response))
}

fn valid_until_ts() -> MilliSecondsSinceUnixEpoch {
    let dur = Duration::from_hours(168);
    let timepoint = timepoint_from_now(dur).expect("SystemTime should not overflow");
    MilliSecondsSinceUnixEpoch::from_system_time(timepoint).expect("UInt should not overflow")
}

fn expires_ts() -> MilliSecondsSinceUnixEpoch {
    let timepoint = SystemTime::now();
    MilliSecondsSinceUnixEpoch::from_system_time(timepoint).expect("UInt should not overflow")
}
