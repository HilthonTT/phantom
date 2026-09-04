//! Room aliases: the `#name:server` a room is reached by.
//!
//! A room's id is opaque and permanent; an alias is a name pointing at one,
//! owned by the server in its own server-name half. This service owns the
//! local ones — who created each, which room it points at, and the reverse
//! listing per room — and resolves remote ones over federation.
//!
//! Three columns, because two questions are asked in both directions and one
//! of them has to be answered without a scan:
//!
//! * `alias_roomid` — localpart to room, which is what resolving an alias
//!   reads.
//! * `alias_userid` — localpart to whoever set it, which is what decides
//!   whether someone else may take it down.
//! * `aliasid_alias` — `(room, count)` to the alias, so a room's aliases are a
//!   prefix scan rather than a walk of every alias on the server. The count is
//!   only there to keep two aliases of one room from colliding on the key.
//!
//! Resolution is not only the local column. An appservice may claim a
//! namespace of aliases and create the room behind one on demand, so an alias
//! inside such a namespace that is not in the column is put to the appservice
//! before it is called missing — see [`resolve_alias`].
//!
//! [`resolve_alias`]: Service::resolve_alias

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

/// Points `alias` at `room_id` as the server itself.
#[implement(Service)]
pub fn set_alias(&self, alias: &RoomAliasId, room_id: &RoomId) -> Result {
    let server_user = self.services.server_state.server_user.clone();

    self.set_alias_by(alias, room_id, &server_user)
}

/// Points `alias` at `room_id`, recording `user_id` as who set it.
///
/// The admin alias is the server's own name for its console, so only the
/// server user may move it; anyone else pointing it elsewhere would be
/// redirecting every admin command.
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

    // The room mapping is written last: an alias that resolves to nothing is a
    // name still free to be taken, while one that resolves with no recorded
    // creator is a name nobody can take down.
    self.db.alias_userid.insert(localpart, user_id)?;
    self.db.aliasid_alias.put_raw((room_id, count), alias)?;
    self.db.alias_roomid.insert(localpart, room_id)?;

    Ok(())
}

/// [`remove_alias`], first checking that `user_id` is allowed to.
///
/// [`remove_alias`]: Service::remove_alias
#[implement(Service)]
pub async fn remove_alias_by(&self, alias: &RoomAliasId, user_id: &UserId) -> Result {
    if !self.user_can_remove_alias(alias, user_id).await? {
        return Err!(Request(Forbidden(
            "User is not permitted to remove this alias."
        )));
    }

    self.remove_alias(alias).await
}

/// Takes `alias` down, freeing the name.
#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn remove_alias(&self, alias: &RoomAliasId) -> Result {
    let localpart = alias.alias();
    let Ok(room_id) = self.db.alias_roomid.get(localpart).await else {
        return Err!(Request(NotFound("Alias does not exist or is invalid.")));
    };

    // The reverse listing is keyed by room and count, so the entry for this
    // one alias cannot be addressed directly; the room's entries are scanned
    // and only the ones naming this alias dropped. The reference clears the
    // whole prefix, which takes a room's other aliases out of the listing as
    // well — they keep resolving, but stop being reported as the room's.
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

/// The room `room` names, which may already be a room id.
#[implement(Service)]
#[inline]
pub async fn maybe_resolve(&self, room: &RoomOrAliasId) -> Result<OwnedRoomId> {
    match <&RoomId>::try_from(room) {
        Ok(room_id) => Ok(room_id.to_owned()),
        Err(alias) => Ok(self.resolve_alias(alias).await?.0),
    }
}

/// [`maybe_resolve`], carrying the servers to try the room through.
///
/// A room id the caller already had comes back with the servers it was given,
/// since a bare id says nothing about who is in the room; an alias comes back
/// with whatever the server that owns it named.
///
/// [`maybe_resolve`]: Service::maybe_resolve
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

/// The room an alias names, and the servers to reach it through.
///
/// A local alias is looked up in the column and then, failing that, put to any
/// appservice whose namespace covers it: an appservice may create the room on
/// demand, and the alias only exists once it has. A remote alias is asked of
/// the server that owns it.
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

/// The room a local alias names, without consulting any appservice.
#[implement(Service)]
#[tracing::instrument(skip(self), level = "trace")]
pub async fn resolve_local_alias(&self, alias: &RoomAliasId) -> Result<OwnedRoomId> {
    self.check_alias_local(alias)?;

    self.db.alias_roomid.get(alias.alias()).await.deserialized()
}

/// Every local alias pointing at `room_id`.
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

/// Every local alias on the server, with the room it points at.
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

/// Whoever set a local alias, which is one of the things that lets them take
/// it down again.
#[implement(Service)]
pub async fn who_created_alias(&self, alias: &RoomAliasId) -> Result<OwnedUserId> {
    self.check_alias_local(alias)?;

    self.db.alias_userid.get(alias.alias()).await.deserialized()
}

/// Refuses an alias belonging to another server.
///
/// Every local operation here goes through this: the columns are keyed by
/// localpart alone, so a remote alias whose localpart happens to match a local
/// one would otherwise read and write the local room's entry.
#[implement(Service)]
fn check_alias_local(&self, alias: &RoomAliasId) -> Result {
    if !self.services.server_state.alias_is_local(alias) {
        return Err!(Request(InvalidParam("Alias is from another server.")));
    }

    Ok(())
}

/// Whether an appservice may claim `room_alias`.
///
/// `appservice_info` is the appservice making the request, where one is: it
/// may only act inside its own namespace. A request from an ordinary user
/// instead has to stay out of every appservice's exclusive namespace.
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

/// Asks the server that owns a remote alias what it points at.
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

/// Puts an unresolved local alias to the appservices that claim it.
///
/// An appservice answering the query is expected to have created the room and
/// set the alias as a side effect, so the local column is read again rather
/// than the response being believed: what the appservice says is only that it
/// is finished, and this server's own record is what a room id comes from.
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

/// Whether `user_id` may take `alias` down.
///
/// Whoever set it may, and so may a server admin. Failing both it is a
/// question about the room: the alias is the room's public name, so being able
/// to change `m.room.canonical_alias` is what stands for being able to take a
/// name away from it.
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

    // Without a power levels event the room's creator is the only one who
    // could have sent that state, so they are the only one who may do this.
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
