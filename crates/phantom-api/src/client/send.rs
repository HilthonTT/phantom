use std::collections::BTreeMap;

use axum::extract::State;
use futures::future::try_join4;
use phantom_core::{
    Err, Result, debug_warn, err,
    matrix::{PduCount, PduEvent, pdu::PduBuilder},
    text::string_from_bytes,
    warn,
};
use phantom_service::Services;
use ruma::{
    DeviceId, RoomId, TransactionId, UserId,
    api::Direction,
    api::client::message::{
        send_message_event, send_message_event::v3::Response as SendMessageResponse,
    },
    events::{
        AnyMessageLikeEventContent, MessageLikeEventType, TimelineEventType,
        reaction::ReactionEventContent,
        room::{encrypted::Relation, redaction::RoomRedactionEventContent},
    },
    serde::Raw,
};
use serde::Deserialize;
use serde_json::from_str;

use crate::router::Ruma;

#[derive(Deserialize)]
struct ExtractRelatesTo {
    #[serde(rename = "m.relates_to")]
    relates_to: Relation,
}

/// # `PUT /_matrix/client/v3/rooms/{roomId}/send/{eventType}/{txnId}`
///
/// Send a message event into the room.
///
/// - Is a NOOP if the txn id was already used before and returns the same event
///   id again
/// - The only requirement for the content is that it has to be valid json
/// - Tries to send the event into the room, auth rules will determine if it is
///   allowed
pub(crate) async fn send_message_event_route(
    State(services): State<crate::router::State>,
    body: Ruma<send_message_event::v3::Request>,
) -> Result<send_message_event::v3::Response> {
    let sender_user = body.sender_user();
    let sender_device = body.sender_device.as_deref();
    let appservice_info = body.appservice_info.as_ref();

    // Forbid m.room.encrypted if encryption is disabled
    if body.event_type == MessageLikeEventType::RoomEncrypted
        && !services.config.client.allow_encryption
    {
        return Err!(Request(Forbidden("Encryption has been disabled")));
    }

    // MSC4169: clients sending m.room.redaction via /send put `redacts` in
    // `content`. Pre-v11 auth rules read it from the top level; lift it so
    // `redacts_id(...)` resolves regardless of room version. Mirrors the
    // /redact handler.
    let redaction_content = || {
        body.body
            .body
            .deserialize_as_unchecked::<RoomRedactionEventContent>()
            .inspect_err(|_| {
                debug_warn!(
                    message = format_args!("Client sent invalid redaction event"),
                    %sender_user,
                    event = %body.body.body.json()
                );
            })
            .ok()
    };

    let redacts_id = body
        .event_type
        .eq(&MessageLikeEventType::RoomRedaction)
        .then(redaction_content)
        .flatten()
        .and_then(|content| content.redacts);

    if body.event_type == MessageLikeEventType::RoomRedaction
        && services.config.client.disable_local_redactions
        && !services.admin.user_is_admin(sender_user).await
    {
        warn!(
            message = format_args!("Local redactions are disabled, non-admin user attempted to redact an event"),
            %sender_user,
            ?redacts_id
        );

        return Err!(Request(Forbidden(
            "Redactions are disabled on this server."
        )));
    }

    let state_lock = services.rooms.state.mutex.lock(&*body.room_id).await;
    let event_type = body.event_type.to_string();

    let (existing_txnid, ..) = try_join4(
        check_existing_txnid(
            &services,
            sender_user,
            sender_device,
            &body.txn_id,
            &body.room_id,
            &event_type,
        ),
        check_duplicate_reaction(
            &services,
            &body.event_type,
            sender_user,
            &body.room_id,
            &body.body.body,
        ),
        check_public_call_invite(&services, &body.event_type, &body.room_id),
        check_nested_thread(&services, &body.body.body),
    )
    .await?;

    if let Some(existing_txnid) = existing_txnid {
        return Ok(existing_txnid);
    }

    let mut unsigned = BTreeMap::new();
    unsigned.insert("transaction_id".to_owned(), body.txn_id.to_string().into());

    let content = from_str(body.body.body.json().get())
        .map_err(|e| err!(Request(BadJson("Invalid JSON body: {e}"))))?;

    let event_id = services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder {
                event_type: body.event_type.clone().into(),
                content,
                unsigned: Some(unsigned),
                timestamp: appservice_info.and(body.timestamp),
                redacts: redacts_id,
                ..Default::default()
            },
            sender_user,
            &body.room_id,
            &state_lock,
        )
        .await?;

    services.transaction_id.add_txnid(
        sender_user,
        sender_device,
        &body.txn_id,
        event_id.as_bytes(),
    );

    drop(state_lock);

    Ok(send_message_event::v3::Response::new(event_id))
}

async fn check_public_call_invite(
    services: &Services,
    event_type: &MessageLikeEventType,
    room_id: &RoomId,
) -> Result {
    if *event_type != MessageLikeEventType::CallInvite {
        return Ok(());
    }

    if !services.rooms.directory.is_public_room(room_id).await {
        return Ok(());
    }

    Err!(Request(Forbidden(
        "Room call invites are not allowed in public rooms"
    )))
}

