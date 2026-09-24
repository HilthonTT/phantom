mod auto_accept;
mod ban;
mod invite;
mod join;
mod kick;
mod knock;
mod leave;
mod servers;
mod stripped_state;
mod unban;

use std::sync::Arc;

use async_trait::async_trait;
use loole::{Receiver, Sender, unbounded};
use phantom_core::{
    Result, implement, matrix::state_res::RoomVersion, server::Server, time::now_millis,
};
use phantom_database::Database;
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, OwnedServerName, RoomId, RoomVersionId, UserId,
    canonical_json::to_canonical_value,
    events::{AnyStrippedStateEvent, room::member::RoomMemberEventContent},
    serde::Raw,
};
use serde_json::value::{RawValue as RawJsonValue, to_raw_value};

use self::auto_accept::Pending;
pub use self::{
    join::Join,
    stripped_state::{
        StrippedCreateVerdict, dedup_stripped_state, enforce_stripped_create, into_client_stripped,
        v12_room_ids, without_member,
    },
};
use crate::{
    Dep, accounts::account_data, accounts::profile, accounts::users, net::federation, net::sending,
    net::server_keys, ops::server_state, rooms,
};

pub struct Service {
    services: Services,
    queue: (Sender<Pending>, Receiver<Pending>),
}

struct Services {
    server: Arc<Server>,
    db: Arc<Database>,
    account_data: Dep<account_data::Service>,
    event_handler: Dep<rooms::event_handler::Service>,
    federation: Dep<federation::Service>,
    metadata: Dep<rooms::metadata::Service>,
    outlier: Dep<rooms::outlier::Service>,
    profile: Dep<profile::Service>,
    sending: Dep<sending::Service>,
    server_keys: Dep<server_keys::Service>,
    server_state: Dep<server_state::Service>,
    short: Dep<rooms::short::Service>,
    state: Dep<rooms::state::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    state_compressor: Dep<rooms::state_compressor::Service>,
    timeline: Dep<rooms::timeline::Service>,
    users: Dep<users::Service>,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
                db: args.db.clone(),
                account_data: args.depend::<account_data::Service>("accounts::account_data"),
                event_handler: args.depend::<rooms::event_handler::Service>("rooms::event_handler"),
                federation: args.depend::<federation::Service>("net::federation"),
                metadata: args.depend::<rooms::metadata::Service>("rooms::metadata"),
                outlier: args.depend::<rooms::outlier::Service>("rooms::outlier"),
                profile: args.depend::<profile::Service>("accounts::profile"),
                sending: args.depend::<sending::Service>("net::sending"),
                server_keys: args.depend::<server_keys::Service>("net::server_keys"),
                server_state: args.depend::<server_state::Service>("ops::server_state"),
                short: args.depend::<rooms::short::Service>("rooms::short"),
                state: args.depend::<rooms::state::Service>("rooms::state"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                state_compressor: args
                    .depend::<rooms::state_compressor::Service>("rooms::state_compressor"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
                users: args.depend::<users::Service>("accounts::users"),
            },
            queue: unbounded(),
        }))
    }

    async fn worker(self: Arc<Self>) -> Result<()> {
        self.accept_worker().await;

        Ok(())
    }

    fn interrupt(&self) {
        let (sender, _) = &self.queue;

        if !sender.is_closed() {
            sender.close();
        }
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
fn supported_room_version(&self, room_version: &RoomVersionId) -> bool {
    RoomVersion::is_supported(room_version)
}

fn supported_room_versions() -> Vec<RoomVersionId> {
    RoomVersion::supported().collect()
}

#[implement(Service)]
fn is_local_only(&self, servers: &[OwnedServerName]) -> bool {
    servers.is_empty()
        || (servers.len() == 1 && self.services.server_state.server_is_ours(&servers[0]))
}

#[implement(Service)]
fn complete_member_event(
    &self,
    event: &mut CanonicalJsonObject,
    room_id: &RoomId,
    user_id: &UserId,
    content: CanonicalJsonValue,
) -> Result {
    let origin = self.services.server_state.server_name().as_str().to_owned();
    let origin_server_ts = i64::try_from(now_millis())?;

    event.insert("content".into(), content);
    event.insert("origin".into(), CanonicalJsonValue::String(origin));
    event.insert(
        "origin_server_ts".into(),
        CanonicalJsonValue::Integer(origin_server_ts.try_into()?),
    );
    event.insert(
        "room_id".into(),
        CanonicalJsonValue::String(room_id.as_str().into()),
    );
    event.insert(
        "sender".into(),
        CanonicalJsonValue::String(user_id.as_str().into()),
    );
    event.insert(
        "state_key".into(),
        CanonicalJsonValue::String(user_id.as_str().into()),
    );
    event.insert(
        "type".into(),
        CanonicalJsonValue::String("m.room.member".into()),
    );

    Ok(())
}

fn merge_member_content(
    content: RoomMemberEventContent,
    extra_content: Option<&CanonicalJsonObject>,
) -> Result<CanonicalJsonValue> {
    let mut content = to_canonical_value(content)?;

    if let (CanonicalJsonValue::Object(content), Some(extra_content)) =
        (&mut content, extra_content)
    {
        for (key, value) in extra_content {
            content.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }

    Ok(content)
}

fn outgoing_pdu(
    mut pdu_json: CanonicalJsonObject,
    room_version: &RoomVersionId,
) -> Box<RawJsonValue> {
    if let Some(unsigned) = pdu_json
        .get_mut("unsigned")
        .and_then(CanonicalJsonValue::as_object_mut)
    {
        unsigned.remove("transaction_id");
    }

    let keeps_event_id = room_version
        .rules()
        .is_some_and(|rules| rules.event_format.require_event_id);

    if !keeps_event_id {
        pdu_json.remove("event_id");
    }

    to_raw_value(&pdu_json).expect("CanonicalJson is valid serde_json::Value")
}

fn sender_servers(state: &[Raw<AnyStrippedStateEvent>]) -> impl Iterator<Item = OwnedServerName> {
    state
        .iter()
        .filter_map(|event| event.get_field::<String>("sender").ok().flatten())
        .filter_map(|sender| UserId::parse(sender).ok())
        .map(|user| user.server_name().to_owned())
}

#[cfg(test)]
mod tests {
    use ruma::{CanonicalJsonObject, RoomVersionId, events::AnyStrippedStateEvent, serde::Raw};
    use serde_json::json;

    use super::{outgoing_pdu, sender_servers, supported_room_versions};

    fn object(value: serde_json::Value) -> CanonicalJsonObject {
        serde_json::from_value(value).expect("canonical json object")
    }

    #[test]
    fn a_modern_room_sends_no_event_id() {
        let pdu = object(json!({"event_id": "$abc", "type": "m.room.member"}));
        let sent: serde_json::Value =
            serde_json::from_str(outgoing_pdu(pdu, &RoomVersionId::V11).get()).expect("json");

        assert!(sent.get("event_id").is_none());
    }

    #[test]
    fn a_v1_room_keeps_its_event_id() {
        let pdu = object(json!({"event_id": "$abc:example.org", "type": "m.room.member"}));
        let sent: serde_json::Value =
            serde_json::from_str(outgoing_pdu(pdu, &RoomVersionId::V1).get()).expect("json");

        assert_eq!(sent["event_id"], "$abc:example.org");
    }

    #[test]
    fn the_transaction_id_never_leaves() {
        let pdu = object(json!({
            "type": "m.room.member",
            "unsigned": {"transaction_id": "secret", "age": 1},
        }));
        let sent: serde_json::Value =
            serde_json::from_str(outgoing_pdu(pdu, &RoomVersionId::V11).get()).expect("json");

        assert!(sent["unsigned"].get("transaction_id").is_none());
        assert_eq!(sent["unsigned"]["age"], 1);
    }

    #[test]
    fn sender_servers_skip_what_does_not_parse() {
        let state: Vec<Raw<AnyStrippedStateEvent>> = [
            json!({"type": "m.room.name", "state_key": "", "sender": "@a:one.test", "content": {}}),
            json!({"type": "m.room.topic", "state_key": "", "sender": "garbage", "content": {}}),
            json!({"type": "m.room.create", "state_key": "", "content": {}}),
        ]
        .iter()
        .map(|event| Raw::new(event).expect("json").cast_unchecked())
        .collect();

        let servers: Vec<String> = sender_servers(&state)
            .map(|server| server.to_string())
            .collect();

        assert_eq!(servers, ["one.test"]);
    }

    #[test]
    fn the_advertised_versions_are_the_supported_ones() {
        let versions = supported_room_versions();

        assert!(versions.contains(&RoomVersionId::V11));
        assert!(!versions.contains(&RoomVersionId::V12));
    }
}
