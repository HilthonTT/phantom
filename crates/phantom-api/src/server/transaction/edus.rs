use std::collections::BTreeMap;

use futures::{StreamExt, stream::FuturesUnordered};
use phantom_core::{debug_warn, result::LogErr, time::now_millis, trace};
use phantom_service::Services;
use ruma::{
    OwnedRoomId, OwnedUserId, RoomId, ServerName, UserId,
    api::federation::transactions::edu::{
        DeviceListUpdateContent, DirectDeviceContent, Edu, PresenceContent, PresenceUpdate,
        ReceiptContent, ReceiptData, ReceiptMap, SigningKeyUpdateContent, TypingContent,
    },
    events::receipt::{ReceiptEvent, ReceiptEventContent, ReceiptType},
    serde::Raw,
    to_device::DeviceIdOrAllDevices,
};

pub(super) async fn handle(services: &Services, origin: &ServerName, edus: Vec<Edu>) {
    let mut handled = FuturesUnordered::new();
    for edu in edus {
        handled.push(handle_edu(services, origin, edu));
    }

    while handled.next().await.is_some() {}
}

async fn handle_edu(services: &Services, origin: &ServerName, edu: Edu) {
    let config = &services.server.config.presence;

    match edu {
        Edu::Presence(presence) if config.allow_incoming_presence => {
            presence_updates(services, origin, presence).await;
        }
        Edu::Receipt(receipt) if config.allow_incoming_read_receipts => {
            receipts(services, origin, receipt).await;
        }
        Edu::Typing(typing) if config.allow_incoming_typing => {
            typing_update(services, origin, typing).await;
        }
        Edu::DeviceListUpdate(content) => device_list_update(services, origin, content).await,
        Edu::DirectToDevice(content) => direct_to_device(services, origin, content).await,
        Edu::SigningKeyUpdate(content) => signing_key_update(services, origin, content).await,
        edu => trace!(?edu, "Skipped EDU"),
    }
}

fn from_origin(user_id: &UserId, origin: &ServerName, kind: &str) -> bool {
    let matches = user_id.server_name() == origin;
    if !matches {
        debug_warn!("Received {kind} EDU for {user_id}, who does not belong to {origin}");
    }

    matches
}

async fn presence_updates(services: &Services, origin: &ServerName, presence: PresenceContent) {
    for update in presence.push {
        presence_update(services, origin, update).await;
    }
}

async fn presence_update(services: &Services, origin: &ServerName, update: PresenceUpdate) {
    if !from_origin(&update.user_id, origin, "presence") {
        return;
    }

    services
        .presence
        .set_presence(
            &update.user_id,
            &update.presence,
            Some(update.currently_active),
            Some(update.last_active_ago),
            update.status_msg,
        )
        .await
        .log_err()
        .ok();
}

async fn receipts(services: &Services, origin: &ServerName, receipt: ReceiptContent) {
    for (room_id, updates) in receipt.receipts {
        room_receipts(services, origin, room_id, updates).await;
    }
}

async fn room_receipts(
    services: &Services,
    origin: &ServerName,
    room_id: OwnedRoomId,
    updates: ReceiptMap,
) {
    if services
        .rooms
        .event_handler
        .acl_check(origin, &room_id)
        .await
        .is_err()
    {
        debug_warn!("Received read receipt EDU for {room_id} from ACL'd server {origin}");
        return;
    }

    if !services
        .rooms
        .state_cache
        .server_in_room(origin, &room_id)
        .await
    {
        debug_warn!(
            "Received read receipt EDU for {room_id} from {origin}, which has no member in the room"
        );
        return;
    }

    for (user_id, data) in updates.read {
        if from_origin(&user_id, origin, "read receipt") {
            user_receipts(services, &room_id, user_id, data).await;
        }
    }
}

async fn user_receipts(
    services: &Services,
    room_id: &RoomId,
    user_id: OwnedUserId,
    data: ReceiptData,
) {
    for event_id in data.event_ids {
        let receipts = BTreeMap::from([(user_id.clone(), data.data.clone())]);
        let content = BTreeMap::from([(event_id, BTreeMap::from([(ReceiptType::Read, receipts)]))]);
        let event = ReceiptEvent::new(room_id.to_owned(), ReceiptEventContent(content));

        services
            .rooms
            .read_receipt
            .readreceipt_update(&user_id, room_id, &event)
            .await
            .log_err()
            .ok();
    }
}

