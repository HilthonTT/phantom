use std::collections::BTreeMap;

use axum::extract::State;
use futures::{StreamExt, stream};
use phantom_core::{Error, Result, err};
use phantom_service::net::sending::EduBuf;
use ruma::{
    api::{
        client::to_device::send_event_to_device,
        error::ErrorKind,
        federation::transactions::edu::{DirectDeviceContent, Edu},
    },
    to_device::DeviceIdOrAllDevices,
};

use crate::router::Ruma;

/// # `PUT /_matrix/client/r0/sendToDevice/{eventType}/{txnId}`
///
/// Send a to-device event to a set of client devices.
pub(crate) async fn send_event_to_device_route(
    State(services): State<crate::router::State>,
    body: Ruma<send_event_to_device::v3::Request>,
) -> Result<send_event_to_device::v3::Response> {
    let sender_user = body.sender_user();
    let sender_device = body.sender_device.as_deref();

    // Check if this is a new transaction id
    if services
        .transaction_id
        .existing_txnid(sender_user, sender_device, &body.txn_id)
        .await
        .is_ok()
    {
        return Ok(send_event_to_device::v3::Response::new());
    }

    for (target_user_id, map) in &body.messages {
        for (target_device_id_maybe, event) in map {
            if !services.server_state.user_is_local(target_user_id) {
                let messages = BTreeMap::from([(
                    target_user_id.clone(),
                    BTreeMap::from([(target_device_id_maybe.clone(), event.clone())]),
                )]);

                let message_id = services.server_state.next_count()?.to_string();
                let mut content = DirectDeviceContent::new(
                    sender_user.to_owned(),
                    body.event_type.clone(),
                    message_id.into(),
                );
                content.messages = messages;

                let mut buf = EduBuf::new();
                serde_json::to_writer(&mut buf, &Edu::DirectToDevice(content)).map_err(|e| {
                    err!(Request(Unknown("Failed to serialize to-device EDU: {e}")))
                })?;

                services
                    .sending
                    .send_edu_servers(stream::once(async { target_user_id.server_name() }), buf)
                    .await?;

                continue;
            }

            let event_type = body.event_type.to_string();

            let event: serde_json::Value = event
                .deserialize_as()
                .map_err(|_| Error::BadRequest(ErrorKind::InvalidParam, "Event is invalid"))?;

            match target_device_id_maybe {
                DeviceIdOrAllDevices::DeviceId(target_device_id) => {
                    services
                        .users
                        .add_to_device_event(
                            sender_user,
                            target_user_id,
                            target_device_id,
                            &event_type,
                            event,
                        )
                        .await;
                }

                DeviceIdOrAllDevices::AllDevices => {
                    let device_ids: Vec<_> = services
                        .users
                        .all_device_ids(target_user_id)
                        .map(ToOwned::to_owned)
                        .collect()
                        .await;

                    for target_device_id in device_ids {
                        services
                            .users
                            .add_to_device_event(
                                sender_user,
                                target_user_id,
                                &target_device_id,
                                &event_type,
                                event.clone(),
                            )
                            .await;
                    }
                }
            }
        }
    }

    // Save transaction id with empty data
    services
        .transaction_id
        .add_txnid(sender_user, sender_device, &body.txn_id, &[]);

    Ok(send_event_to_device::v3::Response::new())
}
