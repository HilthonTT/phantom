use std::{collections::BTreeMap, sync::Arc};

use futures::StreamExt;
use phantom_core::{
    Err, Result, err, implement,
    matrix::pdu::{PduCount, PduEvent, PduId, RawPduId},
    result::LogErr,
    stream::ReadyExt,
};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, EventId, OwnedEventId, OwnedUserId, RoomVersionId,
    UserId,
    events::{
        GlobalAccountDataEventType, TimelineEventType,
        push_rules::PushRulesEvent,
        relation::{InReplyTo, RelationType},
        room::{
            member::{MembershipState, RoomMemberEventContent},
            redaction::RoomRedactionEventContent,
        },
    },
    push::{Action, Ruleset},
};
use serde::Deserialize;

use super::Service;
use crate::{
    appservice::{NamespaceRegex, RegistrationInfo},
    rooms::{short::ShortRoomId, state::RoomMutexGuard, state_compressor::CompressedState},
};

#[derive(Deserialize)]
struct ExtractBody {
    body: Option<String>,
}

#[derive(Deserialize)]
struct ExtractRelatesTo {
    #[serde(rename = "m.relates_to")]
    relates_to: RelatesTo,
}

#[derive(Deserialize)]
struct RelatesTo {
    rel_type: Option<RelationType>,
    event_id: Option<OwnedEventId>,

    #[serde(rename = "m.in_reply_to")]
    in_reply_to: Option<InReplyTo>,
}

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all)]
pub async fn append_pdu<'a, Leafs>(
    &'a self,
    pdu: &'a PduEvent,
    mut pdu_json: CanonicalJsonObject,
    leafs: Leafs,
    state_lock: &'a RoomMutexGuard,
) -> Result<RawPduId>
where
    Leafs: Iterator<Item = &'a EventId> + Send + 'a,
{
    let _cork = self.db.engine.cork_and_flush();

    let shortroomid = self
        .services
        .short
        .get_shortroomid(&pdu.room_id)
        .await
        .map_err(|_| err!(Database("Room does not exist")))?;

    if pdu.state_key.is_some() {
        self.add_prev_content(pdu, &mut pdu_json).await?;
    }

    self.services
        .pdu_metadata
        .mark_as_referenced(&pdu.room_id, pdu.prev_events.iter().map(AsRef::as_ref));

    self.services
        .state
        .set_forward_extremities(&pdu.room_id, leafs, state_lock)
        .await;

    let pdu_id = self.insert(pdu, &pdu_json, shortroomid).await?;

    self.notify_local_users(pdu, &pdu_id).await?;
    self.act_on_kind(pdu, &pdu_id, shortroomid).await?;
    self.record_relations(pdu, pdu_id.pdu_count()).await?;
    self.notify_appservices(pdu, &pdu_id).await?;

    Ok(pdu_id)
}

#[implement(Service)]
async fn insert(
    &self,
    pdu: &PduEvent,
    pdu_json: &CanonicalJsonObject,
    shortroomid: ShortRoomId,
) -> Result<RawPduId> {
    let insert_lock = self.mutex_insert.lock(&*pdu.room_id).await;

    let read_count = self.services.server_state.next_count()?;
    self.services
        .read_receipt
        .private_read_set(&pdu.room_id, &pdu.sender, read_count)?;

    self.services
        .user
        .reset_notification_counts(&pdu.sender, &pdu.room_id);

    let count = PduCount::Normal(self.services.server_state.next_count()?);
    let pdu_id: RawPduId = PduId {
        shortroomid,
        shorteventid: count,
    }
    .into();

    self.db.append_pdu(&pdu_id, pdu, pdu_json, count).await;

    drop(insert_lock);

    Ok(pdu_id)
}

#[implement(Service)]
async fn add_prev_content(&self, pdu: &PduEvent, pdu_json: &mut CanonicalJsonObject) -> Result {
    let Some(state_key) = &pdu.state_key else {
        return Ok(());
    };

    let CanonicalJsonValue::Object(unsigned) = pdu_json
        .entry("unsigned".to_owned())
        .or_insert_with(|| CanonicalJsonValue::Object(BTreeMap::default()))
    else {
        return Err!(Database("Invalid unsigned type in pdu."));
    };

    let Ok(shortstatehash) = self
        .services
        .state_accessor
        .pdu_shortstatehash(&pdu.event_id)
        .await
    else {
        return Ok(());
    };

    let Ok(prev_state) = self
        .services
        .state_accessor
        .state_get(shortstatehash, &pdu.kind.to_string().into(), state_key)
        .await
    else {
        return Ok(());
    };

    let content =
        phantom_core::json::to_canonical_object(prev_state.content.clone()).map_err(|e| {
            err!(Database(
                "Failed to convert prev_state to canonical JSON: {e}"
            ))
        })?;

    unsigned.insert(
        "prev_content".to_owned(),
        CanonicalJsonValue::Object(content),
    );
    unsigned.insert(
        "prev_sender".to_owned(),
        CanonicalJsonValue::String(prev_state.sender.to_string()),
    );
    unsigned.insert(
        "replaces_state".to_owned(),
        CanonicalJsonValue::String(prev_state.event_id.to_string()),
    );

    Ok(())
}