async fn typing_update(services: &Services, origin: &ServerName, typing: TypingContent) {
    let TypingContent {
        room_id,
        user_id,
        typing,
        ..
    } = typing;

    if !from_origin(&user_id, origin, "typing") {
        return;
    }

    if services
        .rooms
        .event_handler
        .acl_check(origin, &room_id)
        .await
        .is_err()
    {
        debug_warn!("Received typing EDU for {room_id} from ACL'd server {origin}");
        return;
    }

    if !services
        .rooms
        .state_cache
        .is_joined(&user_id, &room_id)
        .await
    {
        debug_warn!("Received typing EDU for {user_id}, who is not in {room_id}");
        return;
    }

    let typing_service = &services.rooms.typing;
    let result = if typing {
        let timeout_secs = services.server.config.presence.typing_federation_timeout_s;
        let timeout = now_millis().saturating_add(timeout_secs.saturating_mul(1000));

        typing_service.typing_add(&user_id, &room_id, timeout).await
    } else {
        typing_service.typing_remove(&user_id, &room_id).await
    };

    result.log_err().ok();
}

async fn device_list_update(
    services: &Services,
    origin: &ServerName,
    content: DeviceListUpdateContent,
) {
    if from_origin(&content.user_id, origin, "device list update") {
        services
            .users
            .mark_device_key_update(&content.user_id)
            .await;
    }
}

async fn direct_to_device(services: &Services, origin: &ServerName, content: DirectDeviceContent) {
    let DirectDeviceContent {
        sender,
        ev_type,
        message_id,
        messages,
        ..
    } = content;

    if !from_origin(&sender, origin, "to-device") {
        return;
    }

    if services
        .transaction_id
        .existing_txnid(&sender, None, &message_id)
        .await
        .is_ok()
    {
        return;
    }

    let ev_type = ev_type.to_string();

    for (target_user, messages) in messages {
        if to_device_deliverable(services, &target_user).await {
            deliver_to_device(services, &sender, &target_user, &ev_type, messages).await;
        }
    }

    services
        .transaction_id
        .add_txnid(&sender, None, &message_id, &[]);
}

async fn to_device_deliverable(services: &Services, user_id: &UserId) -> bool {
    if !services.server_state.user_is_local(user_id) {
        return false;
    }

    if *user_id == services.server_state.server_user {
        return services.users.exists(user_id).await;
    }

    services.users.is_active(user_id).await
        || services.appservice.is_interested_in_user(user_id).await
}

async fn deliver_to_device<Event: Send + Sync>(
    services: &Services,
    sender: &UserId,
    target_user: &UserId,
    ev_type: &str,
    messages: BTreeMap<DeviceIdOrAllDevices, Raw<Event>>,
) {
    for (target, raw) in messages {
        let Ok(event) = raw.deserialize_as::<serde_json::Value>() else {
            debug_warn!("Dropping invalid to-device event from {sender} for {target_user}");
            continue;
        };

        match target {
            DeviceIdOrAllDevices::DeviceId(device_id) => {
                services
                    .users
                    .add_to_device_event(sender, target_user, &device_id, ev_type, event)
                    .await;
            }
            DeviceIdOrAllDevices::AllDevices => {
                let device_ids: Vec<_> = services
                    .users
                    .all_device_ids(target_user)
                    .map(ToOwned::to_owned)
                    .collect()
                    .await;

                for device_id in device_ids {
                    services
                        .users
                        .add_to_device_event(
                            sender,
                            target_user,
                            &device_id,
                            ev_type,
                            event.clone(),
                        )
                        .await;
                }
            }
        }
    }
}

async fn signing_key_update(
    services: &Services,
    origin: &ServerName,
    content: SigningKeyUpdateContent,
) {
    let SigningKeyUpdateContent {
        user_id,
        master_key,
        self_signing_key,
        ..
    } = content;

    if !from_origin(&user_id, origin, "signing key update") {
        return;
    }

    services
        .users
        .add_cross_signing_keys(&user_id, &master_key, &self_signing_key, &None, true)
        .await
        .log_err()
        .ok();
}
