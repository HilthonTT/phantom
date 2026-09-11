use std::sync::Arc;

use futures::{Stream, StreamExt};
use phantom_core::{
    Err, Result, err, implement,
    stream::{ReadyExt, TryIgnore},
};
use phantom_database::{Deserialized, Ignore, Interfix, Map, serialize_to_vec};
use ruma::{
    OwnedRoomId, OwnedServerName, OwnedUserId, RoomAliasId, RoomId, RoomOrAliasId, UserId,
    api::federation::query::get_room_information, events::StateEventType,
};

use crate::{
    Dep, admin, appservice, appservice::RegistrationInfo, federation, rooms, server_state,
};

pub struct Service {
    db: Data,
    services: Services,
}

struct Data {
    alias_userid: Arc<Map>,
    alias_roomid: Arc<Map>,
    aliasid_alias: Arc<Map>,
}

struct Services {
    admin: Dep<admin::Service>,
    appservice: Dep<appservice::Service>,
    federation: Dep<federation::Service>,
    server_state: Dep<server_state::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            db: Data {
                alias_userid: args.db["alias_userid"].clone(),
                alias_roomid: args.db["alias_roomid"].clone(),
                aliasid_alias: args.db["aliasid_alias"].clone(),
            },
            services: Services {
                admin: args.depend::<admin::Service>("admin"),
                appservice: args.depend::<appservice::Service>("appservice"),
                federation: args.depend::<federation::Service>("federation"),
                server_state: args.depend::<server_state::Service>("server_state"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
pub fn set_alias(&self, alias: &RoomAliasId, room_id: &RoomId) -> Result {
    let server_user = self.services.server_state.server_user.clone();

    self.set_alias_by(alias, room_id, &server_user)
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub fn set_alias_by(&self, alias: &RoomAliasId, room_id: &RoomId, user_id: &UserId) -> Result {
    self.check_alias_local(alias)?;

    if *alias == self.services.server_state.admin_alias
        && user_id != self.services.server_state.server_user
    {
        return Err!(Request(Forbidden(
            "Only the server user can set this alias"
        )));
    }

    let count = self.services.server_state.next_count()?;
    let localpart = alias.alias();

    self.db.alias_userid.insert(localpart, user_id)?;
    self.db.aliasid_alias.put_raw((room_id, count), alias)?;
    self.db.alias_roomid.insert(localpart, room_id)?;

    Ok(())
}

#[implement(Service)]
pub async fn remove_alias_by(&self, alias: &RoomAliasId, user_id: &UserId) -> Result {
    if !self.user_can_remove_alias(alias, user_id).await? {
        return Err!(Request(Forbidden(
            "User is not permitted to remove this alias."
        )));
    }

    self.remove_alias(alias).await
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn remove_alias(&self, alias: &RoomAliasId) -> Result {
    let localpart = alias.alias();
    let Ok(room_id) = self.db.alias_roomid.get(localpart).await else {
        return Err!(Request(NotFound("Alias does not exist or is invalid.")));
    };

    let prefix = (&room_id, Interfix);
    let prefix = serialize_to_vec(prefix).expect("failed to serialize prefix");
    self.db
        .aliasid_alias
        .raw_stream_prefix(&prefix)
        .ignore_err()
        .ready_for_each(|(key, alias_bytes)| {
            if alias_bytes == alias.as_str().as_bytes() {
                self.db.aliasid_alias.remove(key).ok();
            }
        })
        .await;

    self.db.alias_roomid.remove(localpart.as_bytes())?;
    self.db.alias_userid.remove(localpart.as_bytes())?;

    Ok(())
}

#[implement(Service)]
#[inline]
pub async fn maybe_resolve(&self, room: &RoomOrAliasId) -> Result<OwnedRoomId> {
    match <&RoomId>::try_from(room) {
        Ok(room_id) => Ok(room_id.to_owned()),
        Err(alias) => Ok(self.resolve_alias(alias).await?.0),
    }
}

#[implement(Service)]
pub async fn maybe_resolve_with_servers(
    &self,
    room: &RoomOrAliasId,
    servers: Option<&[OwnedServerName]>,
) -> Result<(OwnedRoomId, Vec<OwnedServerName>)> {
    match <&RoomId>::try_from(room) {
        Ok(room_id) => Ok((room_id.to_owned(), Vec::from(servers.unwrap_or_default()))),
        Err(alias) => self.resolve_alias(alias).await,
    }
}

#[implement(Service)]
#[tracing::instrument(skip(self), name = "resolve")]
pub async fn resolve_alias(
    &self,
    room_alias: &RoomAliasId,
) -> Result<(OwnedRoomId, Vec<OwnedServerName>)> {
    if !self.services.server_state.alias_is_local(room_alias) {
        return self.remote_resolve(room_alias).await;
    }

    if let Ok(room_id) = self.resolve_local_alias(room_alias).await {
        return Ok((room_id, Vec::new()));
    }

    if let Ok(room_id) = self.resolve_appservice_alias(room_alias).await {
        return Ok((room_id, Vec::new()));
    }

    Err!(Request(NotFound("Room with alias not found.")))
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "trace")]
pub async fn resolve_local_alias(&self, alias: &RoomAliasId) -> Result<OwnedRoomId> {
    self.check_alias_local(alias)?;

    self.db.alias_roomid.get(alias.alias()).await.deserialized()
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub fn local_aliases_for_room<'a>(
    &'a self,
    room_id: &'a RoomId,
) -> impl Stream<Item = &'a RoomAliasId> + Send + 'a {
    let prefix = (room_id, Interfix);

    self.db
        .aliasid_alias
        .stream_prefix(&prefix)
        .ignore_err()
        .map(|(_, alias): (Ignore, &str)| {
            <&RoomAliasId>::try_from(alias).expect("valid room alias in db")
        })
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub fn all_local_aliases(&self) -> impl Stream<Item = (&RoomId, &str)> + Send + '_ {
    self.db
        .alias_roomid
        .stream()
        .ignore_err()
        .map(|(localpart, room_id): (&str, &str)| {
            (
                <&RoomId>::try_from(room_id).expect("valid room id in db"),
                localpart,
            )
        })
}

#[implement(Service)]
pub async fn who_created_alias(&self, alias: &RoomAliasId) -> Result<OwnedUserId> {
    self.check_alias_local(alias)?;

    self.db.alias_userid.get(alias.alias()).await.deserialized()
}

#[implement(Service)]
fn check_alias_local(&self, alias: &RoomAliasId) -> Result {
    if !self.services.server_state.alias_is_local(alias) {
        return Err!(Request(InvalidParam("Alias is from another server.")));
    }

    Ok(())
}

#[implement(Service)]
#[tracing::instrument(skip(self, appservice_info), level = "trace")]
pub async fn appservice_checks(
    &self,
    room_alias: &RoomAliasId,
    appservice_info: &Option<RegistrationInfo>,
) -> Result {
    self.check_alias_local(room_alias)?;

    if let Some(info) = appservice_info {
        if !info.aliases.is_match(room_alias.as_str()) {
            return Err!(Request(Exclusive("Room alias is not in namespace.")));
        }
    } else if self
        .services
        .appservice
        .is_exclusive_alias(room_alias)
        .await
    {
        return Err!(Request(Exclusive("Room alias reserved by appservice.")));
    }

    Ok(())
}

#[implement(Service)]
async fn remote_resolve(
    &self,
    room_alias: &RoomAliasId,
) -> Result<(OwnedRoomId, Vec<OwnedServerName>)> {
    let server = room_alias.server_name();
    let request = get_room_information::v1::Request::new(room_alias.to_owned());

    let response = self.services.federation.execute(server, request).await?;

    Ok((response.room_id, response.servers))
}

#[implement(Service)]
async fn resolve_appservice_alias(&self, room_alias: &RoomAliasId) -> Result<OwnedRoomId> {
    use ruma::api::appservice::query::query_room_alias;

    self.check_alias_local(room_alias)?;

    let claimants: Vec<_> = self
        .services
        .appservice
        .read()
        .await
        .values()
        .filter(|appservice| appservice.aliases.is_match(room_alias.as_str()))
        .map(|appservice| appservice.registration.clone())
        .collect();

    for registration in claimants {
        let request = query_room_alias::v1::Request::new(room_alias.to_owned());

        if matches!(
            self.services
                .appservice
                .send_request(registration, request)
                .await,
            Ok(Some(_))
        ) {
            return self
                .resolve_local_alias(room_alias)
                .await
                .map_err(|_| err!(Request(NotFound("Room does not exist."))));
        }
    }

    Err!(Request(NotFound("Room does not exist.")))
}

#[implement(Service)]
async fn user_can_remove_alias(&self, alias: &RoomAliasId, user_id: &UserId) -> Result<bool> {
    self.check_alias_local(alias)?;

    let room_id = self
        .resolve_local_alias(alias)
        .await
        .map_err(|_| err!(Request(NotFound("Alias not found."))))?;

    if self
        .who_created_alias(alias)
        .await
        .is_ok_and(|creator| creator == user_id)
        || self.services.admin.user_is_admin(user_id).await
    {
        return Ok(true);
    }

    if let Ok(power_levels) = self
        .services
        .state_accessor
        .get_power_levels(&room_id)
        .await
    {
        return Ok(power_levels.user_can_send_state(user_id, StateEventType::RoomCanonicalAlias));
    }

    if let Ok(create) = self
        .services
        .state_accessor
        .room_state_get(&room_id, &StateEventType::RoomCreate, "")
        .await
    {
        return Ok(create.sender == user_id);
    }

    Err!(Database("Room has no m.room.create event"))
}