#[implement(Service)]
async fn notify_local_users(&self, pdu: &PduEvent, pdu_id: &RawPduId) -> Result {
    let power_levels = self
        .services
        .state_accessor
        .room_power_levels(&pdu.room_id)
        .await;

    let sync_pdu = pdu.to_sync_room_event();

    let mut targets = self.push_targets(pdu).await;

    if pdu.kind == TimelineEventType::RoomMember
        && let Some(state_key) = &pdu.state_key
    {
        let target = UserId::parse(state_key.as_str())?;

        if self.services.users.is_active_local(&target).await {
            targets.push(target);
        }
    }

    let mut notifies = Vec::with_capacity(targets.len());
    let mut highlights = Vec::with_capacity(targets.len());

    for user in &targets {
        let rules = self
            .services
            .account_data
            .get_global(user, GlobalAccountDataEventType::PushRules)
            .await
            .map_or_else(
                |_| Ruleset::server_default(user),
                |event: PushRulesEvent| event.content.global,
            );

        let actions = self
            .services
            .pusher
            .get_actions(user, &rules, power_levels.clone(), &sync_pdu, &pdu.room_id)
            .await;

        let notify = actions.iter().any(Action::should_notify);
        let highlight = actions.iter().any(Action::is_highlight);

        if notify {
            notifies.push(user.clone());
        }

        if highlight {
            highlights.push(user.clone());
        }

        self.services
            .pusher
            .get_pushkeys(user)
            .ready_for_each(|push_key| {
                self.services
                    .sending
                    .send_pdu_push(pdu_id, user, push_key.to_owned())
                    .log_err()
                    .ok();
            })
            .await;
    }

    self.db
        .increment_notification_counts(&pdu.room_id, notifies, highlights);

    Ok(())
}

#[implement(Service)]
async fn push_targets(&self, pdu: &PduEvent) -> Vec<OwnedUserId> {
    self.services
        .state_cache
        .active_local_users_in_room(&pdu.room_id)
        .map(ToOwned::to_owned)
        .ready_filter(|user| *user != pdu.sender)
        .filter_map(|recipient| async move {
            let ignored = self
                .services
                .users
                .user_is_ignored(&pdu.sender, &recipient)
                .await;

            (!ignored).then_some(recipient)
        })
        .collect()
        .await
}

#[implement(Service)]
async fn act_on_kind(&self, pdu: &PduEvent, pdu_id: &RawPduId, shortroomid: ShortRoomId) -> Result {
    match pdu.kind {
        TimelineEventType::RoomRedaction => self.act_on_redaction(pdu, shortroomid).await,
        TimelineEventType::SpaceChild => {
            self.services.spaces.forget(&pdu.room_id);

            Ok(())
        }
        TimelineEventType::RoomMember => self.act_on_membership(pdu).await,
        TimelineEventType::RoomMessage => self.act_on_message(pdu, pdu_id, shortroomid).await,
        _ => Ok(()),
    }
}

#[implement(Service)]
async fn act_on_redaction(&self, pdu: &PduEvent, shortroomid: ShortRoomId) -> Result {
    use RoomVersionId::*;

    let room_version = self.services.state.get_room_version(&pdu.room_id).await?;

    let redacts = match room_version {
        V1 | V2 | V3 | V4 | V5 | V6 | V7 | V8 | V9 | V10 => pdu.redacts.clone(),
        _ => pdu.get_content::<RoomRedactionEventContent>()?.redacts,
    };

    let Some(redacts) = redacts else {
        return Ok(());
    };

    let permitted = self
        .services
        .state_accessor
        .user_can_redact(&redacts, &pdu.sender, &pdu.room_id, false)
        .await?;

    if permitted {
        self.redact_pdu(&redacts, pdu, shortroomid).await?;
    }

    Ok(())
}

#[implement(Service)]
async fn act_on_membership(&self, pdu: &PduEvent) -> Result {
    let Some(state_key) = &pdu.state_key else {
        return Ok(());
    };

    let target = UserId::parse(state_key.as_str())?;
    let content: RoomMemberEventContent = pdu.get_content()?;

    let stripped = match content.membership {
        MembershipState::Invite | MembershipState::Knock => {
            Some(self.services.state.summary_stripped(pdu).await)
        }
        _ => None,
    };

    self.services
        .state_cache
        .update_membership(
            &pdu.room_id,
            &target,
            content,
            &pdu.sender,
            stripped,
            None,
            true,
        )
        .await
}

#[implement(Service)]
async fn act_on_message(
    &self,
    pdu: &PduEvent,
    pdu_id: &RawPduId,
    shortroomid: ShortRoomId,
) -> Result {
    let Ok(ExtractBody { body: Some(body) }) = pdu.get_content::<ExtractBody>() else {
        return Ok(());
    };

    self.services.search.index_pdu(shortroomid, pdu_id, &body)?;

    if self.services.admin.is_admin_command(pdu, &body).await {
        self.services
            .admin
            .command(body, Some(pdu.event_id.clone()))?;
    }

    Ok(())
}

