use phantom_core::{Err, Result, err};
use phantom_service::{Services, ops::appservice::RegistrationInfo};
use ruma::{OwnedDeviceId, UserId};

use super::{Auth, RawRequest};

pub(super) async fn authenticate(
    services: &Services,
    request: &RawRequest,
    info: RegistrationInfo,
) -> Result<Auth> {
    let user_id = match request.query.user_id.as_deref() {
        Some(user_id) => UserId::parse(user_id),
        None => info.sender_user(services.server_state.server_name()),
    }
    .map_err(|_| err!(Request(InvalidUsername("Username is invalid."))))?;

    if !info.is_user_match(&user_id) {
        return Err!(Request(Exclusive("User is not in namespace.")));
    }

    let sender_device = request.query.device_id().map(OwnedDeviceId::from);

    if let Some(device_id) = sender_device.as_deref()
        && services
            .users
            .get_device_metadata(&user_id, device_id)
            .await
            .is_err()
    {
        return Err!(Request(InvalidParam("Unknown device for user.")));
    }

    Ok(Auth {
        sender_user: Some(user_id),
        sender_device,
        appservice_info: Some(info),
        ..Auth::default()
    })
}