// Forbid duplicate reactions
async fn check_duplicate_reaction(
    services: &Services,
    event_type: &MessageLikeEventType,
    sender_user: &UserId,
    room_id: &RoomId,
    body: &Raw<AnyMessageLikeEventContent>,
) -> Result {
    if *event_type != MessageLikeEventType::Reaction {
        return Ok(());
    }

    let Ok(content) = body.deserialize_as_unchecked::<ReactionEventContent>() else {
        return Ok(());
    };

    let relations = services
        .rooms
        .pdu_metadata
        .get_relations(
            sender_user,
            room_id,
            &content.relates_to.event_id,
            PduCount::max(),
            usize::MAX,
            0,
            Direction::Backward,
        )
        .await;

    let duplicate = relations.iter().any(|(_, pdu)| {
        pdu.sender == sender_user
            && pdu.kind == TimelineEventType::Reaction
            && pdu
                .get_content::<ReactionEventContent>()
                .is_ok_and(|reaction| reaction.relates_to.key == content.relates_to.key)
    });

    if !duplicate {
        return Ok(());
    }

    Err!(Request(DuplicateAnnotation(
        "Duplicate reactions are not allowed."
    )))
}

// MSC3440/Matrix 1.4: a thread may only target an event which itself carries
// no rel_type; the spec assigns this rejection 400 M_UNKNOWN.
async fn check_nested_thread(
    services: &Services,
    body: &Raw<AnyMessageLikeEventContent>,
) -> Result {
    let Ok(ExtractRelatesTo {
        relates_to: Relation::Thread(thread),
    }) = body.deserialize_as_unchecked()
    else {
        return Ok(());
    };

    let Ok(root) = services.rooms.timeline.get_pdu(&thread.event_id).await else {
        return Ok(());
    };

    let nested = root
        .get_content()
        .is_ok_and(|content: ExtractRelatesTo| content.relates_to.rel_type().is_some());

    if !nested {
        return Ok(());
    }

    Err!(Request(Unknown(
        "Cannot start threads from an event with a relation."
    )))
}

/// Lifts a not-found error into `None`, keeping every other error.
fn optional<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.is_not_found() => Ok(None),
        Err(e) => Err(e),
    }
}

/// Check if this is a new transaction id. Returns Some when the transaction id
/// exists and the send must then be terminated by returning the contained
/// result.
///
/// Phantom keys transaction ids by user and device only, so a reused id is
/// honoured only when the stored event matches this room, type and sender.
async fn check_existing_txnid(
    services: &Services,
    sender_user: &UserId,
    sender_device: Option<&DeviceId>,
    txn_id: &TransactionId,
    room_id: &RoomId,
    event_type: &str,
) -> Result<Option<SendMessageResponse>> {
    let response = optional(
        services
            .transaction_id
            .existing_txnid(sender_user, sender_device, txn_id)
            .await,
    )?;

    let Some(response) = response else {
        return Ok(None);
    };

    let Some(response) = legacy_txnid_response(response.as_ref())? else {
        return Ok(None);
    };

    let pdu = optional(
        services
            .rooms
            .timeline
            .get_non_outlier_pdu(&response.event_id)
            .await,
    )?;

    let Some(pdu) = pdu else {
        return Ok(None);
    };

    if !legacy_txnid_matches(&pdu, room_id, event_type, sender_user) {
        return Ok(None);
    }

    Ok(Some(response))
}

fn txnid_response(response: &[u8]) -> Result<SendMessageResponse> {
    let event_id = string_from_bytes(response)?
        .try_into()
        .map_err(|_| err!(Database("Invalid event_id in txn_id data: {response:?}.")))?;

    Ok(SendMessageResponse::new(event_id))
}

fn legacy_txnid_response(response: &[u8]) -> Result<Option<SendMessageResponse>> {
    if response.is_empty() {
        return Ok(None);
    }

    txnid_response(response).map(Some)
}

fn legacy_txnid_matches(
    pdu: &PduEvent,
    room_id: &RoomId,
    event_type: &str,
    sender_user: &UserId,
) -> bool {
    pdu.room_id == room_id && pdu.kind.to_string() == event_type && pdu.sender == sender_user
}

#[cfg(test)]
mod tests {
    use ruma::{event_id, room_id, user_id};
    use serde_json::json;

    use super::{PduEvent, legacy_txnid_matches, legacy_txnid_response};

    #[test]
    fn legacy_response_requires_a_valid_nonempty_event_id() {
        assert!(
            legacy_txnid_response(b"")
                .expect("empty marker is valid")
                .is_none()
        );

        legacy_txnid_response(b"not an event ID").expect_err("invalid event ID");
        legacy_txnid_response(&[0xFF]).expect_err("invalid UTF-8");

        let response = legacy_txnid_response(b"$event:example.com")
            .expect("valid event ID")
            .expect("nonempty response");

        assert_eq!(response.event_id, event_id!("$event:example.com"));
    }

    #[test]
    fn legacy_response_requires_matching_provenance() {
        let pdu = pdu();
        assert!(legacy_txnid_matches(
            &pdu,
            room_id!("!room:example.com"),
            "m.room.message",
            user_id!("@alice:example.com"),
        ));

        assert!(!legacy_txnid_matches(
            &pdu,
            room_id!("!other:example.com"),
            "m.room.message",
            user_id!("@alice:example.com"),
        ));

        assert!(!legacy_txnid_matches(
            &pdu,
            room_id!("!room:example.com"),
            "m.room.encrypted",
            user_id!("@alice:example.com"),
        ));

        assert!(!legacy_txnid_matches(
            &pdu,
            room_id!("!room:example.com"),
            "m.room.message",
            user_id!("@bob:example.com"),
        ));
    }

    fn pdu() -> PduEvent {
        serde_json::from_value(json!({
            "type": "m.room.message",
            "content": {},
            "event_id": "$event:example.com",
            "room_id": "!room:example.com",
            "sender": "@alice:example.com",
            "prev_events": ["$prev:example.com"],
            "auth_events": ["$auth:example.com"],
            "origin_server_ts": 1,
            "depth": 1,
            "hashes": { "sha256": "thishashcoversallfieldsincasethisisredacted" },
        }))
        .expect("valid PDU")
    }
}