#[implement(Service)]
async fn record_relations(&self, pdu: &PduEvent, count: PduCount) -> Result {
    let Ok(ExtractRelatesTo { relates_to }) = pdu.get_content::<ExtractRelatesTo>() else {
        return Ok(());
    };

    let related = relates_to
        .event_id
        .or_else(|| relates_to.in_reply_to.map(|reply| reply.event_id));

    let Some(related) = related else {
        return Ok(());
    };

    if relates_to.rel_type == Some(RelationType::Thread) {
        self.services.threads.add_to_thread(&related, pdu).await?;
    }

    if let Ok(related_count) = self.get_pdu_count(&related).await {
        self.services
            .pdu_metadata
            .add_relation(count, related_count);
    }

    Ok(())
}

#[implement(Service)]
async fn notify_appservices(&self, pdu: &PduEvent, pdu_id: &RawPduId) -> Result {
    for appservice in self.services.appservice.read().await.values() {
        if self.appservice_wants(pdu, appservice).await {
            self.services
                .sending
                .send_pdu_appservice(appservice.registration.id.clone(), *pdu_id)?;
        }
    }

    Ok(())
}

#[implement(Service)]
async fn appservice_wants(&self, pdu: &PduEvent, appservice: &RegistrationInfo) -> bool {
    if self
        .services
        .state_cache
        .appservice_in_room(&pdu.room_id, appservice)
        .await
    {
        return true;
    }

    let is_own_membership = pdu.kind == TimelineEventType::RoomMember
        && pdu
            .state_key
            .as_deref()
            .is_some_and(|state_key| state_key == appservice.registration.sender_localpart);

    if is_own_membership {
        return true;
    }

    if appservice.rooms.is_match(pdu.room_id.as_str()) || self.matches_users(pdu, appservice) {
        return true;
    }

    self.matches_aliases(pdu, &appservice.aliases).await
}

#[implement(Service)]
fn matches_users(&self, pdu: &PduEvent, appservice: &RegistrationInfo) -> bool {
    appservice.users.is_match(pdu.sender.as_str())
        || (pdu.kind == TimelineEventType::RoomMember
            && pdu
                .state_key
                .as_deref()
                .is_some_and(|state_key| appservice.users.is_match(state_key)))
}

#[implement(Service)]
async fn matches_aliases(&self, pdu: &PduEvent, aliases: &NamespaceRegex) -> bool {
    self.services
        .alias
        .local_aliases_for_room(&pdu.room_id)
        .ready_any(|alias| aliases.is_match(alias.as_str()))
        .await
}

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all)]
pub async fn append_incoming_pdu<'a, Leafs>(
    &'a self,
    pdu: &'a PduEvent,
    pdu_json: CanonicalJsonObject,
    new_room_leafs: Leafs,
    state_ids_compressed: Arc<CompressedState>,
    soft_fail: bool,
    state_lock: &'a RoomMutexGuard,
) -> Result<Option<RawPduId>>
where
    Leafs: Iterator<Item = &'a EventId> + Send + 'a,
{
    self.services
        .state
        .set_event_state(&pdu.event_id, &pdu.room_id, state_ids_compressed)
        .await?;

    if soft_fail {
        self.services
            .pdu_metadata
            .mark_as_referenced(&pdu.room_id, pdu.prev_events.iter().map(AsRef::as_ref));

        self.services
            .state
            .set_forward_extremities(&pdu.room_id, new_room_leafs, state_lock)
            .await;

        return Ok(None);
    }

    self.append_pdu(pdu, pdu_json, new_room_leafs, state_lock)
        .await
        .map(Some)
}

#[implement(Service)]
#[tracing::instrument(name = "redact", level = "debug", skip(self, reason))]
pub async fn redact_pdu(
    &self,
    event_id: &EventId,
    reason: &PduEvent,
    shortroomid: ShortRoomId,
) -> Result {
    let Ok(pdu_id) = self.get_pdu_id(event_id).await else {
        return Ok(());
    };

    let mut pdu = self.get_pdu_from_id(&pdu_id).await.map_err(|e| {
        err!(Database(error!(
            message = format_args!("PDU ID points to invalid PDU"),
            ?pdu_id,
            ?event_id,
            ?e,
        )))
    })?;

    if let Ok(ExtractBody { body: Some(body) }) = pdu.get_content::<ExtractBody>() {
        self.services
            .search
            .deindex_pdu(shortroomid, &pdu_id, &body)?;
    }

    let room_version = self.services.state.get_room_version(&pdu.room_id).await?;

    pdu.redact(&room_version, reason)?;

    let obj = phantom_core::json::to_canonical_object(&pdu)
        .map_err(|e| err!(Database("Failed to convert PDU to canonical JSON: {e}")))?;

    self.db.replace_pdu(&pdu_id, &obj, &pdu).await
}
