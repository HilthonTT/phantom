use super::{SendingFuture, SendingResult};
use crate::sending::{Destination, SendingEvent, Service};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures::{FutureExt, StreamExt};
use phantom_core::{
    Result, err,
    future::TryExt,
    hash::sha256,
    stream::{IterStream, WidebandExt},
    tracing, warn,
};
use ruma::{
    CanonicalJsonObject, MilliSecondsSinceUnixEpoch, OwnedServerName, OwnedTransactionId,
    OwnedUserId, RoomId, RoomVersionId, UInt,
    api::{
        appservice::event::push_events::v1::EphemeralData,
        federation::transactions::{edu::Edu, send_transaction_message},
    },
    events::{GlobalAccountDataEventType, push_rules::PushRulesEvent},
    push,
    serde::Raw,
};
use serde_json::value::{RawValue as RawJsonValue, to_raw_value};

impl Service {
    pub(super) fn send_events(
        &self,
        dest: Destination,
        events: Vec<SendingEvent>,
    ) -> SendingFuture<'_> {
        debug_assert!(!events.is_empty(), "sending empty transaction");
        match dest {
            Destination::Federation(server) => {
                self.send_events_dest_federation(server, events).boxed()
            }
            Destination::Appservice(id) => self.send_events_dest_appservice(id, events).boxed(),
            Destination::Push(user_id, pushkey) => {
                self.send_events_dest_push(user_id, pushkey, events).boxed()
            }
        }
    }

    #[tracing::instrument(
        name = "appservice",
        level = "debug",
        skip(self, events),
        fields(
            events = %events.len(),
        ),
    )]
    async fn send_events_dest_appservice(
        &self,
        id: String,
        events: Vec<SendingEvent>,
    ) -> SendingResult {
        let Some(appservice) = self.services.appservice.get_registration(&id).await else {
            return Err((
                Destination::Appservice(id.clone()),
                err!(Database(warn!(
                    message = format_args!("Missing appservice registration"),
                    ?id,
                ))),
            ));
        };

        let mut pdu_jsons = Vec::with_capacity(
            events
                .iter()
                .filter(|event| matches!(event, SendingEvent::Pdu(_)))
                .count(),
        );
        let mut edu_jsons: Vec<Raw<EphemeralData>> = Vec::with_capacity(
            events
                .iter()
                .filter(|event| matches!(event, SendingEvent::Edu(_)))
                .count(),
        );
        for event in &events {
            match event {
                SendingEvent::Pdu(pdu_id) => {
                    if let Ok(pdu) = self.services.timeline.get_pdu_from_id(pdu_id).await {
                        pdu_jsons.push(pdu.into_room_event());
                    }
                }
                SendingEvent::Edu(edu) => {
                    if appservice.receive_ephemeral
                        && let Ok(edu) = serde_json::from_slice::<EphemeralData>(edu)
                        && let Ok(raw) = Raw::new(&edu)
                    {
                        edu_jsons.push(raw);
                    }
                }
                SendingEvent::Flush => {}
            }
        }

        let txn_hash = sha256::delimited(events.iter().filter_map(|e| match e {
            SendingEvent::Edu(b) => Some(&**b),
            SendingEvent::Pdu(b) => Some(b.as_ref()),
            SendingEvent::Flush => None,
        }));

        let txn_id = OwnedTransactionId::from(URL_SAFE_NO_PAD.encode(txn_hash));

        let mut request =
            ruma::api::appservice::event::push_events::v1::Request::new(txn_id, pdu_jsons);
        request.ephemeral = edu_jsons;

        match self
            .services
            .appservice
            .send_request(appservice, request)
            .await
        {
            Ok(_) => Ok(Destination::Appservice(id)),
            Err(e) => Err((Destination::Appservice(id), e)),
        }
    }

    #[tracing::instrument(
        name = "push",
        level = "info",
        skip(self, events),
        fields(
            events = %events.len(),
        ),
    )]
    async fn send_events_dest_push(
        &self,
        user_id: OwnedUserId,
        pushkey: String,
        events: Vec<SendingEvent>,
    ) -> SendingResult {
        let Ok(pusher) = self.services.pusher.get_pusher(&user_id, &pushkey).await else {
            return Err((
                Destination::Push(user_id.clone(), pushkey.clone()),
                err!(Database(error!(
                    message = format_args!("Missing pusher"),
                    ?user_id,
                    ?pushkey,
                ))),
            ));
        };

        let mut pdus = Vec::with_capacity(
            events
                .iter()
                .filter(|event| matches!(event, SendingEvent::Pdu(_)))
                .count(),
        );
        for event in &events {
            match event {
                SendingEvent::Pdu(pdu_id) => {
                    if let Ok(pdu) = self.services.timeline.get_pdu_from_id(pdu_id).await {
                        pdus.push(pdu);
                    }
                }
                SendingEvent::Edu(_) | SendingEvent::Flush => {}
            }
        }

        for pdu in pdus {
            if pdu.contains_unsigned_property("redacted_because", serde_json::Value::is_string) {
                continue;
            }

            let rules_for_user = self
                .services
                .account_data
                .get_global(&user_id, GlobalAccountDataEventType::PushRules)
                .await
                .map_or_else(
                    |_| push::Ruleset::server_default(&user_id),
                    |ev: PushRulesEvent| ev.content.global,
                );

            let unread: UInt = self
                .services
                .user
                .notification_count(&user_id, &pdu.room_id)
                .await
                .try_into()
                .expect("notification count can't go that high");

            let _response = self
                .services
                .pusher
                .send_push_notice(&user_id, unread, &pusher, rules_for_user, &pdu)
                .await
                .map_err(|e| (Destination::Push(user_id.clone(), pushkey.clone()), e));
        }

        Ok(Destination::Push(user_id, pushkey))
    }

    async fn send_events_dest_federation(
        &self,
        server: OwnedServerName,
        events: Vec<SendingEvent>,
    ) -> SendingResult {
        let pdus: Vec<_> = events
            .iter()
            .filter_map(|pdu| match pdu {
                SendingEvent::Pdu(pdu) => Some(pdu),
                _ => None,
            })
            .stream()
            .wide_filter_map(|pdu_id| self.services.timeline.get_pdu_json_from_id(pdu_id).ok())
            .wide_then(|pdu| self.convert_to_outgoing_federation_event(pdu))
            .collect()
            .await;

        let edus: Vec<Raw<Edu>> = events
            .iter()
            .filter_map(|edu| match edu {
                SendingEvent::Edu(edu) => Some(edu.as_ref()),
                _ => None,
            })
            .map(serde_json::from_slice)
            .filter_map(Result::ok)
            .collect();

        if pdus.is_empty() && edus.is_empty() {
            return Ok(Destination::Federation(server));
        }

        let preimage = pdus
            .iter()
            .map(|raw| raw.get().as_bytes())
            .chain(edus.iter().map(|raw| raw.json().get().as_bytes()));

        let txn_hash = sha256::delimited(preimage);
        let txn_id = OwnedTransactionId::from(URL_SAFE_NO_PAD.encode(txn_hash));

        let mut request = send_transaction_message::v1::Request::new(
            txn_id.clone(),
            self.server.name.clone(),
            MilliSecondsSinceUnixEpoch::now(),
        );
        request.pdus = pdus;
        request.edus = edus;

        let result = self
            .services
            .federation
            .execute_with(&self.services.client.sender, &server, request)
            .await;

        for (event_id, result) in result.iter().flat_map(|resp| resp.pdus.iter()) {
            if let Err(e) = result {
                warn!(
                    %txn_id, %server,
                    "error sending PDU {event_id} to remote server: {e:?}"
                );
            }
        }

        match result {
            Err(error) => Err((Destination::Federation(server), error)),
            Ok(_) => Ok(Destination::Federation(server)),
        }
    }

    pub async fn convert_to_outgoing_federation_event(
        &self,
        mut pdu_json: CanonicalJsonObject,
    ) -> Box<RawJsonValue> {
        if let Some(unsigned) = pdu_json
            .get_mut("unsigned")
            .and_then(|val| val.as_object_mut())
        {
            unsigned.remove("transaction_id");
        }

        if let Some(room_id) = pdu_json
            .get("room_id")
            .and_then(|val| RoomId::parse(val.as_str()?).ok())
        {
            match self.services.state.get_room_version(&room_id).await {
                Ok(room_version_id) => match room_version_id {
                    RoomVersionId::V1 | RoomVersionId::V2 => {}
                    _ => _ = pdu_json.remove("event_id"),
                },
                Err(_) => _ = pdu_json.remove("event_id"),
            }
        } else {
            pdu_json.remove("event_id");
        }

        to_raw_value(&pdu_json).expect("CanonicalJson is valid serde_json::Value")
    }
}
